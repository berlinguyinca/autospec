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

@test "core-to-CLI direction ignores core test fixture paths" {
    scratch="$(mktemp -d)"
    mkdir -p "$scratch/crates/autospec-core/src" "$scratch/crates/autospec-core/tests"
    : > "$scratch/crates/autospec-core/src/lib.rs"
    printf '%s\n' '// fixture stored beside core tests: autospec-cli' > "$scratch/crates/autospec-core/tests/fixture.rs"
    cp "$REGISTRY" "$scratch/registry.yml"

    run bash "$FITNESS" run --registry "$scratch/registry.yml" --repo "$scratch" --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.results[] | select(.id=="rust_core_cli_direction" and .observed==0)' >/dev/null

    printf '%s\n' '// autospec-cli must not appear in core production source' > "$scratch/crates/autospec-core/src/leak.rs"
    run bash "$FITNESS" run --registry "$scratch/registry.yml" --repo "$scratch" --json
    [ "$status" -ne 0 ]
    echo "$output" | jq -e '.results[] | select(.id=="rust_core_cli_direction" and .observed==1 and .locations[0].path=="crates/autospec-core/src/leak.rs")' >/dev/null
    rm "$scratch/crates/autospec-core/src/leak.rs"

    printf '%s\n' 'autospec-cli = "0.1"' > "$scratch/crates/autospec-core/Cargo.toml"
    run bash "$FITNESS" run --registry "$scratch/registry.yml" --repo "$scratch" --json
    [ "$status" -ne 0 ]
    echo "$output" | jq -e '.results[] | select(.id=="rust_core_cli_direction" and .observed==1 and .locations[0].path=="crates/autospec-core/Cargo.toml")' >/dev/null

    rm -rf "$scratch"
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

@test "direct Rust validation owns the architecture fitness gate and test suite" {
    catalog="$REPO_ROOT/crates/autospec-core/src/validation/catalog.rs"
    external="$REPO_ROOT/crates/autospec-core/src/validation/external.rs"
    grep -q 'ArchitectureFitnessEngine' "$catalog"
    grep -q 'tests/architecture-fitness' "$external"
    grep -q 'scripts/architecture-fitness.sh' "$external"
    rg -U -q '"check_architecture_fitness_engine" => \{\s+CheckOwner::ExternalBatch\(ExternalCheck::ArchitectureFitnessEngine\)' "$catalog"
    grep -q 'Self::ArchitectureFitnessEngine => run_architecture_fitness_engine' "$external"
}
