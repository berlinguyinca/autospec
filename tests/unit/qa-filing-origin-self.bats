#!/usr/bin/env bats

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
QA_FINDING="$REPO_ROOT/scripts/qa-finding-to-issue.sh"
QA_SWEEP="$REPO_ROOT/scripts/qa-brute-force-sweep.sh"

setup() {
  TMP="$(mktemp -d)"
  export AUTOSPEC_BIN="$REPO_ROOT/tests/fixtures/autospec-project-sync-stub.sh"
  mkdir -p "$TMP/bin"
  cat > "$TMP/bin/gh" <<'SH'
#!/usr/bin/env bash
printf '%s ' "$@" | tr '\n' ' ' >> "$GH_LOG"; printf '\n' >> "$GH_LOG"
if [ "$1 $2" = "label create" ] && [ -n "${GH_LABEL_FAIL:-}" ]; then
  echo "label create failed" >&2
  exit 1
fi
if [ "$1 $2" = "issue list" ]; then
  echo "[]"
  exit 0
fi
if [ "$1 $2" = "issue create" ]; then
  count="$(cat "$GH_CREATE_COUNT" 2>/dev/null || echo 0)"
  count=$((count + 1))
  echo "$count" > "$GH_CREATE_COUNT"
  if [ -n "${GH_CREATE_FAIL_ONCE:-}" ] && [ "$count" -eq 1 ]; then
    echo "issue create failed once" >&2
    exit 1
  fi
  echo "https://github.com/acme/repo/issues/$count"
  exit 0
fi
echo "unsupported gh invocation: $*" >&2
exit 1
SH
  chmod +x "$TMP/bin/gh"
  export GH_LOG="$TMP/gh.log"
  export GH_CREATE_COUNT="$TMP/gh-create-count"
  export PATH="$TMP/bin:$PATH"
}

teardown() {
  rm -rf "$TMP"
  unset GH_LOG GH_CREATE_COUNT GH_LABEL_FAIL GH_CREATE_FAIL_ONCE REPO_DIR VERDICT_FILE AUTOSPEC_BIN
}

create_lines() {
  grep '^issue create ' "$GH_LOG"
}

@test "qa-finding-to-issue labels filed issues as origin:self even when label creation fails" {
  export GH_LABEL_FAIL=1
  finding='{"category":"code_health:test","summary":"self qa finding","evidence":"tests/unit/example.bats","file":"scripts/example.sh","status":"FAIL","remediation":"Fix it."}'

  run bash "$QA_FINDING" --finding "$finding" --dedup-cache "$TMP/dedup.txt"

  [ "$status" -eq 0 ]
  grep -q '^label create origin:self --color 8250df --force[[:space:]]*$' "$GH_LOG"
  [ "$(create_lines | wc -l | tr -d ' ')" -eq 1 ]
  create_lines | grep -q -- '--label origin:self'
}

@test "qa-brute-force-sweep labels primary and fallback filed issues as origin:self" {
  export GH_LABEL_FAIL=1
  export GH_CREATE_FAIL_ONCE=1
  mkdir -p "$TMP/repo"
  git -C "$TMP/repo" init -q
  cat > "$TMP/repo/smelly.py" <<'PY'
import ast

def classify(name):
    if "acid" in name:
        return "acid"
    if "base" in name:
        return "base"
    if "salt" in name:
        return "salt"
    return "other"
PY
  export REPO_DIR="$TMP/repo"
  export VERDICT_FILE="$TMP/qa-verdict.json"

  run bash "$QA_SWEEP"

  [ "$status" -eq 0 ]
  grep -q '^label create origin:self --color 8250df --force[[:space:]]*$' "$GH_LOG"
  [ "$(create_lines | wc -l | tr -d ' ')" -ge 2 ]
  ! create_lines | grep -v -- '--label origin:self'
}
