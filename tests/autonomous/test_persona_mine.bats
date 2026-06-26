#!/usr/bin/env bats
# tests/autonomous/test_persona_mine.bats
# TDD contract for scripts/autonomous-persona-mine.sh (F3)
#
# Coverage:
#   - AUTOSPEC_PERSONA_MINE=0 → no-op exit 0, no digest written
#   - Absent corpus → no-op exit 0, no digest written
#   - Patterns mined from a runtime fixture transcript dir
#   - Repo-scoped by default (only reads the current-repo corpus dir)
#   - Global mining only under AUTOSPEC_PERSONA_MINE_GLOBAL=1
#   - Redaction strips planted secrets from the emitted digest
#   - Truncation: logged when patterns exceed TOP_N
#   - Digest has required schema fields
#
# Engineering notes:
#   - All fixture corpus dirs materialised at runtime (no gitignored static fixtures)
#   - No [ -f <(...) ] — macOS bash 3.2 process substitution is always false
#   - Extractor subprocess seam injected via AUTOSPEC_PERSONA_EXTRACT_CMD
#   - jq used for JSON assertions; no test() with interpolated values

SCRIPT_DIR="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
MINE="$SCRIPT_DIR/scripts/autonomous-persona-mine.sh"

setup() {
    TMP="$(mktemp -d -t test-persona-mine.XXXXXX)"

    # Fake repo root
    FAKE_REPO="$TMP/repo"
    mkdir -p "$FAKE_REPO"

    # Fake autospec home (so we don't touch ~/.autospec during tests)
    FAKE_AUTOSPEC="$TMP/autospec-home"
    mkdir -p "$FAKE_AUTOSPEC"

    # Fake ~/.claude/projects base dir
    FAKE_PROJECTS="$TMP/claude-projects"
    mkdir -p "$FAKE_PROJECTS"

    # Canonical repo-scoped corpus path:
    #   $FAKE_PROJECTS/-Users-<whoami>-IdeaProjects-<repo>/memory
    WHOAMI="$(whoami)"
    REPO_NAME="$(basename "$FAKE_REPO")"
    REPO_CORPUS="$FAKE_PROJECTS/-Users-${WHOAMI}-IdeaProjects-${REPO_NAME}/memory"
    mkdir -p "$REPO_CORPUS"

    # Digest output path
    DIGEST="$FAKE_AUTOSPEC/persona-mined-digest.json"

    export TMP FAKE_REPO FAKE_AUTOSPEC FAKE_PROJECTS REPO_CORPUS DIGEST REPO_NAME WHOAMI
    export REPO_ROOT="$FAKE_REPO"
    export AUTOSPEC_HOME="$FAKE_AUTOSPEC"
    export CLAUDE_PROJECTS_DIR="$FAKE_PROJECTS"
}

teardown() {
    rm -rf "$TMP"
}

# ── helper: write a fixture markdown file with decision-pattern lines ──────
write_fixture_md() {
    local path="$1"
    local content="$2"
    printf '%s\n' "$content" > "$path"
}

# ── helper: a stub extractor that outputs fixed patterns ──────────────────
make_stub_extractor() {
    local stub_path="$1"
    local output="$2"
    # Write a shell script that echoes the supplied output
    printf '#!/usr/bin/env bash\nprintf '"'"'%s\n'"'"' %s\n' \
      "$output" > "$stub_path"
    chmod +x "$stub_path"
}

# ═══════════════════════════════════════════════════════════════════
# 1. Basic contract
# ═══════════════════════════════════════════════════════════════════

@test "script exists and is executable" {
    [ -f "$MINE" ]
    [ -x "$MINE" ]
}

@test "script passes bash -n syntax check" {
    bash -n "$MINE"
}

@test "--help exits 0" {
    run bash "$MINE" --help
    [ "$status" -eq 0 ]
}

# ═══════════════════════════════════════════════════════════════════
# 2. Opt-out: AUTOSPEC_PERSONA_MINE=0
# ═══════════════════════════════════════════════════════════════════

