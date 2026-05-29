#!/usr/bin/env bats
# tests/refine/test_refine_path_security.bats — issue #680.
#
# Path-security hardening for autospec-refine:
#   1. Symlinks resolving to forbidden targets are rejected.
#   2. --slug containing /, .., whitespace, or control chars rejected.
#   3. --artifact-dir and renderer --output-dir / --json are allowlist-checked.
#   4. Valid cross-directory paths still pass.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    REFINE="$REPO_ROOT/scripts/refine-prompt.sh"
    RENDER="$REPO_ROOT/scripts/refine-render-overview.sh"
    TMPDIR_T="$(mktemp -d)"
    export PATH="$REPO_ROOT/scripts:$PATH"
}

teardown() {
    [ -n "${TMPDIR_T:-}" ] && rm -rf "$TMPDIR_T"
}

@test "symlink-to-.env rejected with refine_path_violation" {
    # Create a secret file and a safe-looking symlink to it.
    printf 'SECRET=1\n' > "$TMPDIR_T/.env"
    ln -sf "$TMPDIR_T/.env" "$TMPDIR_T/safe-prompt.md"

    run bash "$REFINE" --from-file "$TMPDIR_T/safe-prompt.md" --dry-run \
        --artifact-dir "$TMPDIR_T/refinements" \
        --repo-root "$TMPDIR_T" \
        --memory-root "$TMPDIR_T/memory"

    [ "$status" -eq 3 ]
    [[ "$output" == *"refine_path_violation"* ]]
}

@test "slug with path traversal rejected by renderer" {
    # Build a minimal valid input JSON for the renderer.
    cat > "$TMPDIR_T/input.json" <<'EOF'
{"original_prompt":"x","rounds":[],"final_prompt":"x","status":"completed",
 "metadata":{"head_sha":"abc","timestamp":"2026-05-28T00-00-00Z",
   "rounds_requested":1,"rounds_executed":1,"converged_early":false,
   "degraded_rounds":[],"handoff_target":"dry-run","handoff_executed":false}}
EOF
    run bash "$RENDER" --json "$TMPDIR_T/input.json" \
        --slug "../../leak" \
        --output-dir "$TMPDIR_T/out"
    [ "$status" -eq 2 ]
    [[ "$output" == *"slug"* ]]
}

@test "slug with slash rejected by renderer" {
    cat > "$TMPDIR_T/input.json" <<'EOF'
{"original_prompt":"x","rounds":[],"final_prompt":"x","status":"completed",
 "metadata":{"head_sha":"abc","timestamp":"2026-05-28T00-00-00Z",
   "rounds_requested":1,"rounds_executed":1,"converged_early":false,
   "degraded_rounds":[],"handoff_target":"dry-run","handoff_executed":false}}
EOF
    run bash "$RENDER" --json "$TMPDIR_T/input.json" \
        --slug "evil/slug" \
        --output-dir "$TMPDIR_T/out"
    [ "$status" -eq 2 ]
    [[ "$output" == *"slug"* ]]
}

@test "artifact-dir to .git/ rejected" {
    run bash "$REFINE" "test prompt" --dry-run \
        --artifact-dir "$TMPDIR_T/.git/leak" \
        --repo-root "$TMPDIR_T" \
        --memory-root "$TMPDIR_T/memory"
    [ "$status" -eq 3 ]
    [[ "$output" == *"refine_path_violation"* ]]
}

@test "renderer output-dir under node_modules rejected" {
    cat > "$TMPDIR_T/input.json" <<'EOF'
{"original_prompt":"x","rounds":[],"final_prompt":"x","status":"completed",
 "metadata":{"head_sha":"abc","timestamp":"2026-05-28T00-00-00Z",
   "rounds_requested":1,"rounds_executed":1,"converged_early":false,
   "degraded_rounds":[],"handoff_target":"dry-run","handoff_executed":false}}
EOF
    run bash "$RENDER" --json "$TMPDIR_T/input.json" \
        --slug "valid-slug" \
        --output-dir "$TMPDIR_T/node_modules/leak"
    [ "$status" -eq 3 ]
    [[ "$output" == *"refine_path_violation"* ]]
}

@test "valid cross-directory --output path accepted" {
    mkdir -p "$TMPDIR_T/sibling"
    run bash "$REFINE" "valid cross-dir test prompt body" --dry-run \
        --output "$TMPDIR_T/sibling/legit.md" \
        --artifact-dir "$TMPDIR_T/refinements" \
        --repo-root "$TMPDIR_T" \
        --memory-root "$TMPDIR_T/memory"
    [ "$status" -eq 0 ]
    [ -f "$TMPDIR_T/sibling/legit.md" ]
}

@test "renderer --json input pointing to .git/ rejected" {
    mkdir -p "$TMPDIR_T/.git"
    cat > "$TMPDIR_T/.git/leak.json" <<'EOF'
{"original_prompt":"x","rounds":[],"final_prompt":"x","status":"completed",
 "metadata":{"head_sha":"abc","timestamp":"2026-05-28T00-00-00Z",
   "rounds_requested":1,"rounds_executed":1,"converged_early":false,
   "degraded_rounds":[],"handoff_target":"dry-run","handoff_executed":false}}
EOF
    run bash "$RENDER" --json "$TMPDIR_T/.git/leak.json" \
        --slug "valid-slug" \
        --output-dir "$TMPDIR_T/out"
    [ "$status" -eq 3 ]
    [[ "$output" == *"refine_path_violation"* ]]
}
