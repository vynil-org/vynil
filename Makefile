VARIANT    ?=
CONTEXT    ?=
KUBECTL    := kubectl$(if $(CONTEXT), --context $(CONTEXT),)
TAG_REMOTE := $(shell git remote | grep -q '^upstream$$' && echo upstream || echo origin)
LATEST_TAG := $(shell git ls-remote --tags $(TAG_REMOTE) 2>/dev/null | grep -v '\^{}' | awk -F/ '{print $$3}' | sort -V | tail -1)
NEXT_PATCH := $(shell echo $(LATEST_TAG) | awk -F. '{print $$1"."$$2"."$$3+1}')
VERSION    := $(shell cargo run --bin agent -- version 2>/dev/null)
REGISTRY   := ghcr.io/vynil-org
TAG         = $(if $(VARIANT),$(NEXT_PATCH)-$(VARIANT),$(VERSION))
MATURITY    = $(if $(VARIANT),$(VARIANT),stable)

DOCKER_AUTH_FILE := /run/user/$(shell id -u)/containers/auth.json
DOCKER_USER       = $(shell jq -r '.auths["ghcr.io"].auth' <$(DOCKER_AUTH_FILE) | base64 -d | awk -F: '{print $$1}')
DOCKER_PASS       = $(shell jq -r '.auths["ghcr.io"].auth' <$(DOCKER_AUTH_FILE) | base64 -d | awk -F: '{print $$2}')

# build and push an image: $(call img-build-push, binary, tag)
define img-build-push
	podman build . -f $(1)/Dockerfile -t $(REGISTRY)/vynil-$(1):$(2)
	podman push $(REGISTRY)/vynil-$(1):$(2)
endef

# push only if image not already in registry: $(call ensure-image, binary)
define ensure-image
	podman manifest inspect $(REGISTRY)/vynil-$(1):$(TAG) >/dev/null 2>&1 \
		|| (podman build . -f $(1)/Dockerfile -t $(REGISTRY)/vynil-$(1):$(TAG) \
		    && podman push $(REGISTRY)/vynil-$(1):$(TAG))
endef

.PHONY: all help
.PHONY: generate-crd generate crd fmt precommit bump-version
.PHONY: agent-dev agent agent-alpha agent-beta
.PHONY: operator operator-alpha operator-beta
.PHONY: server server-alpha server-beta
.PHONY: box deploy

help:
	@echo "Usage: make <target> [VARIANT=alpha|beta] [CONTEXT=<kubecontext>]"
	@echo "       VARIANT default: (stable) — CONTEXT default: current"
	@echo ""
	@echo "Dev:"
	@echo "  generate-crd   Generate CRD yaml files"
	@echo "  generate       generate-crd + parent.toml files"
	@echo "  crd            generate-crd + kubectl apply"
	@echo "  fmt            cargo +nightly fmt"
	@echo "  precommit      update + clippy + generate + fmt + test"
	@echo ""
	@echo "Images  (stable: $(VERSION), alpha/beta: $(NEXT_PATCH), registry: $(REGISTRY)):"
	@echo "  agent-dev      Build and push agent $(VERSION)-dev"
	@echo "  agent          Build and push agent $(VERSION)"
	@echo "  agent-alpha    Build and push agent $(NEXT_PATCH)-alpha"
	@echo "  agent-beta     Build and push agent $(NEXT_PATCH)-beta"
	@echo "  operator       Build and push operator $(VERSION)"
	@echo "  operator-alpha Build and push operator $(NEXT_PATCH)-alpha"
	@echo "  operator-beta  Build and push operator $(NEXT_PATCH)-beta"
	@echo "  server         Build and push server $(VERSION)"
	@echo "  server-alpha   Build and push server $(NEXT_PATCH)-alpha"
	@echo "  server-beta    Build and push server $(NEXT_PATCH)-beta"
	@echo ""
	@echo "Box (checks images, builds if missing, then packages):"
	@echo "  box            make box [VARIANT=alpha|beta]"
	@echo ""
	@echo "Deploy (patches bootstrap.yaml on the fly):"
	@echo "  deploy         make deploy [VARIANT=alpha|beta] [CONTEXT=default]"

