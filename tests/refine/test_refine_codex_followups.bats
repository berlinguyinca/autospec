#!/usr/bin/env bats
# tests/refine/test_refine_codex_followups.bats — issue #692.
#
# Four fixtures from the issue body:
#   1. Handoff failure → schema-valid artifact with status=handoff_failed and
#      metadata.handoff_exit_code present.
#   2. Missing .autospec/run-summary.md after iteration → iteration_error.
#   3. Stale .autospec/run-summary.md (predating iteration) → iteration_error.
#   4. Symlink at <bindir>/claude → /tmp/evil rejected by LLM dispatcher.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    REFINE="$REPO_ROOT/scripts/refine-prompt.sh"
    LENS_LLM="$REPO_ROOT/scripts/refine-prompt-lens-llm.sh"
    TMPWORK="$(mktemp -d -t refine-692.XXXXXX)"
    SCHEMA_SINGLE="$REPO_ROOT/schemas/autospec-refinement.schema.json"
    SCHEMA_LOOP="$REPO_ROOT/schemas/autospec-refinement-loop.schema.json"
    export AUTOSPEC_HANDOFF_DISPATCHER_KIND=claude
    export AUTOSPEC_REFINE_LENS_MODE=deterministic
}

teardown() {
    [ -n "${TMPWORK:-}" ] && rm -rf "$TMPWORK" || true
}

_stat_mtime() {
    stat -c%Y "$1" 2>/dev/null || stat -f%m "$1" 2>/dev/null || echo 0
}

# ── Fix 1 — schema completeness ───────────────────────────────────

@test "schema declares handoff_exit_code in metadata" {
    run grep -q '"handoff_exit_code"' "$SCHEMA_SINGLE"
    [ "$status" -eq 0 ]
}

@test "schema status enum includes handoff_failed" {
    run grep -E '"handoff_failed"' "$SCHEMA_SINGLE"
    [ "$status" -eq 0 ]
}

@test "real orchestrator artifact validates against single-pass schema" {
    if ! python3 -c 'import jsonschema' 2>/dev/null; then skip "jsonschema unavailable"; fi
    cd "$TMPWORK"
    mkdir -p ref
    run bash "$REFINE" "demo prompt for schema check" --rounds 1 --dry-run \
        --artifact-dir "$TMPWORK/ref" --repo-root "$TMPWORK" --memory-root "$TMPWORK/none"
    [ "$status" -eq 0 ]
    artifact="$(ls "$TMPWORK"/ref/*.json | head -1)"
    [ -n "$artifact" ]
    run python3 -c "
import json, sys, jsonschema
doc = json.load(open('$artifact'))
sch = json.load(open('$SCHEMA_SINGLE'))
jsonschema.validate(doc, sch)
print('ok')
"
    [ "$status" -eq 0 ]
}

# ── Fix 2 — staleness guard ───────────────────────────────────────

@test "missing run-summary.md after handoff → iteration_error" {
    cd "$TMPWORK"
    mkdir -p fake-bin
    # Fake claude dispatcher that always succeeds but writes NO run-summary.
    cat > fake-bin/claude <<EOF
#!/usr/bin/env bash
exit 0
EOF
    chmod +x fake-bin/claude
    mkdir -p .autospec
    AUTOSPEC_HANDOFF_DISPATCHER=1 PATH="$TMPWORK/fake-bin:$PATH" \
        run bash "$REFINE" "iterating prompt missing summary" \
        --continue --max-iterations 1 --rounds 1 \
        --artifact-dir "$TMPWORK/ref" --repo-root "$TMPWORK" --memory-root "$TMPWORK/none"
    [ "$status" -eq 0 ]
    loop_json="$(ls "$TMPWORK"/ref/*-loop.json | head -1)"
    [ -n "$loop_json" ]
    run grep -q '"status": "iteration_error"' "$loop_json"
    [ "$status" -eq 0 ]
}

@test "stale run-summary.md predating iteration → iteration_error" {
    cd "$TMPWORK"
    mkdir -p fake-bin .autospec
    # Pre-stage stale run-summary.md with old mtime.
    cat > .autospec/run-summary.md <<'EOF'
## Next steps
- stale work from prior run
EOF
    # Set very old mtime (2020-01-01).
    touch -t 202001010000 .autospec/run-summary.md
    # Fake dispatcher exits 0 but does NOT touch run-summary.md.
    cat > fake-bin/claude <<EOF
#!/usr/bin/env bash
exit 0
EOF
    chmod +x fake-bin/claude
    AUTOSPEC_HANDOFF_DISPATCHER=1 PATH="$TMPWORK/fake-bin:$PATH" \
        run bash "$REFINE" "stale summary check" \
        --continue --max-iterations 1 --rounds 1 \
        --artifact-dir "$TMPWORK/ref" --repo-root "$TMPWORK" --memory-root "$TMPWORK/none"
    [ "$status" -eq 0 ]
    loop_json="$(ls "$TMPWORK"/ref/*-loop.json | head -1)"
    [ -n "$loop_json" ]
    run grep -q '"status": "iteration_error"' "$loop_json"
    [ "$status" -eq 0 ]
}

# ── Fix 3 — dispatcher canonicalization ───────────────────────────

@test "LLM dispatcher rejects symlink whose target is in /tmp" {
    cd "$TMPWORK"
    mkdir -p safe-bin
    # Real target lives in /tmp (forbidden after canonicalization).
    cat > /tmp/evil-claude-692-$$ <<'EOF'
#!/usr/bin/env bash
echo "evil"; exit 0
EOF
    chmod +x /tmp/evil-claude-692-$$
    ln -sf "/tmp/evil-claude-692-$$" "$TMPWORK/safe-bin/claude"
    run bash "$LENS_LLM" --lens repo-grounding --prompt "x" \
        --llm-binary "$TMPWORK/safe-bin/claude"
    rm -f "/tmp/evil-claude-692-$$"
    [ "$status" -ne 0 ]
    [[ "$output" =~ "tmpdir" ]] || [[ "$output" =~ "refusing" ]]
}

@test "handoff dispatcher rejects symlink whose target is in /tmp" {
    cd "$TMPWORK"
    mkdir -p safe-bin .autospec
    cat > /tmp/evil-claude-692-h-$$ <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
    chmod +x /tmp/evil-claude-692-h-$$
    ln -sf "/tmp/evil-claude-692-h-$$" "$TMPWORK/safe-bin/claude"
    PATH="$TMPWORK/safe-bin:/usr/bin:/bin" \
        run bash "$REFINE" "handoff symlink check" --rounds 1 \
        --artifact-dir "$TMPWORK/ref" --repo-root "$TMPWORK" --memory-root "$TMPWORK/none"
    rm -f "/tmp/evil-claude-692-h-$$"
    # Should fail with rc 5 (unsafe dispatcher) — handoff_dispatcher_safe rejects.
    [ "$status" -eq 5 ]
}
