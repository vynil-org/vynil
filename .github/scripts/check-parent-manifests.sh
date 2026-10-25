#!/usr/bin/env bash
# Checks that every parent.toml (the standalone workspace manifests used by the image
# builds) is exactly what `cargo cmd generate` writes from Cargo.toml.
# The commands are read from [package.metadata.commands].generate in Cargo.toml and
# replayed in a temporary directory: the repository is never modified.
# Usage: check-parent-manifests.sh [repository root, default .]
# Exit 1 if any parent.toml is out of sync, or if no command writes one.
set -euo pipefail

ROOT="$(cd "${1:-.}" && pwd)"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

mapfile -t COMMANDS < <(python3 - "$ROOT/Cargo.toml" <<'PY'
import sys
import tomllib

with open(sys.argv[1], "rb") as manifest:
    data = tomllib.load(manifest)
generate = data.get("package", {}).get("metadata", {}).get("commands", {}).get("generate", {})
for command in generate.get("cmd", []):
    if "parent.toml" in command:
        print(command)
PY
)

if [ "${#COMMANDS[@]}" -eq 0 ]; then
  echo "ERROR: no 'generate' command of Cargo.toml writes a parent.toml"
  exit 1
fi

cp "$ROOT/Cargo.toml" "$WORK/Cargo.toml"
status=0
for command in "${COMMANDS[@]}"; do
  target="$(grep -oE '[A-Za-z0-9_./-]*parent\.toml' <<<"$command" | tail -1)"
  mkdir -p "$WORK/$(dirname "$target")"
  (cd "$WORK" && bash -c "$command")
  if ! diff -u "$ROOT/$target" "$WORK/$target"; then
    echo "ERROR: $target is out of sync with Cargo.toml, run 'cargo cmd generate'"
    status=1
  fi
done

if [ "$status" -eq 0 ]; then
  echo "OK: ${#COMMANDS[@]} parent.toml in sync with Cargo.toml"
fi
exit "$status"
