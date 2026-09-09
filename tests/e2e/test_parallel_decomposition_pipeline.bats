#!/usr/bin/env bats
# tests/e2e/test_parallel_decomposition_pipeline.bats
#
# End-to-end pipeline validation for parallel decomposition (issue #3835).
# Source spec: docs/specs/2026-09-08-parallel-decomposition-fleet-saturation.md
#   §19/§20 (DAG analyzer + graph metrics), §22 (DAG lint), §34 AC-6 (cycle
#   check before issue creation), §35.5 (capacity tests at 10/32/50/100),
#   §35.6 (three real-world golden spec fixtures).
#
# Drives spec -> proposed decomposition -> DAG analysis -> issue lint using
# real binaries and real files in a temp dir. No network, no GitHub writes,
# no mocks. The analyzer is the script-first form permitted by spec §19:
# deterministic Kahn-wave metrics over the proposed hard-dependency graph.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    FIXTURES="$REPO_ROOT/tests/fixtures/parallel-decomposition"
    LINT_ISSUE="$REPO_ROOT/scripts/lint-issue.sh"
    WORK="$(mktemp -d /tmp/autospec-pdecompose-e2e-XXXXXX)"
    FILED_DIR="$WORK/filed"
    mkdir -p "$FILED_DIR"
    SPEC_DIRS=(cli-feature ui-feature cross-cutting)
}

teardown() {
    rm -rf "$WORK"
}

