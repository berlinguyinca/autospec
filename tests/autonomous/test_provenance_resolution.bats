#!/usr/bin/env bats
# tests/autonomous/test_provenance_resolution.bats — TDD contract for
# scripts/autonomous-provenance.sh `resolve` subcommand.
#
# Design:
#   - Stubs gh as a PATH-resident subprocess (writes real temp files, not
#     process substitutions — macOS bash 3.2 [ -f <(...) ] is false).
#   - The real repo's .autospec/autospec.yml (trusted_actors: berlinguyinca)
#     is used as-is via autospec_runtime_config_get's script-dir fallback, so
#     "berlinguyinca" is the trusted login and any other login is untrusted.
#   - No real GitHub API calls: gh is fully mocked.

SCRIPT_DIR="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
PROVENANCE="$SCRIPT_DIR/scripts/autonomous-provenance.sh"
REPO="berlinguyinca/autospec"

setup() {
    TMP="$(mktemp -d -t provenance.XXXXXX)"
    mkdir -p "$TMP/bin"
    export PATH="$TMP/bin:$PATH"
}

teardown() {
    rm -rf "$TMP"
}

# Helper: build a gh stub that routes `gh api repos/.../issues/N[...]` calls
# to per-endpoint fixture files. Any endpoint without a matching fixture file
# returns an empty JSON array (comments/timeline default) so tests only need
# to supply the fixtures relevant to the scenario under test.
make_gh_stub() {
    local issue_file="$1" timeline_file="$2" comments_file="$3"
    cat > "$TMP/bin/gh" <<EOF
#!/usr/bin/env bash
# Find the request path among the args (may follow --paginate).
path=""
for a in "\$@"; do
    case "\$a" in
        repos/*) path="\$a" ;;
    esac
done
case "\$path" in
    */timeline) cat "$timeline_file" ;;
    */comments) cat "$comments_file" ;;
    *) cat "$issue_file" ;;
esac
EOF
    chmod +x "$TMP/bin/gh"
}

empty_json() {
    printf '%s' "$TMP/empty.json"
}

write_empty() {
    printf '[]' > "$TMP/empty.json"
}

# ---------------------------------------------------------------------------
# Rule 1: origin:self present, no approval marker -> self
# ---------------------------------------------------------------------------

@test "origin:self label with no approval marker resolves self" {
    write_empty
    printf '{"number":1,"labels":[{"name":"origin:self"}],"user":{"login":"autospec-bot","type":"Bot"}}' > "$TMP/issue.json"
    make_gh_stub "$TMP/issue.json" "$(empty_json)" "$(empty_json)"

    run bash "$PROVENANCE" resolve --issue 1 --repo "$REPO"
    [ "$status" -eq 0 ]
    [ "$output" = "self" ]
}

# ---------------------------------------------------------------------------
# Rule guard: approved-by-operator label last applied by an untrusted actor
# must NOT count -> falls through to self (origin:self present)
# ---------------------------------------------------------------------------

@test "approved-by-operator last applied by untrusted actor resolves self" {
    printf '[{"event":"labeled","label":{"name":"approved-by-operator"},"actor":{"login":"mallory-bot"}}]' > "$TMP/timeline.json"
    write_empty
    printf '{"number":2,"labels":[{"name":"origin:self"},{"name":"approved-by-operator"}],"user":{"login":"autospec-bot","type":"Bot"}}' > "$TMP/issue.json"
    make_gh_stub "$TMP/issue.json" "$TMP/timeline.json" "$(empty_json)"

    run bash "$PROVENANCE" resolve --issue 2 --repo "$REPO"
    [ "$status" -eq 0 ]
    [ "$output" = "self" ]
}

# ---------------------------------------------------------------------------
# Pagination boundary: --paginate flattens multi-page timeline responses into
# one array; an OLDER trusted label event followed by a NEWER untrusted
# relabel must still resolve self (the untrusted actor's re-application is
# authoritative as the LAST event, not the first).
# ---------------------------------------------------------------------------

@test "older trusted label event followed by newer untrusted relabel resolves self" {
    printf '[{"event":"labeled","label":{"name":"approved-by-operator"},"actor":{"login":"berlinguyinca"}},{"event":"labeled","label":{"name":"approved-by-operator"},"actor":{"login":"mallory-bot"}}]' > "$TMP/timeline.json"
    write_empty
    printf '{"number":11,"labels":[{"name":"origin:self"},{"name":"approved-by-operator"}],"user":{"login":"autospec-bot","type":"Bot"}}' > "$TMP/issue.json"
    make_gh_stub "$TMP/issue.json" "$TMP/timeline.json" "$(empty_json)"

    run bash "$PROVENANCE" resolve --issue 11 --repo "$REPO"
    [ "$status" -eq 0 ]
    [ "$output" = "self" ]
}

# ---------------------------------------------------------------------------
# Rule 2: trusted-actor approval label resolves operator
# ---------------------------------------------------------------------------

