# Agent CLI Reference

The agent (`agent`) binary runs in Kubernetes Jobs, but is also used as a CLI tool
for developing and testing packages. Subcommands are defined in
[`agent/src/main.rs`](../../agent/src/main.rs).

```text
agent <COMMAND>
  package   package lifecycle management (build/lint/update/test/validate/unpack)
  system    operations on a SystemInstance (install/delete/…)
  service   operations on a ServiceInstance
  tenant    operations on a TenantInstance
  box       operations on a JukeBox (scan, file-scan)
  template  template rendering
  run       runs a git repository as a JukeBox source
  crdgen    generates CRD manifests
  version   prints the version
```

## `agent package`

| Subcommand | Role |
|---|---|
| `build` | Packages a directory as an OCI image and pushes it (option `--signing-key` to sign, see [Build & signing](build-signing.md)). |
| `unpack` | Downloads and extracts a package image into a directory. |
| `update` | Updates image tags in `package.yaml` by querying registries, then runs the `update_post` hook (template regeneration). |
| `lint` | Static analysis of the package (structure, Handlebars, Rhai). See [Lint](tooling/lint.md). |
| `test` | Runs package tests with K8s/HTTP mocks. See [Package tests](tooling/test.md). |
| `validate` | Validates `package.yaml` (schema, options). |

### `agent package lint`

```text
agent package lint -p <package-dir> [-c <config-dir>]
                   [--format text|junit] [--level error|warn|all]
                   [--junit-output-filename <file>]
```

### `agent package test`

```text
agent package test  -p <package-dir>
                   [--test-name <name> | --all]
                   [--testsets-dir <dir>]
                   [--format text|json]
                   [--junit-output-filename <file>]
                   [--template-output-filename <file>]   # for a single test only => yamllint, kubelinter, ... usage
                   [--lint-dir <dir> [--schemas-dir <dir>]]
```

The `<package-dir>/tests` directory must exist (otherwise error `MissingTestDirectory`).

| Flag | Environment variable | Role |
|---|---|---|
| `--lint-dir <dir>` | `LINT_DIRECTORY` | Directory of the distribution's render lint script (`lint.rhai`). When given, the script runs at the end of each test and its findings are added to the verdict. Without it, no render lint runs. |
| `--schemas-dir <dir>` | `SCHEMAS_DIRECTORY` | Directory of JSON Schemas and CRDs used by the lint script's `schema_*` functions. Requires `--lint-dir` (usage error otherwise): `SCHEMAS_DIRECTORY` alone in the environment is that same usage error. |

See [Package tests](tooling/test.md#render-lint) and [Render lint](tooling/render-lint.md).

## `agent box`

| Subcommand | Role |
|---|---|
| `scan` | Scans a JukeBox (used by the operator's CronJob). |
| `file-scan` | Standalone scan to files (`index.yaml` + package files), without a Kubernetes connection. See [Reconciliation](reconciliation.md#scan-standalone-box-file-scan). |

## `agent {system,service,tenant}`

Instance operations executed in Jobs: `install`, `delete`, `reconfigure`, and —
for service/tenant — `backup`, `restore`. Common parameters (via flags or env vars):

| Flag | Env | Default | Role |
|---|---|---|---|
| `-n`, `--namespace` | `NAMESPACE` | — | Instance namespace. |
| `-i`, `--instance` | `INSTANCE` | — | Instance name. |
| `-v`, `--vynil-namespace` | `VYNIL_NAMESPACE` | — | Vynil system namespace. |
| `-p`, `--package-dir` | `PACKAGE_DIRECTORY` | `/tmp/package` | Unpacked package directory. |
| `-s`, `--script-dir` | `SCRIPT_DIRECTORY` | `./agent/scripts` | Agent scripts. |
| `-t`, `--template-dir` | `TEMPLATE_DIRECTORY` | `./agent/templates` | Agent templates. |
| `-c`, `--config-dir` | `CONFIG_DIR` | `.` | Additional Rhai scripts. |
| `--controller-values` | `CONTROLLER_VALUES` | `{}` | Values computed by the operator. |
| `--agent-image` | `AGENT_IMAGE` | (compiled default) | Agent image. |

## `agent crdgen`

Generates CRD manifests from Rust types. Used to regenerate
[`deploy/crd/crd.yaml`](../../deploy/crd/crd.yaml).

## Exit codes

- `0`: success
- `1`: execution error (or lint with errors)
- `2`: lint with warnings only / CRD generation failure
- `3`: `agent package test` failed (an assertion failed, or the render lint reported a finding of severity `error`)
