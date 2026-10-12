# Architecture — Vynil

## Overview

Vynil is a package manager for Kubernetes. Its goal is to provide an integrated distribution
of Kubernetes, in the spirit of dpkg/rpm but for the cluster. Unlike Helm, ArgoCD, or Flux,
Vynil targets integration by default rather than maximum flexibility.

The project is a **Rust workspace** composed of three crates:

```
vynil/
├── common/     — shared library: CRDs, types, script engines
├── operator/   — Kubernetes controller (binary + lib)
└── agent/      — CLI tool (binary + lib)
```

---

## Components

### vynil-core (library)

The generic, vynil-agnostic toolbox, extracted from `common` so it can be reused outside vynil
(kuberest, kydah). Published as [`vynil-core`](https://crates.io/crates/vynil-core) on crates.io;
`common` consumes it as a regular crates.io dependency. Source: https://github.com/vynil-org/vynil-core.

- **Rhai engine**: `Script` (`new_bare`) + generic helpers (datetime, hashes, password, ed25519,
  semver, glob, shell, serde-YAML, base64/json, file I/O)
- **Handlebars engine**: `HandleBars` (generic helpers only) + `engine_mut()` extension hook
- **Handlers**: HTTP (always), and behind features: `k8s` (generic handlers + mocks), `oci`, `s3`
- **Own `Error`/`Result`/`RhaiRes`**; feature-gated variants for k8s/oci

### common (library)

Contains all vynil-specific types shared between the operator and the agent, built **on top of
`vynil-core`** (newtypes `Script`/`HandleBars` with `Deref`, re-exporting the generic modules):

- **Kubernetes CRDs**: definitions of the four custom resources
- **Rhai engine**: vynil layer over `vynil-core::Script` — registers `vynil_owner`, the package and
  instance/jukebox types, and the order-preserving `yaml_*_ordered` (`YamlDoc`)
- **Handlebars engine**: vynil layer over `vynil-core::HandleBars` — registers the context-aware
  helpers (`selector_from_ctx`, `labels_from_ctx`, `image_from_ctx`, …) and the rhai `new_hbs` binding
- **Handlers**: vynil package model, OCI/k8s mocks for instances and jukebox
- **Macros**: boilerplate code generation for status conditions

### operator (controller)

Binary `operator` — Actix HTTP server on port 9000 + four kube-rs controllers.

Responsibilities:
- Watch CRDs (`JukeBox`, `TenantInstance`, `ServiceInstance`, `SystemInstance`)
- Cache available packages (from `JukeBox` status)
- For each instance: select the right package, verify requirements, create the Job
- Expose Prometheus metrics (`GET /metrics`)

### agent (CLI)

Binary `agent` — command-line tool launched inside Kubernetes Jobs.

Main subcommands:
- `package {build,update,test,validate,unpack,lint}` — OCI package lifecycle management and static linting
- `{system,service,tenant} {install,delete,reconfigure,backup,restore}` — instance operations
- `crdgen` — CRD manifest generation
- `box`, `template`, `run` — utilities
- `box file-scan` — standalone scan to files (without K8s)

### kubectl-vynil (CLI plugin)

Binary `kubectl-vynil` — client-side `kubectl` plugin (`kubectl vynil …`). Drives the operator
from the user's kubeconfig context: trigger upgrades/scans, pull diagnostics, tail logs.
Ships a `kubectl_complete-vynil` shim for dynamic shell completion (see [Shell completion](#shell-completion) below).

---

## Shell completion

The `kubectl_complete-vynil` shim implements dynamic shell completion for `kubectl vynil …` via the
Cobra `__complete` protocol (kubectl 1.26+): kubectl invokes the shim, which delegates to the hidden
`kubectl-vynil __complete` verb. The `kubectl-vynil` binary also hosts a `completion <shell>`
subcommand for sourcing completion scripts when invoking the binary directly.

---

## Custom Kubernetes resources

All in the group `vynil.solidite.fr/v1`.

### JukeBox (cluster-scoped)

Vynil package source. Contains a source definition (OCI list, Harbor project, script,
HTTP URL, or S3 bucket) and a scan schedule (cron). The status stores the list of available
packages (upgrade waypoints).

```
spec:
  source:  List | Harbor | Script | Http | S3
  maturity: stable | beta | alpha
  pull_secret: <imagePull secret name>
  schedule: <cron expression>

status:
  packages: [VynilPackage]   ← cache of scanned packages
```

Source types:
- `List` / `Harbor` / `Script`: direct OCI registry scan
- `Http`: URL to a pre-computed package cache; Basic or Bearer auth via Secret
- `S3`: S3/MinIO/OVH bucket with optional prefix; credentials via Secret or IAM role

### TenantInstance / ServiceInstance (namespaced)

Installation of a package for a tenant or a service. Both types share the same structure,
with backup/restore.

```
spec:
  jukebox, category, package
  init_from:
    secret_name, sub_path, snapshot
    version: <exact version for restore>
  options: { key: value }

status:
  tag:    <currently installed version>
  digest: <options fingerprint>
  conditions: [Ready, Installed, BeforeApplied, VitalApplied, ...]
```

### SystemInstance (namespaced)

Installation of a system package (cluster-level). No backup/restore, no `initFrom`.

```
spec:
  jukebox, category, package
  options: { key: value }

status:
  tag, digest
  conditions: [Ready, Installed, SystemApplied, ...]
```

---

## Package format

A Vynil package is an **OCI image** with metadata annotations:

| Annotation | Content |
|---|---|
| `fr.solidite.vynil.metadata` | JSON: name, category, type (tenant/system/service), features |
| `fr.solidite.vynil.requirements` | JSON: requirements (CRDs, versions, resources) |
| `fr.solidite.vynil.options` | JSON: configurable parameters |
| `fr.solidite.vynil.recommandations` | JSON: recommendations (services, CRDs) |
| `fr.solidite.vynil.value_script` | Rhai script for dynamic values |

The image content contains the Rhai scripts for the package lifecycle.

### Package requirements (`VynilPackageRequirement`)

- `MinimumPreviousVersion` — minimum version already installed to be able to upgrade
- `VynilVersion` — minimum version of the Vynil framework
- `ClusterVersion` — minimum version of Kubernetes
- `CustomResourceDefinition`, `SystemService`, `TenantService` — dependencies on other packages
- `Cpu`, `Memory`, `Disk`, `StorageCapability` — required resources
- `Prefly` — custom Rhai verification script

---

## Reconciliation flow

### JukeBox scan

```
JukeBox CRD
    → (cron) Job agent "scan.rhai"
        → OCI sources (List/Harbor/Script): lists OCI tags
        → Http/S3 sources: downloads index.yaml + package files from cache
        → filter: valid semver + maturity + compatible Vynil version
        → partial filter if force-scan=<category>[/<name>] annotation is present
        → retains upgrade waypoints (1 version per "epoch" of MinimumPreviousVersion)
        → updates JukeBox.status.packages
```

Waypoints enable progressive upgrades without storing all versions.
Example: available versions [4.0(min:3.0), 3.5(min:2.0), 3.0(min:2.0), 2.5, 1.5]
→ stores [4.0, 3.5, 2.5, 1.5]

### Standalone scan (file-scan)

```
agent box file-scan
    → reads a local JukeBox YAML spec (source + pull_secret file)
    → scans OCI registries without a K8s connection
    → computes waypoints for 3 maturity levels and stores the union
    → produces <cache_dir>/index.yaml + <cache_dir>/<category>_<name>.yaml
    [upload to HTTP/S3]
JukeBox source Http/S3
    → downloads index.yaml + package files
    → applies maturity filter + recomputes waypoints
    → updates JukeBox.status.packages (identical to OCI sources)
```

### Instance reconciliation

```
Instance CRD (TenantInstance / ServiceInstance / SystemInstance)
    → operator: do_reconcile<T>()
        1. current_version = status.tag (empty on first install)
        2. package selection from the jukebox cache:
           - matching name + category + type
           - is_min_version_ok(current_version) — upgrade chain respected
           - is_vynil_version_ok() — framework compatible
        3. requirements check (CRDs, services, resources...)
        4. recommendations build (optional CRDs/services lists)
        5. Rhai value_script execution (if present)
        6. [if initFrom.version and first install] direct OCI tag verification
        7. Kubernetes Job rendering via Handlebars template (package.yaml.hbs)
        8. Job creation/update (Server-Side Apply)
        → requeue every 15 minutes
```

### Control annotations on instances

| Annotation | Value | Effect |
|---|---|---|
| `vynil.solidite.fr/suspend` | `"true"` | Suspends reconciliation until the annotation is removed. The controller requeues normally (15 min) but does nothing. |
| `vynil.solidite.fr/force-reinstall` | present | Forces reinstallation: deletes the existing Job before recreating it, then removes the annotation automatically. |

### Control annotations on JukeBox resources

| Annotation | Value | Behavior |
|---|---|---|
| `vynil.solidite.fr/force-scan` | `"true"` (or present without value) | Full scan of all packages |
| `vynil.solidite.fr/force-scan` | `"<category>"` | Partial scan of the entire category |
| `vynil.solidite.fr/force-scan` | `"<category>/<name>"` | Partial scan of a single package |

### Instance deletion (finalizer)

```
Instance CRD (deletion)
    → do_cleanup<T>()
        → same package selection
        → Job with action "delete"
        → wait for completion
        → finalizer removal
```

---

## Generic pattern: InstanceKind trait

The three instance types share a single reconciliation algorithm via the `InstanceKind` trait
in `operator/src/instance_common.rs`.

Key trait methods:

| Method | Role |
|---|---|
| `type_name()`, `package_type()` | Type constants |
| `spec_jukebox()`, `spec_category()`, `spec_package()` | Spec accessors |
| `current_tag()` | Installed version (from `status.tag`) |
| `init_from_version()` | Restore version (`initFrom.version`), default `None` |
| `check_requirements()` | Verifies package requirements |
| `build_recommendations()` | Builds recommendation lists |
| `set_rhai_instance()` | Injects the instance into the Rhai scope |
| `set_missing_box()`, `set_missing_package()`, ... | Error status updates |

---

## YAML strategy

| Usage | Library | Key order | Lives in |
|---|---|---|---|
| All Rust code (serialization/deserialization) | `serde_yaml` | Alphabetical | `vynil-core` (`src/yaml.rs`) |
| `yaml_decode_ordered` / `yaml_encode_ordered` (Rhai) | `rust-yaml` | Preserved | `common` (`yamlhandler.rs`) |

The serde-YAML helpers live in `vynil-core`; the order-preserving `YamlDoc` (rust-yaml) stays in
`common` and is registered by `yaml_ordered_rhai_register`. `rust-yaml` is used only in `update.rhai`
to avoid reordering the keys of `package.yaml`. The `YamlError(String)` variant exists in both
`vynil_core::Error` and `common::Error` (the latter wrapping the former via `Core(..)`).

---

## Rhai scripts

Rhai scripts are embedded in package OCI images and executed by the agent.
The generic engine is built by `vynil_core::Script::new_bare`; `common/src/rhaihandler.rs` wraps it
(newtype + `Deref`) and adds the vynil layer through the four constructors
(`new_core`/`new_file_scan`/`new`/`new_mock`).

Reusable script library (`agent/scripts/lib/`):
- `secret_dockerconfigjson.rhai` — imagePull secret reading
- `scan_harbor.rhai` — Harbor repository listing

Agent scripts (`agent/scripts/`):
- `boxes/scan.rhai` — jukebox scan
- `packages/{build,update,test,validate}.rhai` — package lifecycle
- `service/`, `tenant/`, `system/` — install, delete, backup, restore hooks

---

## Handlebars templates

Directory: `operator/templates/`

| Template | Usage |
|---|---|
| `package.yaml.hbs` | Instance install/delete Job |
| `cronscan.yaml.hbs` | JukeBox scan CronJob |
| `scan.yaml.hbs` | Manual JukeBox scan Job |

Variables systematically available in context: `tag`, `image`, `registry`,
`namespace`, `name`, `job_name`, `package_type`, `package_action`, `digest`, `ctrl_values`.

The generic helpers live in `vynil_core::HandleBars`; the context-aware helpers
(`selector_from_ctx`, `labels_from_ctx`, `ctx_have_crd`, `have_system_service`,
`have_tenant_service`, `image_from_ctx`, `resources_from_ctx`) and `render_template`/`render_file`
are registered by `common`. The rhai `new_hbs()` binding builds the **full** `common::HandleBars`
so that scripts calling `new_hbs().render_*(…, context)` (e.g. `install_crds`, `template_crds`,
`schedule_backup`) keep access to the context-aware helpers.

---

## Metrics

The operator exposes Prometheus metrics on `GET /metrics` (OpenMetrics format).

Four separate registries (one per resource type) expose:
- Reconciliation duration (histogram)
- Success/failure counters
- In-progress reconciliation gauge
- Last event timestamp

---

## Rhai regression tests

The test suite in `agent/tests/rhai_*.rs` runs the agent's internal Rhai scripts with a real
Rhai engine and K8s/HTTP mocks. It is structured in two levels:

- **`rhai_lib.rs`**: unit tests for `agent/scripts/lib/` scripts (isolated functions, assertions on returned values)
  - Covers patterns likely to regress during Rhai updates (`.filter()`, `.replace()`, `.reduce()`, closures)
  - Around 20 unit tests covering storage_class, wait, install_from_dir, gen_package, backup_context, resolv_service

- **`rhai_lifecycle.rs`**: integration tests for service/install and service/delete lifecycle scripts (full flow with K8s mocks)
  - Runs end-to-end flows: context → install / context → delete
  - Validates that all lib/ scripts assemble correctly

These tests serve as a regression safety net during Rhai version upgrades, capturing semantic
changes in string and collection manipulation functions.

---

## Environment variables (operator)

| Variable | Default | Role |
|---|---|---|
| `CONTROLLER_BASE_DIR` | `./operator` | Handlebars templates directory |
| `VYNIL_NAMESPACE` | `vynil-system` | Vynil system namespace |
| `AGENT_IMAGE` | `ghcr.io/vynil-org/vynil-agent:0.7.10` | Agent image for Jobs |
| `AGENT_ACCOUNT` | `vynil-agent` | Job ServiceAccount |
| `AGENT_LOG_LEVEL` | `info` | Log level |
| `TENANT_LABEL` | `vynil.solidite.fr/tenant` | Tenant label key |
| `SCAN_PACKAGE` | (absent) | Partial filter for `box scan` and `box file-scan` |

---

## Main dependencies

| Crate | Version | Role |
|---|---|---|
| `kube` | 3.1.0 | Kubernetes client + controllers |
| `k8s-openapi` | 0.27.1 | Kubernetes types |
| `rhai` | ~1.20 | Script engine |
| `handlebars` | ~6 | Template rendering |
| `oci-client` | ~0.12 | OCI registry |
| `serde_yaml` | 0.9 | YAML serialization (alphabetical sort) |
| `rust-yaml` | git `sebt3/rust-yaml` | Order-preserving YAML serialization |
| `tokio` | 1.48 | Async runtime |
| `actix-web` | 4.12 | Metrics HTTP server |
| `prometheus-client` | ~0.22 | Prometheus metrics |

> Workspace version at time of writing: **0.7.10** (Rust 2024 edition, BSD-3-Clause license).
