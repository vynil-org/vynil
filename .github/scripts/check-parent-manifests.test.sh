#!/usr/bin/env bash
# Tests check-parent-manifests.sh against throwaway repositories, one per scenario of
# .github/workflows/workflows.sdd. Exit 1 if any scenario fails.
set -euo pipefail

CHECK="$(cd "$(dirname "$0")" && pwd)/check-parent-manifests.sh"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
failures=0

GENERATE_AGENT="awk 'BEGIN{p=1}/profile.release/{p=0}p==1&&!/\\\"operator\\\",/'<Cargo.toml>agent/parent.toml"

# Writes a minimal workspace in $1 whose `generate` command list is $2 (TOML array body).
make_repo() {
  local dir="$1" cmds="$2"
  mkdir -p "$dir/agent"
  cat > "$dir/Cargo.toml" <<TOML
[workspace]
members = [
    "agent",
    "operator",
]

[workspace.dependencies]
serde = "1"
jsonschema = { version = "0.30", default-features = false }

[package.metadata.commands]
generate = { cmd=[
    ${cmds}
]}

[profile.release]
lto = true
TOML
}

# Runs the check on $1; stores its exit code in $status and its output in $output.
run_check() {
  set +e
  output="$(bash "$CHECK" "$1" 2>&1)"
  status=$?
  set -e
}

fail() {
  echo "FAIL: $1"
  echo "$output" | sed 's/^/    /'
  failures=$((failures + 1))
}

# Scenario: synchronized parent manifests pass
repo="$WORK/synced"
make_repo "$repo" "\"$GENERATE_AGENT\","
(cd "$repo" && bash -c "${GENERATE_AGENT//\\\"/\"}")
before="$(cd "$repo" && find . -type f -exec sha256sum {} + | sort)"
run_check "$repo"
after="$(cd "$repo" && find . -type f -exec sha256sum {} + | sort)"
if [ "$status" -ne 0 ]; then
  fail "synchronized parent manifests pass (exit $status)"
elif [ "$before" != "$after" ]; then
  fail "synchronized parent manifests pass (repository modified)"
else
  echo "ok: synchronized parent manifests pass"
fi

# Scenario: a workspace dependency missing from a parent manifest fails
repo="$WORK/missing"
make_repo "$repo" "\"$GENERATE_AGENT\","
(cd "$repo" && bash -c "${GENERATE_AGENT//\\\"/\"}")
sed -i '/^jsonschema/d' "$repo/agent/parent.toml"
run_check "$repo"
if [ "$status" -ne 1 ]; then
  fail "a workspace dependency missing from a parent manifest fails (exit $status)"
elif ! grep -q "agent/parent.toml" <<<"$output" || ! grep -q "^+jsonschema" <<<"$output"; then
  fail "a workspace dependency missing from a parent manifest fails (output)"
else
  echo "ok: a workspace dependency missing from a parent manifest fails"
fi

# Scenario: no generate command writing a parent manifest fails
repo="$WORK/none"
make_repo "$repo" "\"echo nothing\","
run_check "$repo"
if [ "$status" -ne 1 ]; then
  fail "no generate command writing a parent manifest fails (exit $status)"
else
  echo "ok: no generate command writing a parent manifest fails"
fi

if [ "$failures" -ne 0 ]; then
  echo "$failures scenario(s) failed"
  exit 1
fi