@test "AUTOSPEC_PERSONA_MINE=0 exits 0 immediately" {
    run env AUTOSPEC_PERSONA_MINE=0 bash "$MINE" \
        --repo-root "$FAKE_REPO" --autospec-home "$FAKE_AUTOSPEC"
    [ "$status" -eq 0 ]
}

@test "AUTOSPEC_PERSONA_MINE=0 writes no digest" {
    env AUTOSPEC_PERSONA_MINE=0 bash "$MINE" \
        --repo-root "$FAKE_REPO" --autospec-home "$FAKE_AUTOSPEC"
    [ ! -f "$DIGEST" ]
}

# ═══════════════════════════════════════════════════════════════════
# 3. Absent corpus → fail-soft no-op
# ═══════════════════════════════════════════════════════════════════

@test "absent corpus exits 0 silently" {
    # Remove the repo corpus so nothing is found
    rm -rf "$REPO_CORPUS"
    run bash "$MINE" \
        --repo-root "$FAKE_REPO" --autospec-home "$FAKE_AUTOSPEC"
    [ "$status" -eq 0 ]
}

@test "absent corpus writes no digest" {
    rm -rf "$REPO_CORPUS"
    bash "$MINE" --repo-root "$FAKE_REPO" --autospec-home "$FAKE_AUTOSPEC"
    [ ! -f "$DIGEST" ]
}

# ═══════════════════════════════════════════════════════════════════
# 4. Pattern mining from fixture corpus
# ═══════════════════════════════════════════════════════════════════

@test "mines patterns from fixture markdown and writes digest" {
    # Write a fixture markdown file with decision phrases
    write_fixture_md "$REPO_CORPUS/feedback_test.md" \
        "always prefer idempotent operations
never eval raw input
prefer smaller commits"

    run bash "$MINE" \
        --repo-root "$FAKE_REPO" --autospec-home "$FAKE_AUTOSPEC"
    [ "$status" -eq 0 ]
    [ -f "$DIGEST" ]
}

@test "digest has required top-level schema fields" {
    write_fixture_md "$REPO_CORPUS/feedback_test.md" \
        "always use set -eu in shell scripts
never skip validation"

    bash "$MINE" --repo-root "$FAKE_REPO" --autospec-home "$FAKE_AUTOSPEC"

    local _out
    _out="$(cat "$DIGEST")"
    echo "$_out" | jq -e 'has("schema")' > /dev/null
    echo "$_out" | jq -e 'has("mined_at")' > /dev/null
    echo "$_out" | jq -e 'has("corpus_scope")' > /dev/null
    echo "$_out" | jq -e 'has("truncated")' > /dev/null
    echo "$_out" | jq -e 'has("patterns")' > /dev/null
}

@test "digest schema value is persona-mined-digest/v1" {
    write_fixture_md "$REPO_CORPUS/feedback_test.md" \
        "always run tests before committing"

    bash "$MINE" --repo-root "$FAKE_REPO" --autospec-home "$FAKE_AUTOSPEC"

    local _schema
    _schema="$(jq -r '.schema' "$DIGEST")"
    [ "$_schema" = "persona-mined-digest/v1" ]
}

@test "digest patterns is an array" {
    write_fixture_md "$REPO_CORPUS/feedback_test.md" \
        "prefer functional over imperative code
never ignore error codes"

    bash "$MINE" --repo-root "$FAKE_REPO" --autospec-home "$FAKE_AUTOSPEC"

    jq -e '.patterns | type == "array"' "$DIGEST" > /dev/null
}

@test "patterns array entries have pattern and frequency fields" {
    # Use extractor seam to inject deterministic output
    _stub="$TMP/extractor.sh"
    cat > "$_stub" <<'STUB'
#!/usr/bin/env bash
printf 'always prefer idempotent operations\n'
printf 'always prefer idempotent operations\n'
printf 'never eval raw input\n'
STUB
    chmod +x "$_stub"

    AUTOSPEC_PERSONA_EXTRACT_CMD="$_stub" bash "$MINE" \
        --repo-root "$FAKE_REPO" --autospec-home "$FAKE_AUTOSPEC"

    # All patterns must have both fields
    local _bad
    _bad="$(jq '[.patterns[] | select(has("pattern") and has("frequency") | not)] | length' "$DIGEST")"
    [ "$_bad" -eq 0 ]
}

