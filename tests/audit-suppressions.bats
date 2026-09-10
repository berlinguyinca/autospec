#!/usr/bin/env bats
# tests/audit-suppressions.bats — policy gate over `cargo audit` (issue #4111).
#
# Covers the AC5 checks: a suppressed advisory passes the gate, an
# unsuppressed one fails it, and suppression entries missing their reason or
# removal trigger are rejected. The cargo-audit invocation is stubbed with
# strict fake `cargo` / `cargo-audit` binaries on PATH (both reject the calls
# the real tools refuse, and record argv for assertions), so the suite needs
# no network access.

setup() {
  REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/.." && pwd)"
  LINT="$REPO_ROOT/scripts/audit-suppressions.sh"
  TMP="$(mktemp -d)"
  SUP="$TMP/suppressions"
  FAKE_BIN="$TMP/bin"
  AUDIT_ARGS="$TMP/audit-args.log"
  mkdir -p "$SUP" "$FAKE_BIN"
}

teardown() {
  rm -rf "$TMP"
}

# write_entry DIR ID [key value]... — write a suppression file; keys are the
# 3rd+ args as (key, value) pairs, so omitting a pair omits the field.
write_entry() {
  local dir="$1" id="$2"
  shift 2
  {
    while [ $# -gt 0 ]; do
      printf '%s: %s\n' "$1" "$2"
      shift 2
    done
  } > "$dir/$id.txt"
}

full_entry() { # full_entry DIR ID SINCE
  write_entry "$1" "$2" \
    "advisory" "$2" \
    "since" "$3" \
    "reason" "vulnerable code path is unreachable from any workspace entry point" \
    "removal_trigger" "upstream fix lands in Cargo.lock or the dependency is removed" \
    "dependency_path" "vuln 0.1.0 <- lib 0.2.0 <- autospec-core"
}

# make_audit_stub NAME OUT RC — a PATH stub named NAME printing OUT, exiting
# RC. Every invocation is recorded to AUDIT_ARGS (binary name + args) so a
# test can assert on exactly how the gate invoked the tool: recording argv
# is the cheapest form of stub, and the default for every command stub.
# A double must be at least as strict as the thing it replaces -- each stub
# rejects the invocations the real binary refuses (#4164, #4165), so the suite
# verifies the invocation the real tool would actually see.
make_audit_stub() {
  local name="$1" out="$2" rc="$3"
  {
    printf '%s\n' '#!/usr/bin/env bash'
    printf '%s\n' "printf '%s %s\\n' '$name' \"\$@\" >> '$AUDIT_ARGS'"
    if [ "$name" = "cargo-audit" ]; then
      # The real cargo-audit invoked directly requires `audit` as argv[1]; a
      # bare `cargo-audit` prints usage and exits 2. The old stub ignored its
      # arguments, accepted the bare call the real tool rejects, and the suite
      # was green while CI failed on every run.
      printf '%s\n' 'if [ "${1:-}" != "audit" ]; then'
      printf '%s\n' '  echo "Audit Cargo.lock for crates with security vulnerabilities" >&2'
      printf '%s\n' '  echo "Usage: cargo [OPTIONS] <COMMAND>" >&2'
      printf '%s\n' '  exit 2'
      printf '%s\n' 'fi'
    else
      # The real cargo rejects an unknown subcommand.
      printf '%s\n' 'if [ "${1:-}" != "audit" ]; then'
      printf '%s\n' '  echo "error: no such command: ${1:-}" >&2'
      printf '%s\n' '  exit 101'
      printf '%s\n' 'fi'
    fi
    printf '%s\n' 'cat <<FAKE_EOF'
    printf '%s\n' "$out"
    printf '%s\n' 'FAKE_EOF'
    printf 'exit %s\n' "$rc"
  } > "$FAKE_BIN/$name"
  chmod +x "$FAKE_BIN/$name"
}

# make_audit_stubs OUT RC — install both stubs (cargo and cargo-audit) with the
# same behavior, so `command -v cargo` selects the stub instead of a system
# cargo and the gate's primary branch is exercised deterministically.
make_audit_stubs() {
  make_audit_stub cargo-audit "$1" "$2"
  make_audit_stub cargo "$1" "$2"
}

run_gate() {
  run env PATH="$FAKE_BIN:$PATH" bash "$LINT" gate --dir "$SUP" --today 2026-09-10
}

@test "--help exits 0 and names the subcommands" {
  run bash "$LINT" --help
  [ "$status" -eq 0 ]
  [[ "$output" == *"gate"* ]]
  [[ "$output" == *"report"* ]]
  [[ "$output" == *"validate"* ]]
}

@test "gate passes when every failing advisory is suppressed" {
  full_entry "$SUP" RUSTSEC-2023-0071 2026-09-10
  make_audit_stubs "    Summary:      1 vulnerability found in 1 dependency crate!
        Crate: rsa
         Advisory: RUSTSEC-2023-0071
         Title: Marvin Attack
error: 1 vulnerability found in 1 dependency crate!" 1
  run_gate
  [ "$status" -eq 0 ]
  [[ "$output" == *AUDIT_SUPPRESSED:RUSTSEC-2023-0071* ]]
  [[ "$output" == *"removal_trigger"* ]]
  # Assert the exact invocation the stub recorded: with cargo on PATH the
  # gate must call `cargo audit`, and nothing else.
  [ "$(cat "$AUDIT_ARGS")" = "cargo audit" ]
}

@test "gate fails when a failing advisory is not suppressed" {
  full_entry "$SUP" RUSTSEC-2023-0071 2026-09-10
  make_audit_stubs "        Crate: ring
         Advisory: RUSTSEC-2020-0013
error: 1 vulnerability found in 1 dependency crate!" 1
  run_gate
  [ "$status" -eq 1 ]
  [[ "$output" == *AUDIT_UNSUPPRESSED:RUSTSEC-2020-0013* ]]
}

@test "gate passes when cargo-audit finds no vulnerabilities" {
  full_entry "$SUP" RUSTSEC-2023-0071 2026-09-10
  make_audit_stubs "    Summary: No vulnerabilities found." 0
  run_gate
  [ "$status" -eq 0 ]
  [[ "$output" == *"no vulnerabilities found"* ]]
}

@test "gate fails closed when cargo-audit errors without naming an advisory" {
  make_audit_stubs "error: could not load the advisory database" 1
  run_gate
  [ "$status" -eq 1 ]
  [[ "$output" == *"failing closed"* ]]
}

@test "gate falls back to cargo-audit audit when cargo is not installed" {
  # Only the cargo-audit stub exists, and every PATH entry that provides a
  # `cargo` executable is excluded: this deterministically selects the
  # standalone-binary branch (the environment-selected branch #4164 broke and
  # the suite previously never exercised). The stub rejects any call missing
  # `audit` as argv[1], exactly like the real binary.
  make_audit_stub cargo-audit "    Summary: No vulnerabilities found." 0
  p=""
  d=""
  IFS=':' read -ra dirs <<<"$PATH"
  for d in "${dirs[@]}"; do
    [ -n "$d" ] && [ ! -x "$d/cargo" ] && p+="${p:+:}$d"
  done
  run env PATH="$FAKE_BIN:$p" bash "$LINT" gate --dir "$SUP" --today 2026-09-10
  [ "$status" -eq 0 ]
  [[ "$output" == *"no vulnerabilities found"* ]]
  [ "$(cat "$AUDIT_ARGS")" = "cargo-audit audit" ]
}

@test "gate rejects a suppression missing its reason" {
  write_entry "$SUP" RUSTSEC-2023-0071 \
    "advisory" "RUSTSEC-2023-0071" \
    "since" "2026-09-10" \
    "removal_trigger" "upstream fix lands" \
    "dependency_path" "rsa 0.9.10 <- sqlx-mysql 0.8.6"
  make_audit_stubs "         Advisory: RUSTSEC-2023-0071
error: 1 vulnerability found in 1 dependency crate!" 1
  run_gate
  [ "$status" -eq 2 ]
  [[ "$output" == *"missing required key: reason"* ]]
}

@test "gate rejects a suppression missing its removal trigger" {
  write_entry "$SUP" RUSTSEC-2023-0071 \
    "advisory" "RUSTSEC-2023-0071" \
    "since" "2026-09-10" \
    "reason" "vulnerable code path is unreachable" \
    "dependency_path" "rsa 0.9.10 <- sqlx-mysql 0.8.6"
  make_audit_stubs "         Advisory: RUSTSEC-2023-0071
error: 1 vulnerability found in 1 dependency crate!" 1
  run_gate
  [ "$status" -eq 2 ]
  [[ "$output" == *"missing required key: removal_trigger"* ]]
}

@test "validate passes for a complete entry" {
  full_entry "$SUP" RUSTSEC-2023-0071 2026-09-10
  run bash "$LINT" validate --dir "$SUP" --today 2026-09-10
  [ "$status" -eq 0 ]
  [ -z "$output" ]
}

@test "validate rejects a filename that does not match the advisory field" {
  # advisory field points at a different ID than the file name.
  write_entry "$SUP" RUSTSEC-2023-0071 \
    "advisory" "RUSTSEC-2023-0000" \
    "since" "2026-09-10" \
    "reason" "vulnerable code path is unreachable" \
    "removal_trigger" "upstream fix lands" \
    "dependency_path" "rsa 0.9.10 <- sqlx-mysql 0.8.6"
  run bash "$LINT" validate --dir "$SUP" --today 2026-09-10
  [ "$status" -eq 2 ]
  [[ "$output" == *"does not match filename"* ]]
}

@test "validate rejects a non-calendar since date" {
  full_entry "$SUP" RUSTSEC-2023-0071 2026-02-30
  run bash "$LINT" validate --dir "$SUP" --today 2026-09-10
  [ "$status" -eq 2 ]
  [[ "$output" == *"not a valid YYYY-MM-DD calendar date"* ]]
}

@test "validate rejects a since date in the future" {
  full_entry "$SUP" RUSTSEC-2023-0071 2027-01-01
  run bash "$LINT" validate --dir "$SUP" --today 2026-09-10
  [ "$status" -eq 2 ]
  [[ "$output" == *"is after today"* ]]
}

@test "report lists each active suppression with its age in days" {
  full_entry "$SUP" RUSTSEC-2023-0071 2026-07-13
  run bash "$LINT" report --dir "$SUP" --today 2026-09-10
  [ "$status" -eq 0 ]
  [[ "$output" == *"RUSTSEC-2023-0071"* ]]
  # 2026-07-13 -> 2026-09-10 is 18 (Jul) + 31 (Aug) + 10 (Sep) = 59 days.
  [[ "$output" == *"age 59 days"* ]]
}

@test "report marks an invalid entry and exits non-zero" {
  write_entry "$SUP" RUSTSEC-2023-0071 \
    "advisory" "RUSTSEC-2023-0071" \
    "since" "2026-09-10" \
    "reason" "reason only, no trigger" \
    "dependency_path" "rsa 0.9.10 <- sqlx-mysql 0.8.6"
  run bash "$LINT" report --dir "$SUP" --today 2026-09-10
  [ "$status" -eq 2 ]
  [[ "$output" == *"missing required key: removal_trigger"* ]]
}

@test "report with an empty directory exits 0 and says none active" {
  run bash "$LINT" report --dir "$SUP" --today 2026-09-10
  [ "$status" -eq 0 ]
  [[ "$output" == *"no active suppressions"* ]]
}

@test "the committed RUSTSEC-2023-0071 suppression satisfies the metadata contract" {
  # Pins the real config/audit-suppressions entry against the gate contract:
  # reason and removal trigger present, since valid, advisory matches file.
  run bash "$LINT" validate
  [ "$status" -eq 0 ]
  [ -z "$output" ]
}

@test "gate with no cargo-audit or cargo on PATH exits 3" {
  full_entry "$SUP" RUSTSEC-2023-0071 2026-09-10
  # Restricted PATH (empty bin dir) + absolute bash so a system cargo cannot
  # satisfy the gate's binary lookup.
  run env PATH="$FAKE_BIN" /bin/bash "$LINT" gate --dir "$SUP" --today 2026-09-10
  [ "$status" -eq 3 ]
  [[ "$output" == *"neither cargo-audit nor cargo"* ]]
}
