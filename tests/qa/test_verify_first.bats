#!/usr/bin/env bats
# tests/qa/test_verify_first.bats
#
# Coverage for verify-first discipline (issue #643):
#   1. Finding whose symptom no longer reproduces at HEAD must be DROPped.
#   2. Finding that still reproduces must KEEP (probe exit 0).
#   3. Stale prior-cycle finding ingested from a previous QA verdict is
#      probed again; resolved → dropped to stale_dropped[].
#   4. Two clusters racing on the same tuple — the second cluster's claim
#      must fail (cluster_skip_dedup).
#   5. A simulated QA report includes the four count fields:
#      findings_emitted, verified_dropped, stale_dropped, cluster_skip_dedup.

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
VERIFY="$REPO_ROOT/scripts/qa-verify-finding.sh"
COVERAGE="$REPO_ROOT/scripts/qa-cluster-coverage.sh"

setup() {
    TMPDIR_TEST="$(mktemp -d -t qa-verify-first.XXXXXX)"
    cd "$TMPDIR_TEST"
    git init -q .
    git config user.email "test@example.com"
    git config user.name "test"
    git commit -q --allow-empty -m "init"
    export AUTOSPEC_QA_COVERAGE_FILE="$TMPDIR_TEST/.autospec/qa-cluster-coverage.json"
    mkdir -p "$TMPDIR_TEST/.autospec"
}

teardown() {
    cd /
    rm -rf "$TMPDIR_TEST"
}

@test "verify-first: missing_function symbol now present → DROP (exit 1)" {
    mkdir -p src
    cat > src/foo.py <<'PY'
def already_implemented():
    return 42
PY
    git add . && git commit -q -m "add already_implemented"
    run bash "$VERIFY" --category missing_function --symbol already_implemented --file src/foo.py
    [ "$status" -eq 1 ]
}

@test "verify-first: missing_function symbol still absent → KEEP (exit 0)" {
    mkdir -p src
    printf 'x = 1\n' > src/foo.py
    git add . && git commit -q -m "stub"
    run bash "$VERIFY" --category missing_function --symbol not_yet_implemented --file src/foo.py
    [ "$status" -eq 0 ]
}

@test "verify-first: failing_test now passes → DROP (stale prior-cycle finding)" {
    # Simulate prior-cycle ingested finding: previously the test failed.
    # At current HEAD the test passes, so the finding must be dropped.
    run bash "$VERIFY" --category failing_test --test-cmd "true"
    [ "$status" -eq 1 ]
}

@test "verify-first: failing_test still fails → KEEP" {
    run bash "$VERIFY" --category failing_test --test-cmd "false"
    [ "$status" -eq 0 ]
}

@test "verify-first: spec_mismatch still mismatched → KEEP" {
    echo "must call foobar()" > spec.md
    echo "no such symbol here" > impl.py
    git add . && git commit -q -m "spec mismatch fixture"
    run bash "$VERIFY" --category spec_mismatch --spec spec.md --impl impl.py --claim "foobar()"
    [ "$status" -eq 0 ]
}

@test "verify-first: spec_mismatch resolved → DROP" {
    echo "must call foobar()" > spec.md
    echo "result = foobar()" > impl.py
    git add . && git commit -q -m "spec match"
    run bash "$VERIFY" --category spec_mismatch --spec spec.md --impl impl.py --claim "foobar()"
    [ "$status" -eq 1 ]
}

@test "cluster-coverage: first claim wins, second on same tuple is skipped" {
    run bash "$COVERAGE" claim --cluster c1 --file src/a.py --section auth --rule MISSING_FUNCTION
    [ "$status" -eq 0 ]
    run bash "$COVERAGE" claim --cluster c2 --file src/a.py --section auth --rule MISSING_FUNCTION
    [ "$status" -eq 1 ]
}

@test "cluster-coverage: different tuple coexists" {
    run bash "$COVERAGE" claim --cluster c1 --file src/a.py --section auth --rule MISSING_FUNCTION
    [ "$status" -eq 0 ]
    run bash "$COVERAGE" claim --cluster c2 --file src/b.py --section auth --rule MISSING_FUNCTION
    [ "$status" -eq 0 ]
    run bash "$COVERAGE" list
    [ "$status" -eq 0 ]
    [[ "$output" == *"src/a.py"* ]]
    [[ "$output" == *"src/b.py"* ]]
}

@test "qa report carries findings_emitted/verified_dropped/stale_dropped/cluster_skip_dedup counts" {
    # Simulate the QA orchestrator assembling a verdict file.
    #
    # Walk a tiny set of synthetic findings, run the verify probe per
    # category, attempt to claim coverage, and tally the four count fields
    # the spec requires.
    mkdir -p src
    cat > src/foo.py <<'PY'
def already_implemented():
    return 1
PY
    git add . && git commit -q -m "fixture"

    findings_emitted=0
    verified_dropped=0
    stale_dropped=0
    cluster_skip_dedup=0

    # Current-cycle finding #1: still reproduces → KEEP, claim wins → emit.
    if bash "$VERIFY" --category missing_function --symbol absent_symbol --file src/foo.py; then
        if bash "$COVERAGE" claim --cluster c1 --file src/foo.py --section x --rule R1; then
            findings_emitted=$((findings_emitted + 1))
        else
            cluster_skip_dedup=$((cluster_skip_dedup + 1))
        fi
    else
        verified_dropped=$((verified_dropped + 1))
    fi

    # Current-cycle finding #2: resolved → DROP into verified_dropped.
    if bash "$VERIFY" --category missing_function --symbol already_implemented --file src/foo.py; then
        if bash "$COVERAGE" claim --cluster c1 --file src/foo.py --section y --rule R2; then
            findings_emitted=$((findings_emitted + 1))
        else
            cluster_skip_dedup=$((cluster_skip_dedup + 1))
        fi
    else
        verified_dropped=$((verified_dropped + 1))
    fi

    # Prior-cycle stale finding: re-probed at HEAD, now resolved → stale_dropped.
    # verify exits 0 = KEEP, exit 1 = DROP. For a resolved stale finding the
    # probe exits 1, so the else branch is the drop path.
    if bash "$VERIFY" --category failing_test --test-cmd "true"; then
        findings_emitted=$((findings_emitted + 1))
    else
        stale_dropped=$((stale_dropped + 1))
    fi

    # Second cluster races on the same tuple as finding #1 → skip_dedup.
    if bash "$COVERAGE" claim --cluster c2 --file src/foo.py --section x --rule R1; then
        findings_emitted=$((findings_emitted + 1))
    else
        cluster_skip_dedup=$((cluster_skip_dedup + 1))
    fi

    cat > .autospec/qa-verdict.json <<EOF
{
  "verdict": "PARTIAL",
  "findings_emitted": $findings_emitted,
  "verified_dropped": $verified_dropped,
  "stale_dropped": $stale_dropped,
  "cluster_skip_dedup": $cluster_skip_dedup
}
EOF
    grep -q '"findings_emitted": 1'   .autospec/qa-verdict.json
    grep -q '"verified_dropped": 1'   .autospec/qa-verdict.json
    grep -q '"stale_dropped": 1'      .autospec/qa-verdict.json
    grep -q '"cluster_skip_dedup": 1' .autospec/qa-verdict.json
}
