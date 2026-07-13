#!/usr/bin/env bats
# tests/agent-env-install.bats — installer coverage for runtime broker commands

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/.." && pwd)"

@test "install.sh exposes agent-env and autospec-env command wrappers" {
  grep -q 'install_agent_env_commands' "$REPO_ROOT/install.sh"
  grep -q 'for command in agent-env autospec-env' "$REPO_ROOT/install.sh"
  grep -q 'agent-env.sh" "$@"' "$REPO_ROOT/install.sh"
  grep -q '^install_agent_env_commands$' "$REPO_ROOT/install.sh"
}
