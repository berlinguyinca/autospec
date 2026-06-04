#!/usr/bin/env bats
# tests/refine/test_refine_overview.bats — dual-artifact renderer (issue #671).
# Covers: (a) markdown renders correctly for a 3-round happy path,
#         (b) JSON validates against schemas/autospec-refinement.schema.json,
#         (c) atomic write via `.tmp` + `mv`.

SCRIPT="${BATS_TEST_DIRNAME}/../../scripts/refine-render-overview.sh"
SCHEMA="${BATS_TEST_DIRNAME}/../../schemas/autospec-refinement.schema.json"

setup() {
    # Pin deterministic template lens — default is now auto/LLM-first (#1024),
    # but the overview render asserts deterministic-path artifact fields.
    export AUTOSPEC_REFINE_LENS_MODE=deterministic
    TEST_TMP="$(mktemp -d -t refine-render.XXXXXX)"
    OUT_DIR="$TEST_TMP/out"
    INPUT="$TEST_TMP/input.json"

    cat > "$INPUT" <<'JSON'
{
  "original_prompt": "fix login button",
  "rounds": [
    {
      "round_number": 1,
      "lens": "repo-grounding",
      "sources_used": ["AGENTS.md", "docs/specs/2026-05-28-foo.md"],
      "refined_prompt": "fix login button\n\nGrounded in AGENTS.md and docs/specs.",
      "diff_summary": "added paths/conventions",
      "word_count_delta": 7,
      "reasoning": "repo-grounding lens v1"
    },
    {
      "round_number": 2,
      "lens": "clarity-ac",
      "sources_used": ["AGENTS.md"],
      "refined_prompt": "fix login button\n\nGrounded in AGENTS.md and docs/specs.\n\n- [ ] Implementation matches the disambiguated prompt.\n- [ ] Tests cover happy + adversarial.",
      "diff_summary": "added AC checkboxes",
      "word_count_delta": 13,
      "reasoning": "clarity-ac lens v1"
    },
    {
      "round_number": 3,
      "lens": "sizing",
      "sources_used": [],
      "refined_prompt": "fix login button\n\nGrounded in AGENTS.md and docs/specs.\n\n- [ ] Implementation matches the disambiguated prompt.\n- [ ] Tests cover happy + adversarial.\n\nWithin sizing cap.",
      "diff_summary": "sizing review",
      "word_count_delta": 3,
      "reasoning": "sizing lens v1"
    }
  ],
  "final_prompt": "fix login button\n\nGrounded in AGENTS.md and docs/specs.\n\n- [ ] Implementation matches the disambiguated prompt.\n- [ ] Tests cover happy + adversarial.\n\nWithin sizing cap.",
  "status": "completed",
  "metadata": {
    "head_sha": "abcdef1",
    "timestamp": "2026-05-28T12-34-56Z",
    "rounds_requested": 3,
    "rounds_executed": 3,
    "converged_early": false,
    "degraded_rounds": [],
    "context_sparse": false,
    "handoff_target": "dry-run",
    "handoff_executed": false
  }
}
JSON
}

teardown() {
    [ -d "${TEST_TMP:-}" ] && rm -rf "$TEST_TMP"
}

