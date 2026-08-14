#!/usr/bin/env bats
# tests/agent-env-install.bats — installer coverage for runtime broker commands

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/.." && pwd)"

@test "install.sh delegates Rust runtime publication to immutable generations" {
  grep -q '^install_autospec_runtime_binary()' "$REPO_ROOT/install.sh"
  grep -qF 'cargo build --release -p autospec-cli' "$REPO_ROOT/install.sh"
  grep -qF 'scripts/autospec-runtime-install.sh" --repo-dir "$REPO_ROOT"' "$REPO_ROOT/install.sh"
  grep -qF 'runtime-generations' "$REPO_ROOT/scripts/autospec-runtime-install.sh"
  grep -qF 'runtime_atomic_replace "$temporary" "$pointer"' "$REPO_ROOT/scripts/autospec-runtime-install.sh"
  grep -qF '[dry-run] install_autospec_runtime_binary: cargo build --release -p autospec-cli (from $REPO_ROOT)' "$REPO_ROOT/install.sh"
  grep -qF '[dry-run] install_autospec_runtime_binary: $REPO_ROOT/scripts/autospec-runtime-install.sh --repo-dir $REPO_ROOT' "$REPO_ROOT/install.sh"
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
  grep -qF 'templates/generated/harness-runtime-aliases.$format' "$REPO_ROOT/install.sh"
  grep -qF "alias claude='autospec-env session -- claude --dangerously-skip-permissions'" "$REPO_ROOT/templates/generated/harness-runtime-aliases.sh"
  grep -qF "alias codex='autospec-env session -- codex --yolo'" "$REPO_ROOT/templates/generated/harness-runtime-aliases.sh"
  grep -qF "alias opencode='autospec-env session -- opencode'" "$REPO_ROOT/templates/generated/harness-runtime-aliases.sh"
  grep -q 'Install isolated runtime aliases' "$REPO_ROOT/install.sh"
  grep -q '\[n/Y\]' "$REPO_ROOT/install.sh"
  grep -q 'AUTOSPEC_SKIP_AGENT_ENV_ALIASES' "$REPO_ROOT/install.sh"
  grep -q '^install_agent_env_aliases$' "$REPO_ROOT/install.sh"
}

@test "custom runtime config install is discovered by the installed detector" {
  install_root="$BATS_TEST_TMPDIR/install"
  scripts_dir="$install_root/scripts"
  config_dir="$BATS_TEST_TMPDIR/custom-config"
  helpers="$BATS_TEST_TMPDIR/copy-runtime-subdirs.sh"
  {
    printf 'DRY_RUN=0\n'
    printf 'info() { :; }\nwarn() { printf "%%s\\n" "$*" >&2; }\n'
    awk '
      /^copy_runtime_subdirs\(\)/ { capture=1 }
      capture { print }
      capture && /^}$/ { exit }
    ' "$REPO_ROOT/install.sh"
  } > "$helpers"
  # shellcheck source=/dev/null
  source "$helpers"

  AUTOSPEC_SCRIPTS_DIR="$scripts_dir" AUTOSPEC_CONFIG_DIR="$config_dir" copy_runtime_subdirs
  run env AUTOSPEC_CONFIG_DIR="$config_dir" bash -c '
    source "$1/lib/autospec-harness-detect.sh"
    autospec_harness_supported_ids
  ' _ "$scripts_dir"

  [ "$status" -eq 0 ]
  [ "$output" = $'claude\ncodex\nopencode' ]
}