# analyze_graph <proposed-graph.json> <capacity>
#
# Deterministic offline DAG analyzer (spec §19/§20). Edges are [from, to]
# pairs meaning "to hard-depends on from". Emits metrics JSON on stdout.
# Exits 3 on a cycle (AS-DAG-010) so the caller aborts before filing.
analyze_graph() {
    local graph="$1" capacity="$2"
    local node_count edge_count
    node_count="$(jq '.nodes | length' "$graph")"
    edge_count="$(jq '.hard_edges | length' "$graph")"

    local -A indeg done depth
    local node
    while IFS= read -r node; do
        indeg["$node"]=0
        done["$node"]=0
        depth["$node"]=0
    done < <(jq -r '.nodes[].id' "$graph")

    local -a froms=() tos=()
    local from to
    while IFS=$'\t' read -r from to; do
        [ -z "$from" ] && continue
        froms+=("$from")
        tos+=("$to")
        indeg["$to"]=$(( ${indeg["$to"]} + 1 ))
    done < <(jq -r '.hard_edges[] | @tsv' "$graph")

    local initial_width=0 maximum_width=0 critical_path=1
    local remaining="$node_count" wave=0
    local -a ready=()
    local -A edge_used=()
    while [ "$remaining" -gt 0 ]; do
        ready=()
        for node in "${!indeg[@]}"; do
            if [ "${done[$node]}" -eq 0 ] && [ "${indeg[$node]}" -eq 0 ]; then
                ready+=("$node")
            fi
        done
        if [ "${#ready[@]}" -eq 0 ]; then
            printf 'DAG cycle detected: %d of %d nodes remain unschedulable (AS-DAG-010)\n' \
                "$remaining" "$node_count" >&2
            return 3
        fi
        wave=$((wave + 1))
        if [ "${#ready[@]}" -gt "$maximum_width" ]; then
            maximum_width="${#ready[@]}"
        fi
        if [ "$wave" -eq 1 ]; then
            initial_width="${#ready[@]}"
        fi
        for node in "${ready[@]}"; do
            done["$node"]=1
        done
        local i
        for i in "${!froms[@]}"; do
            # Consume each edge exactly once, when its source node finishes,
            # so a sink with several predecessors is not decremented again on
            # later waves.
            if [ "${edge_used[$i]:-0}" -eq 0 ] && [ "${done[${froms[$i]}]}" -eq 1 ]; then
                edge_used["$i"]=1
                indeg["${tos[$i]}"]=$(( ${indeg["${tos[$i]}"]} - 1 ))
                if [ $(( ${depth[${froms[$i]}]} + 1 )) -gt "${depth[${tos[$i]}]}" ]; then
                    depth["${tos[$i]}"]=$(( ${depth[${froms[$i]}]} + 1 ))
                fi
            fi
        done
        remaining=$(( remaining - ${#ready[@]} ))
    done

    for node in "${!depth[@]}"; do
        if [ $(( ${depth[$node]} + 1 )) -gt "$critical_path" ]; then
            critical_path=$(( ${depth[$node]} + 1 ))
        fi
    done

    # Metrics per spec §20: initial width = root count; fleet saturation =
    # min(width, C) / C.
    jq -n \
        --argjson issue_count "$node_count" \
        --argjson hard_edge_count "$edge_count" \
        --argjson capacity "$capacity" \
        --argjson initial_width "$initial_width" \
        --argjson maximum_width "$maximum_width" \
        --argjson critical_path_length "$critical_path" \
        '{
            issue_count: $issue_count,
            hard_edge_count: $hard_edge_count,
            capacity: $capacity,
            root_count: $initial_width,
            initial_width: $initial_width,
            maximum_width: $maximum_width,
            critical_path_length: $critical_path_length,
            serialization_ratio: ($hard_edge_count / (if ($issue_count * ($issue_count - 1)) / 2 < 1 then 1 else ($issue_count * ($issue_count - 1)) / 2 end)),
            initial_saturation: ((if $initial_width < $capacity then $initial_width else $capacity end) / $capacity),
            peak_saturation: ((if $maximum_width < $capacity then $maximum_width else $capacity end) / $capacity)
        }'
}

# run_pipeline <spec-fixture-dir> <capacity>
#
# Offline pipeline: spec -> proposed decomposition -> DAG analysis -> file
# generated bodies (Phase 3.7 stand-in in a temp dir; no GitHub writes).
# DAG analysis runs BEFORE anything is filed (spec §34 AC-6): a cycle aborts
# with a non-zero exit and zero filed issues.
run_pipeline() {
    local spec_dir="$1" capacity="$2"
    local graph="$spec_dir/proposed-graph.json"

    if [ ! -f "$spec_dir/spec.md" ]; then
        printf 'pipeline: missing spec.md in %s\n' "$spec_dir" >&2
        return 1
    fi
    if [ ! -f "$graph" ]; then
        printf 'pipeline: missing proposed-graph.json in %s\n' "$spec_dir" >&2
        return 1
    fi

    # Decomposition integrity: every node has a generated body on disk.
    local body missing=""
    while IFS= read -r body; do
        if [ -n "$body" ] && [ ! -f "$spec_dir/$body" ]; then
            missing="$missing $body"
        fi
    done < <(jq -r '.nodes[].body' "$graph")
    if [ -n "$missing" ]; then
        printf 'pipeline: missing generated bodies:%s\n' "$missing" >&2
        return 1
    fi

    # DAG analysis BEFORE filing.
    local metrics rc=0
    metrics="$(analyze_graph "$graph" "$capacity")" || rc=$?
    if [ "$rc" -ne 0 ]; then
        return "$rc"
    fi

    # Phase 3.7 stand-in: file the generated bodies (offline, temp dir only).
    while IFS= read -r body; do
        [ -n "$body" ] && cp "$spec_dir/$body" "$FILED_DIR/"
    done < <(jq -r '.nodes[].body' "$graph")

    printf '%s\n' "$metrics"
}

# check_body_graph_consistency <spec-fixture-dir>
#
# Body-side DAG lint invariants (spec §22) against real files:
#   AS-DAG-009: the body's ## Dependencies section must equal the graph's
#               hard edges into the node (machine metadata agrees with
#               Markdown).
#   AS-DAG-001: every hard dependency carries a justification line naming a
#               recognized reason code (spec §12) or an artifact.
check_body_graph_consistency() {
    local spec_dir="$1"
    local graph="$spec_dir/proposed-graph.json"
    local id body body_path body_set graph_set deps line
    while IFS=$'\t' read -r id body; do
        body_path="$spec_dir/$body"
        body_set="$(awk '/^## Dependencies[[:space:]]*$/{d=1;next} d&&/^## /{exit} d{print}' \
            "$body_path" | grep -oE 'issue #[0-9]+' | grep -oE '[0-9]+' | sort -u)"
        graph_set="$(jq -r --arg id "$id" '.hard_edges[] | select(.[1] == $id) | .[0]' \
            "$graph" | sort -u)"
        if [ "$body_set" != "$graph_set" ]; then
            printf 'AS-DAG-009: dependency mismatch for %s: body=[%s] graph=[%s]\n' \
                "$id" "${body_set//$'\n'/,}" "${graph_set//$'\n'/,}"
            return 1
        fi
        while IFS= read -r deps; do
            [ -z "$deps" ] && continue
            line="$(awk -v dep="$deps" \
                '/^## Dependency justification[[:space:]]*$/{j=1;next} j&&/^## /{exit} j{print}' \
                "$body_path" | grep -E "^- #$deps[[:space:]]")"
            if [ -z "$line" ]; then
                printf 'AS-DAG-001: issue %s depends on #%s with no justification line\n' \
                    "$id" "$deps"
                return 1
            fi
            if ! printf '%s\n' "$line" | grep -qE \
                '\b(consumes-new-interface|consumes-new-type|consumes-new-schema|consumes-migration|consumes-generated-artifact|requires-structural-migration|requires-new-protocol|verification-requires-predecessor|external-prerequisite)\b|\`[^\`]+\`'; then
                printf 'AS-DAG-001: issue %s justification for #%s names no recognized reason code or artifact\n' \
                    "$id" "$deps"
                return 1
            fi
        done <<< "$body_set"
    done < <(jq -r '.nodes[] | [.id, .body] | @tsv' "$graph")
    return 0
}

# expected_filed_count — total nodes across the three golden specs.
expected_filed_count() {
    local spec total=0 n
    for spec in "${SPEC_DIRS[@]}"; do
        n="$(jq '.nodes | length' "$FIXTURES/$spec/proposed-graph.json")"
        total=$((total + n))
    done
    printf '%s' "$total"
}

# run_capacity_case <capacity>
#
# Run the pipeline for all three golden specs at one capacity, assert the
# recorded metrics, and count filed bodies.
run_capacity_case() {
    local cap="$1"
    rm -rf "$FILED_DIR"
    mkdir -p "$FILED_DIR"
    local spec filed expected
    for spec in "${SPEC_DIRS[@]}"; do
        run run_pipeline "$FIXTURES/$spec" "$cap"
        if [ "$status" -ne 0 ]; then
            printf 'pipeline failed for %s at capacity %s (exit %s): %s\n' \
                "$spec" "$cap" "$status" "$output"
            return 1
        fi
        # Spec §7.1: initial width meets the min(C, ceil(0.6N)) target.
        # Spec §20: widths, critical path and saturations are well formed.
        if ! printf '%s\n' "$output" | jq -e "
            .capacity == $cap
            and .initial_width == .root_count
            and .maximum_width >= .initial_width
            and .critical_path_length >= 1
            and (.initial_width >= (if $cap < ((.issue_count * 6 + 9) / 10 | floor) then $cap else ((.issue_count * 6 + 9) / 10 | floor) end))
            and (.initial_saturation == ((if .initial_width < .capacity then .initial_width else .capacity end) / .capacity))
            and (.peak_saturation == ((if .maximum_width < .capacity then .maximum_width else .capacity end) / .capacity))
            and (.serialization_ratio > 0)
            and (.serialization_ratio <= 1)
        " > /dev/null; then
            printf 'bad metrics for %s at capacity %s: %s\n' "$spec" "$cap" "$output"
            return 1
        fi
        printf '%s\n' "$output" > "$WORK/metrics-$spec-c$cap.json"
    done
    filed="$(find "$FILED_DIR" -name '*.md' | wc -l | tr -d ' ')"
    expected="$(expected_filed_count)"
    if [ "$filed" -ne "$expected" ]; then
        printf 'filed %s bodies at capacity %s, expected %s\n' "$filed" "$cap" "$expected"
        return 1
    fi
    return 0
}

@test "three golden spec fixtures exist with spec, proposed graph and generated bodies" {
    local spec
    for spec in "${SPEC_DIRS[@]}"; do
        [ -f "$FIXTURES/$spec/spec.md" ]
        [ -f "$FIXTURES/$spec/proposed-graph.json" ]
        jq -e '.nodes | length > 0' "$FIXTURES/$spec/proposed-graph.json" > /dev/null
        [ -n "$(find "$FIXTURES/$spec/issues" -name '*.md' 2>/dev/null)" ]
    done
    [ -f "$FIXTURES/cyclic/proposed-graph.json" ]
}

@test "capacity 10: pipeline analyzes all 3 specs and files every generated body" {
    run run_capacity_case 10
    if [ "$status" -ne 0 ]; then
        printf 'capacity 10 case failed (exit %s):\n%s\n' "$status" "$output"
    fi
    [ "$status" -eq 0 ]
}

@test "capacity 32: pipeline analyzes all 3 specs and files every generated body" {
    run run_capacity_case 32
    if [ "$status" -ne 0 ]; then
        printf 'capacity 32 case failed (exit %s):\n%s\n' "$status" "$output"
    fi
    [ "$status" -eq 0 ]
}

@test "capacity 50: pipeline analyzes all 3 specs and files every generated body" {
    run run_capacity_case 50
    if [ "$status" -ne 0 ]; then
        printf 'capacity 50 case failed (exit %s):\n%s\n' "$status" "$output"
    fi
    [ "$status" -eq 0 ]
}

@test "capacity 100: pipeline analyzes all 3 specs and files every generated body" {
    run run_capacity_case 100
    if [ "$status" -ne 0 ]; then
        printf 'capacity 100 case failed (exit %s):\n%s\n' "$status" "$output"
    fi
    [ "$status" -eq 0 ]
}

@test "initial width never falls as capacity rises 10 -> 32 -> 50 -> 100" {
    local caps=(10 32 50 100) spec cap prev cur
    for spec in "${SPEC_DIRS[@]}"; do
        prev=0
        for cap in "${caps[@]}"; do
            run run_pipeline "$FIXTURES/$spec" "$cap"
            if [ "$status" -ne 0 ]; then
                printf 'pipeline failed for %s at capacity %s: %s\n' "$spec" "$cap" "$output"
                return 1
            fi
            cur="$(printf '%s\n' "$output" | jq -r '.initial_width')"
            if [ "$cur" -lt "$prev" ]; then
                printf 'initial width fell for %s: %s -> %s at capacity %s\n' \
                    "$spec" "$prev" "$cur" "$cap"
                return 1
            fi
            prev="$cur"
        done
    done
}

@test "initial width recorded at capacity 100 is at least that at capacity 10" {
    local spec w10 w100
    for spec in "${SPEC_DIRS[@]}"; do
        run run_pipeline "$FIXTURES/$spec" 10
        [ "$status" -eq 0 ]
        w10="$(printf '%s\n' "$output" | jq -r '.initial_width')"
        run run_pipeline "$FIXTURES/$spec" 100
        [ "$status" -eq 0 ]
        w100="$(printf '%s\n' "$output" | jq -r '.initial_width')"
        if [ "$w100" -lt "$w10" ]; then
            printf 'initial width at capacity 100 (%s) < capacity 10 (%s) for %s\n' \
                "$w100" "$w10" "$spec"
            return 1
        fi
    done
}

@test "cyclic fixture aborts with non-zero exit before filing and files 0 issues" {
    rm -rf "$FILED_DIR"
    mkdir -p "$FILED_DIR"
    run run_pipeline "$FIXTURES/cyclic" 32
    [ "$status" -ne 0 ]
    printf '%s\n' "$output" | grep -qi 'cycle'
    local filed
    filed="$(find "$FILED_DIR" -name '*.md' | wc -l | tr -d ' ')"
    [ "$filed" -eq 0 ]
}

@test "every generated body passes scripts/lint-issue.sh with exit 0" {
    local spec body failures=0
    for spec in "${SPEC_DIRS[@]}" cyclic; do
        for body in "$FIXTURES/$spec"/issues/*.md; do
            [ -f "$body" ] || continue
            run bash "$LINT_ISSUE" "$body"
            if [ "$status" -ne 0 ]; then
                printf 'lint-issue.sh failed on %s (exit %s):\n%s\n' \
                    "$body" "$status" "$output"
                failures=$((failures + 1))
            fi
        done
    done
    [ "$failures" -eq 0 ]
}

@test "body dependency sections agree with the proposed graphs and carry DAG-lint reasons" {
    local spec
    for spec in "${SPEC_DIRS[@]}" cyclic; do
        run check_body_graph_consistency "$FIXTURES/$spec"
        if [ "$status" -ne 0 ]; then
            printf 'body/graph DAG-lint inconsistency in %s:\n%s\n' "$spec" "$output"
            return 1
        fi
    done
}

@test "graph metrics match the golden per-spec values at capacity 32" {
    run run_pipeline "$FIXTURES/cli-feature" 32
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | jq -e '
        .issue_count == 10 and .hard_edge_count == 5
        and .initial_width == 6 and .maximum_width == 6
        and .critical_path_length == 2
    '
    run run_pipeline "$FIXTURES/ui-feature" 32
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | jq -e '
        .issue_count == 12 and .hard_edge_count == 5
        and .initial_width == 8 and .maximum_width == 8
        and .critical_path_length == 2
    '
    run run_pipeline "$FIXTURES/cross-cutting" 32
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | jq -e '
        .issue_count == 16 and .hard_edge_count == 8
        and .initial_width == 10 and .maximum_width == 10
        and .critical_path_length == 3
    '
}
