#!/usr/bin/env bats
# tests/explore/test_explore_gap_confirm.bats — the deterministic
# gap-confirmation stage + fail-closed verify + anti-saturation in
# explore-research-cycle.sh (precision refinement 2026-06-26).
#
# Stage order: dedup -> gap-confirm -> verify -> ROI -> synthesis -> rank.
# gap-confirm re-verifies each proposal's gap_check against the CURRENT files;
# a gap-claiming source (source-analysis/self-leverage) with no valid gap_check
# is refuted by default.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    TMP="$(mktemp -d -t gap-confirm.XXXXXX)"
    cd "$TMP"
    git init -q
    git config user.email t@t.t
    git config user.name t
    export AUTOSPEC_REPO_ROOT="$TMP"
    export PATH="$TMP/bin:$PATH"
    mkdir -p "$TMP/bin"
    printf '#!/usr/bin/env bash\nexit 1\n' > "$TMP/bin/gh"
    chmod +x "$TMP/bin/gh"
    export AUTOSPEC_RESEARCH_DIR="$TMP/fake-research"
    mkdir -p "$AUTOSPEC_RESEARCH_DIR"
    # A real file gap_checks can resolve against.
    printf 'present-token lives here\n' > probe.txt
    git add probe.txt && git commit -q -m probe
}

teardown() { rm -rf "$TMP"; }

mk() {
    cat > "$AUTOSPEC_RESEARCH_DIR/$1.sh" <<EOF
#!/usr/bin/env bash
cat <<'JSON'
$2
JSON
EOF
    chmod +x "$AUTOSPEC_RESEARCH_DIR/$1.sh"
}

run_cycle() { run bash "$REPO_ROOT/scripts/explore-research-cycle.sh" "$@"; }

# Helper: titles in final proposals as a newline list.
titles() { printf '%s' "$1" | python3 -c 'import json,sys;[print(p["title"]) for p in json.load(sys.stdin)["proposals"]]'; }
count_key() { printf '%s' "$1" | python3 -c "import json,sys;print(json.load(sys.stdin)['$2'])"; }

@test "gap-confirm: absent claim that is FOUND in file is dropped" {
    mk source-analysis '{"source":"source-analysis","proposals":[{"title":"feat: add present-token","evidence":"e","estimated_complexity":"small","confidence":0.9,"gap_check":{"kind":"absent","needle":"present-token","haystack":"probe.txt"}}]}'
    run_cycle --research-sources source-analysis --max-issues-per-round 5
    [ "$status" -eq 0 ]
    [ -z "$(titles "$output")" ]
    [ "$(count_key "$output" proposals_after_gap_confirm)" -eq 0 ]
}

@test "gap-confirm: absent claim that is genuinely MISSING is kept" {
    mk source-analysis '{"source":"source-analysis","proposals":[{"title":"feat: add telemetry-xyz","evidence":"e","estimated_complexity":"small","confidence":0.9,"gap_check":{"kind":"absent","needle":"telemetry-xyz","haystack":"probe.txt"}}]}'
    run_cycle --research-sources source-analysis --max-issues-per-round 5
    [ "$status" -eq 0 ]
    [[ "$(titles "$output")" == *"telemetry-xyz"* ]]
}

@test "gap-confirm: present claim that EXISTS is kept" {
    mk self-leverage '{"source":"self-leverage","proposals":[{"title":"feat: real call site","evidence":"e","estimated_complexity":"small","confidence":0.9,"named_consumer":"x","gap_check":{"kind":"present","needle":"present-token","haystack":"probe.txt"}}]}'
    run_cycle --research-sources self-leverage --max-issues-per-round 5
    [ "$status" -eq 0 ]
    [[ "$(titles "$output")" == *"real call site"* ]]
}

@test "gap-confirm: present claim that is ABSENT is dropped" {
    mk self-leverage '{"source":"self-leverage","proposals":[{"title":"feat: phantom call","evidence":"e","estimated_complexity":"small","confidence":0.9,"named_consumer":"x","gap_check":{"kind":"present","needle":"does-not-exist-anywhere","haystack":"probe.txt"}}]}'
    run_cycle --research-sources self-leverage --max-issues-per-round 5
    [ "$status" -eq 0 ]
    [ -z "$(titles "$output")" ]
}

@test "gap-confirm: gap-claiming source with NO gap_check is refuted by default" {
    mk source-analysis '{"source":"source-analysis","proposals":[{"title":"feat: unbacked claim","evidence":"e","estimated_complexity":"small","confidence":0.9}]}'
    run_cycle --research-sources source-analysis --max-issues-per-round 5
    [ "$status" -eq 0 ]
    [ -z "$(titles "$output")" ]
}

@test "gap-confirm: non-gap-claiming source with no gap_check passes through" {
    mk spec-vs-code '{"source":"spec-vs-code","proposals":[{"title":"feat: legacy passthrough","evidence":"e","estimated_complexity":"small","confidence":0.9}]}'
    run_cycle --research-sources spec-vs-code --max-issues-per-round 5
    [ "$status" -eq 0 ]
    [[ "$(titles "$output")" == *"legacy passthrough"* ]]
}

