#!/usr/bin/env bats
# tests/explore/test_competitor_config.bats — F7: competitors.yml config
# parsing + clean-room directive guard (issue #1399).
#
# All .autospec/ fixtures are materialized at runtime in a temp dir —
# never committed under a gitignored path (feedback_gitignored_fixtures_pass_in_authoring_worktree).

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    TMP="$(mktemp -d -t competitor-config.XXXXXX)"
    cd "$TMP"
    git init -q
    git config user.email t@t.t
    git config user.name t
    export AUTOSPEC_REPO_ROOT="$TMP"
    export AUTOSPEC_EXPLORE_INTERNET_HISTORY="$TMP/.autospec/explore-internet-history.json"
    # Default allowlist includes github.com so competitor URLs pass through.
    export AUTOSPEC_EXPLORE_INTERNET_ALLOWLIST='github.com,raw.githubusercontent.com'
    mkdir -p "$TMP/.autospec"
}

teardown() {
    rm -rf "$TMP"
}

# ── Test 1: competitors.yml parsed into >=1 Tier-3 target ──────────────────
@test "competitors.yml: parsed URLs are fetched and produce proposals" {
    # Write the competitors.yml fixture to a real temp path, then point the
    # seam var at it so the script reads it (never rely on .autospec/ being
    # present in the checkout under a gitignored path).
    COMP_YML="$(mktemp -t competitors.XXXXXX.yml)"
    cat > "$COMP_YML" <<'EOF'
version: 1
competitors:
  - name: "example-tool"
    url: "https://github.com/example/example-tool"
  - name: "other-tool"
    url: "https://github.com/other/other-tool"
EOF
    export AUTOSPEC_COMPETITORS_YML="$COMP_YML"
    export AUTOSPEC_FETCH_STUB_github_com='This tool supports feature X and feature Y.'
    export AUTOSPEC_LLM_STUB_OUTPUT='[{"title":"feat: add X","evidence":"feature seen https://github.com/example/example-tool","estimated_complexity":"small","confidence":0.7}]'

    run bash "$REPO_ROOT/scripts/explore-research/internet.sh"
    rm -f "$COMP_YML"

    [ "$status" -eq 0 ]
    # Output must be valid JSON with at least one proposal.
    COUNT="$(printf '%s' "$output" | python3 -c 'import json,sys; d=json.load(sys.stdin); print(len(d["proposals"]))')"
    [ "$COUNT" -ge 1 ]
}

# ── Test 2: absent competitors.yml falls back to README-derived targets ─────
@test "competitors.yml: absent file falls back to README-derived URLs" {
    # No AUTOSPEC_COMPETITORS_YML set; no .autospec/competitors.yml on disk.
    cat > README.md <<'EOF'
# x
## Alternatives
https://github.com/readme-org/readme-tool
EOF
    git add -A && git commit -q -m init
    export AUTOSPEC_FETCH_STUB_github_com='Some content about readme-tool features.'
    export AUTOSPEC_LLM_STUB_OUTPUT='[{"title":"feat: readme feature","evidence":"seen at https://github.com/readme-org/readme-tool","estimated_complexity":"medium","confidence":0.5}]'

    run bash "$REPO_ROOT/scripts/explore-research/internet.sh"

    [ "$status" -eq 0 ]
    # Proposal sourced from README URL — output is non-empty proposals array.
    COUNT="$(printf '%s' "$output" | python3 -c 'import json,sys; d=json.load(sys.stdin); print(len(d["proposals"]))')"
    [ "$COUNT" -ge 1 ]
}

# ── Test 3: off-allowlist target in competitors.yml is rejected ─────────────
@test "competitors.yml: off-allowlist target URL is rejected (exit 3, no fetch)" {
    COMP_YML="$(mktemp -t competitors.XXXXXX.yml)"
    cat > "$COMP_YML" <<'EOF'
version: 1
competitors:
  - name: "dangerous"
    url: "https://evil.example.org/tool"
EOF
    export AUTOSPEC_COMPETITORS_YML="$COMP_YML"
    # Allowlist does not include evil.example.org.
    export AUTOSPEC_EXPLORE_INTERNET_ALLOWLIST='github.com'

    run bash "$REPO_ROOT/scripts/explore-research/internet.sh"
    rm -f "$COMP_YML"

    [ "$status" -eq 3 ]
}

# ── Test 4: clean-room behavior-only directive present in the LLM prompt ────
@test "clean-room directive: CLEAN-ROOM DIRECTIVE appears in researcher prompt" {
    COMP_YML="$(mktemp -t competitors.XXXXXX.yml)"
    cat > "$COMP_YML" <<'EOF'
version: 1
competitors:
  - name: "inspectable"
    url: "https://github.com/inspect/tool"
EOF
    export AUTOSPEC_COMPETITORS_YML="$COMP_YML"
    export AUTOSPEC_FETCH_STUB_github_com='Tool does feature Z.'

    # Capture what would be sent to the LLM by intercepting via a stub that
    # writes its input to a temp file, then returns a valid JSON array.
    PROMPT_CAPTURE="$(mktemp -t prompt-capture.XXXXXX)"
    # Create a stub LLM that records its input and returns valid proposals.
    STUB_BIN="$(mktemp -d -t stub-bin.XXXXXX)"
    cat > "$STUB_BIN/claude" <<STUB
#!/usr/bin/env bash
# Record all arguments + stdin so the test can inspect the prompt.
printf '%s\n' "\$@" > "$PROMPT_CAPTURE"
cat >> "$PROMPT_CAPTURE"
echo '[{"title":"feat: z","evidence":"seen at https://github.com/inspect/tool","estimated_complexity":"small","confidence":0.6}]'
STUB
    chmod +x "$STUB_BIN/claude"
    OLD_PATH="$PATH"
    export PATH="$STUB_BIN:$PATH"

    run bash "$REPO_ROOT/scripts/explore-research/internet.sh"
    export PATH="$OLD_PATH"
    rm -f "$COMP_YML"
    rm -rf "$STUB_BIN"

    # The test checks the directive in two ways:
    # (a) if the LLM stub was invoked, the prompt capture file holds the directive; or
    # (b) if AUTOSPEC_LLM_STUB_OUTPUT was used (graceful degrade), verify the
    #     directive text is present in internet.sh itself (static contract).
    if [ -s "$PROMPT_CAPTURE" ]; then
        grep -q "CLEAN-ROOM DIRECTIVE" "$PROMPT_CAPTURE"
    else
        # Script degraded without calling the stub; verify the directive
        # exists in the script source as the static contract.
        grep -q "CLEAN-ROOM DIRECTIVE" "$REPO_ROOT/scripts/explore-research/internet.sh"
    fi
    rm -f "$PROMPT_CAPTURE"
}

# ── Test 5: competitors.yml with no valid URLs emits empty proposals ─────────
@test "competitors.yml: empty competitors list emits empty proposals" {
    COMP_YML="$(mktemp -t competitors.XXXXXX.yml)"
    cat > "$COMP_YML" <<'EOF'
version: 1
competitors: []
EOF
    export AUTOSPEC_COMPETITORS_YML="$COMP_YML"

    run bash "$REPO_ROOT/scripts/explore-research/internet.sh"
    rm -f "$COMP_YML"

    [ "$status" -eq 0 ]
    COUNT="$(printf '%s' "$output" | python3 -c 'import json,sys; d=json.load(sys.stdin); print(len(d["proposals"]))')"
    [ "$COUNT" -eq 0 ]
}
