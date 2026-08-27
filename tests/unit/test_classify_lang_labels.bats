#!/usr/bin/env bats
# test_classify_lang_labels.bats — issue #3110
#
# Covers the `## Language fit` splice step that /autospec-classify runs per
# issue: the `apply_lang_step` function embedded in
# skills/autospec-classify/SKILL.md between the
# `# autospec-classify lang-step begin` / `end` anchors. The function is
# extracted verbatim (same sed pipeline the skill instructs), sourced, and
# driven through the A-F matrix.
#
# Capture pattern: `run bash -c 'source ...; apply_lang_step ... 2>err'`
# (house style for sourced functions, cf. test_autospec_fleet_url.bats). It is
# robust to a non-zero rc and keeps classifier stderr out of $output.
#
# No date pins: the block's **Classified** line is today's date by design, so
# assertions grep markers and lang values only.

REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"

setup() {
  WORK="$(mktemp -d)"
  SKILL="$REPO_ROOT/skills/autospec-classify/SKILL.md"
  [ -f "$SKILL" ]

  # Extract the lang-step function verbatim (fence body is 4-space indented).
  FN="$WORK/lang-step.sh"
  sed -n '/^[[:space:]]*# autospec-classify lang-step begin$/,/^[[:space:]]*# autospec-classify lang-step end$/p' "$SKILL" \
    | sed '1d;$d' \
    | sed 's/^    //' > "$FN"
  [ -s "$FN" ]
  bash -n "$FN"

  export AUTOSPEC_SCRIPTS_DIR="$REPO_ROOT/scripts"

  # Self-contained fixture repos (no network, no shared state).
  mkdir -p "$WORK/repo-rust/src" "$WORK/repo-py/tests/unit" "$WORK/repo-empty"
  printf 'fn main() {}\n' > "$WORK/repo-rust/src/main.rs"
  printf 'def test_x():\n    assert True\n' > "$WORK/repo-py/tests/unit/test_x.py"
}

teardown() {
  rm -rf "$WORK"
}

# invoke_lang_step <body-file> <labels-csv>
# Runs apply_lang_step on <body-file>; sets $status/$output (run semantics).
invoke_lang_step() {
  run bash -c 'source "$1"; apply_lang_step "$2" "$3" 2>"$4"' _ "$FN" "$1" "$2" "$WORK/err.txt"
}

line_of() {
  printf '%s\n' "$1" | sed -n "${2}p"
}

count() {
  grep -c "$1" "$2"
}

# Clean rust-classifying body (rank 2: Files touched, no block yet).
write_rust_body() {
  cat > "$1" <<'EOF'
## Goal
Add a parser for the wire format.

## Files touched

- `src/main.rs`
- `src/parse.rs`

## Dependencies

- none
EOF
}

# Python-classifying body carrying a stale rust block (standard shape:
# heading inside the markers) before ## Dependencies.
write_py_body_stale() {
  cat > "$1" <<'EOF'
## Goal
Add a parser for the wire format.

## Files touched

- `tests/unit/test_x.py`

<!-- autospec-language:begin -->
## Language fit

- **Language:** `lang:rust`
- **Source:** inherited
- **Rationale:** old stale entry
- **Classified:** 2026-08-20

<!-- autospec-language:end -->

## Dependencies

- none
EOF
}

# Rust-classifying body with a legacy block: the ## Language fit heading sits
# directly above the begin marker (outside it), stale python content inside,
# no heading inside the markers.
write_rust_body_legacy() {
  cat > "$1" <<'EOF'
## Goal
Add a parser for the wire format.

## Files touched

- `src/main.rs`
- `src/parse.rs`

## Language fit

<!-- autospec-language:begin -->
- **Language:** `lang:python`
- **Source:** inherited
- **Rationale:** old stale entry
- **Classified:** 2026-08-20

<!-- autospec-language:end -->

## Dependencies

- none
EOF
}

# No-signal body (verified to abstain to unknown with an empty repo root).
write_unknown_body() {
  cat > "$1" <<'EOF'
# Investigate queue behavior

## Goal

Document the monitor queue drain order.

## Acceptance criteria

- [ ] `docs/runbooks/queue-drain.md` exists with 3+ steps.

## Dependencies

None.
EOF
}

# ---------------------------------------------------------------- A: fresh

@test "A: fresh body splices one canonical block before ## Dependencies" {
  write_rust_body "$WORK/body.md"
  export AUTOSPEC_REPO_ROOT="$WORK/repo-rust"
  invoke_lang_step "$WORK/body.md" "ctx:medium,reasoning:shallow"
  [ "$status" -eq 0 ]
  [ "$(line_of "$output" 1)" = "rust" ]
  [ "$(line_of "$output" 2)" = "--add-label lang:rust" ]
  [ "$(count 'autospec-language:begin' "$WORK/body.md.lang")" -eq 1 ]
  [ "$(count 'autospec-language:end' "$WORK/body.md.lang")" -eq 1 ]
  [ "$(count '^## Language fit' "$WORK/body.md.lang")" -eq 1 ]
  grep -Fq 'lang:rust' "$WORK/body.md.lang"
  local li di
  li="$(grep -n 'autospec-language:begin' "$WORK/body.md.lang" | cut -d: -f1)"
  di="$(grep -n '^## Dependencies' "$WORK/body.md.lang" | cut -d: -f1)"
  [ -n "$li" ]
  [ -n "$di" ]
  [ "$li" -lt "$di" ]
}

# ------------------------------------------------------- B: stale replace