@test "gap-confirm: malformed gap_check is dropped (fail closed) and counted" {
    mk source-analysis '{"source":"source-analysis","proposals":[{"title":"feat: malformed","evidence":"e","estimated_complexity":"small","confidence":0.9,"gap_check":{"kind":"bogus","needle":"x","haystack":"probe.txt"}}]}'
    run_cycle --research-sources source-analysis --max-issues-per-round 5
    [ "$status" -eq 0 ]
    [ -z "$(titles "$output")" ]
    [ "$(count_key "$output" gap_check_malformed)" -ge 1 ]
}

@test "gap-confirm: haystack that escapes the repo is rejected (dropped)" {
    mk source-analysis '{"source":"source-analysis","proposals":[{"title":"feat: escape","evidence":"e","estimated_complexity":"small","confidence":0.9,"gap_check":{"kind":"absent","needle":"x","haystack":"../../../etc/hosts"}}]}'
    run_cycle --research-sources source-analysis --max-issues-per-round 5
    [ "$status" -eq 0 ]
    [ -z "$(titles "$output")" ]
    [ "$(count_key "$output" gap_check_malformed)" -ge 1 ]
}

@test "gap-confirm: newline in the needle is rejected as malformed" {
    # A multi-line needle would be OR-split by git grep (any-line match) and
    # disagree with the file branch's substring semantics — reject it.
    mk source-analysis '{"source":"source-analysis","proposals":[{"title":"feat: multiline","evidence":"e","estimated_complexity":"small","confidence":0.9,"gap_check":{"kind":"present","needle":"line one\nline two","haystack":"probe.txt"}}]}'
    run_cycle --research-sources source-analysis --max-issues-per-round 5
    [ "$status" -eq 0 ]
    [ -z "$(titles "$output")" ]
    [ "$(count_key "$output" gap_check_malformed)" -ge 1 ]
}

@test "gap-confirm: a symlink pointing outside the repo is not followed (dropped)" {
    ln -s /etc/hosts escape_link
    mk self-leverage '{"source":"self-leverage","proposals":[{"title":"feat: via symlink","evidence":"e","estimated_complexity":"small","confidence":0.9,"named_consumer":"x","gap_check":{"kind":"present","needle":"localhost","haystack":"escape_link"}}]}'
    run_cycle --research-sources self-leverage --max-issues-per-round 5
    [ "$status" -eq 0 ]
    # present-claim against an out-of-repo symlink target is unconfirmable -> dropped
    [ -z "$(titles "$output")" ]
}

@test "fail-closed: autonomous run with no verdict map files ZERO" {
    mk spec-vs-code '{"source":"spec-vs-code","proposals":[{"title":"feat: would-ship","evidence":"e","estimated_complexity":"small","confidence":0.9}]}'
    AUTOSPEC_EXPLORE_AUTONOMOUS=1 run_cycle --research-sources spec-vs-code --max-issues-per-round 5
    [ "$status" -eq 0 ]
    [ "$(count_key "$output" failclosed)" = "True" ]
    [ -z "$(titles "$output")" ]
}

@test "fail-closed: INTERACTIVE run with no verdict map still files (operator is skeptic)" {
    mk spec-vs-code '{"source":"spec-vs-code","proposals":[{"title":"feat: interactive-ships","evidence":"e","estimated_complexity":"small","confidence":0.9}]}'
    run_cycle --research-sources spec-vs-code --max-issues-per-round 5
    [ "$status" -eq 0 ]
    [ "$(count_key "$output" failclosed)" = "False" ]
    [[ "$(titles "$output")" == *"interactive-ships"* ]]
}

@test "anti-saturation: a source flooding a large multi-source pool is flagged" {
    # 20 proposals from one source + 1 from another => the flooder exceeds the
    # 40% cap and is flagged + down-sampled; the small source is untouched.
    big=$(python3 -c "import json;print(json.dumps({'source':'codebase-signals','proposals':[{'title':f'feat: flood {i}','evidence':'e','estimated_complexity':'small','confidence':0.8} for i in range(20)]}))")
    mk codebase-signals "$big"
    mk spec-vs-code '{"source":"spec-vs-code","proposals":[{"title":"feat: lone item","evidence":"e","estimated_complexity":"small","confidence":0.9}]}'
    run_cycle --research-sources codebase-signals,spec-vs-code --max-issues-per-round 50
    [ "$status" -eq 0 ]
    [[ "$(printf '%s' "$output" | python3 -c "import json,sys;print(json.load(sys.stdin)['saturated_sources'])")" == *"codebase-signals"* ]]
}

@test "anti-saturation: a small single-source pool is NOT capped" {
    mk spec-vs-code '{"source":"spec-vs-code","proposals":[{"title":"feat: one","evidence":"e","estimated_complexity":"small","confidence":0.9},{"title":"feat: two","evidence":"e","estimated_complexity":"small","confidence":0.9},{"title":"feat: three","evidence":"e","estimated_complexity":"small","confidence":0.9}]}'
    run_cycle --research-sources spec-vs-code --max-issues-per-round 20
    [ "$status" -eq 0 ]
    [ "$(count_key "$output" proposals_after_recent_filter)" -eq 3 ]
    [ "$(printf '%s' "$output" | python3 -c "import json,sys;print(len(json.load(sys.stdin)['saturated_sources']))")" -eq 0 ]
}