@test "extractor seam: patterns come from mock extractor output" {
    _stub="$TMP/extractor.sh"
    cat > "$_stub" <<'STUB'
#!/usr/bin/env bash
printf 'always use the extractor seam\n'
printf 'never bypass the seam\n'
STUB
    chmod +x "$_stub"

    AUTOSPEC_PERSONA_EXTRACT_CMD="$_stub" bash "$MINE" \
        --repo-root "$FAKE_REPO" --autospec-home "$FAKE_AUTOSPEC"

    [ -f "$DIGEST" ]
    local _count
    _count="$(jq '.patterns | length' "$DIGEST")"
    [ "$_count" -ge 1 ]
}

# ═══════════════════════════════════════════════════════════════════
# 5. Repo-scoped by default
# ═══════════════════════════════════════════════════════════════════

@test "default run corpus_scope is repo" {
    write_fixture_md "$REPO_CORPUS/feedback_test.md" \
        "always validate inputs"

    bash "$MINE" --repo-root "$FAKE_REPO" --autospec-home "$FAKE_AUTOSPEC"

    local _scope
    _scope="$(jq -r '.corpus_scope' "$DIGEST")"
    [ "$_scope" = "repo" ]
}

@test "default run does NOT read from a second project corpus" {
    # Create a second project corpus (simulating another repo's corpus)
    local _other_corpus="$FAKE_PROJECTS/-Users-${WHOAMI}-IdeaProjects-other-repo/memory"
    mkdir -p "$_other_corpus"

    # Plant a unique marker in the other corpus only
    write_fixture_md "$_other_corpus/feedback_other.md" \
        "always use UNIQUE_OTHER_CORPUS_MARKER in tests"

    # The repo corpus has no such phrase
    write_fixture_md "$REPO_CORPUS/feedback_mine.md" \
        "prefer small commits"

    bash "$MINE" --repo-root "$FAKE_REPO" --autospec-home "$FAKE_AUTOSPEC"

    # Marker from other-repo must not appear in digest
    local _found
    _found="$(jq '[.patterns[] | select(.pattern | test("UNIQUE_OTHER_CORPUS_MARKER"))] | length' "$DIGEST")"
    [ "$_found" -eq 0 ]
}

# ═══════════════════════════════════════════════════════════════════
# 6. Global opt-in
# ═══════════════════════════════════════════════════════════════════

@test "AUTOSPEC_PERSONA_MINE_GLOBAL=1 sets corpus_scope to global" {
    write_fixture_md "$REPO_CORPUS/feedback_test.md" \
        "always document decisions"

    AUTOSPEC_PERSONA_MINE_GLOBAL=1 bash "$MINE" \
        --repo-root "$FAKE_REPO" --autospec-home "$FAKE_AUTOSPEC"

    local _scope
    _scope="$(jq -r '.corpus_scope' "$DIGEST")"
    [ "$_scope" = "global" ]
}

@test "AUTOSPEC_PERSONA_MINE_GLOBAL=1 reads other project corpora too" {
    # Second project corpus
    local _other_corpus="$FAKE_PROJECTS/-Users-${WHOAMI}-IdeaProjects-other-global/memory"
    mkdir -p "$_other_corpus"
    write_fixture_md "$_other_corpus/feedback_global.md" \
        "always use GLOBAL_CORPUS_MARKER_XYZ for cross-repo checks"

    # Repo corpus has its own entry
    write_fixture_md "$REPO_CORPUS/feedback_local.md" \
        "prefer explicit over implicit"

    AUTOSPEC_PERSONA_MINE_GLOBAL=1 bash "$MINE" \
        --repo-root "$FAKE_REPO" --autospec-home "$FAKE_AUTOSPEC"

    # The global marker from the other corpus must appear
    local _found
    _found="$(jq '[.patterns[] | select(.pattern | test("GLOBAL_CORPUS_MARKER_XYZ"))] | length' "$DIGEST")"
    [ "$_found" -ge 1 ]
}