@test "renderer emits both .md and .json under output-dir" {
    run bash "$SCRIPT" --json "$INPUT" --slug "fix-login-button" \
        --output-dir "$OUT_DIR" --schema "$SCHEMA"
    [ "$status" -eq 0 ]
    local md json
    md=$(ls "$OUT_DIR"/*.md | head -1)
    json=$(ls "$OUT_DIR"/*.json | head -1)
    [ -f "$md" ]
    [ -f "$json" ]
}

@test "markdown overview contains per-round headings, diff blocks, sources_used, final prompt" {
    run bash "$SCRIPT" --json "$INPUT" --slug "fix-login-button" \
        --output-dir "$OUT_DIR" --schema "$SCHEMA"
    [ "$status" -eq 0 ]
    local md
    md=$(ls "$OUT_DIR"/*.md | head -1)

    grep -q "^# autospec-refine overview" "$md"
    grep -q "^## Original prompt" "$md"
    grep -q "^## Round 1 — lens: \`repo-grounding\`" "$md"
    grep -q "^## Round 2 — lens: \`clarity-ac\`" "$md"
    grep -q "^## Round 3 — lens: \`sizing\`" "$md"
    grep -q "^## Final prompt" "$md"
    grep -q "AGENTS.md" "$md"
    grep -q "docs/specs/2026-05-28-foo.md" "$md"
    grep -q '```diff' "$md"
}

@test "JSON copy validates against schemas/autospec-refinement.schema.json" {
    run bash "$SCRIPT" --json "$INPUT" --slug "fix-login-button" \
        --output-dir "$OUT_DIR" --schema "$SCHEMA"
    [ "$status" -eq 0 ]
    local json
    json=$(ls "$OUT_DIR"/*.json | head -1)
    [ -f "$json" ]

    # Schema must exist and be JSON-parseable.
    run python3 -c "import json; json.load(open('$SCHEMA'))"
    [ "$status" -eq 0 ]

    # Validate via jsonschema if available; else assert shape.
    if python3 -c "import jsonschema" 2>/dev/null; then
        run python3 -c "
import json, jsonschema
schema = json.load(open('$SCHEMA'))
data = json.load(open('$json'))
jsonschema.validate(data, schema)
print('ok')
"
        [ "$status" -eq 0 ]
        [[ "$output" == *"ok"* ]]
    else
        run python3 -c "
import json, sys
d = json.load(open('$json'))
required = ['original_prompt','rounds','final_prompt','status','metadata']
assert all(k in d for k in required)
assert d['status'] in ('approved','completed','converged','degraded','round_cap_reached','aborted')
print('ok')
"
        [ "$status" -eq 0 ]
    fi
}

@test "atomic write: no .tmp files survive a successful run" {
    run bash "$SCRIPT" --json "$INPUT" --slug "fix-login-button" \
        --output-dir "$OUT_DIR" --schema "$SCHEMA"
    [ "$status" -eq 0 ]
    run bash -c "ls $OUT_DIR/*.tmp 2>/dev/null | wc -l | tr -d ' '"
    [ "$output" = "0" ]
}

@test "atomic write: final paths exist and .tmp paths do not" {
    run bash "$SCRIPT" --json "$INPUT" --slug "fix-login-button" \
        --output-dir "$OUT_DIR" --schema "$SCHEMA"
    [ "$status" -eq 0 ]
    local md json
    md=$(ls "$OUT_DIR"/*.md | head -1)
    json=$(ls "$OUT_DIR"/*.json | head -1)
    [ -f "$md" ]
    [ -f "$json" ]
    [ ! -f "$md.tmp" ]
    [ ! -f "$json.tmp" ]
}

@test "schema rejects payload missing required metadata fields" {
    local bad="$TEST_TMP/bad.json"
    cat > "$bad" <<'JSON'
{
  "original_prompt": "x",
  "rounds": [],
  "final_prompt": "x",
  "status": "completed",
  "metadata": {"head_sha": "abc", "timestamp": "t"}
}
JSON
    run bash "$SCRIPT" --json "$bad" --slug "bad" \
        --output-dir "$OUT_DIR" --schema "$SCHEMA"
    [ "$status" -eq 3 ]
}

@test "happy-path orchestrator output renders cleanly end-to-end" {
    # Generate a real orchestrator artifact and feed it through the renderer.
    local repo="$TEST_TMP/repo"
    local mem="$TEST_TMP/mem"
    local art="$TEST_TMP/art"
    mkdir -p "$repo/docs/specs" "$mem/proj/memory"
    cat > "$repo/AGENTS.md" <<'EOF'
# AGENTS
Follow lockstep. See scripts/refine-prompt.sh.
EOF
    cat > "$repo/docs/specs/2026-05-28-foo.md" <<'EOF'
# Foo
EOF
    run bash "${BATS_TEST_DIRNAME}/../../scripts/refine-prompt.sh" \
        "fix login button" --rounds 3 --dry-run \
        --artifact-dir "$art" --repo-root "$repo" --memory-root "$mem"
    [ "$status" -eq 0 ]
    local orch
    orch=$(ls "$art"/*.json | head -1)
    [ -f "$orch" ]
    run bash "$SCRIPT" --json "$orch" --slug "fix-login-button" \
        --output-dir "$OUT_DIR" --schema "$SCHEMA"
    [ "$status" -eq 0 ]
}