@test "approved-by-operator last applied by trusted actor resolves operator" {
    printf '[{"event":"labeled","label":{"name":"approved-by-operator"},"actor":{"login":"berlinguyinca"}}]' > "$TMP/timeline.json"
    write_empty
    printf '{"number":3,"labels":[{"name":"origin:self"},{"name":"approved-by-operator"}],"user":{"login":"autospec-bot","type":"Bot"}}' > "$TMP/issue.json"
    make_gh_stub "$TMP/issue.json" "$TMP/timeline.json" "$(empty_json)"

    run bash "$PROVENANCE" resolve --issue 3 --repo "$REPO"
    [ "$status" -eq 0 ]
    [ "$output" = "operator" ]
}

# ---------------------------------------------------------------------------
# Rule 2 (comment path): trusted-actor `^approve(d)?\b` comment resolves
# operator, reusing lint-issue-safety.sh's trusted-actor parsing intent.
# ---------------------------------------------------------------------------

@test "approve comment from trusted actor resolves operator" {
    write_empty
    printf '[{"body":"approved, ship it","user":{"login":"berlinguyinca"}}]' > "$TMP/comments.json"
    printf '{"number":4,"labels":[{"name":"origin:self"}],"user":{"login":"autospec-bot","type":"Bot"}}' > "$TMP/issue.json"
    make_gh_stub "$TMP/issue.json" "$(empty_json)" "$TMP/comments.json"

    run bash "$PROVENANCE" resolve --issue 4 --repo "$REPO"
    [ "$status" -eq 0 ]
    [ "$output" = "operator" ]
}

@test "approve comment from untrusted actor does not resolve operator" {
    write_empty
    printf '[{"body":"approve this please","user":{"login":"mallory-bot"}}]' > "$TMP/comments.json"
    printf '{"number":5,"labels":[{"name":"origin:self"}],"user":{"login":"autospec-bot","type":"Bot"}}' > "$TMP/issue.json"
    make_gh_stub "$TMP/issue.json" "$(empty_json)" "$TMP/comments.json"

    run bash "$PROVENANCE" resolve --issue 5 --repo "$REPO"
    [ "$status" -eq 0 ]
    [ "$output" = "self" ]
}

# ---------------------------------------------------------------------------
# Rule 3: no origin:self, human author -> operator
# ---------------------------------------------------------------------------

@test "no origin:self label with human author resolves operator" {
    write_empty
    printf '{"number":6,"labels":[],"user":{"login":"berlinguyinca","type":"User"}}' > "$TMP/issue.json"
    make_gh_stub "$TMP/issue.json" "$(empty_json)" "$(empty_json)"

    run bash "$PROVENANCE" resolve --issue 6 --repo "$REPO"
    [ "$status" -eq 0 ]
    [ "$output" = "operator" ]
}

# ---------------------------------------------------------------------------
# Rule 4: fail-closed paths -> self
# ---------------------------------------------------------------------------

@test "no origin:self label with bot author resolves self (fail closed)" {
    write_empty
    printf '{"number":7,"labels":[],"user":{"login":"autospec-bot","type":"Bot"}}' > "$TMP/issue.json"
    make_gh_stub "$TMP/issue.json" "$(empty_json)" "$(empty_json)"

    run bash "$PROVENANCE" resolve --issue 7 --repo "$REPO"
    [ "$status" -eq 0 ]
    [ "$output" = "self" ]
}

@test "missing labels field resolves self when author unknown" {
    write_empty
    printf '{"number":8,"user":{}}' > "$TMP/issue.json"
    make_gh_stub "$TMP/issue.json" "$(empty_json)" "$(empty_json)"

    run bash "$PROVENANCE" resolve --issue 8 --repo "$REPO"
    [ "$status" -eq 0 ]
    [ "$output" = "self" ]
}

@test "gh api failure resolves self (fail closed)" {
    cat > "$TMP/bin/gh" <<'EOF'
#!/usr/bin/env bash
exit 1
EOF
    chmod +x "$TMP/bin/gh"

    run bash "$PROVENANCE" resolve --issue 9 --repo "$REPO"
    [ "$status" -eq 0 ]
    [ "$output" = "self" ]
}

@test "gh api returns malformed JSON resolves self (fail closed)" {
    cat > "$TMP/bin/gh" <<'EOF'
#!/usr/bin/env bash
echo "not json"
EOF
    chmod +x "$TMP/bin/gh"

    run bash "$PROVENANCE" resolve --issue 10 --repo "$REPO"
    [ "$status" -eq 0 ]
    [ "$output" = "self" ]
}

# ---------------------------------------------------------------------------
# CLI surface
# ---------------------------------------------------------------------------

@test "--help exits 0" {
    run bash "$PROVENANCE" --help
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -qi "usage"
}

@test "missing --issue exits non-zero" {
    run bash "$PROVENANCE" resolve --repo "$REPO"
    [ "$status" -ne 0 ]
}

@test "unknown subcommand exits non-zero" {
    run bash "$PROVENANCE" bogus
    [ "$status" -ne 0 ]
}
