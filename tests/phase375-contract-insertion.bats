#!/usr/bin/env bats
# tests/phase375-contract-insertion.bats — Phase 3.75 `## Shared contracts`
# insertion point (#3200).
#
# The block must be inserted immediately BEFORE the first `## Dependencies`
# line of each child issue body. scripts/lint-issue.sh reads the Dependencies
# section until the next `## ` heading and reports DEPS_MALFORMED for any
# content line that is not `Depends on issue #N` or `none`; the old
# append-at-end-of-body placement landed the block's HTML markers inside that
# section on bodies where `## Dependencies` was the last section, which broke
# the 13 `#3112`-generation bodies.
#
# Fixtures are real files driven through the real deterministic tools
# (gen-issue-skeleton.sh, extract-shared-contracts.sh, lint-issue.sh); no mocks.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
    GEN="$REPO_ROOT/scripts/gen-issue-skeleton.sh"
    EXTRACT="$REPO_ROOT/scripts/extract-shared-contracts.sh"
    LINT="$REPO_ROOT/scripts/lint-issue.sh"
    TMP="$(mktemp -d)"
}

teardown() {
    rm -rf "$TMP"
}

# Generate a real child-issue body via the deterministic renderer, with
# `Depends on issue #123` as the single dependency line.
write_generated_body() {
    cat > "$TMP/issue.yml" <<'YAML'
issue_id: 3200-insertion-test
spec_path: docs/specs/2026-08-16-phase375-insertion-point-design.md
spec_url: https://github.com/berlinguyinca/autospec/blob/main/docs/specs/2026-08-16-phase375-insertion-point-design.md
goal_sentence: "Insert the Phase 3.75 shared contracts block before the issue dependencies section."
team_personality:
  - "Tooling maintainers: shell developer, test engineer"
  - "Emphasis: deterministic insertion and lint compatibility"
review_counter_team:
  - "Reliability review: regression tester, docs-drift reviewer"
  - "Challenge: patched bodies must survive scripts/lint-issue.sh"
files_to_read:
  - scripts/gen-issue-skeleton.sh
  - scripts/extract-shared-contracts.sh
  - scripts/lint-issue.sh
files_touched:
  - tests/phase375-contract-insertion.bats
local_llm_notes:
  - "32k routine; single-pass, keep the renderer and lint in context."
dependencies:
  - "Depends on issue #123"
implementation_scope:
  - "Assert the shared contracts block lands before the dependencies section"
out_of_scope:
  - "Retro-fixing the 13 already-affected issue bodies"
implementation_outline_lines:
  - "tests/phase375-contract-insertion.bats: lint the patched generated body"
tests_required:
  - "bats tests/phase375-contract-insertion.bats"
acceptance_criteria:
  - "scripts/lint-issue.sh reports 0 DEPS_MALFORMED findings on the patched body"
  - "The `## Shared contracts` heading line number is below `## Dependencies`"
verification:
  primary_smoke: "bats tests/phase375-contract-insertion.bats"
  operator_full: "bash scripts/lint-issue.sh <patched-body.md>"
branch_name: fix/phase375-insertion-point
YAML
    bash "$GEN" --input "$TMP/issue.yml" > "$1"
}

# Produce a real `## Shared contracts` block from two sibling issue bodies
# that share one file path and one ALL-CAPS name.
write_contracts_block() {
    cat > "$TMP/child-a.md" <<'MD'
## Implementation outline

- Edit `scripts/shared-lib.sh` and export `SHARED_CONTRACT_TOKEN`.
MD
    cat > "$TMP/child-b.md" <<'MD'
## Implementation outline

- Call into `scripts/shared-lib.sh` and read `SHARED_CONTRACT_TOKEN`.
MD
    bash "$EXTRACT" "$TMP/child-a.md" "$TMP/child-b.md" > "$1"
}

# Replicate the documented Phase 3.75 patch: insert the block immediately
# before the first `## Dependencies` line (fallback: end of body). Idempotent
# — skip when the begin marker is already present.
insert_before_dependencies() {
    body="$1"; block="$2"; out="$3"
    if grep -q '<!-- autospec-shared-contracts:begin -->' "$body"; then
        cp "$body" "$out"
        return 0
    fi
    awk -v f="$block" '
      !done && /^## Dependencies$/ {
        while ((getline line < f) > 0) print line
        print ""
        close(f)
        done = 1
      }
      { print }
      END { if (!done) while ((getline line < f) > 0) print line }
    ' "$body" > "$out"
}

# Replicate the OLD (broken) placement: the block lands after the dependency
# lines, still inside the `## Dependencies` section — exactly what plain
# append-to-end-of-body did when `## Dependencies` was the last section.
append_after_dependencies() {
    body="$1"; block="$2"; out="$3"
    awk -v f="$block" '
      !in_deps && /^## Dependencies$/ { in_deps = 1; print; next }
      in_deps && /^## / {
        while ((getline line < f) > 0) print line
        print ""
        in_deps = 0
      }
      { print }
    ' "$body" > "$out"
}

