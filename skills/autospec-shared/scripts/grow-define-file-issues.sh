#!/usr/bin/env bash
# grow-define-file-issues.sh — file a GitHub issue per ranked candidate and
# append a pending ledger line only after successful creation.
set -euo pipefail

RANKED="${1:?usage: grow-define-file-issues.sh <ranked.jsonl> <config.json>}"
CONFIG="${2:?usage: grow-define-file-issues.sh <ranked.jsonl> <config.json>}"
for f in "$RANKED" "$CONFIG"; do
  if [ ! -f "$f" ]; then echo "not found: $f" >&2; exit 2; fi
done

HERE="$(cd "$(dirname "$0")" && pwd)"
LEDGER_SH="$HERE/growth-ledger.sh"

while IFS= read -r line; do
  [ -n "$line" ] || continue
  lens="$(echo "$line" | jq -r '.lens')"
  kind="$(echo "$line" | jq -r '.kind')"
  channel="$(echo "$line" | jq -r '.channel')"
  title="$(echo "$line" | jq -r '.title')"
  norm="$(echo "$line" | jq -r '.norm_title')"
  rationale="$(echo "$line" | jq -r '.rationale // ""')"

  if [ "$kind" = "artifact" ]; then
    labels="auto-implement,growth:artifact,growth:$channel,origin:self"
    body="$(printf 'Growth artifact (lens: %s, channel: %s)\n\n%s\n\nFiled by /autospec-grow-define.' "$lens" "$channel" "$rationale")"
  else
    labels="growth:outbound,growth/needs-draft,growth:$channel,origin:self"
    body="$(printf 'Growth outbound draft needed (lens: %s, channel: %s)\n\nTarget/rule: see rationale.\n\n%s\n\nDrafted + gated + queued by /autospec-grow-run.' "$lens" "$channel" "$rationale")"
  fi

  # origin:self provenance (issue #1745): idempotent, best-effort label
  # auto-creation — a create/exists failure never blocks filing.
  gh label create origin:self --color 8250df --force >/dev/null 2>&1 || true
  if ! url="$(gh issue create --title "$title" --body "$body" --label "$labels" 2>/dev/null)"; then
    echo "grow-define: gh issue create failed for: $title (skipped)" >&2
    continue
  fi
  num="$(printf '%s' "$url" | grep -oE '[0-9]+$' || true)"
  if [ -z "$num" ]; then
    echo "grow-define: could not parse issue number from: $url (skipped)" >&2
    continue
  fi

  ledline="$(jq -n --arg s "$lens" --arg t "$title" --arg n "$norm" \
     --arg c "$channel" --arg k "$kind" --argjson i "$num" \
     '{round:1,source:$s,title:$t,norm_title:$n,channel:$c,kind:$k,issue:$i,outcome:"pending",reason:"",ts:"1970-01-01T00:00:00Z"}')"
  "$LEDGER_SH" --append "$ledline"
  echo "$num"
done < "$RANKED"
