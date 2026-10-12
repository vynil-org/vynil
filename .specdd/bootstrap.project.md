# SpecDD project specific overrides

These rules extend `.specdd/bootstrap.md` for the Vynil repository. When stricter or
more specific than the framework defaults, they win.

## Language and stack

- Product language is Rust (edition 2024, single workspace).
- Templating is Handlebars; package lifecycle logic is Rhai.
- Runtime target is Kubernetes. See `docs/architecture.md` for the authoritative
  component model.

## Spec granularity

- One `.sdd` spec describes one source file. Co-locate it as a same-basename spec, e.g.
  `common/src/vynilpackage.sdd` for `common/src/vynilpackage.rs`.
- Group two or three files under one spec only when they are inseparable (a trait and its
  single implementation, a file and its exclusive test file). Record the reason in a comment.
- A file without substantial, non-obvious behaviour (re-export shims, single-constant
  modules) may have no file spec; it stays covered by the nearest directory spec.
- A file spec states the file's complete observable contract: `Purpose`, `Exposes`,
  `Accepts`, `Returns`, `Raises`, `Handles`, and the invariants in `Must`. A reader must be
  able to reimplement the file from its spec alone.
- Directory and crate specs stay coarse: purpose, immediate child roles, boundary
  constraints, and `References` to child specs. They must not restate file behaviour.

## Dependency versions

- A version that changes with every bump of a dependency is named in exactly one spec: the
  one owning the manifest that pins it (for `vynil-core`, `common/common.sdd`). Every other
  spec names the dependency without a version and points to that manifest.
- A version stays in a spec only where it *is* the information: behaviour that changed
  starting from a given release, or a bug fixed in a given release.

## Test-driven specification (TTD)

- No product code change ships without tests derived from the owning spec's `Scenario`
  sections. A `Scenario` maps to one or more tests; name the test after the scenario.
- Sequence for any behaviour change: write failing tests from the spec scenarios, then the
  implementation, then reach green. The code satisfies the tests; the tests do not chase
  the code.
- Mark a `Tasks` entry `[x]` only once tests, clippy, and fmt are green.

## Verification commands

- `cargo test` — unit and integration tests.
- `cargo clippy --all-targets -- -D warnings` — static analysis harness (see `linting.sdd`).
- `cargo +nightly fmt --check` — Rust formatting (`rustfmt.toml`).
- `specdd lint` — spec syntax validation.

## Product code constraints

- No `unwrap()` or `expect()` in product code; tests may use them freely (enforced by the
  per-crate `#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]`).
- Every product error carries a stable, unique identifier so a log line maps to one origin.
- Do not widen a crate's public surface unless the owning spec changes it.

## Retired workflow

- The former `docs/conception/featureX.md` plus `.tasks/featureX/...` workflow is fully
  retired. Specs alongside the source replace it. Do not create, read, or depend on
  `.tasks/` or `docs/conception/` files.

## Git conventions

- Branch from `origin/main`: `feat/<name>` for features, `fix/<short-desc>` for fixes.
- Commit message first line: `feat: <summary>` or `fix: <summary>`; body explains the why.
- Do not chain implementation tasks automatically; one behaviour slice per change, then
  report and wait.
