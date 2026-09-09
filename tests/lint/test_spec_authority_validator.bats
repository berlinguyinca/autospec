#!/usr/bin/env bats
# tests/lint/test_spec_authority_validator.bats
#
# Self-enforcement for scripts/validate-spec-authority.sh (issue #3947).
#
# A fleet merged forty times against a charter a newer program had already
# superseded, because nothing asked which spec set governs a component and
# whether that document is still in force. The validator is the ratchet that
# keeps the gate wired in; these tests pin that it passes on this repository
# and FAILS when each part of the gate is removed, so the ratchet cannot rot
# into a script that is green whatever the tree looks like.

bats_require_minimum_version 1.5.0

setup() {
    REPO_ROOT="$(cd "${BATS_TEST_DIRNAME}/../.." && pwd -P)"
    VALIDATOR="${REPO_ROOT}/scripts/validate-spec-authority.sh"
    FAKE="$(mktemp -d)"
    mkdir -p "${FAKE}/crates/autospec-core/src" \
        "${FAKE}/crates/autospec-cli/src/commands" \
        "${FAKE}/docs"
    cp "${REPO_ROOT}/crates/autospec-core/src/spec_authority.rs" \
        "${REPO_ROOT}/crates/autospec-core/src/lib.rs" \
        "${FAKE}/crates/autospec-core/src/"
    cp "${REPO_ROOT}/crates/autospec-cli/src/commands/dispatch_authority.rs" \
        "${REPO_ROOT}/crates/autospec-cli/src/commands/mod.rs" \
        "${REPO_ROOT}/crates/autospec-cli/src/commands/dispatch.rs" \
        "${FAKE}/crates/autospec-cli/src/commands/"
    cp "${REPO_ROOT}/docs/cli-reference.md" "${REPO_ROOT}/docs/invariants.md" "${FAKE}/docs/"
}

teardown() {
    rm -rf "${FAKE}"
}

# A tree with no binary available skips the behavior probes rather than
# inventing a verdict, so the fake-root cases below stay about wiring and docs.
fake_run() {
    run "${VALIDATOR}" --root "${FAKE}" --bin /nonexistent/autospec --quiet
}

@test "the validator passes on this repository" {
    run "${VALIDATOR}" --quiet
    [ "$status" -eq 0 ]
}

@test "an unwired authority subcommand is a finding" {
    sed -i 's/"authority" => super::dispatch_authority::run(rest),//' \
        "${FAKE}/crates/autospec-cli/src/commands/dispatch.rs"
    fake_run
    [ "$status" -ne 0 ]
    [[ "$output" == *"REGISTRATION_MISSING"* ]]
    [[ "$output" == *"super::dispatch_authority::run("* ]]
}

@test "a missing core module is a finding" {
    rm "${FAKE}/crates/autospec-core/src/spec_authority.rs"
    fake_run
    [ "$status" -ne 0 ]
    [[ "$output" == *"spec_authority.rs is absent"* ]]
}

@test "undocumented flags are findings" {
    sed -i 's/--tasks/-removed-/g' "${FAKE}/docs/cli-reference.md"
    fake_run
    [ "$status" -ne 0 ]
    [[ "$output" == *"DOCUMENTATION_MISSING"* ]]
    [[ "$output" == *"--tasks"* ]]
}

@test "an invariant with no entry in docs/invariants.md is a finding" {
    rm "${FAKE}/docs/invariants.md"
    fake_run
    [ "$status" -ne 0 ]
    [[ "$output" == *"docs/invariants.md is absent"* ]]
}

@test "the gate vocabulary stays public" {
    sed -i 's/^pub fn parse_task_records/fn parse_task_records/' \
        "${FAKE}/crates/autospec-core/src/spec_authority.rs"
    fake_run
    [ "$status" -ne 0 ]
    [[ "$output" == *"pub fn parse_task_records"* ]]
}

@test "help documents the finding codes and the exit contract" {
    run "${VALIDATOR}" --help
    [ "$status" -eq 0 ]
    [[ "$output" == *"REGISTRATION_MISSING"* ]]
    [[ "$output" == *"BEHAVIOR"* ]]
    [[ "$output" == *"Exit 0 when clean"* ]]
}