# ── Dev ──────────────────────────────────────────────────────────────────────

generate-crd:
	cargo run --bin agent -- crdgen > box/vynil/crds/crd.yaml
	cp box/vynil/crds/crd.yaml deploy/crd/crd.yaml

generate: generate-crd
	awk 'BEGIN{p=1}/profile.release/{p=0}p==1&&!/"operator",|"server",|"kubectl-vynil",/' <Cargo.toml >agent/parent.toml
	awk 'BEGIN{p=1}/profile.release/{p=0}p==1&&!/"agent",|"server",|"kubectl-vynil",/' <Cargo.toml >operator/parent.toml
	awk 'BEGIN{p=1}/profile.release/{p=0}p==1&&!/"agent",|"operator",|"kubectl-vynil",/' <Cargo.toml >server/parent.toml

crd: generate-crd
	$(KUBECTL) apply -f box/vynil/crds/crd.yaml

fmt:
	cargo +nightly fmt

precommit:
	cargo update
	cargo clippy --fix --allow-dirty --allow-staged
	$(MAKE) generate
	cargo +nightly fmt
	cargo test

bump-version:
	@current=$$(grep '^version = ' Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/'); \
	if [ "$$current" = "$(NEXT_PATCH)" ]; then \
		echo "Cargo.toml already at $(NEXT_PATCH), nothing to do."; \
	else \
		echo "Bumping Cargo.toml $$current → $(NEXT_PATCH)"; \
		sed -i 's/^version = "[0-9]*\.[0-9]*\.[0-9]*"/version = "$(NEXT_PATCH)"/' Cargo.toml; \
	fi

# ── Images ───────────────────────────────────────────────────────────────────

agent-dev: bump-version
	$(call img-build-push,agent,$(NEXT_PATCH)-dev)

agent:
	$(call img-build-push,agent,$(VERSION))

agent-alpha: bump-version
	$(call img-build-push,agent,$(NEXT_PATCH)-alpha)

agent-beta: bump-version
	$(call img-build-push,agent,$(NEXT_PATCH)-beta)

operator:
	$(call img-build-push,operator,$(VERSION))

operator-alpha: bump-version
	$(call img-build-push,operator,$(NEXT_PATCH)-alpha)

operator-beta: bump-version
	$(call img-build-push,operator,$(NEXT_PATCH)-beta)

server:
	$(call img-build-push,server,$(VERSION))

server-alpha: bump-version
	$(call img-build-push,server,$(NEXT_PATCH)-alpha)

server-beta: bump-version
	$(call img-build-push,server,$(NEXT_PATCH)-beta)

# ── Box ──────────────────────────────────────────────────────────────────────

BOX_TMP = /tmp/vynil-box

box:
	$(call img-build-push,agent,$(TAG))
	$(call img-build-push,operator,$(TAG))
	$(call img-build-push,server,$(TAG))
	rm -rf $(BOX_TMP) && cp -r box/vynil $(BOX_TMP)
	sed -i 's|app_version:.*|app_version: $(TAG)|' $(BOX_TMP)/package.yaml
	cargo run --bin agent -- package build -o $(BOX_TMP)/ \
		--tag $(TAG) -r ghcr.io -n vynil-org/vynil \
		-u $(DOCKER_USER) -p $(DOCKER_PASS)
	rm -rf $(BOX_TMP)

# ── Deploy ───────────────────────────────────────────────────────────────────

deploy: generate-crd
	$(KUBECTL) create ns vynil-system || true
	$(KUBECTL) apply -f deploy/crd/crd.yaml
	$(KUBECTL) delete job vynil-bootstrap -n vynil-system --ignore-not-found
	sed -E \
		-e 's|[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9.]+)?|$(TAG)|g' \
		-e 's|maturity: stable|maturity: $(MATURITY)|g' \
		deploy/bootstrap/bootstrap.yaml | $(KUBECTL) apply -f -
