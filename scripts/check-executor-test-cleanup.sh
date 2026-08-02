#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
source_file="$repo_root/crates/autospec-cli/src/commands/autonomous/executor_bridge.rs"

if ! perl -0ne '
  $found = 1 if /fs::remove_dir_all\(\s*worktree\s*\.path\s*\.parent\(\)\s*\.and_then\(Path::parent\)/s;
  END { die "destructive teardown\n" if $found }
' "$source_file"; then
  echo "ERROR: executor test teardown may not remove the shared executor root" >&2
  exit 1
fi

echo "executor test teardown stays inside fixture scopes"
