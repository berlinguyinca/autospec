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
    if ! jq -e . "$cfg" >/dev/null 2>&1; then
      echo "cannot parse config: $cfg" >&2; exit 1
    fi
    extra="$(jq -r '(.guardrails.extra_blocks // [])[]' "$cfg" 2>/dev/null || true)"
    printf '%s\n%s\n' "$BUILTIN_BLOCKS" "$extra" | awk 'NF' | sort -u
    ;;
  --assert-not-weakened)
    cfg="${1:?config path required}"
    [ -f "$cfg" ] || { echo "config not found: $cfg" >&2; exit 2; }
    if ! jq -e . "$cfg" >/dev/null 2>&1; then
      echo "cannot parse config: $cfg" >&2; exit 1
    fi
    # Fail closed if the config names a builtin anywhere under .guardrails
    # (except .guardrails.extra_blocks, which legitimately adds blocks).
    if ! strings="$(jq -r 'del(.guardrails.extra_blocks) | .guardrails // {} | [.. | strings] | .[]' "$cfg")"; then
      echo "cannot evaluate guardrails: $cfg" >&2; exit 1
    fi
    denied=""
    for s in $strings; do
      if printf '%s\n' "$BUILTIN_BLOCKS" | grep -qx "$s"; then
        denied="$denied $s"
      fi
    done
    if [ -n "$denied" ]; then
      echo "growth.yml attempts to weaken built-in ethics blocks:$denied" >&2
      exit 1
    fi
    exit 0
    ;;
  *) usage ;;
esac
