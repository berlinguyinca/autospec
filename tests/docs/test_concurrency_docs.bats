#!/usr/bin/env bats

# Docs assertions for issue #3822: capacity-aware decomposition concepts and
# user manual. Source spec: docs/specs/2026-09-08-parallel-decomposition-fleet-saturation.md

repo_root="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"

@test "concepts page has a Parallel decomposition heading" {
  run grep -nE '^##[[:space:]]+Parallel decomposition' "$repo_root/docs/concepts.md"
  [ "$status" -eq 0 ]
}

@test "concepts page lists all 8 valid hard-dependency reason codes from section 5.1" {
  for code in \
      required-public-api \
      required-type-or-interface \
      required-schema \
      required-database-migration \
      required-wire-protocol-version \
      generated-artifact \
      structural-migration \
      acceptance-tests-require-output; do
    run grep -nF "$code" "$repo_root/docs/concepts.md"
    [ "$status" -eq 0 ]
  done
}

@test "concepts page lists the 12 rejected dependency reasons that must not create an edge" {
  for phrase in \
      "issue appears earlier in the spec" \
      "foundational" \
      "implementation order would be convenient" \
      "belong to the same epic" \
      "files are nearby" \
      "one issue is documentation" \
      "one issue is testing" \
      "conceptual relationship" \
      "expected merge conflicts" \
      "parent/child relationship" \
      "planner preference" \
      "do this first"; do
    run grep -nF "$phrase" "$repo_root/docs/concepts.md"
    [ "$status" -eq 0 ]
  done
}

@test "concepts page carries a worked example with initial-width values" {
  run grep -nF "Initial width" "$repo_root/docs/concepts.md"
  [ "$status" -eq 0 ]
  run grep -nF "Best When Contract Already Exists" "$repo_root/docs/concepts.md"
  [ "$status" -eq 0 ]
}

@test "user manual has a Fleet capacity walkthrough heading" {
  run grep -nE '^##[[:space:]]+Fleet capacity walkthrough' "$repo_root/docs/USER_MANUAL.md"
  [ "$status" -eq 0 ]
}

@test "user manual documents the 10-100 supported fleet range" {
  run grep -nE '10[–-]100|10 <= fleet_capacity <= 100' "$repo_root/docs/USER_MANUAL.md"
  [ "$status" -eq 0 ]
}

@test "user manual documents the capacity flag so an operator finds it from the manual alone" {
  run grep -nF -- "--agents" "$repo_root/docs/USER_MANUAL.md"
  [ "$status" -eq 0 ]
}

@test "user manual states that conflict domains never block readiness" {
  run grep -niE 'conflict domains? .{0,60}(never block|never blocks|blockers)' "$repo_root/docs/USER_MANUAL.md"
  [ "$status" -eq 0 ]
}
