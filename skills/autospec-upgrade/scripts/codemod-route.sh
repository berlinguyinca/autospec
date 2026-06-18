#!/usr/bin/env bash
# codemod-route.sh — shared codemod dispatcher for autospec-upgrade (issue #1177)
#
# Usage (CLI):
#   codemod-route.sh <framework> <target_major> [--standalone]
#
# Usage (sourced):
#   . codemod-route.sh
#   route_codemod <framework> <target_major> [--standalone]
#   route_angular_codemod <target_major> [--standalone]
#
# Design: this file is designed to be extended by #1178 (next) and #1179 (react).
# Each issue adds exactly one function (route_next_codemod / route_react_codemod)
# and one case arm in route_codemod(). No existing code needs to change.
#
# Exit codes:
#   0  — codemod dispatched successfully
#   1  — unknown framework (code_health:upgrade_unknown_framework)
#   2  — missing required argument

set -uo pipefail

# ── Angular codemod (issue #1177) ─────────────────────────────────────────────
#
# Shells the OFFICIAL Angular CLI tooling only — never hand-rolls migrations.
#   ng update @angular/core@<to> @angular/cli@<to>   (standard hop)
#   ng generate @angular/core:standalone              (standalone mode, optional)

route_angular_codemod() {
  local target_major="$1"
  local standalone="${2:-}"

  if [ -z "$target_major" ]; then
    printf 'codemod-route: route_angular_codemod requires <target_major>\n' >&2
    return 2
  fi

  # Official Angular upgrade schematic — never hand-roll this
  ng update "@angular/core@${target_major}" "@angular/cli@${target_major}"

  # Optional: migrate to standalone components via the official schematic
  if [ "$standalone" = "--standalone" ]; then
    ng generate @angular/core:standalone
  fi
}

# ── Shared dispatcher ─────────────────────────────────────────────────────────
#
# route_codemod <framework> <target_major> [--standalone]
#
# The case statement is structured for clean extension:
#   #1178 adds a "next)" arm calling route_next_codemod
#   #1179 adds a "react)" arm calling route_react_codemod

route_codemod() {
  local framework="${1:-}"
  local target_major="${2:-}"
  local standalone="${3:-}"

  if [ -z "$framework" ]; then
    printf 'codemod-route: <framework> argument is required\n' >&2
    return 2
  fi

  if [ -z "$target_major" ]; then
    printf 'codemod-route: <target_major> argument is required\n' >&2
    return 2
  fi

  case "$framework" in
    angular)
      route_angular_codemod "$target_major" "$standalone"
      ;;
    # additional framework arms added by #1178 (next) / #1179 (react)
    *)
      printf 'codemod-route: unknown framework "%s" — code_health:upgrade_unknown_framework\n' \
        "$framework" >&2
      return 1
      ;;
  esac
}

# ── CLI entrypoint ────────────────────────────────────────────────────────────
# Only runs when executed directly (not when sourced).

_codemod_route_main() {
  local framework="${1:-}"
  local target_major="${2:-}"
  local standalone="${3:-}"

  if [ -z "$framework" ]; then
    printf 'Usage: codemod-route.sh <framework> <target_major> [--standalone]\n' >&2
    exit 2
  fi

  route_codemod "$framework" "$target_major" "$standalone"
}

# Detect sourced vs executed: in bash the source produces $BASH_SOURCE[0] != $0
# In bash 3.2 (macOS), use $0 comparison; works for both bash and bats execution.
_self="${BASH_SOURCE[0]:-$0}"
if [ "$_self" = "$0" ]; then
  _codemod_route_main "$@"
fi
