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
| `--fix` | Applies the distribution's fixes to `package.yaml` (see [Render lint](render-lint.md#fixing-findings-fix)). Requires `--lint-dir`. |
| `--dry-run` | With `--fix`, shows what would be written without writing it. Requires `--fix`. |
| `--fix-rounds <n>` | With `--fix`, maximum number of runs of the suite (default `1`). Requires `--fix`. |
| `--verbose` | Prints the hints of the rules that still have findings, on stderr. Requires `--lint-dir`. |

The `tests/` directory must exist, otherwise the agent returns `MissingTestDirectory`.

### What a test can do

- provide instance **options** and a **context** (cluster/tenant);
- declare **mocks**: K8s objects present, HTTP responses, OCI manifests;
- execute a flow (`context → install`, `context → delete`, …);
- **assert** on created objects, status, or template rendering.

> The test framework is subject to ongoing improvements (assertion context variables,
> cluster/tenant overrides) — see the repository issues if an expected behavior is missing.

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
shape, suppressions, `schema_*` functions, `render_lint` library) is in
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
- **With `--fix`**: after the summary, each rule that produced findings is given to the `fix` function of the
  script, and `package.yaml` is rewritten if a fix applied. The key order is kept; the comments and the quoting
  of the file are not (a file where no fix applied is never rewritten). The report goes to stderr, so a `--format json`
  on stdout stays parsable. With the default of one run, the exit code is the one of the run *before* the
  fix: run the command again to check that the findings are gone.
- **With `--fix-rounds N`**: after a fix applied, the suite runs again on the rewritten `package.yaml`, up to
  `N` runs. The loop stops when no fix changed anything, or when a run does not have strictly fewer findings
  than the previous one (it then applies no fix: the fixes are not converging). Each run prints
  `round {k}: {n} finding(s)` on stderr. The summary, the JUnit file and the template dump are produced
  once, from the last run, and so is the exit code: a fix that cleared everything exits `0` with `N >= 2`.
- **With `--verbose`**: for each rule that still has findings, `hint {rule}:` followed by the text of the
  script's `hint` function, indented by two spaces, on stderr. A rule that a fix has just corrected has no
  hint. See [Hints](render-lint.md#hints-hint).

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
