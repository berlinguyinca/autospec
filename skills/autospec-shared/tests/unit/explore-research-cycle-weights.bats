#!/usr/bin/env bats
# explore-research-cycle-weights.bats — proves the research cycle ranks
# proposals by DYNAMIC ledger-derived source weights, while staying byte-for-byte
# identical to the old static path when there is no ledger / no weights script.
#
# Strategy: two fixture researchers (spec-vs-code, internet) each emit ONE
# proposal with IDENTICAL confidence + complexity, so the RANK ORDER is decided
# purely by the per-source weight. score = confidence * weight / complexity_scale.
#
# Tests:
#   1. No ledger / weights script absent -> static priors -> spec-vs-code (1.0)
#      outranks internet (0.4); chosen scores match the OLD hardcoded math.
#   2. Dynamic flip -> a ledger where internet ships clean and spec-vs-code
#      repeatedly fails pushes internet's weight above spec-vs-code's, flipping
#      the ranking. Closes the loop.
#   3. Weights script pointed at a nonexistent path -> falls back to the static
#      priors (parity with test 1).

SHARED_DIR="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
REPO_ROOT="$(cd "$SHARED_DIR/../.." && pwd)"
CYCLE_SH="$REPO_ROOT/scripts/explore-research-cycle.sh"
WEIGHTS_SH="$SHARED_DIR/scripts/explore-source-weights.sh"
LEDGER_SH="$SHARED_DIR/scripts/explore-ledger.sh"

setup() {
    TEST_TMP="$(mktemp -d)"
    RESEARCH_DIR="$TEST_TMP/researchers"
    mkdir -p "$RESEARCH_DIR"
    LEDGER="$TEST_TMP/l.jsonl"

    # Two fixture researchers emitting the EXACT contract the python expects:
    #   {source, proposals:[{title, evidence, estimated_complexity, confidence}]}
    # Equal confidence (0.8) + complexity (small) => weight is the only variable.
    _make_researcher spec-vs-code "Add spec-vs-code proposal"
    _make_researcher internet     "Add internet proposal"
}
teardown() {
    rm -rf "$TEST_TMP"
}

# $1=source $2=title — write a researcher script that prints one proposal.
_make_researcher() {
    local src="$1" title="$2"
    cat > "$RESEARCH_DIR/$src.sh" <<EOF
#!/usr/bin/env bash
cat <<'JSON'
{"source":"$src","proposals":[{"title":"$title","evidence":"e","estimated_complexity":"small","confidence":0.8}]}
JSON
EOF
    chmod +x "$RESEARCH_DIR/$src.sh"
}

# Run the cycle over the two fixture researchers. Extra args forwarded.
run_cycle() {
    AUTOSPEC_REPO_ROOT="$REPO_ROOT" \
    AUTOSPEC_RESEARCH_DIR="$RESEARCH_DIR" \
    AUTOSPEC_TEST_RECENT_TITLES="" \
    run bash "$CYCLE_SH" --research-sources spec-vs-code,internet "$@"
}

# Source of the proposal at rank index $2 (0-based) in cycle JSON $1.
source_at() { jq -r --argjson i "$2" '.proposals[$i].source' <<<"$1"; }
# Score of the source named $2 in cycle JSON $1.
score_of() { jq -r --arg s "$2" '.proposals[] | select(.source==$s) | .score' <<<"$1"; }

# ── 1. No ledger / weights absent -> static priors, spec-vs-code first ────────
@test "no ledger: spec-vs-code (1.0) outranks internet (0.4); scores match old math" {
    # Force the weights bin to a nonexistent path so resolution yields no dynamic
    # weights => DEFAULT_SRC_WEIGHTS path (the pre-change behavior).
    AUTOSPEC_EXPLORE_WEIGHTS_BIN="$TEST_TMP/nope.sh" run_cycle
    [ "$status" -eq 0 ]

    [ "$(source_at "$output" 0)" = "spec-vs-code" ]
    [ "$(source_at "$output" 1)" = "internet" ]

    # OLD hardcoded math: score = conf * weight / complexity_scale.
    #   conf=0.8, complexity small => scale 1.0
    #   spec-vs-code: 0.8 * 1.0 / 1.0 = 0.8
    #   internet:     0.8 * 0.4 / 1.0 = 0.32
    [ "$(score_of "$output" spec-vs-code)" = "0.8" ]
    [ "$(score_of "$output" internet)" = "0.32" ]
}

# ── 2. Dynamic flip: ledger learning makes internet outrank spec-vs-code ──────
@test "dynamic flip: clean internet + failing spec-vs-code reverses the ranking" {
    # internet ships clean repeatedly; spec-vs-code repeatedly fails.
    # With alpha=5 (weights default):
    #   internet     = (12 + 5*0.4)/(12+5) = 14/17 = 0.8235
    #   spec-vs-code = (0  + 5*1.0)/(12+5) = 5/17  = 0.2941
    # internet weight now exceeds spec-vs-code => ranking flips.
    _append_n internet     12 merged_clean 100
    _append_n spec-vs-code 12 qa_failed    300

    AUTOSPEC_EXPLORE_WEIGHTS_BIN="$WEIGHTS_SH" run_cycle --ledger "$LEDGER"
    [ "$status" -eq 0 ]

    [ "$(source_at "$output" 0)" = "internet" ]
    [ "$(source_at "$output" 1)" = "spec-vs-code" ]

    # Sanity: internet's score must exceed spec-vs-code's now.
    is="$(score_of "$output" internet)"
    ss="$(score_of "$output" spec-vs-code)"
    [ "$(python3 -c "print($is > $ss)")" = "True" ]
}

# ── 3. Weights script absent/broken -> static fallback (parity with test 1) ───
@test "weights script at nonexistent path -> static priors fallback" {
    # Even with a ledger that WOULD flip the ranking, an unreachable weights bin
    # must leave behavior identical to the static path.
    _append_n internet     12 merged_clean 100
    _append_n spec-vs-code 12 qa_failed    300

    AUTOSPEC_EXPLORE_WEIGHTS_BIN="$TEST_TMP/does-not-exist.sh" run_cycle --ledger "$LEDGER"
    [ "$status" -eq 0 ]

    [ "$(source_at "$output" 0)" = "spec-vs-code" ]
    [ "$(source_at "$output" 1)" = "internet" ]
    [ "$(score_of "$output" spec-vs-code)" = "0.8" ]
    [ "$(score_of "$output" internet)" = "0.32" ]
}

# Append N ledger records for a source with a distinct issue per record.
# $1=source $2=count $3=outcome $4=issue_base
_append_n() {
    local source="$1" count="$2" outcome="$3" base="$4" i issue rec
    for i in $(seq 1 "$count"); do
        issue=$((base + i))
        rec="$(printf '{"round":1,"source":"%s","title":"t%s","norm_title":"t%s","complexity":"small","confidence":0.8,"issue":%s,"pr":0,"outcome":"%s","reason":"","ts":"2026-06-14T00:00:00Z"}' \
            "$source" "$issue" "$issue" "$issue" "$outcome")"
        bash "$LEDGER_SH" --ledger "$LEDGER" --append "$rec"
    done
}
