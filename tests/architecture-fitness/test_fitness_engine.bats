#!/usr/bin/env bats
# tests/architecture-fitness/test_fitness_engine.bats — architecture fitness-function engine contracts.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    FITNESS="$REPO_ROOT/scripts/architecture-fitness.sh"
    REGISTRY="$REPO_ROOT/.autospec/architecture-fitness.yml"
    export REPO_ROOT FITNESS REGISTRY
}

@test "architecture-fitness script exists and passes bash syntax" {
    [ -f "$FITNESS" ]
    run bash -n "$FITNESS"
    [ "$status" -eq 0 ]
}

@test "registry declares runnable fitness functions with thresholds" {
    [ -f "$REGISTRY" ]
    grep -q '^fitness_functions:' "$REGISTRY"
    grep -q 'id: financial_no_f64' "$REGISTRY"
    grep -q 'id: latency_budget_validate_fast' "$REGISTRY"
    grep -q 'threshold:' "$REGISTRY"
    grep -q 'gate: true' "$REGISTRY"
}

@test "run gate emits JSON results for the declarative registry" {
    run bash "$FITNESS" run --registry "$REGISTRY" --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.summary.total >= 1' >/dev/null
    echo "$output" | jq -e '.results[] | select(.id=="financial_no_f64" and .gate==true and .threshold != null)' >/dev/null
}

@test "breach produces auto-implement issue body with metric and location" {
    scratch="$(mktemp -d)"
    mkdir -p "$scratch/src"
    cat > "$scratch/src/money.rs" <<'SRC'
pub fn pnl() -> f64 { 1.0 }
SRC
    cat > "$scratch/registry.yml" <<'YAML'
fitness_functions:
  - id: financial_no_f64
    name: Financial paths avoid f64
    type: forbidden_pattern
    gate: true
    threshold: 0
    paths:
      - src/**
    pattern: 'f64'
    metric: forbidden_f64_occurrences
    issue:
      title: 'fix: remove f64 from financial path'
      labels: ['auto-implement', 'architecture-fitness']
YAML
    run bash "$FITNESS" run --registry "$scratch/registry.yml" --repo "$scratch" --emit-issues "$scratch/issues" --json
    [ "$status" -ne 0 ]
    [ -f "$scratch/issues/financial_no_f64.md" ]
    grep -q 'auto-implement' "$scratch/issues/financial_no_f64.md"
    grep -q 'metric: forbidden_f64_occurrences' "$scratch/issues/financial_no_f64.md"
    grep -q 'src/money.rs' "$scratch/issues/financial_no_f64.md"
    rm -rf "$scratch"
}

@test "validate.sh wires the architecture fitness gate and test suite" {
    grep -q '^check_architecture_fitness_engine()' "$REPO_ROOT/scripts/validate.sh"
    grep -q 'tests/architecture-fitness/.*\.bats' "$REPO_ROOT/scripts/validate.sh"
    grep -q 'architecture-fitness.sh run' "$REPO_ROOT/scripts/validate.sh"
}
