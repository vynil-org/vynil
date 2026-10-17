# Render lint

The render lint runs at the end of each `agent package test` run. It hands the objects that the
test produced to a lint script owned by the **distribution**, and folds the findings of that script
into the ordinary test verdict.

Vynil provides the engine and the data types. It ships no rule: the script decides what is checked,
with which severity, and what is excluded. Without a lint script, the render lint does not run and
`agent package test` behaves exactly as before.

> **Status.** The `render_lint` library ports 60 of the 63 checks of kube-linter v0.8.3. The other
> three are not ported, each for a stated reason: `scc-deny-privileged-container` needs a
> third-party CRD (OpenShift `SecurityContextConstraints`) that is not in the object list;
> `sorted-keys` needs the key order of the source document, which is lost before the objects reach
> the script; `schema-validation` is covered by `render_lint_schema`, composed separately. The
> library's `checks()` function is the authoritative list.

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

The `render_lint` and `render_lint_schema` libraries are therefore taken from the agent's script
directory, not from the distribution.

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

A third, optional function, `fix`, turns findings into a corrected `package.yaml` (see
[Fixing findings](#fixing-findings-fix)). It is only called with `agent package test --fix`. A fourth one, `hint`, says how to correct what `fix`
could not (see [Hints](#hints-hint)); it is only called with `--verbose`.

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
section is empty and every object is in `unsectioned`; the test is not failed for it.

Each call runs in a fresh Rhai engine, with no state shared between tests. The script receives copies:
it cannot modify the objects of the test. The available functions are the data-manipulation functions
of the base engine and the `schema_*` functions. No Kubernetes, HTTP, S3, shell or file function is
registered: a script that calls one fails with an unknown-function error.

Execution is bounded by an operation limit so that a looping script cannot block the run. The limit
value is not fixed by the contract yet.

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
| `reason` | string | no | Reason given for a suppression. |
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

The engine does not decide what is suppressed. The `render_lint` library marks findings as suppressed
from annotations on the object (see below), but a script may also set `suppressed` itself.

The inventory is rendered only when it is not empty:

- **text**: after the `Results:` line, a `Suppressed: {n}` line, then one line per suppression:
  `  [SKIP] {test}: {rule} {kind} {namespace}/{name}: {reason}`, with `(no reason)` when the reason is
  absent;
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
| `schema_validate(obj)` | array of `#{path, message}`, empty when the object conforms. Same as `strict: true`. |
| `schema_validate(obj, #{strict: bool})` | same, with the strict mode set. `strict: false` disables only the strict mode; schema normalisation still applies. |
| `schema_native_version()` | version of the native Kubernetes schemas embedded in the binary, as a string. |

The `path` returned by `schema_validate` is a JSON pointer (for example `/spec/template/spec/containers/0`,
empty at the root). The conversion to dotted notation is done by the `render_lint_schema` library.

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

## The `render_lint` library

The library is derived from [kube-linter](https://github.com/stackrox/kube-linter) (Apache License
2.0): the names, parameters, default values and measured behaviour of its checks are those of the
reference tool, ported to Rhai so that they run on the objects of a test render without a cluster.
Credit is given in the repository `NOTICE`. The library is meant to be read as much as called: a
distribution that writes its own rules can take these as a model, and each module documents the
behaviour it reproduces.

The distribution script composes the library with `import`. The alias is the module name
(`render_lint`), never `lint`: that is the name of the script's entry point `fn lint(objs, ctx)`.

```rhai
import "render_lint" as render_lint;
```

### Exposed functions

| Function | Role |
|---|---|
| `render_lint::finding(rule, severity, obj, message)` and `render_lint::finding(rule, severity, obj, path, message)` | Builds a finding map. |
| `render_lint::object_ref(obj)` | The `object` map of an object. |
| `render_lint::api_group_of(api_version)` | API group of an `apiVersion`: the part before `/` (`apps` for `apps/v1`), the empty string for `v1` (core group), `()` for a version without `/` other than `v1`. |
| `render_lint::kind_of_group(obj, kinds, group)` | `true` when the object has one of `kinds` in API group `group` (`""` for core). The only way a check recognises an object: a kind alone is never enough, a `Deployment` of another group is not an `apps` workload. |
| `render_lint::pod_templates(obj)` | Array of `#{pod, path}` for objects that embed a pod template. `path` is `spec.template`, `spec.jobTemplate.spec.template`, or an empty string for a `Pod`. Empty for other objects. |
| `render_lint::containers(entry)` | Array of `#{container, path, init}` for an entry of `pod_templates`: containers then init containers, with full paths such as `spec.template.spec.containers[0]`. |
| `render_lint::suppression_reason(obj, rule, opts)` | Value of the annotation `{ignore_prefix}{rule}` of the object, or `()` when absent or when `ignore_prefix` is not given. |
| `render_lint::apply_suppressions(findings, objs, opts)` | Marks findings suppressed when their object carries the annotation. Sets `reason` (when not empty) and `origin`. |
| `render_lint::all(objs, ctx, opts)` | Runs the enabled checks on every object, and applies suppressions. Order: objects in order, then checks in alphabetical order of their names. Without options, only the checks that kube-linter enables by default run. |
| `render_lint::checks()` | Sorted array of the names of the checks provided, in kube-linter naming (dashes). |
| `render_lint::default_checks()` | Sorted array of the checks that kube-linter enables by default (31 of the 60). |
| `render_lint::index(objs)` | The index of an object list (see below). Only needed to call a check directly. |

Each check is also a function `{name}(obj, idx, ctx, params)`, where the dashes of the kube-linter name
become underscores. It returns an array of findings, empty when nothing is reported, and returns an empty
array for kinds it does not apply to. The `rule` of a finding is the kube-linter name, dashes included, so
that an `exclude` list or a suppression annotation works across tools.

`idx` is the index of the object list, not the list itself. Rhai passes every value by copy: handing the
whole list to each of the sixty checks for each object copies it sixty times per object, which is quadratic.
`render_lint::all` builds the index once with `render_lint::index(objs)`, a map of shared handles that the
checks which relate several objects (a Service to its pods, a binding to its role, an env reference to a
Secret) query instead of walking the list. A script that calls a check directly passes
`render_lint::index(objs.all)`. The `ctx` argument is reserved: no ported check reads it and `all` passes `()`.

The cost of `render_lint::all` is therefore linear in the number of objects.

### Options of `render_lint::all`

All keys are optional.

| Key | Default | Meaning |
|---|---|---|
| `exclude` | none | Names of checks not to run. Always applied. |
| `only` | absent | When present, exactly these checks run, including checks that are not enabled by default. |
| `include_optional` | `false` | Also run the checks that kube-linter does not enable by default (`required-label-owner`, `minimum-three-replicas`, `no-node-affinity`, `use-namespace`…). |
| `severity` | `"error"` for each check | Map from check name to `"error"` or `"warn"`. Applied after the check runs. |
| `params` | empty | Map from check name to a map of parameters, using kube-linter parameter names. |
| `ignore_prefix` | absent | Prefix of the suppression annotations. Without it, no suppression happens at all. |
| `require_reason` | `false` | When `true`, a suppression with an empty annotation value is not applied. The finding stays, and one `suppression-without-reason` finding of severity `error` is added per object and rule, naming the annotation. When `false`, an empty annotation suppresses with the reason `(no reason given)`: the engine refuses a suppression without a reason. Set it to `true` in a distribution that wants every suppression justified; the default only keeps the inventory honest. |

Without options, `render_lint::all` runs the checks that kube-linter enables by default, which is what a
distribution that copies the example will expect. Pass `include_optional: true` to run all sixty.

**An option that cannot apply is an error, not a no-op.** The script fails with a message naming the
option, and the test reports a `lint:execution` row, for: an unknown option key; a check name that is not in
`render_lint::checks()` in `exclude`, `only`, `severity` or `params` (`exclude: ["latest_tag"]` with an
underscore would otherwise exclude nothing); a severity other than `"error"` or `"warn"`; a parameter the
check does not have; and a parameter value of the wrong type or outside its enumeration
(`requirementsType: "requests"` instead of `"request"`, a string where a list is expected).

### Schema validation

`render_lint_schema` is separate from `render_lint::all`: schema validation is opt-in.

```rhai
import "render_lint_schema" as schema;
```

`schema.check(obj)` and `schema.check(obj, opts)` return an array of findings for one object:

| Key | Default | Meaning |
|---|---|---|
| `strict` | `true` | An undeclared field is an error. `false` only disables strict mode. |
| `unknown` | `"warn"` | Severity of `schema/unknown-kind` when no schema exists: `"warn"`, `"error"`, or `"ignore"` (no finding). |
| `severity` | `"error"` | Severity of the `schema/invalid` findings. |

An invalid object reports one `schema/invalid` finding per validation error, with the dotted `path` and the
message of the engine. An object without `apiVersion` or `kind` gets a `schema/invalid` finding at the root.

## Example: `lint.rhai`

A minimal distribution script with one house rule, plus the call to the library:

```rhai
import "render_lint" as render_lint;
import "render_lint_schema" as schema;

// House rule: every Deployment must carry a `team` label.
fn team_label(objs) {
    let findings = [];
    for obj in objs {
        if render_lint::kind_of_group(obj, ["Deployment"], "apps") {
            let labels = if "labels" in obj.metadata { obj.metadata.labels } else { #{} };
            if !("team" in labels) {
                findings.push(render_lint::finding("team-label", "error", obj, "label `team` is required"));
            }
        }
    }
    findings
}

fn lint(objs, ctx) {
    let findings = team_label(objs.all);
    findings += render_lint::all(objs.all, ctx, #{
        exclude: ["latest-tag"],
        severity: #{ "no-read-only-root-fs": "warn" },
        ignore_prefix: "ignore-check.example/",
        require_reason: true
    });
    // The checks above stay silent on a malformed object (they return nothing):
    // the schema validation is what reports it.
    for obj in objs.all {
        findings += schema::check(obj);
    }
    findings
}
```

The checks of `render_lint::all` return no finding for an object whose shape they cannot read (a `spec` that
is not a map, an `env` of the wrong type). This is how the library behaves, and it is only safe
next to the schema validation, which reports exactly those shapes: `render_lint::all` does not call it, so
the example calls both.

Run it with:

```bash
agent package test -p ./my-package --all --lint-dir ./distribution/lint
```

A Deployment without a `team` label produces a failed row `lint:team-label`. An annotation
`ignore-check.example/team-label: "legacy"` on that Deployment does not suppress it, because the house
rule builds its finding directly and does not go through `render_lint::apply_suppressions`. To make a house rule
suppressible, pass its findings through `render_lint::apply_suppressions(findings, objs.all, opts)`.

## Fixing findings: `fix`

A finding carries what is needed to correct it. When the command runs with `--fix`, the script's
`fix` function is called **once per rule** that produced at least one finding, in alphabetical order
of the rules:

```rhai
fn fix(rule, findings, pkg) { ... }
```

| Argument | Content |
|---|---|
| `rule` | Name of the rule. |
| `findings` | All the findings of that rule, from every test and from the suite, suppressed ones included, in order of arrival. Same shape as a finding returned by `lint`. |
| `pkg` | The `package.yaml` as an ordered document (`yaml_decode_ordered`). `package` is a reserved word in Rhai, hence `pkg`. |

It returns a map:

| Key | Content |
|---|---|
| `status` | `"applied"` or `"not_applicable"`. |
| `pkg` | The complete, modified document. Required with `applied`. |
| `message` | Optional text, shown in the report. |

Anything else (an error, an unknown status, `applied` without `pkg`, a return that is not a map) is a
failed fix: the document is left as it was and the following rules still run. A distribution without
`fix` is not an error: its rules simply have no fix. Each rule sees the document as the previous rule
left it. Vynil ships no fix: the catalogue is the distribution's.

The script stays pure: it receives the document and returns the new one, only the command writes
`package.yaml`. The key order is kept at every level. Comments and quoting are not (the same limit as
`agent package update`), so `package.yaml` is only rewritten when a fix applied.

Two traps of the document type: reading a nested key returns a copy, so to change it, read the parent,
modify the copy and assign it back; and test the presence of a key with `keys().contains(...)`, because
`contains`, `len` and `is_empty` do not resolve on the document itself.

A suppressed finding is passed to `fix` on purpose. A fix that edits the whole package should not
override an author's explicit decision: the example below declines when any finding is suppressed.

### Example: no CPU limits

The rule (`lint`) reports every container with a CPU limit; the fix removes `limits.cpu` from the
`resources` of the package, and the `limits` block when it becomes empty. The rule is global to the
package, so the fix is too: it declines when a finding is suppressed, to not override an explicit
decision.

```rhai
fn lint(objs, ctx) {
    let out = [];
    for obj in objs.all {
        if obj.kind == "Deployment" {
            for c in obj.spec.template.spec.containers {
                if "resources" in c && "limits" in c.resources && "cpu" in c.resources.limits {
                    out.push(#{
                        rule: "no-cpu-limits", severity: "error",
                        object: #{ apiVersion: obj.apiVersion, kind: obj.kind, namespace: obj.metadata.namespace, name: obj.metadata.name },
                        message: "cpu limit on " + c.name
                    });
                }
            }
        }
    }
    out
}

fn fix(rule, findings, pkg) {
    for f in findings {
        if f.suppressed { return #{ status: "not_applicable", message: "an object is suppressed" }; }
    }
    let resources = pkg["resources"];
    if type_of(resources) != "map" { return #{ status: "not_applicable" }; }
    let changed = false;
    for name in resources.keys() {
        let component = resources[name];
        let limits = component["limits"];
        if type_of(limits) == "map" && limits.keys().contains("cpu") {
            limits.remove("cpu");
            if limits.keys().len() == 0 {
                component.remove("limits");
            } else {
                component["limits"] = limits;
            }
            resources[name] = component;
            changed = true;
        }
    }
    if !changed { return #{ status: "not_applicable" }; }
    pkg["resources"] = resources;
    #{ status: "applied", pkg: pkg }
}
```

```bash
agent package test -p ./my-package --all --lint-dir ./distribution/lint --fix
# stderr: fix no-cpu-limits: applied
```

The run above still exits with code 3, since the finding existed when the suite ran. Run it again, or
let the command do it with `--fix-rounds`:

```bash
agent package test -p ./my-package --all --lint-dir ./distribution/lint --fix --fix-rounds 3
# stderr: round 1: 2 finding(s)
#         fix no-cpu-limits: applied
#         round 2: 0 finding(s)
```

The suite runs again on the rewritten `package.yaml`, and the exit code is the one of the last run.
`--fix-rounds` is a ceiling: the loop also stops when nothing changed, or when a run does not have
strictly fewer findings than the previous one, in which case it applies no further fix.

## Hints: `hint`

A fix cannot always correct a finding: the value may be hard-coded in a template, or the right value
may need a decision. `hint` is the text that tells the reader, a person or an LLM, how to correct it.

```rhai
fn hint(rule, findings, pkg) { ... }
```

It receives what `fix` receives and returns a string. A non-empty string is the hint; an empty string,
or no `hint` function, means no hint; an error, or a return that is not a string, is shown as a failed
hint. It is only called with `--verbose`, once per rule that still has at least one finding which is not
suppressed, and not for a rule that a fix has just corrected.

```rhai
fn hint(rule, findings, pkg) {
    if rule != "no-cpu-limits" { return ""; }
    let names = [];
    for f in findings { if !f.suppressed { names.push(f.object.name); } }
    `Remove the CPU limit of the containers of ${names}. The limits are not in package.yaml: look for
    "cpu" under "limits" in the templates of this package. Keep the memory limit. Run the tests again
    to check.`
}
```

```text
hint no-cpu-limits:
  Remove the CPU limit of the containers of ["web"]. ...
```

The template literal is the template: the hint is built from the same data as the fix, so it can quote
the findings and any part of `pkg`. The output is plain text on stderr, meant to be handed as is to
whoever corrects the package.

## Limits

- A fix only edits `package.yaml`. A value hard-coded in a template or a script is not reachable by
  `fix`; its finding remains on the next run.
- `package.yaml` loses its comments when a fix applies it. `--dry-run` prints the resulting document,
  not a diff.
- Three kube-linter checks are not ported (see the status note).
- The values of the operation budget and their configuration are not fixed yet. The budget is
  proportional to the number of objects, and `render_lint::all` costs about sixteen thousand
  operations per object with all sixty checks.
- A `warn` finding has no severity field in the verdict: it is a passing row with a prefix, listed as `[WARN]` in the JUnit report.
- `env-value-from` resolves a Secret or ConfigMap reference by kind and name, **in any namespace**, as
  kube-linter does: the fixture `env-value-from` references a Secret of another namespace and the tool
  reports nothing. A Secret of the same name in another namespace therefore satisfies the reference.
- A resource quantity outside the Kubernetes grammar (a sign, an unknown unit, free text) is skipped by
  `unset-cpu-requirements` and `unset-memory-requirements`. The forms with an exponent (`129e6`) and the
  suffixes `P`, `E`, `Pi`, `Ei`, `n`, `u` are read. The extended forms are tested against the definition of
  the Kubernetes `Quantity`, not against the reference tool.
- kube-linter crashes (SIGSEGV) on a `dnsConfig` option whose name is the searched key but which has no
  value. That case cannot be compared with the oracle, so it has no `expected.json`; the library reports
  it without crashing. It is the only known divergence recorded here.
