#!/usr/bin/env bash
# growth-outbound-queue.sh — deterministic body/label logic for the outbound
# approval control issue. No gh calls (the skill shells gh). Fail-closed.
set -euo pipefail

build_body() {
  local draft="${1:?draft required}"
  [ -f "$draft" ] || { echo "draft not found: $draft" >&2; exit 2; }
  jq . "$draft" >/dev/null 2>&1 || { echo "malformed draft: $draft" >&2; exit 1; }
  # Render a stable markdown body. jq -r pulls fields; missing -> fail-closed.
  jq -r '
    "## Outbound draft ready for approval",
    "",
    "**Source issue:** #\(.issue)",
    "**Platform:** \(.platform)",
    "**Target:** \(.target_url)",
    "**Self-promo rule satisfied:** \(.self_promo_rule)",
    (if (.disclosure? // "") != "" then "**Disclosure:** \(.disclosure)" else empty end),
    "",
    "### Ready-to-paste content",
    "",
    .body,
    "",
    "### Evidence",
    (.evidence[] | "- \(.)"),
    "",
    "---",
    "Apply `growth/approved` to receive the one-click package, edit this body to re-gate, or `growth/rejected` to drop.  The agent never posts on your behalf."
  ' "$draft"
}

read_state() {
  local csv="${1:-}"
  # Split on commas; exact match each token (string equality, never regex).
  local IFS=','; local tok; local approved=0 rejected=0 edited=0 published=0
  for tok in $csv; do
    case "$tok" in
      "growth/published") published=1 ;;
      "growth/rejected")  rejected=1 ;;
      "growth/approved")  approved=1 ;;
      "growth/edited")    edited=1 ;;
    esac
  done
  # `published` is terminal (human confirmed the post is live): it outranks the
  # decision labels so R3 records the ledger line and closes the issue, even if
  # a prior decision label still lingers.
  if [ "$published" -eq 1 ]; then echo "published"
  elif [ "$rejected" -eq 1 ]; then echo "rejected"
  elif [ "$approved" -eq 1 ]; then echo "approved"
  elif [ "$edited" -eq 1 ]; then echo "edited"
  else echo "pending"; fi
}

cmd="${1:-}"; shift || true
case "$cmd" in
  --build-body) build_body "$@" ;;
  --read-state) read_state "$@" ;;
  *) echo "usage: growth-outbound-queue.sh --build-body <draft.json> | --read-state <label-csv>" >&2; exit 2 ;;
esac
