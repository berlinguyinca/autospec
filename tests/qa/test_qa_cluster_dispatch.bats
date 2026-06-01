#!/usr/bin/env bats
# tests/qa/test_qa_cluster_dispatch.bats — fixtures for issue #730.
#
# Asserts the cluster-dispatch contract:
#   - 8 canonical cluster files exist under skills/autospec-qa/clusters/.
#   - scripts/qa-cluster-dispatch.sh exists, is executable, and bash -n clean.
#   - --dry-run lists all 8 clusters by default.
#   - Running the dispatcher produces a valid qa-verdict.json envelope
#     with cluster_count == active set size.
#   - --cluster filters to a single cluster.
#   - --skip-cluster removes a cluster from the active set.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    DISPATCH="$REPO_ROOT/scripts/qa-cluster-dispatch.sh"
    CLUSTERS_DIR="$REPO_ROOT/skills/autospec-qa/clusters"
    TMPDIR_RUN="$(mktemp -d -t qa-cluster-dispatch.XXXXXX)"
    OUT="$TMPDIR_RUN/qa-verdict.json"
}

teardown() {
    rm -rf "$TMPDIR_RUN"
}

@test "8 canonical cluster files exist under skills/autospec-qa/clusters/" {
    for c in spec-traceability functional-coverage backend-integration \
             reliability-contract legacy-and-cleanup \
             benchmark-and-outsourcing accessibility-and-responsive \
             production-incidents; do
        [ -f "$CLUSTERS_DIR/$c.md" ] || {
            echo "missing cluster file: $c.md" >&2
            return 1
        }
    done
    # Exactly 8, no extras (guards against drift).
    count="$(find "$CLUSTERS_DIR" -maxdepth 1 -type f -name '*.md' | wc -l | tr -d ' ')"
    [ "$count" = "8" ]
}

@test "qa-cluster-dispatch.sh exists, is executable, and bash -n clean" {
    [ -x "$DISPATCH" ]
    bash -n "$DISPATCH"
}

@test "--dry-run lists all 8 canonical clusters" {
    run "$DISPATCH" --dry-run
    [ "$status" -eq 0 ]
    lines_count="$(printf '%s\n' "$output" | wc -l | tr -d ' ')"
    [ "$lines_count" = "8" ]
    printf '%s\n' "$output" | grep -q spec-traceability
    printf '%s\n' "$output" | grep -q production-incidents
}

@test "default run writes a valid qa-verdict.json with cluster_count=8" {
    run "$DISPATCH" --out "$OUT"
    [ "$status" -eq 0 ]
    [ -f "$OUT" ]
    grep -q '"cluster_count":8' "$OUT"
    grep -q '"verdict":"PASS"' "$OUT"
    grep -q 'spec-traceability' "$OUT"
}

@test "--cluster filters to a single cluster" {
    run "$DISPATCH" --cluster functional-coverage --out "$OUT"
    [ "$status" -eq 0 ]
    grep -q '"cluster_count":1' "$OUT"
    grep -q 'functional-coverage' "$OUT"
    ! grep -q 'spec-traceability' "$OUT"
}

@test "--skip-cluster removes a cluster from the active set" {
    run "$DISPATCH" --skip-cluster spec-traceability --out "$OUT"
    [ "$status" -eq 0 ]
    grep -q '"cluster_count":7' "$OUT"
    ! grep -q '"cluster":"spec-traceability"' "$OUT"
}

@test "missing clusters dir exits 3" {
    run "$DISPATCH" --clusters-dir /nonexistent/dir --out "$OUT"
    [ "$status" -eq 3 ]
}
