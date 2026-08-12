#!/usr/bin/env bash
# scripts/lib/opencode-containment-adapter.sh — OpenCode implementer containment.
#
# The executor bridge refuses to run the mutating OpenCode implementer without a
# containment adapter (harness.rs: "executor_harness_uncontained"). OpenCode has
# no native filesystem sandbox, so this adapter applies the implementer
# permission profile: workspace-scoped read/edit/bash, no external-directory
# access, no web browsing, and no nested agent/skill dispatch.
#
# Invocation contract (fixed by executor_bridge/harness.rs):
#   <adapter> <opencode-executable> --pure run <prompt>
# The adapter receives the validated OpenCode executable as $1, then the
# remaining argv, and must exec it under containment.
#
# Environment it sets (mirrors the reviewer path in executor_bridge.rs):
#   OPENCODE_CONFIG_CONTENT    inline implementer permission profile
#   OPENCODE_DISABLE_CLAUDE_CODE=1   never load the operator's Claude config
#   OPENCODE_CONFIG_DIR        private per-run config dir (isolated from ~/.config/opencode)
#
# Operators who want OS-level isolation on top of the permission profile can
# point AUTOSPEC_OPENCODE_CONTAINMENT_ADAPTER at a bwrap/firejail wrapper that
# execs THIS script, or replace it entirely; the contract is only the argv form.

set -euo pipefail

if [ "$#" -lt 3 ]; then
    echo "opencode-containment-adapter: usage: <adapter> <opencode> <args...>" >&2
    exit 64
fi

opencode_bin="$1"
shift

if [ ! -x "$opencode_bin" ] && ! command -v "$opencode_bin" >/dev/null 2>&1; then
    echo "opencode-containment-adapter: OpenCode executable not found: $opencode_bin" >&2
    exit 64
fi

# Inline implementer permission profile. Same deny-everything base as the
# reviewer (OPENCODE_REVIEW_CONFIG), with edit + bash re-enabled so the agent can
# write code and run the project's test/validation commands inside the worktree.
# external_directory, webfetch, websearch, task and skill stay denied so the
# implementer cannot escape the workspace, browse the network, or spawn nested
# agents. The autospec Phase 4 implementer absorbs all sub-disciplines inline and
# does not call skills, so denying `skill` costs nothing.
OPENCODE_IMPLEMENTER_CONFIG='{"share":"disabled","instructions":[],"permission":{"*":"deny","read":"allow","glob":"allow","grep":"allow","list":"allow","lsp":"allow","edit":"allow","bash":"allow","task":"deny","external_directory":"deny","webfetch":"deny","websearch":"deny","skill":"deny"}}'

# Isolate config into a private per-run dir rather than mutating ~/.config/opencode.
adapter_tmp="${TMPDIR:-/tmp}/opencode-containment.$$"
mkdir -p "$adapter_tmp"
trap 'rm -rf "$adapter_tmp"' EXIT

export OPENCODE_CONFIG_CONTENT="$OPENCODE_IMPLEMENTER_CONFIG"
export OPENCODE_DISABLE_CLAUDE_CODE=1
export OPENCODE_CONFIG_DIR="$adapter_tmp"

exec "$opencode_bin" "$@"
