# Package Linting

`agent package lint` analyzes a package **statically**, without deployment: structure,
Handlebars templates, and Rhai scripts. This is the tool to integrate in CI before publishing.

```bash
agent package lint -p ./my-package --format junit --junit-output-filename lint.xml
```

| Option | Default | Role |
|---|---|---|
| `-p`, `--package-dir` | `/tmp/package` | Package directory. |
| `-c`, `--config-dir` | `.` | Additional Rhai scripts. |
| `--format` | `text` | `text` or `junit`. |
| `--level` | `all` | Minimum severity displayed: `error`, `warn`, `all`. |
| `--junit-output-filename` | — | Writes a JUnit XML report. |

## Checks performed

### Structure (`package/`)

- presence and format of `package.yaml`;
- required directory structure;
- resource definitions.

### Templates Handlebars (`hbs/`)

- syntax validity;
- unknown helpers (`hbs/unknown-helper`);
- unknown partials;
- consistency of context variables with `package.yaml`;
- validity of resource and image keys;
- correct package type usage.

For `hbs/unused-option`, usage counts from templates and from Rhai scripts, either
directly (`context.values.…`) or through a simple `context.values` alias
(`let v = context.values; v.ingress.provider`).

The alias analysis is textual, with no lexical scope: two variables with the same name in
different functions merge their paths, and a string or comment containing
`let x = context.values;` counts as an alias, so an option may be considered used by mistake
(a missed warning, never a spurious one). An indexed access (`v["a"]`) is not followed and
still yields `hbs/unused-option`.

### Scripts Rhai (`rhai/`)

- syntax validity;
- unresolved imports;
- dead code, unused variables, shadowed variables;
- unused parameters and functions (`rhai/unused-function`, `rhai/unused-variable`);
- API mode validation (no full-API in core scripts);
- package type validation (no tenant access in a system package);
- context hook return value validation (`rhai/context-hook-no-return`).

## Configuration: `.vynil-lint.yaml`

Placed at the package root, it customizes behavior.

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

## Inline disabling

Disable a rule for a single line:

- Rhai: `// vynil-lint-disable rhai/unused-variable`
- Handlebars: `{{!-- vynil-lint-disable hbs/unknown-helper --}}`

Block mode support (enable/disable)

## Exit codes

| Code | Meaning |
|---|---|
| `0` | No issues. |
| `1` | Errors detected. |
| `2` | Warnings only. |
