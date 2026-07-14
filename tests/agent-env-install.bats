#!/usr/bin/env bats
# tests/agent-env-install.bats — installer coverage for runtime broker commands

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/.." && pwd)"

@test "install.sh builds and atomically installs the Rust runtime broker" {
  grep -q '^install_autospec_runtime_binary()' "$REPO_ROOT/install.sh"
  grep -qF 'cargo build --release -p autospec-cli' "$REPO_ROOT/install.sh"
  grep -qF 'runtime_source="$REPO_ROOT/target/release/autospec"' "$REPO_ROOT/install.sh"
  grep -qF 'runtime_target="$HOME/.autospec/bin/autospec"' "$REPO_ROOT/install.sh"
  grep -qF 'runtime_temporary="$(mktemp "$autospec_bin_dir/.autospec.XXXXXX")"' "$REPO_ROOT/install.sh"
  grep -qF 'mv "$runtime_temporary" "$runtime_target"' "$REPO_ROOT/install.sh"
  grep -qF '[dry-run] install_autospec_runtime_binary: cargo build --release -p autospec-cli (from $REPO_ROOT)' "$REPO_ROOT/install.sh"
  grep -qF '[dry-run] install_autospec_runtime_binary: install $REPO_ROOT/target/release/autospec -> $HOME/.autospec/bin/autospec' "$REPO_ROOT/install.sh"
  grep -q '^install_autospec_runtime_binary$' "$REPO_ROOT/install.sh"
}

@test "install.sh exposes Rust-backed agent-env and autospec-env command wrappers" {
  grep -q 'install_agent_env_commands' "$REPO_ROOT/install.sh"
  grep -q 'for command in agent-env autospec-env' "$REPO_ROOT/install.sh"
  grep -qF 'exec "${AUTOSPEC_BIN:-$HOME/.autospec/bin/autospec}" runtime env "$@"' "$REPO_ROOT/install.sh"
  ! grep -q 'agent-env.sh' "$REPO_ROOT/install.sh"
  grep -q '^install_agent_env_commands$' "$REPO_ROOT/install.sh"
}

@test "install.sh registers isolated runtime session aliases during default setup" {
  grep -q 'install_agent_env_aliases' "$REPO_ROOT/install.sh"
  grep -q "alias claude='autospec-env session -- claude --dangerously-skip-permissions'" "$REPO_ROOT/install.sh"
  grep -q "alias codex='autospec-env session -- codex --yolo'" "$REPO_ROOT/install.sh"
  grep -q "alias opencode='autospec-env session -- opencode'" "$REPO_ROOT/install.sh"
  grep -q 'Install isolated runtime aliases' "$REPO_ROOT/install.sh"
  grep -q '\[n/Y\]' "$REPO_ROOT/install.sh"
  grep -q 'AUTOSPEC_SKIP_AGENT_ENV_ALIASES' "$REPO_ROOT/install.sh"
  grep -q '^install_agent_env_aliases$' "$REPO_ROOT/install.sh"
}