# Last non-blank content line of the `## Dependencies` section.
deps_last_line() {
    awk '
      /^## Dependencies$/ { in_deps = 1; next }
      in_deps && /^## / { exit }
      in_deps && NF { last = $0 }
      END { print last }
    ' "$1"
}

# Line number of the first occurrence of an exact heading.
heading_line() {
    awk -v h="$2" 'index($0, h) == 1 { print NR; exit }' "$1"
}

@test "patched generated body: lint-issue.sh reports no DEPS_MALFORMED" {
    write_generated_body "$TMP/body.md"
    write_contracts_block "$TMP/block.md"
    insert_before_dependencies "$TMP/body.md" "$TMP/block.md" "$TMP/patched.md"
    run bash -c "bash '$LINT' '$TMP/patched.md' 2>&1"
    [ "$status" -eq 0 ]
    ! echo "$output" | grep -q "DEPS_MALFORMED"
}

@test "patched generated body: ## Shared contracts precedes ## Dependencies" {
    write_generated_body "$TMP/body.md"
    write_contracts_block "$TMP/block.md"
    insert_before_dependencies "$TMP/body.md" "$TMP/block.md" "$TMP/patched.md"
    contract="$(heading_line "$TMP/patched.md" '## Shared contracts')"
    deps="$(heading_line "$TMP/patched.md" '## Dependencies')"
    [ -n "$contract" ]
    [ -n "$deps" ]
    [ "$contract" -lt "$deps" ]
}

@test "patched generated body: Depends on issue #N remains last line under ## Dependencies" {
    write_generated_body "$TMP/body.md"
    write_contracts_block "$TMP/block.md"
    insert_before_dependencies "$TMP/body.md" "$TMP/block.md" "$TMP/patched.md"
    [ "$(deps_last_line "$TMP/patched.md")" = "Depends on issue #123" ]
}

@test "insertion is idempotent: re-patching a marked body is a no-op" {
    write_generated_body "$TMP/body.md"
    write_contracts_block "$TMP/block.md"
    insert_before_dependencies "$TMP/body.md" "$TMP/block.md" "$TMP/patched.md"
    insert_before_dependencies "$TMP/patched.md" "$TMP/block.md" "$TMP/repatched.md"
    run diff "$TMP/patched.md" "$TMP/repatched.md"
    [ "$status" -eq 0 ]
}

@test "negative: block appended after ## Dependencies triggers DEPS_MALFORMED" {
    write_generated_body "$TMP/body.md"
    write_contracts_block "$TMP/block.md"
    append_after_dependencies "$TMP/body.md" "$TMP/block.md" "$TMP/broken.md"
    run bash -c "bash '$LINT' '$TMP/broken.md' 2>&1"
    [ "$status" -ge 1 ]
    echo "$output" | grep -q "DEPS_MALFORMED"
}

@test "negative: plain end-of-body append breaks a body ending in ## Dependencies" {
    # A realistic body where `## Dependencies` is the last section (the shape
    # of the 13 broken bodies): plain append-to-end lands the markers inside
    # the Dependencies section.
    cat > "$TMP/last-deps.md" <<'MD'
## Goal

Move the contract marker placement out of the dependencies section.

## Acceptance criteria

- [ ] scripts/lint-issue.sh reports 0 DEPS_MALFORMED findings

## Verification

### Primary smoke test

```
bats tests/phase375-contract-insertion.bats
```

## Dependencies

Depends on issue #123
MD
    write_contracts_block "$TMP/block.md"
    cat "$TMP/last-deps.md" "$TMP/block.md" > "$TMP/broken2.md"
    run bash -c "bash '$LINT' '$TMP/broken2.md' 2>&1"
    [ "$status" -ge 1 ]
    echo "$output" | grep -q "DEPS_MALFORMED"
}

@test "both skills document insertion before ## Dependencies, not plain append" {
    local member
    for member in \
        skills/autospec-define/SKILL.md \
        skills/autospec-define/codex/prompt.md \
        skills/autospec-define/opencode/agent.md \
        skills/autospec-split/SKILL.md \
        skills/autospec-split/codex/prompt.md \
        skills/autospec-split/opencode/agent.md
    do
        run grep -qF 'immediately before the first `## Dependencies` line' "$REPO_ROOT/$member"
        [ "$status" -eq 0 ]
    done
    # The old plain-append one-liner must be gone from both source skills.
    run grep -qF -- '-q .body)<newblock>' "$REPO_ROOT/skills/autospec-define/SKILL.md"
    [ "$status" -ne 0 ]
    run grep -qF -- '-q .body)<newblock>' "$REPO_ROOT/skills/autospec-split/SKILL.md"
    [ "$status" -ne 0 ]
}