@test "B: stale block is replaced in place, no stacking, label swapped" {
  write_py_body_stale "$WORK/body.md"
  export AUTOSPEC_REPO_ROOT="$WORK/repo-py"
  invoke_lang_step "$WORK/body.md" "ctx:medium,reasoning:shallow,lang:rust"
  [ "$status" -eq 0 ]
  [ "$(line_of "$output" 1)" = "python" ]
  [ "$(line_of "$output" 2)" = "--add-label lang:python --remove-label lang:rust" ]
  [ "$(count 'autospec-language:begin' "$WORK/body.md.lang")" -eq 1 ]
  [ "$(count 'autospec-language:end' "$WORK/body.md.lang")" -eq 1 ]
  [ "$(count '^## Language fit' "$WORK/body.md.lang")" -eq 1 ]
  [ "$(count 'lang:python' "$WORK/body.md.lang")" -ge 1 ]
  [ "$(count 'lang:rust' "$WORK/body.md.lang")" -eq 0 ]
  [ "$(count 'old stale entry' "$WORK/body.md.lang")" -eq 0 ]
}

# --------------------------------------------------------- C: legacy shape

@test "C: legacy heading above markers is removed, block replaced, label swapped" {
  write_rust_body_legacy "$WORK/body.md"
  export AUTOSPEC_REPO_ROOT="$WORK/repo-rust"
  invoke_lang_step "$WORK/body.md" "ctx:medium,reasoning:shallow,lang:python"
  [ "$status" -eq 0 ]
  [ "$(line_of "$output" 1)" = "rust" ]
  [ "$(line_of "$output" 2)" = "--add-label lang:rust --remove-label lang:python" ]
  [ "$(count 'autospec-language:begin' "$WORK/body.md.lang")" -eq 1 ]
  [ "$(count 'autospec-language:end' "$WORK/body.md.lang")" -eq 1 ]
  [ "$(count '^## Language fit' "$WORK/body.md.lang")" -eq 1 ]
  [ "$(count 'lang:rust' "$WORK/body.md.lang")" -ge 1 ]
  [ "$(count 'lang:python' "$WORK/body.md.lang")" -eq 0 ]
  [ "$(count 'old stale entry' "$WORK/body.md.lang")" -eq 0 ]
}

# ------------------------------------------------------------- D: unknown

@test "D: no-signal body abstains to unknown and still splices a block" {
  write_unknown_body "$WORK/body.md"
  export AUTOSPEC_REPO_ROOT="$WORK/repo-empty"
  invoke_lang_step "$WORK/body.md" "ctx:medium"
  [ "$status" -eq 0 ]
  [ "$(line_of "$output" 1)" = "unknown" ]
  [ "$(line_of "$output" 2)" = "--add-label lang:unknown" ]
  [ "$(count 'autospec-language:begin' "$WORK/body.md.lang")" -eq 1 ]
  [ "$(count 'autospec-language:end' "$WORK/body.md.lang")" -eq 1 ]
  [ "$(count '^## Language fit' "$WORK/body.md.lang")" -eq 1 ]
  grep -Fq 'lang:unknown' "$WORK/body.md.lang"
}

# -------------------------------------------------------- E: idempotency

@test "E: re-running on a patched body is byte-identical and LABEL_NOOP" {
  write_rust_body "$WORK/body.md"
  export AUTOSPEC_REPO_ROOT="$WORK/repo-rust"
  invoke_lang_step "$WORK/body.md" "ctx:medium,reasoning:shallow"
  [ "$status" -eq 0 ]
  [ "$(line_of "$output" 2)" = "--add-label lang:rust" ]

  # Second pass: the issue now carries lang:rust; the body is already patched.
  invoke_lang_step "$WORK/body.md.lang" "ctx:medium,reasoning:shallow,lang:rust"
  [ "$status" -eq 0 ]
  [ "$(line_of "$output" 1)" = "rust" ]
  [ "$(line_of "$output" 2)" = "LABEL_NOOP" ]
  cmp -s "$WORK/body.md.lang" "$WORK/body.md.lang.lang"
}

# -------------------------------------------- F: same-answer NOOP on fresh

@test "F: label already matches the classification on an unpatched body" {
  write_rust_body "$WORK/body.md"
  export AUTOSPEC_REPO_ROOT="$WORK/repo-rust"
  invoke_lang_step "$WORK/body.md" "ctx:medium,reasoning:shallow,lang:rust"
  [ "$status" -eq 0 ]
  [ "$(line_of "$output" 1)" = "rust" ]
  [ "$(line_of "$output" 2)" = "LABEL_NOOP" ]
  # The body is still patched (body and label are tracked independently).
  [ "$(count 'autospec-language:begin' "$WORK/body.md.lang")" -eq 1 ]
  grep -Fq 'lang:rust' "$WORK/body.md.lang"
}

# ------------------------------------------------- wiring: trio members pin

@test "wiring: all three trio members pin the lang-step contract" {
  local member v
  for member in \
    "$REPO_ROOT/skills/autospec-classify/SKILL.md" \
    "$REPO_ROOT/skills/autospec-classify/codex/prompt.md" \
    "$REPO_ROOT/skills/autospec-classify/opencode/agent.md"
  do
    [ -f "$member" ]
    grep -Fq 'autospec-language:begin' "$member"
    grep -Fq 'autospec-language:end' "$member"
    grep -Fq 'classify-language.sh' "$member"
    grep -Fq 'd4c5f9' "$member"
    grep -Fq -- '--force' "$member"
    for v in rust go python typescript javascript java bash ruby csharp markdown mixed unknown; do
      grep -Fq "lang:$v" "$member"
    done
  done
}
