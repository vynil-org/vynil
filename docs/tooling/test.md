# Package Tests

Vynil distinguishes two testing levels: **package tests** (`agent package test`,
written by the package author) and **internal regression tests** (agent Rust test suite,
ensuring Rhai engine stability).

## Package tests — `agent package test`

Runs scenarios defined in `<package-dir>/tests/` with a real Rhai engine and
**K8s/HTTP/OCI mocks**: no cluster required. Template rendering and lifecycle hooks are
executed, and assertions validate the result.

```bash
# all tests
agent package test -p ./my-package --all

# a specific test, with rendered output dump
agent package test -p ./my-package --test-name install-default \
  --template-output-filename rendered.yaml

# JUnit report for CI
agent package test -p ./my-package --all --format json \
  --junit-output-filename results.xml
```

| Option | Role |
|---|---|
| `--test-name <name>` | Runs a single test. |
| `--all` | Runs all tests. |
| `--testsets-dir <dir>` | Additional test set directory. |
| `--format text\|json` | Output format. |
| `--junit-output-filename <file>` | JUnit XML report. |
| `--template-output-filename <file>` | Rendered output dump (single test only). |
| `--lint-dir <dir>` | Distribution lint script directory (`lint.rhai`). Enables the render lint. |
| `--schemas-dir <dir>` | Distribution schema directory. Requires `--lint-dir`. |

The `tests/` directory must exist, otherwise the agent returns `MissingTestDirectory`.

### What a test can do

- provide instance **options** and a **context** (cluster/tenant);
- declare **mocks**: K8s objects present, HTTP responses, OCI manifests;
- execute a flow (`context → install`, `context → delete`, …);
- **assert** on created objects, status, or template rendering.

### Assertion variables

Assertion `selector` and `value` fields are Handlebars templates rendered after the
install, against the same `context` that `ctx::run()` handed to the package templates and
scripts. Mocks are rendered earlier, before any script runs, and only see `package` and
`instance`.

| Variable | Source |
|----------|--------|
| `values`, `defaults` | Options merged with the package defaults, raw defaults |
| `cluster` | `agent.yaml` and the mocked cluster (storage classes, nodes…) |
| `tenant` | Tenant name and namespaces (tenant packages) |
| `controller` | Output of the package `value_script` |
| `instance` | `name`, `namespace`, `appslug`, `options` from the test, plus the context instance keys (`labels`, `selector`…) |
| `package` | The package definition (`package.metadata.name`…) |
| `var` | Variables of the referencing testSet (testSet asserts only) |

A variable missing from the context fails the test with an `execution` row naming it.

### Best practices

- **Predictable tests**: do not assert on fields that depend on upstream versions
  (container image tags, `app_version`) — they would break on every
  `agent package update`. Assert instead on structure, names, and values derived from options.
- **One test set per posture**: a minimal `default.yaml`, then dedicated sets for
  significant variants (HA enabled, major option activated…), rather than a single test
  that mixes everything.
- **Use context placeholders** (`{{instance.appslug}}`,
  `{{instance.namespace}}`) in expected values, so tests remain valid regardless of the
  instance name.

## Render lint

When `--lint-dir` is given, the distribution's lint script runs at the end of each test, on the
objects that the test created. It is the place where a distribution enforces its own rules on a
package; Vynil itself ships no rule. The full contract of the script (entry points, context, finding
shape, suppressions, `schema_*` functions) is in
[Render lint](render-lint.md).

```bash
agent package test -p ./my-package --all \
  --lint-dir ./distribution/lint --schemas-dir ./distribution/schemas
```

- **When it runs**: after the install and the asserts of a test, and only if neither the
  `value_script` step nor the install step failed. The suite hook `lint_suite` runs once per
  invocation, in both `--all` and `--test-name` modes.
- **Effect on the verdict**: each `error` finding is a failed assert named `lint:{rule}`; the
  test fails with exit code 3, like any other failed assert. A `warn` finding is a passing row
  with a `warning: ` prefix and never fails the run. A script error is a failed `lint:execution`
  row, an invalid finding is a failed `lint:invalid-finding` row, and an unreadable instance
  status (every object then counts as unsectioned) is a failed `lint:sections` row.
- **Without `--lint-dir`**: nothing changes. No lint row and no suppression is produced.

### Suppressions

A finding marked as suppressed never fails a test. It is listed in an inventory, which is
rendered only when it is not empty:

- **text**: a `Suppressed: {n}` line after the `Results:` line, then one `[SKIP]` line per
  suppression, with its test, rule, object and reason;
- **json**: a `suppressions` array after `summary`, one object per suppression with `test`,
  `rule`, `kind`, `namespace`, `name`, `path`, `reason` and `origin` (absent fields are `null`);
- **JUnit**: a `<system-out>` element on the `<testsuite>` of the test concerned, one suppression
  per line.

## Internal regression tests (`agent/tests/rhai_*.rs`)

The Rust suite runs the agent's Rhai scripts (not a package's) with a real engine
and mocks. It serves as a safety net during Rhai version upgrades, capturing semantic
changes in string and collection manipulation functions (`.filter()`, `.replace()`,
`.reduce()`, closures).

- **unit level**: isolated functions from `agent/scripts/lib/` (storage_class, wait,
  install_from_dir, gen_package, backup_context, resolv_service…);
- **integration level**: end-to-end service/install and service/delete lifecycle flows
  with K8s mocks, validating that the `lib/` scripts assemble correctly.

```bash
# run the full suite
cargo test -p agent

# a test family
cargo test -p agent --test rhai_build
```

## In CI

Recommended sequence for a package:

```bash
agent package lint -p ./pkg --format junit --junit-output-filename lint.xml
agent package test -p ./pkg --all --format json --junit-output-filename test.xml
agent package build -p ./pkg --signing-key "$SIGNING_KEY"   # signed publication
```
