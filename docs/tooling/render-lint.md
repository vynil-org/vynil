# Render lint

The render lint runs at the end of each `agent package test` run. It hands the objects that the
test produced to a lint script owned by the **distribution**, and folds the findings of that script
into the ordinary test verdict.

Vynil provides the engine and the data types. It ships no rule: the script decides what is checked,
with which severity, and what is excluded. Without a lint script, the render lint does not run and
`agent package test` behaves exactly as before.

## Activation

The render lint is enabled by `--lint-dir` on `agent package test` (see [Package tests](test.md#render-lint)
and [CLI reference](../cli.md#agent-package-test)):

```bash
agent package test -p ./my-package --all \
  --lint-dir ./distribution/lint \
  --schemas-dir ./distribution/schemas
```

| Flag | Environment variable | Role |
|---|---|---|
| `--lint-dir <dir>` | `LINT_DIRECTORY` | Directory containing `lint.rhai`, the entry point of the distribution script. |
| `--schemas-dir <dir>` | `SCHEMAS_DIRECTORY` | Directory of JSON Schemas and CRDs used by the `schema_*` functions. Requires `--lint-dir`. |

Both variables follow the other directory variables of the agent (`PACKAGE_DIRECTORY`). `SCHEMAS_DIRECTORY`
alone, for instance exported by a CI job that sets no `--lint-dir`, is a clap usage error because of the
`requires`: set `LINT_DIRECTORY` with it.

## The lint directory

`lint.rhai` is the fixed entry point. The modules it imports are resolved in this order:

1. the lint directory itself;
2. `{script-dir}/lib`, where `script-dir` is the `--script-dir` flag (default `./agent/scripts`).

The lint directory comes first: a module of the same name placed there replaces the standard module
of the agent, the way a package can override a file provided by the agent. A distribution that shadows
a standard module by accident therefore changes what its own script imports.

A missing `lint.rhai`, a script that does not compile, or a script that defines neither `lint` nor
`lint_suite` stops the command with exit code 3 before any output.

## Entry points

The script may define one or both of these functions. A function that is not defined is skipped.

| Function | Called | Arguments | Returns |
|---|---|---|---|
| `lint(objs, ctx)` | Once per test, after its install and asserts | `objs`: map of copies of the objects created by the test, by section (see below); `ctx`: test context | array of findings |
| `lint_suite(ctx)` | Once per invocation, after the suite | `ctx`: suite context | array of findings |

`lint` is not called for a test whose `value_script` or install step failed: the failure is already
reported on the test, and a partial render would produce misleading findings.

### The `objs` map

`objs` is a map with a fixed set of keys. Every key is always present; a key with no object holds an
empty array, so a script never has to test for a missing key.

| Key | Holds |
|---|---|
| `all` | Every object created by the test, in creation order |
| `crds`, `systems`, `befores`, `vitals`, `scalables`, `others`, `posts` | The objects the package declared in that section |
| `unsectioned` | The objects that no section declares, in the order of `all` |

A section is what the package itself declared while installing, by calling
`instance.set_status_<section>` (for example `set_status_scalables`). The declaration is read back from
the `status` of the instance after the install. The lint never guesses a section from the object's `kind`:
a `Deployment` that the package did not declare as scalable is in `unsectioned`, not in `scalables`.

An entry designates an object when `kind`, `name` and `namespace` match (a missing `namespace` must be
missing on both sides), and `apiVersion` when the entry carries one. The `crds` section lists CRD names
only. An object designated by several sections appears in each of them. If the status cannot be read, every
section is empty, every object is in `unsectioned`, and a failed row `lint:sections` is added: a rule
written as "for each object of `objs.scalables`" would otherwise pass without having checked anything. A
status that is read successfully but declares no section is legitimate and adds no row.

Each call runs in a fresh Rhai engine, with no state shared between tests. The script receives copies:
it cannot modify the objects of the test. The available functions are the data-manipulation functions
of the base engine and the `schema_*` functions. No Kubernetes, HTTP, S3, shell or file function is
registered: a script that calls one fails with an unknown-function error.

Execution is bounded so that a looping script cannot block the run: by an operation budget, which is a
base plus a share per object the script receives, and by a size limit on strings, arrays and maps (a
script that doubles a string in a loop fails instead of exhausting the memory). The values of the
budget are not fixed by the contract yet.

### Errors of the script

Any of the following becomes one failed row named `lint:execution`, carrying the error as its message,
and produces no suppression for that call:

- a Rhai runtime error, including a call to an unknown function;
- the operation limit being reached;
- a return value that is not an array (for example a map).

Scripts are deterministic: the same objects and the same contexts give the same rows.

## Test context: `ctx` in `lint`

`ctx` is read-only and contains only serialisable data. It is built from the resolved test.

```text
ctx
├── package
│   ├── name
│   ├── type            tenant | service | system
│   ├── category
│   ├── description
│   ├── app_version     absent when not set
│   ├── features
│   ├── options         options declared in package.yaml
│   ├── images          as in package.yaml
│   └── resources       as in package.yaml
├── instance
│   ├── name
│   ├── namespace
│   ├── appslug         appslug of the package and the instance
│   └── options         options given by the test, not resolved
└── test
    ├── name
    ├── description     absent when not set
    ├── testsets        [{name, variables}] for each referenced testSet
    └── definition      resolved Test: apiVersion, kind, metadata, instance, mocks, asserts
```

`test.definition.asserts` contains the asserts of the test followed by those of its testSets.

`instance.options` does not contain the default values of the package options: a rule that needs
them combines `package.options` and `instance.options`.

## Suite context: `ctx` in `lint_suite`

```text
ctx
├── package             same shape as in the test context
└── suite
    ├── tests           map: test name -> loaded Test document, not rendered
    ├── testsets        map: testSet name -> loaded TestSet document
    └── executed        names of the tests run by this invocation, ascending
```

`lint_suite` also runs with `--test-name`: `executed` then holds that single test. A rule that requires
something of *every* test must compare `executed` with `tests`, not assume that the whole suite ran.

## Shape of a finding

A finding is a map returned in the array. Fields:

| Field | Type | Required | Meaning |
|---|---|---|---|
| `rule` | non-empty string | yes | Rule name. Kept verbatim in the verdict, `/` included (`schema/invalid`). The names `execution`, `invalid-finding` and `sections` are reserved: the engine produces the rows `lint:execution`, `lint:invalid-finding` and `lint:sections` itself, and a finding that uses one of them is invalid. |
| `severity` | `"error"` or `"warn"` | yes | Effect on the verdict (see below). |
| `object` | map `{apiVersion, kind, namespace?, name}` | yes | Object the finding is about. `apiVersion`, `kind` and `name` are non-empty strings; `namespace` is optional, and an empty one counts as absent. |
| `message` | non-empty string | yes | Message shown in the verdict. |
| `path` | string | no | Dotted path inside the object, for example `spec.template.spec.containers[0].image`. |
| `source` | string | no | Free-form origin information. |
| `suppressed` | boolean | no | Default `false`. A suppressed finding is inventoried, not reported. |
| `reason` | string | when `suppressed` is `true` | Reason given for a suppression. Must then be non-empty. |
| `origin` | string | no | Where the suppression comes from, for example `annotation:{prefix}{rule}`. |

Unknown keys are ignored.

An element that does not match this shape is **never dropped**. It becomes a failed row named
`lint:invalid-finding`, whose message names its index (`finding #0: ...`). A badly written script
therefore turns the test red rather than silently passing.

The order of the array returned by the script is kept: nothing is sorted or de-duplicated.

## Verdict

Each finding becomes a row of the test verdict:

| Case | Row name | Passed | Message |
|---|---|---|---|
| `error`, not suppressed | `lint:{rule}` | no | `message`, followed by ` [{path}]` when `path` is set |
| `warn`, not suppressed | `lint:{rule}` | yes | `warning: ` followed by the message (and path) |
| suppressed (any severity) | none | — | inventoried, see below |
| invalid element | `lint:invalid-finding` | no | description of the invalidity |
| script error | `lint:execution` | no | the error |
| status of the instance unreadable | `lint:sections` | no | the read error |

The description of a row is `{kind} {namespace}/{name}`, or `{kind} {name}` for a cluster-scoped object.
For the suite hook, the rows belong to a record named `(suite)`.

A failed row counts like any failed assert: the summary, the JUnit report and exit code 3 apply
unchanged. A `warn` finding never fails the test.

The verdict model has no severity field: a warning is a passing row with the `warning: ` prefix. The text
summary shows it as such. The JUnit report adds two things for the rows named `lint:...` (an ordinary
assert keeps its report unchanged, byte for byte):

- the description of the row (`Deployment app/web`) is written as the `classname` of the test case, so that
  ten objects failing the same rule are ten distinct cases rather than ten identical `lint:{rule}` ones;
- a passing row whose message starts with `warning: ` is listed in the `<system-out>` of the suite as
  `[WARN] {name} ({description}): {message}`, after the `[SKIP]` lines of the suppressions. Without it, a
  lint whose findings are all warnings would look green and silent.

## Suppressions

A finding marked `suppressed: true` never fails a test. It is kept in an inventory, so that every
exception stays visible in the reports.

The engine does not decide what is suppressed: the script sets `suppressed` itself, for example from
an annotation on the object. A suppression must say why: `suppressed: true` without a non-empty `reason`
is an invalid finding (`lint:invalid-finding`), so that an error can never be silenced without a trace of
its justification.

The inventory is rendered only when it is not empty:

- **text**: after the `Results:` line, a `Suppressed: {n}` line, then one line per suppression:
  `  [SKIP] {test}: {rule} {kind} {namespace}/{name}: {reason}`;
- **json**: after `summary`, a `suppressions` array. Each entry has the keys `test`, `rule`, `kind`,
  `namespace`, `name`, `path`, `reason`, `origin`; absent fields are `null`;
- **JUnit**: a `<system-out>` element on the `<testsuite>` of the test concerned, one suppression per
  line. No other suite carries a `<system-out>` for suppressions.

A suppression does not change the counters or the passing state of the run.

## The `schema_*` functions

These functions are registered in the engine of the lint script only when `--lint-dir` is given.

| Function | Returns |
|---|---|
| `schema_known(obj)` | `true` when a schema exists for the object's `apiVersion` and `kind`. An unknown kind is a fact, not an error. |
| `schema_validate(obj)` | array of `#{path, message}`, empty when the object conforms **or when no schema is known for its `apiVersion` and `kind`**. Same as `strict: true`. |
| `schema_validate(obj, #{strict: bool})` | same, with the strict mode set. `strict: false` disables only the strict mode; schema normalisation still applies. |
| `schema_native_version()` | version of the native Kubernetes schemas embedded in the binary, as a string. |

An object of unknown kind is therefore reported as conforming, and a typo in an `apiVersion` (`apps/v1beta`
instead of `apps/v1`) goes through silently. A script that wants to flag it calls `schema_known(obj)` first.

The `path` returned by `schema_validate` is a JSON pointer (for example `/spec/template/spec/containers/0`,
empty at the root). Converting it to dotted notation is left to the script.

Schemas come from three sources, in this resolution order:

1. the CRDs of the package, read from `<package-dir>/crds`;
2. the schema directory of the distribution (`--schemas-dir`);
3. the native Kubernetes schema table embedded in the binary.

The schema directory contains:

- JSON Schema files named `<group>/<Kind>_<version>.json`, with the exact case of the Kind, for example
  `apps/Deployment_v1.json`; the core group is the literal directory `core` (`core/Pod_v1.json`);
- YAML files containing `CustomResourceDefinition` documents.

Other files are ignored. A missing or empty schema directory adds no source; the native table remains.
A JSON file that cannot be parsed stops the command, and the error names the file.

## Example: `lint.rhai`

A minimal distribution script with one house rule. It builds its findings itself and needs no import:

```rhai
// House rule: every Deployment of the apps group must carry a `team` label.
fn lint(objs, ctx) {
    let findings = [];
    for obj in objs.all {
        if obj.apiVersion.starts_with("apps/") && obj.kind == "Deployment" {
            let labels = if "labels" in obj.metadata { obj.metadata.labels } else { #{} };
            if !("team" in labels) {
                findings.push(#{
                    rule: "team-label",
                    severity: "error",
                    object: #{
                        apiVersion: obj.apiVersion,
                        kind: obj.kind,
                        namespace: obj.metadata.namespace,
                        name: obj.metadata.name
                    },
                    path: "metadata.labels",
                    message: "label `team` is required"
                });
            }
        }
    }
    findings
}
```

Run it with:

```bash
agent package test -p ./my-package --all --lint-dir ./distribution/lint
```

A Deployment without a `team` label produces a failed row `lint:team-label`. A script that reads `objs.all`
should identify an object by its API group and kind together, never by its kind alone: a `Deployment` of
another group is not the workload the rule is about.

## Limits

- The values of the operation budget and their configuration are not fixed yet.
- A `warn` finding has no severity field in the verdict: it is a passing row with a prefix, listed as `[WARN]` in the JUnit report.
