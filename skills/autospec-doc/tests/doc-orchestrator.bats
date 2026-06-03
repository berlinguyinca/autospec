#!/usr/bin/env bats
# doc-orchestrator.bats — TDD for skills/autospec-doc/scripts/doc-orchestrator.mjs (issue #916)
#
# The orchestrator is a router stub: it parses the §D1 subcommand contract,
# dispatches to distinct named handlers, and exits 2 when a non-`init`
# subcommand runs without a `documentation:` config.

ORCH="${BATS_TEST_DIRNAME}/../scripts/doc-orchestrator.mjs"

setup() {
  TESTDIR="$(mktemp -d)"
  cd "$TESTDIR"
}

teardown() {
  rm -rf "$TESTDIR"
}

# Seed a minimal .autospec/autospec.yml carrying a documentation: block so the
# config gate passes for non-init subcommands.
seed_config() {
  mkdir -p .autospec
  cat > .autospec/autospec.yml <<'EOF'
documentation:
  audiences:
    - name: user
      path: docs/user
EOF
}

@test "init prints a scaffold plan and exits 0 (no config required)" {
  run node "$ORCH" init
  [ "$status" -eq 0 ]
  [[ "$output" == *"init: scaffold plan"* ]]
}

@test "init works even with no documentation config present" {
  # No seed_config — init is the bootstrap that creates the config.
  run node "$ORCH" init
  [ "$status" -eq 0 ]
}

@test "bare invocation routes to the incremental handler" {
  seed_config
  run node "$ORCH"
  [ "$status" -eq 0 ]
  [[ "$output" == *"incremental:"* ]]
}

@test "--full routes to the full handler" {
  seed_config
  run node "$ORCH" --full
  [ "$status" -eq 0 ]
  [[ "$output" == *"full:"* ]]
}

@test "--audit routes to the audit handler" {
  seed_config
  run node "$ORCH" --audit
  [ "$status" -eq 0 ]
  [[ "$output" == *"audit:"* ]]
}

@test "--audience <name> routes to the audience handler with the name" {
  seed_config
  run node "$ORCH" --audience developer
  [ "$status" -eq 0 ]
  [[ "$output" == *"developer"* ]]
}

@test "bare, --full, --audit, --audience route to DISTINCT handlers" {
  seed_config
  bare="$(node "$ORCH" 2>&1)"
  full="$(node "$ORCH" --full 2>&1)"
  audit="$(node "$ORCH" --audit 2>&1)"
  aud="$(node "$ORCH" --audience admin 2>&1)"
  # All four outputs must differ from each other.
  [ "$bare" != "$full" ]
  [ "$bare" != "$audit" ]
  [ "$bare" != "$aud" ]
  [ "$full" != "$audit" ]
  [ "$full" != "$aud" ]
  [ "$audit" != "$aud" ]
}

@test "non-init subcommand with no documentation config exits 2" {
  # No seed_config.
  run node "$ORCH" --full
  [ "$status" -eq 2 ]
  [[ "$output" == *"init"* ]]
}

@test "bare incremental with no documentation config exits 2" {
  run node "$ORCH"
  [ "$status" -eq 2 ]
}

@test "--audience without a name exits 2" {
  seed_config
  run node "$ORCH" --audience
  [ "$status" -eq 2 ]
}

@test "--audience followed by a flag is a usage error, not an audience name" {
  seed_config
  run node "$ORCH" --audience --full
  [ "$status" -eq 2 ]
}

@test "unknown argument prints usage and exits 2" {
  seed_config
  run node "$ORCH" --bogus
  [ "$status" -eq 2 ]
  [[ "$output" == *"Usage:"* ]]
}
