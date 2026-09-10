#!/usr/bin/env bats

# Issue #3823 — docs assertions for the `planning.parallelism` config block
# (docs/CONFIG_REFERENCE.md) and the graph analyzer metric contract
# (docs/API_REFERENCE.md). No mocks: every test greps the checked-in
# reference docs and counts exact occurrences.

setup() {
    repo_root="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    config_ref="$repo_root/docs/CONFIG_REFERENCE.md"
    api_ref="$repo_root/docs/API_REFERENCE.md"
}

count_occurrences() {
    grep -oF -- "$1" "$2" | wc -l | tr -d ' '
}

@test "config reference documents every planning.parallelism key exactly once" {
    local key occurrences
    for key in \
        target_agents \
        min_supported_agents \
        max_supported_agents \
        target_initial_width_ratio \
        optimization_retry_threshold \
        max_optimization_retries \
        max_dependency_fan_in_before_warning \
        shared_write_hotspot_threshold \
        require_dependency_justification \
        size_split_implies_dependency \
        parent_relationship_implies_dependency \
        conflict_risk_implies_dependency
    do
        occurrences="$(count_occurrences "planning.parallelism.$key" "$config_ref")"
        [ "$occurrences" -eq 1 ]
    done
}

@test "config reference pins planning.parallelism.target_agents default to 32" {
    run grep -E '^\| `planning\.parallelism\.target_agents` \|' "$config_ref"
    [ "$status" -eq 0 ]
    [[ "$output" == *"32"* ]]
}

@test "config reference pins planning.parallelism.optimization_retry_threshold default to 65" {
    run grep -E '^\| `planning\.parallelism\.optimization_retry_threshold` \|' "$config_ref"
    [ "$status" -eq 0 ]
    [[ "$output" == *"65"* ]]
}

@test "config reference parallelism rows each carry a type, default, and valid range" {
    # 12 rows, each with exactly five cells (key | type | default | range | effect).
    run awk -F'|' '/^\| `planning\.parallelism\./ { m++; if (NF != 7) bad++ } END { print bad+0, m+0 }' \
        "$config_ref"
    [ "$status" -eq 0 ]
    [ "$output" = "0 12" ]
}

@test "api reference documents the analyzer width and path fields" {
    local field occurrences
    for field in initial_width maximum_width critical_path_length; do
        occurrences="$(count_occurrences "$field" "$api_ref")"
        [ "$occurrences" -ge 1 ]
    done
}

@test "api reference lists every AS-DAG rule code exactly once" {
    local code occurrences
    for code in 001 002 003 004 005 006 007 008 009 010; do
        occurrences="$(count_occurrences "AS-DAG-$code" "$api_ref")"
        [ "$occurrences" -eq 1 ]
    done
}

@test "api reference states the parallelization score weights 30 20 20 15 15" {
    run grep -E 'Initial saturation.*30' "$api_ref"
    [ "$status" -eq 0 ]
    run grep -E 'Peak saturation.*20' "$api_ref"
    [ "$status" -eq 0 ]
    run grep -E 'Inverse critical-path pressure.*20' "$api_ref"
    [ "$status" -eq 0 ]
    run grep -E 'Low shared-write overlap.*15' "$api_ref"
    [ "$status" -eq 0 ]
    run grep -E 'Dependency justification quality.*15' "$api_ref"
    [ "$status" -eq 0 ]
}

@test "api reference marks AS-DAG-010 CYCLE fatal" {
    run grep -E 'AS-DAG-010.*CYCLE.*Fatal' "$api_ref"
    [ "$status" -eq 0 ]
}
