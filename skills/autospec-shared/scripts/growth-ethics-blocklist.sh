#!/usr/bin/env bash
# growth-ethics-blocklist.sh — single source of truth for the white-hat
# hard-block list. Repo config MAY add blocks; it MAY NOT remove a built-in.
set -euo pipefail

# Built-in hard blocks. This list is the immutable spine of the ethics gate.
BUILTIN_BLOCKS="fake_reviews
undisclosed_incentivized_reviews
review_gating
rating_vote_manipulation
sockpuppets
bot_or_fake_signups
scraped_email_spam
cloaking_doorway
link_schemes_pbn
platform_tos_violation"

usage() { echo "usage: growth-ethics-blocklist.sh --list | --effective <cfg> | --assert-not-weakened <cfg>" >&2; exit 2; }

cmd="${1:-}"; shift || true
case "$cmd" in
  --list)
    printf '%s\n' "$BUILTIN_BLOCKS"
    ;;
  --effective)
    cfg="${1:?config path required}"
    [ -f "$cfg" ] || { echo "config not found: $cfg" >&2; exit 2; }
    extra="$(jq -r '(.guardrails.extra_blocks // [])[]' "$cfg" 2>/dev/null || true)"
    printf '%s\n%s\n' "$BUILTIN_BLOCKS" "$extra" | awk 'NF' | sort -u
    ;;
  --assert-not-weakened)
    cfg="${1:?config path required}"
    [ -f "$cfg" ] || { echo "config not found: $cfg" >&2; exit 2; }
    # Fail closed if the config declares ANY allow/exclude list naming a builtin.
    denied=""
    for key in allow allowlist exclude remove disable; do
      hits="$(jq -r --arg k "$key" '(.guardrails[$k] // [])[]' "$cfg" 2>/dev/null || true)"
      for h in $hits; do
        if printf '%s\n' "$BUILTIN_BLOCKS" | grep -qx "$h"; then
          denied="$denied $h"
        fi
      done
    done
    if [ -n "$denied" ]; then
      echo "growth.yml attempts to weaken built-in ethics blocks:$denied" >&2
      exit 1
    fi
    exit 0
    ;;
  *) usage ;;
esac
