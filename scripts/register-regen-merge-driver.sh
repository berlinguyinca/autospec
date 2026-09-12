#!/usr/bin/env bash
# Issue #4058 — register the `autospec-regen` merge driver in this clone.
#
# .gitattributes declares the driver by name, but git only runs a named
# driver after it is registered in this clone's git config. Without
# registration git silently text-merges the declared files — exactly the
# corruption the driver exists to prevent. Run once per clone (idempotent;
# safe to re-run from any worktree of the clone).

set -euo pipefail

top="$(git rev-parse --show-toplevel)"
driver="$top/scripts/merge-regen-driver.sh"

if [ ! -f "$driver" ]; then
  echo "FATAL: $driver not found (is this an autospec checkout?)" >&2
  exit 1
fi

git config merge.autospec-regen.driver "bash $driver %O %A %B"
echo "registered: $(git config merge.autospec-regen.driver)"
