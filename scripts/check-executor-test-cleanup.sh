#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
# The executor-bridge tests are a directory now. Scanning tests.rs alone would still exit 0
# while every case it used to hold sat unchecked in a sibling module.
tests_root="$repo_root/crates/autospec-cli/src/commands/autonomous/executor_bridge"
source_files=("$tests_root/tests.rs")
while IFS= read -r module; do
  source_files+=("$module")
done < <(find "$tests_root/tests" -name '*.rs' 2>/dev/null | sort)

if ! perl -0ne '
  $found = 1 if /fs::remove_dir_all\(\s*worktree\s*\.path\s*\.parent\(\)\s*\.and_then\(Path::parent\)/s;
  END { die "destructive teardown\n" if $found }
' "${source_files[@]}"; then
  echo "ERROR: executor test teardown may not remove the shared executor root" >&2
  exit 1
fi

echo "executor test teardown stays inside fixture scopes (${#source_files[@]} files)"