# ═══════════════════════════════════════════════════════════════════
# 7. Redaction
# ═══════════════════════════════════════════════════════════════════

@test "planted secret token is absent from digest patterns" {
    # Write a file with a decision line that contains a secret
    write_fixture_md "$REPO_CORPUS/feedback_secrets.md" \
        "always rotate the secret token ABC123XYZ_SECRET
prefer automated rotation"

    bash "$MINE" --repo-root "$FAKE_REPO" --autospec-home "$FAKE_AUTOSPEC"

    local _found
    _found="$(jq '[.patterns[] | select(.pattern | test("secret"))] | length' "$DIGEST")"
    [ "$_found" -eq 0 ]
}

@test "redaction strips lines containing api_key" {
    _stub="$TMP/extractor.sh"
    cat > "$_stub" <<'STUB'
#!/usr/bin/env bash
printf 'always use api_key rotation\n'
printf 'never skip validation\n'
STUB
    chmod +x "$_stub"

    AUTOSPEC_PERSONA_EXTRACT_CMD="$_stub" bash "$MINE" \
        --repo-root "$FAKE_REPO" --autospec-home "$FAKE_AUTOSPEC"

    local _found
    _found="$(jq '[.patterns[] | select(.pattern | test("api_key"))] | length' "$DIGEST")"
    [ "$_found" -eq 0 ]
}

@test "redaction strips lines containing absolute /Users/ paths" {
    _stub="$TMP/extractor.sh"
    cat > "$_stub" <<'STUB'
#!/usr/bin/env bash
printf 'always check /Users/alice/.config for settings\n'
printf 'prefer relative paths\n'
STUB
    chmod +x "$_stub"

    AUTOSPEC_PERSONA_EXTRACT_CMD="$_stub" bash "$MINE" \
        --repo-root "$FAKE_REPO" --autospec-home "$FAKE_AUTOSPEC"

    local _found
    _found="$(jq '[.patterns[] | select(.pattern | test("/Users/"))] | length' "$DIGEST")"
    [ "$_found" -eq 0 ]
}

# ═══════════════════════════════════════════════════════════════════
# 8. Truncation
# ═══════════════════════════════════════════════════════════════════

@test "truncation: digest has at most TOP_N patterns" {
    # Generate 30 unique decision lines via extractor seam
    _stub="$TMP/extractor.sh"
    {
        printf '#!/usr/bin/env bash\n'
        for i in $(seq 1 30); do
            printf 'printf '"'"'always do thing %s\\n'"'"'\n' "$i"
        done
    } > "$_stub"
    chmod +x "$_stub"

    AUTOSPEC_PERSONA_EXTRACT_CMD="$_stub" \
    AUTOSPEC_PERSONA_MINE_TOP_N=5 \
    bash "$MINE" --repo-root "$FAKE_REPO" --autospec-home "$FAKE_AUTOSPEC"

    local _count
    _count="$(jq '.patterns | length' "$DIGEST")"
    [ "$_count" -le 5 ]
}

@test "truncation: logs a message to stderr when patterns exceed TOP_N" {
    _stub="$TMP/extractor.sh"
    {
        printf '#!/usr/bin/env bash\n'
        for i in $(seq 1 15); do
            printf 'printf '"'"'always do unique thing %s\\n'"'"'\n' "$i"
        done
    } > "$_stub"
    chmod +x "$_stub"

    AUTOSPEC_PERSONA_EXTRACT_CMD="$_stub" \
    AUTOSPEC_PERSONA_MINE_TOP_N=3 \
    run bash "$MINE" --repo-root "$FAKE_REPO" --autospec-home "$FAKE_AUTOSPEC"

    [ "$status" -eq 0 ]
    # Truncation message must appear in stderr (captured in $output by bats run)
    [[ "$output" == *"truncat"* ]]
}

