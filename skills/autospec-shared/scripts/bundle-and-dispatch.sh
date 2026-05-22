#!/usr/bin/env bash
# skills/autospec-shared/scripts/bundle-and-dispatch.sh — Orchestrator helper: assemble cached
# prefix via bundle-static-context.sh, append dynamic suffix, and emit the combined prompt.
#
# Usage:
#   bundle-and-dispatch.sh --role implementer|reviewer|decomposer|classifier
#                          --dynamic-suffix-file <path>
#                          [--issue-labels "label1,label2,..."]
#
# Output (stdout): combined prompt — static cached prefix (framed by <!-- CACHE BOUNDARY -->)
#                  followed by the dynamic suffix contents.
#
# The caller is responsible for passing the output to the Agent/task subagent tool with
# cache_control: { type: "ephemeral" } on the prefix block (framed by CACHE BOUNDARY markers).
#
# Idempotent + deterministic: same inputs → byte-identical output (inherits from bundler).
#
# Environment overrides (forwarded to bundle-static-context.sh):
#   AUTOSPEC_REPO_ROOT    — repo root
#   AUTOSPEC_MEMORY_DIR   — path to memory files
#   AUTOSPEC_SCRIPTS_DIR  — path to scripts dir
#   AUTOSPEC_MANIFEST     — path to memory-tags.yml
#
# Exit codes:
#   0  Success
#   1  Argument/file error
#
# Compatible with bash 3.2+

set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
BUNDLER="$SCRIPT_DIR/bundle-static-context.sh"

HELP_TEXT='Usage:
  bundle-and-dispatch.sh --role implementer|reviewer|decomposer|classifier
                         --dynamic-suffix-file <path>
                         [--issue-labels "label1,label2,..."]
  bundle-and-dispatch.sh --help

Assemble the two-part subagent prompt: static cached prefix (from bundle-static-context.sh)
followed by the dynamic suffix from <path>. Emit combined output to stdout.'

ROLE=""
ISSUE_LABELS=""
DYNAMIC_SUFFIX_FILE=""

while [ $# -gt 0 ]; do
  case "$1" in
    --help|-h)
      printf '%s\n' "$HELP_TEXT"
      exit 0
      ;;
    --role)
      if [ $# -lt 2 ]; then printf 'bundle-and-dispatch.sh: --role requires an argument\n' >&2; exit 1; fi
      ROLE="$2"
      shift 2
      ;;
    --issue-labels)
      if [ $# -lt 2 ]; then printf 'bundle-and-dispatch.sh: --issue-labels requires an argument\n' >&2; exit 1; fi
      ISSUE_LABELS="$2"
      shift 2
      ;;
    --dynamic-suffix-file)
      if [ $# -lt 2 ]; then printf 'bundle-and-dispatch.sh: --dynamic-suffix-file requires an argument\n' >&2; exit 1; fi
      DYNAMIC_SUFFIX_FILE="$2"
      shift 2
      ;;
    -*)
      printf 'bundle-and-dispatch.sh: unknown option: %s\n' "$1" >&2
      exit 1
      ;;
    *)
      printf 'bundle-and-dispatch.sh: unexpected argument: %s\n' "$1" >&2
      exit 1
      ;;
  esac
done

# Validate required args
if [ -z "$ROLE" ]; then
  printf 'bundle-and-dispatch.sh: --role is required\n' >&2
  printf '%s\n' "$HELP_TEXT" >&2
  exit 1
fi

if [ -z "$DYNAMIC_SUFFIX_FILE" ]; then
  printf 'bundle-and-dispatch.sh: --dynamic-suffix-file is required\n' >&2
  printf '%s\n' "$HELP_TEXT" >&2
  exit 1
fi

if [ ! -f "$DYNAMIC_SUFFIX_FILE" ]; then
  printf 'bundle-and-dispatch.sh: dynamic suffix file not found: %s\n' "$DYNAMIC_SUFFIX_FILE" >&2
  exit 1
fi

if [ ! -f "$BUNDLER" ]; then
  printf 'bundle-and-dispatch.sh: bundler not found: %s\n' "$BUNDLER" >&2
  exit 1
fi

# ── emit combined output ──────────────────────────────────────────────────────

# 1. Static cached prefix (framed by <!-- CACHE BOUNDARY --> markers)
if [ -n "$ISSUE_LABELS" ]; then
  bash "$BUNDLER" --role "$ROLE" --issue-labels "$ISSUE_LABELS"
else
  bash "$BUNDLER" --role "$ROLE"
fi

# 2. Dynamic suffix (appended after the closing CACHE BOUNDARY marker)
printf '\n'
cat "$DYNAMIC_SUFFIX_FILE"
