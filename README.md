
# Vynil
Vynil is an installer for kubernetes intended to be used either at home or for SaaS. The goal is to build a package manager akin to dpkg/rpm but for the kubernetes.

Unlike helm, kustomize, argoCD, Flux... which all give you all the flexibility to install as you please. Vynil main goal is to help create an integrated distribution for kubernetes, so customisation come scarse but integration of everything by default. Vynil differ also from openshift since olm can only install operators. Requiering an operator to manage an app while there is already a pseudo-generic installtion operator is madness. Olm should be able to install awx and phpmyadmin, but instead, you need a tower operator to install awx (as if the main use case is running many instances AWX instances). You even need an operator to install kube-virt while there can only be a single instance of kubevirt installed on a k8s cluster. Yet again, this design is madness... Redhat used to known how to install things properly /rant off


## Why Vynil?

**Stop configuring. Start distributing.**

Every generic chart answers every use case — so it answers none of yours. Vynil flips the
model: your distribution makes the integration decisions once, your users install in
minutes with options they actually understand.

- **Your platform, your rules** — packages bake in your resources, storage, network and
  security decisions.
- **Trust what you run** — packages are signed, immutable OCI images you can open and
  read, line by line. Audit teams love it; so do one-person ops teams.
- **Works where internet doesn't** — air-gapped by design: mirror, scan, install. Nothing
  phones home.
- **Your DR plan, executable** — registry + manifests = the whole platform, rebuildable on
  demand.
- **One engine, every scale** — homelab, enterprise platform, SaaS tenant orchestration,
  self-service PaaS.

Dive in: [Building a distribution](docs/distribution.md) ·
[The OCI package](docs/packages/portability.md)

## Installation
```
kubectl create ns vynil-system
kubectl apply -k github.com/vynil-org/vynil//deploy
```

## Shell completion

`kubectl-vynil` ships as a `kubectl` plugin and supports dynamic shell completion for
`kubectl vynil …` (resource kinds, live resource names, verbs and flags).

1. Put **both** `kubectl-vynil` and `kubectl_complete-vynil` somewhere on your `PATH`
   (they are in the same release archive):

   ```sh
   install -m 0755 kubectl-vynil kubectl_complete-vynil ~/.local/bin/
   ```

2. Make sure kubectl's own completion is loaded (bash example):

   ```sh
   source <(kubectl completion bash)
   ```

`kubectl vynil <TAB>` now completes dynamically.

For completion when calling the binary directly (`kubectl-vynil <TAB>`, outside kubectl):

```sh
source <(kubectl-vynil completion bash)   # or: zsh
```

See [kubectl-vynil documentation](docs/tooling/kubectl-vynil.md) for the full CLI syntax and available verbs.

## Documentation

Full documentation lives in [`docs/`](docs/index.md) (English, source of truth). A French
mirror is available under [`docs/fr/`](docs/fr/index.md). Both can be browsed as plain
Markdown on the forge, or built into a bilingual static site with `mkdocs serve` (see
[`mkdocs.yml`](mkdocs.yml)). A machine-readable index for LLMs is provided at
[`llms.txt`](llms.txt) following the [llmstxt.org](https://llmstxt.org) convention.

Start here:
- [Concepts](docs/concepts.md) — jukebox / package / instance model
- [Installation](docs/installation.md)
- [Architecture](docs/architecture.md) & [Reconciliation](docs/reconciliation.md)
- [Authoring packages](docs/packages/format.md)
- [Security & threat model](docs/operations/security.md)
- [Troubleshooting](docs/operations/troubleshooting.md)

## Package Tooling

### `agent package lint <package-dir>`

Analyzes a Vynil package statically without deployment. This command performs comprehensive linting of package structure, templates, and scripts.

**Options:**
- `-p / --package-dir` : Package directory (default: `/tmp/package`)
- `-c / --config-dir` : Directory for additional Rhai scripts (default: `.`)
- `--format` : Output format (default: `text`, options: `text`, `junit`)
- `--level` : Minimum severity level to display (default: `all`, options: `error`, `warn`, `all`)
- `--junit-output-filename` : Write JUnit XML report to file

**Checks Performed:**

**Package Structure (`package/`):**
- Validates `package.yaml` presence and format
- Checks required directory structure
- Verifies resource definitions

**Handlebars Templates (`hbs/`):**
- Syntax validation
- Unknown helper detection
- Unknown partial detection
- Context variable consistency with `package.yaml`
- Resource and image key validation
- Correct package type usage

**Rhai Scripts (`rhai/`):**
- Syntax validation
- Unresolved import detection
- Dead code detection
- Unused variable detection
- Shadowed variable detection
- Unused parameter detection
- Unused function detection
- API mode validation (no full-API in core scripts)
- Package type validation (no tenant access in system packages)
- Context hook return validation

**Configuration:**

Create a `.vynil-lint.yaml` file in the package root to customize linting behavior:

```yaml
disable:
  - rhai/unused-variable
  - hbs/unused-helper

override:
  rhai/unused-function: error
  hbs/unknown-helper: warn

files:
  - glob: "handlebars/helpers/**"
    disable:
      - rhai/unused-function
  - glob: "scripts/context_*.rhai"
    override:
      rhai/context-hook-no-return: error
```

**Inline Disable:**

Disable a specific rule on a single line:

- **Rhai:** `// vynil-lint-disable rhai/unused-variable`
- **Handlebars:** `{{!-- vynil-lint-disable hbs/unknown-helper --}}`

**Exit Codes:**
- `0` : No issues found
- `1` : Errors detected
- `2` : Only warnings detected
