# Installation

## Prerequisites

- A Kubernetes cluster and `kubectl` configured (cluster-admin rights for the initial
  installation — see the security note below).
- Network access to the OCI registry hosting the packages (default `ghcr.io/vynil-org/vynil`).

## Installing the operator

```bash
kubectl create ns vynil-system
kubectl apply -k github.com/vynil-org/vynil//deploy
```

The `deploy/` kustomize installs:

- the **CRDs** (`deploy/crd/`): `JukeBox`, `SystemInstance`, `ServiceInstance`,
  `TenantInstance`;
- the **bootstrap** (`deploy/bootstrap/`): a `vynil-bootstrap` ServiceAccount, a
  `JukeBox` named `vynil` pointing to `ghcr.io/vynil-org/vynil`, a `vynil`
  `SystemInstance`, and a bootstrap Job that scans the JukeBox then installs the
  `core/vynil` package (which in turn deploys the operator itself).

Vynil therefore installs itself *via Vynil*: the bootstrap lays down just enough
material for the operator to take over and manage itself like any other system package.

## Verify the installation

```bash
# the operator is running
kubectl -n vynil-system get pods

# the reference jukebox catalogue is populated
kubectl get jukebox vynil -o jsonpath='{.status.packages[*].metadata.name}'

# the vynil SystemInstance is Ready
kubectl -n vynil-system get systeminstances
```

## Add a package source

Create a `JukeBox` pointing to your registry:

```yaml
apiVersion: vynil.solidite.fr/v1
kind: JukeBox
metadata:
  name: home-alpha
spec:
  source:
    list:
    - "registry.example.com/my-org/vynil"
  maturity: stable
  schedule: "0 3 * * *"        # daily rescan at 3am
  # pull_secret: my-pull-secret # for private registries
```

Force an immediate scan without waiting for the cron:

```bash
kubectl annotate jukebox home-alpha vynil.solidite.fr/force-scan=true --overwrite
```

See [JukeBox sources](jukebox/sources.md) for Harbor, GitLab, HTTP, and S3 sources.

## Install a package

```yaml
apiVersion: vynil.solidite.fr/v1
kind: TenantInstance
metadata:
  name: my-ollama
  namespace: my-namespace
spec:
  jukebox: home-alpha
  category: think
  package: ollama
  options:
    use_rocm: true
```

Follow the progress:

```bash
kubectl -n my-namespace get tenantinstances
kubectl -n my-namespace describe tenantinstance my-ollama   # detailed conditions
kubectl -n vynil-system get jobs                            # agent install job
```

## Uninstall

```bash
kubectl -n my-namespace delete tenantinstance my-ollama
```

Deletion is managed by a finalizer: the operator launches a `delete` Job that cleans
up the children recorded in the `status`, then removes the finalizer. If an
uninstallation remains stuck, see [Troubleshooting](operations/troubleshooting.md).

## Important security note

By default, the Vynil agent runs with **cluster-admin** rights and executes the code
(Rhai) embedded in packages. **Only install packages from trusted JukeBox sources.**
Read [Security & threat model](operations/security.md) before any production
deployment.

## Operator environment variables

The main variables are described in the [Reference](operations/reference.md)
(`VYNIL_NAMESPACE`, `AGENT_IMAGE`, `AGENT_ACCOUNT`, `TENANT_LABEL`, etc.).
