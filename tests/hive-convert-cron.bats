#!/usr/bin/env bats
# tests/hive-convert-cron.bats — scripts/hive/convert-precheck.sh and the skip path of
# scripts/hive/convert-cron.sh. Real autospec binary, real git repos, real fixture dirs.
# No stubs: when srun is absent the "pending" path simply fails to allocate, which is
# observable in the log. Needs `autospec` and `git` on PATH (skipped otherwise).

PRE="${BATS_TEST_DIRNAME}/../scripts/hive/convert-precheck.sh"
CRON="${BATS_TEST_DIRNAME}/../scripts/hive/convert-cron.sh"

setup() {
  command -v autospec >/dev/null || skip "autospec not on PATH"
  command -v git >/dev/null || skip "git not on PATH"
  T="$(mktemp -d)"
  LLM="$T/llm"
  mkdir -p "$LLM/autospec/out" "$LLM/logs" "$LLM/state"
  git init -q --bare "$T/origin.git"
  git clone -q "$T/origin.git" "$T/work" 2>/dev/null
  ( cd "$T/work" && git config user.email t@t && git config user.name t \
    && mkdir -p crates/x/src && echo 'pub fn f() {}' > crates/x/src/lib.rs \
    && git add . && git commit -qm init && git branch -M main && git push -q origin main )
  cd "$T/work" || exit 1
}
teardown() { rm -rf "$T"; }

rust_patch() { # rust_patch N
  mkdir -p "$LLM/autospec/out/issue-$1"
  printf 'diff --git a/crates/x/src/lib.rs b/crates/x/src/lib.rs\n--- a/crates/x/src/lib.rs\n+++ b/crates/x/src/lib.rs\n@@ -1 +1 @@\n-pub fn f() {}\n+pub fn f() { }\n' > "$LLM/autospec/out/issue-$1/changes.patch"
}
precheck() { "$PRE" "$LLM" "$LLM/autospec/held.jsonl" "" main conv- "$@"; }

@test "no candidates: idle, exit 10" {
  run precheck
  [ "$status" -eq 10 ]
  [[ "$output" == idle:* ]]
}

@test "candidate without changes.patch: idle, exit 10 (no plan needed)" {
  mkdir -p "$LLM/autospec/out/issue-5"; : > "$LLM/autospec/out/issue-5/agent.out"
  run precheck 5
  [ "$status" -eq 10 ]
  [[ "$output" == *"has a changes.patch"* ]]
}

@test "fresh rust patch: pending, exit 0" {
  rust_patch 7
  run precheck 7
  [ "$status" -eq 0 ]
  [[ "$output" == "pending: fresh=1"* ]]
}

@test "patch with a current verdict hold: idle, exit 10" {
  rust_patch 7
  key=$(stat -c %Y "$LLM/autospec/out/issue-7/changes.patch")
  sha=$(git rev-parse origin/main)
  printf '{"issue":7,"patch_key":"%s","base_sha":"%s","depends_on":[],"reason":"clippy=2"}\n' "$key" "$sha" > "$LLM/autospec/held.jsonl"
  run precheck 7
  [ "$status" -eq 10 ]
  [[ "$output" == idle:* ]]
}

@test "undecidable plan fails open: pending, exit 0" {
  rust_patch 7
  run env PATH=/usr/bin:/bin "$PRE" "$LLM" "$LLM/autospec/held.jsonl" "" main conv- 7
  [ "$status" -eq 0 ]
  [[ "$output" == pending:*undecidable* ]]
}

cron_env() { # run convert-cron.sh against the fixture, skipping the clone/fetch network
  mkdir -p "$LLM/convert"; git clone -q "$T/origin.git" "$LLM/convert/repo" 2>/dev/null
  CONVERT_LLM_ROOT="$LLM" BATCH_N=4 "$CRON"
}

@test "cron: idle batch skips the allocation and says so in the log" {
  command -v cargo >/dev/null && command -v gh >/dev/null || skip "cargo/gh not on PATH"
  mkdir -p "$LLM/autospec/out/issue-5"
  run cron_env
  [ "$status" -eq 0 ]
  grep -q "pass end SKIPPED, no allocation requested" "$LLM/logs/convert-cron.log"
  ! grep -q "srun" "$LLM/logs/convert-cron.log"
}

@test "cron: pending batch does not skip" {
  command -v cargo >/dev/null && command -v gh >/dev/null || skip "cargo/gh not on PATH"
  rust_patch 7
  run cron_env
  grep -q "precheck: pending" "$LLM/logs/convert-cron.log"
  ! grep -q "SKIPPED" "$LLM/logs/convert-cron.log"
}

@test "cron: unreachable remote is logged with a FATAL line and the pass is skipped" {
  command -v cargo >/dev/null && command -v gh >/dev/null || skip "cargo/gh not on PATH"
  mkdir -p "$LLM/convert"; git clone -q "$T/origin.git" "$LLM/convert/repo" 2>/dev/null
  git -C "$LLM/convert/repo" remote set-url origin "$T/does-not-exist.git"
  run env CONVERT_LLM_ROOT="$LLM" CONVERT_FETCH_SLEEP=0 "$CRON"
  [ "$status" -eq 1 ]
  grep -q "FATAL: git -C failed 3x\|FATAL: git .* failed 3x" "$LLM/logs/convert-cron.log"
}

@test "scripts pass shellcheck at warning level" {
  command -v shellcheck >/dev/null || skip "shellcheck not installed"
  run shellcheck -S warning -x "$PRE" "$CRON"
  [ "$status" -eq 0 ]
}
