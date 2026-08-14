#!/usr/bin/env bats

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  SKILL_DIR="$REPO_ROOT/skills/autospec-autonomous"
  SKILL="$SKILL_DIR/SKILL.md"
  CLI_REFERENCE="$REPO_ROOT/docs/cli-reference.md"
  AGENTS="$REPO_ROOT/AGENTS.md"
}

@test "autonomous skill makes one verified run epic a pre-spawn invariant" {
  grep -Fq 'Every live autonomous launch creates or adopts exactly one verified managed GitHub epic before spawning the conductor.' "$SKILL"
  grep -Fq 'There is no CLI flag, environment variable, degraded mode, or fallback that bypasses the epic or private journal.' "$SKILL"
}

@test "autonomous skill defines concise accountable epic content and both Mermaid views" {
  grep -Fq 'concise `What`, `Why`, and `Evidence` entries' "$SKILL"
  grep -Fq 'dependency and deliverable flowchart' "$SKILL"
  grep -Fq 'run-state diagram' "$SKILL"
}

@test "autonomous skill documents generated and explicit resume epic paths" {
  grep -Fq 'A normal `start` generates its own epic.' "$SKILL"
  grep -Fq '`start --epic N` adopts an active managed epic' "$SKILL"
  grep -Fq '`resume --epic N` may reopen a closed or parked managed epic' "$SKILL"
  grep -Fq '`resumed_from_epic`' "$SKILL"
}

@test "autonomous skill documents exact immutable runtime rebuild semantics" {
  grep -Fq 'A stale or missing runtime always rebuilds an immutable source-digest generation' "$SKILL"
  grep -Fq 'executes that exact verified generation path' "$SKILL"
}

@test "CLI reference exposes epic flags, local accountability health, and recovery rules" {
  grep -Fq 'autospec autonomous start --repo OWNER/REPO --repo-dir DIR [--epic N]' "$CLI_REFERENCE"
  grep -Fq 'autospec autonomous resume --epic N' "$CLI_REFERENCE"
  grep -Fq '`run_id`, `epic_number`, `epic_url`, `event_count`' "$CLI_REFERENCE"
  grep -Fq 'exactly one verified managed run epic before conductor spawn' "$CLI_REFERENCE"
}

@test "AGENTS makes autonomous accountability a repository invariant" {
  grep -Fq '## Autonomous run accountability' "$AGENTS"
  grep -Fq 'exactly one verified managed GitHub epic before conductor spawn' "$AGENTS"
  grep -Fq 'There is no bypass for the epic or its private local journal.' "$AGENTS"
}

@test "autonomous skill adapters remain derived from the canonical body" {
  run "$REPO_ROOT/scripts/derive-trio.sh" "$SKILL_DIR" --check
  [ "$status" -eq 0 ]
}
