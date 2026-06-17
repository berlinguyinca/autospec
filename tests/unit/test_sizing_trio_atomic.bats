#!/usr/bin/env bats
# tests/unit/test_sizing_trio_atomic.bats — Spec 2 child D: trio atomic-unit
# exemption for scripts/sizing-check.sh.
#
# With derive-trio.sh now shipped, a trio edit is one logical change:
# "edit SKILL.md → derive-trio --in-place → gen-skill-goldens". The ≤3-files
# cap must STOP forcing a trio's three prose members + their derived skill
# goldens into separate issues. Two exemptions:
#   1. A skill trio's three members (skills/<x>/{SKILL.md,codex/prompt.md,
#      opencode/agent.md}) count as ONE logical file unit.
#   2. Generated tests/fixtures/skill-goldens/*.sha256 paths are excluded from
#      the count entirely (they are mechanically regenerated, not authored).
#
# NOTE on test mechanics: bodies are written to a real temp file and piped
# into sizing-check.sh on stdin. We do NOT interpolate the body through
# `bash -c "...$body..."` because the outline bullets contain backticks, which
# the outer shell would command-substitute and mangle.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    SIZING="$REPO_ROOT/scripts/sizing-check.sh"
    BODY="$(mktemp -t sizing-trio.XXXXXX)"
}

teardown() {
    rm -f "$BODY"
}

@test "trio members + derived goldens count as one unit (passes the file cap)" {
    cat > "$BODY" <<'EOF'
## Implementation outline
- `skills/autospec-define/SKILL.md` — edit the decomposer prose.
- `skills/autospec-define/codex/prompt.md` — derived via derive-trio --in-place.
- `skills/autospec-define/opencode/agent.md` — derived via derive-trio --in-place.
- `tests/fixtures/skill-goldens/autospec-define.SKILL.md.sha256` — regen.
- `tests/fixtures/skill-goldens/autospec-define.codex.prompt.md.sha256` — regen.
- `tests/fixtures/skill-goldens/autospec-define.opencode.agent.md.sha256` — regen.
EOF
    run "$SIZING" < "$BODY"
    [ "$status" -eq 0 ]
}

@test "a single trio (3 members, no goldens) counts as one unit" {
    cat > "$BODY" <<'EOF'
## Implementation outline
- `skills/autospec-run/SKILL.md` — edit prose.
- `skills/autospec-run/codex/prompt.md` — derived.
- `skills/autospec-run/opencode/agent.md` — derived.
EOF
    run "$SIZING" < "$BODY"
    [ "$status" -eq 0 ]
}

@test "skill-golden sha256 paths alone do not count toward the cap" {
    cat > "$BODY" <<'EOF'
## Implementation outline
- `scripts/foo.sh` — a real source file.
- `tests/fixtures/skill-goldens/a.SKILL.md.sha256` — regen.
- `tests/fixtures/skill-goldens/b.codex.prompt.md.sha256` — regen.
- `tests/fixtures/skill-goldens/c.opencode.agent.md.sha256` — regen.
EOF
    run "$SIZING" < "$BODY"
    [ "$status" -eq 0 ]
}

@test "trio unit + 3 unrelated source files still trips the cap (exemption is scoped)" {
    # One trio (1 unit) + 3 distinct non-trio files = 4 logical units > 3.
    cat > "$BODY" <<'EOF'
## Implementation outline
- `skills/autospec-run/SKILL.md` — edit prose.
- `skills/autospec-run/codex/prompt.md` — derived.
- `skills/autospec-run/opencode/agent.md` — derived.
- `scripts/a.sh` — real file.
- `scripts/b.sh` — real file.
- `scripts/c.sh` — real file.
EOF
    run "$SIZING" < "$BODY"
    [ "$status" -ne 0 ]
    [[ "$output" == *outline-files* ]]
}

@test "two distinct trios count as two units (still within cap)" {
    cat > "$BODY" <<'EOF'
## Implementation outline
- `skills/autospec-run/SKILL.md` — edit prose.
- `skills/autospec-run/codex/prompt.md` — derived.
- `skills/autospec-define/SKILL.md` — edit prose.
- `skills/autospec-define/codex/prompt.md` — derived.
EOF
    run "$SIZING" < "$BODY"
    [ "$status" -eq 0 ]
}

@test "four distinct trios exceed the cap (4 units > 3)" {
    cat > "$BODY" <<'EOF'
## Implementation outline
- `skills/a/SKILL.md` — edit.
- `skills/b/SKILL.md` — edit.
- `skills/c/SKILL.md` — edit.
- `skills/d/SKILL.md` — edit.
EOF
    run "$SIZING" < "$BODY"
    [ "$status" -ne 0 ]
    [[ "$output" == *outline-files* ]]
}

@test "the original 3-distinct-non-trio-files-at-cap case still passes" {
    cat > "$BODY" <<'EOF'
## Implementation outline
- `a.txt` — n.
- `b.txt` — n.
- `c.txt` — n.
EOF
    run "$SIZING" < "$BODY"
    [ "$status" -eq 0 ]
}

@test "the original 4-distinct-non-trio-files case still fails" {
    cat > "$BODY" <<'EOF'
## Implementation outline
- `a.txt` — n.
- `b.txt` — n.
- `c.txt` — n.
- `d.txt` — n.
EOF
    run "$SIZING" < "$BODY"
    [ "$status" -ne 0 ]
    [[ "$output" == *outline-files* ]]
}
