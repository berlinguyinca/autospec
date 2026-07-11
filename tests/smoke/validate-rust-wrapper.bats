#!/usr/bin/env bats

REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"

@test "validate wrapper primary smoke passes through legacy fallback" {
  cd "$REPO_ROOT"
  fake_bin="$(mktemp -d)"
  log="$fake_bin/delegation.log"
  cat > "$fake_bin/autospec" <<'SH'
#!/usr/bin/env bash
set -eu
printf '%s\n' "$*" > "$AUTOSPEC_VALIDATE_DELEGATION_LOG"
printf '%s\n' "${AUTOSPEC_VALIDATE_FROM_SHELL:-}" >> "$AUTOSPEC_VALIDATE_DELEGATION_LOG"
SH
  chmod +x "$fake_bin/autospec"

  AUTOSPEC_VALIDATE_DELEGATION_LOG="$log" \
    AUTOSPEC_RUST_VALIDATE_BIN="$fake_bin/autospec" \
    bash scripts/validate.sh --fast --changed

  grep -qxF "validate --fast --changed" "$log"
  grep -qxF "1" "$log"
}
