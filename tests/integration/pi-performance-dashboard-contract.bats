#!/usr/bin/env bats
# tests/unit/pi-performance-dashboard-contract.bats — unit coverage of the
# provider-neutral contract behind the dashboard (issue #3327): the live card
# field list, the nearest-rank percentile rule, the static-vs-benchmark sample
# threshold, and the model-family pin live in autospec-core and are unit
# tested there, so this harness asserts the unit suite passes.

setup() {
    repo_root="$(cd "${BATS_TEST_DIRNAME}/../.." && pwd)"
}

@test "aar_dashboard unit tests pass (card, percentiles, advice, family pin)" {
    run cargo test --manifest-path "$repo_root/Cargo.toml" -p autospec-core --test aar_dashboard
    [ "$status" -eq 0 ]
}