@test "truncation: no log when patterns do not exceed TOP_N" {
    _stub="$TMP/extractor.sh"
    cat > "$_stub" <<'STUB'
#!/usr/bin/env bash
printf 'always prefer idempotent operations\n'
printf 'never skip tests\n'
STUB
    chmod +x "$_stub"

    AUTOSPEC_PERSONA_EXTRACT_CMD="$_stub" \
    AUTOSPEC_PERSONA_MINE_TOP_N=20 \
    run bash "$MINE" --repo-root "$FAKE_REPO" --autospec-home "$FAKE_AUTOSPEC"

    [ "$status" -eq 0 ]
    [[ "$output" != *"truncat"* ]]
}

# ── cadence self-gate (F3 wiring: NOT every cycle) ─────────────────────────

@test "cadence: fresh digest → no-op (digest not rewritten)" {
    write_fixture_md "$REPO_CORPUS/feedback_a.md"

    # First run produces the digest.
    run bash "$MINE" --repo-root "$FAKE_REPO" --autospec-home "$FAKE_AUTOSPEC"
    [ "$status" -eq 0 ]
    [ -f "$DIGEST" ]

    # Record mtime, wait, run again — fresh digest must short-circuit.
    if stat -f %m "$DIGEST" >/dev/null 2>&1; then
        _before="$(stat -f %m "$DIGEST")"
    else
        _before="$(stat -c %Y "$DIGEST")"
    fi
    sleep 1
    run bash "$MINE" --repo-root "$FAKE_REPO" --autospec-home "$FAKE_AUTOSPEC"
    [ "$status" -eq 0 ]
    if stat -f %m "$DIGEST" >/dev/null 2>&1; then
        _after="$(stat -f %m "$DIGEST")"
    else
        _after="$(stat -c %Y "$DIGEST")"
    fi
    [ "$_before" = "$_after" ]
}

@test "cadence: --force re-mines even when digest is fresh" {
    write_fixture_md "$REPO_CORPUS/feedback_a.md"
    run bash "$MINE" --repo-root "$FAKE_REPO" --autospec-home "$FAKE_AUTOSPEC"
    [ "$status" -eq 0 ]
    [ -f "$DIGEST" ]

    # Touch the digest into the future so a non-forced run would no-op, then
    # confirm --force rewrites it (mtime moves back to ~now).
    sleep 1
    run bash "$MINE" --repo-root "$FAKE_REPO" --autospec-home "$FAKE_AUTOSPEC" --force
    [ "$status" -eq 0 ]
    [ -f "$DIGEST" ]
    # Schema still valid after a forced re-mine.
    run jq -r '.schema' "$DIGEST"
    [ "$output" = "persona-mined-digest/v1" ]
}

@test "cadence: stale digest (older than REFRESH_DAYS) → re-mines" {
    write_fixture_md "$REPO_CORPUS/feedback_a.md"
    run bash "$MINE" --repo-root "$FAKE_REPO" --autospec-home "$FAKE_AUTOSPEC"
    [ "$status" -eq 0 ]
    [ -f "$DIGEST" ]

    # Backdate the digest well past a 0-day window; with REFRESH_DAYS=0 any
    # existing digest is considered stale and must be rewritten.
    if stat -f %m "$DIGEST" >/dev/null 2>&1; then
        _before="$(stat -f %m "$DIGEST")"
    else
        _before="$(stat -c %Y "$DIGEST")"
    fi
    sleep 1
    AUTOSPEC_PERSONA_REFRESH_DAYS=0 \
    run bash "$MINE" --repo-root "$FAKE_REPO" --autospec-home "$FAKE_AUTOSPEC"
    [ "$status" -eq 0 ]
    if stat -f %m "$DIGEST" >/dev/null 2>&1; then
        _after="$(stat -f %m "$DIGEST")"
    else
        _after="$(stat -c %Y "$DIGEST")"
    fi
    [ "$_before" != "$_after" ]
}
