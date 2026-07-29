#!/usr/bin/env bash
# discovery-internet-forums.sh — internet-forums harvester (Reddit / HN / RSS).
# Adapters fetch structured feeds (Reddit .json, HN Algolia API, blog/newsfeed
# RSS), normalize each item to a trend-signal record, and append it via
# trend-ledger.sh. Every fetch is gated: discovery-safety.sh --rate-ok, then
# discovery-blocklist.sh --allowed, BEFORE any network access; the excerpt is
# always run through discovery-safety.sh --sanitize BEFORE it reaches the
# ledger. External content is untrusted DATA — it only ever proposes a
# trend-signal, it never authorizes an action.
#
# Usage:
#   discovery-internet-forums.sh reddit <subreddit> [config]
#   discovery-internet-forums.sh hn <query> [config]
#   discovery-internet-forums.sh rss <url> [config]
#
# Env:
#   DISCOVERY_LIVE            1 = fetch live over the network; else read fixtures.
#   DISCOVERY_FIXTURE_FILE    explicit fixture path (fixture mode only).
#   DISCOVERY_FIXTURE_DIR     fixture directory (fixture mode, default lookup).
#   AUTOSPEC_DISCOVERY_CONFIG default discovery config path.
#   AUTOSPEC_TREND_LEDGER     forwarded to trend-ledger.sh.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SAFETY="$SCRIPT_DIR/discovery-safety.sh"
BLOCKLIST="$SCRIPT_DIR/discovery-blocklist.sh"
LEDGER="$SCRIPT_DIR/trend-ledger.sh"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"

SOURCE_NAME="internet-forums"
DEFAULT_CFG="${AUTOSPEC_DISCOVERY_CONFIG:-.autospec/autospec.yml}"
FIXTURE_DIR="${DISCOVERY_FIXTURE_DIR:-$REPO_ROOT/tests/fixtures/discovery-internet-forums}"

usage() {
  echo "usage: discovery-internet-forums.sh reddit <subreddit> [config] | hn <query> [config] | rss <url> [config]" >&2
  exit 2
}

adapter="${1:-}"; [ -n "$adapter" ] || usage
arg="${2:-}"; [ -n "$arg" ] || usage
cfg="${3:-$DEFAULT_CFG}"

case "$adapter" in
  reddit|hn|rss) : ;;
  *) usage ;;
esac

# ---------------------------------------------------------------------------
# domain resolution (used by the blocklist allowlist gate)
# ---------------------------------------------------------------------------
domain_for() {
  case "$1" in
    reddit) echo "reddit.com" ;;
    hn)     echo "hn.algolia.com" ;;
    rss)    printf '%s' "$2" | sed -E 's#^[A-Za-z][A-Za-z0-9+.-]*://##; s#/.*$##; s#:[0-9]+$##' ;;
  esac
}

domain="$(domain_for "$adapter" "$arg")"

# ---------------------------------------------------------------------------
# gate BEFORE any fetch: per-source rate limit, then domain allowlist.
# ---------------------------------------------------------------------------
if ! "$SAFETY" --rate-ok "$SOURCE_NAME" "$cfg" >&2; then
  echo "discovery-internet-forums: rate limit exceeded for $SOURCE_NAME, refusing fetch" >&2
  exit 1
fi
if ! "$BLOCKLIST" --allowed "$domain" "$cfg" >&2; then
  echo "discovery-internet-forums: domain not allowed, refusing fetch: $domain" >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# fetch: live network behind DISCOVERY_LIVE=1, else local fixtures (offline).
# ---------------------------------------------------------------------------
url_encode() { jq -rn --arg s "$1" '$s|@uri'; }

fetch_raw() {
  if [ "${DISCOVERY_LIVE:-0}" = "1" ]; then
    case "$adapter" in
      reddit) curl -sL --max-time 20 -A "autospec-discovery/1.0 (+https://github.com/berlinguyinca/autospec)" \
                "https://www.reddit.com/r/${arg}/.json" ;;
      hn)     curl -sL --max-time 20 \
                "https://hn.algolia.com/api/v1/search?query=$(url_encode "$arg")" ;;
      rss)    curl -sL --max-time 20 "$arg" ;;
    esac
    return 0
  fi

  local f="${DISCOVERY_FIXTURE_FILE:-}"
  if [ -z "$f" ]; then
    case "$adapter" in
      reddit) f="$FIXTURE_DIR/reddit-${arg}.json" ;;
      hn)     f="$FIXTURE_DIR/hn-${arg}.json" ;;
      rss)    f="$FIXTURE_DIR/rss-default.xml" ;;
    esac
  fi
  [ -f "$f" ] && cat "$f" || true
}

raw="$(fetch_raw)"

# ---------------------------------------------------------------------------
# parse: adapter-specific structured-feed parsers. Each emits TSV rows of
# id \t title \t link (no live network in the parsers themselves).
# ---------------------------------------------------------------------------
parse_reddit() {
  printf '%s' "$1" | jq -r '
    (.data.children // [])[]? | .data |
    [ (.id // ""), (.title // ""), (.url // (if .permalink then ("https://www.reddit.com" + .permalink) else "" end)) ]
    | @tsv' 2>/dev/null || true
}

parse_hn() {
  printf '%s' "$1" | jq -r '
    (.hits // [])[]? |
    [ (.objectID // ""), (.title // ""), (.url // (if .objectID then ("https://news.ycombinator.com/item?id=" + .objectID) else "" end)) ]
    | @tsv' 2>/dev/null || true
}

parse_rss() {
  printf '%s' "$1" | awk '
    BEGIN { initem=0; title=""; link="" }
    /<item[ >]/  { initem=1; title=""; link="" }
    /<\/item>/ {
      if (initem && title != "") { print link "\t" title "\t" link }
      initem=0
    }
    initem && /<title>/ {
      line=$0
      gsub(/.*<title>/, "", line); gsub(/<\/title>.*/, "", line)
      gsub(/<!\[CDATA\[/, "", line); gsub(/\]\]>/, "", line)
      title=line
    }
    initem && /<link>/ {
      line=$0
      gsub(/.*<link>/, "", line); gsub(/<\/link>.*/, "", line)
      gsub(/<!\[CDATA\[/, "", line); gsub(/\]\]>/, "", line)
      link=line
    }
  ' 2>/dev/null || true
}

kind_for() {
  case "$1" in
    reddit) echo "reddit-post" ;;
    hn)     echo "hn-story" ;;
    rss)    echo "rss-item" ;;
  esac
}

case "$adapter" in
  reddit) items="$(parse_reddit "$raw")" ;;
  hn)     items="$(parse_hn "$raw")" ;;
  rss)    items="$(parse_rss "$raw")" ;;
esac

kind_label="$(kind_for "$adapter")"

# ---------------------------------------------------------------------------
# normalize -> sanitize excerpt -> ledger append/bump.
# ---------------------------------------------------------------------------
already_seen() {
  local key="$1"
  "$LEDGER" --show --json --source "$SOURCE_NAME" \
    | jq -r --arg k "$key" '[.[] | select(.norm_key==$k)] | length'
}

emit_signal() {
  local key="$1" title="$2" link="$3" kind="$4"
  local sanitized
  sanitized="$(printf '%s' "$title" | "$SAFETY" --sanitize)"
  # The sanitizer drops whole lines that look like prompt-injection directives;
  # a title that is entirely such a line sanitizes to empty. Fall back to a
  # neutral placeholder rather than fail validation on a blank summary.
  [ -n "$sanitized" ] || sanitized="[redacted: directive-like content stripped by sanitizer]"
  local now
  now="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  local evidence="$link"
  [ -n "$evidence" ] || evidence="https://${domain}/${arg}"
  local seen
  seen="$(already_seen "$key")"
  if [ "${seen:-0}" -gt 0 ]; then
    "$LEDGER" --bump-recurrence "$key" >/dev/null
  else
    local record
    record="$(jq -nc \
      --arg source "$SOURCE_NAME" \
      --arg kind "$kind" \
      --arg summary "$sanitized" \
      --arg norm_key "$key" \
      --arg evidence_ref "$evidence" \
      --arg first_seen "$now" \
      --arg excerpt "$sanitized" \
      --arg ts "$now" \
      '{source:$source, kind:$kind, summary:$summary, norm_key:$norm_key,
        evidence_ref:$evidence_ref, first_seen:$first_seen, recurrence:1,
        sanitized_excerpt:$excerpt, ts:$ts}')"
    "$LEDGER" --append "$record" >/dev/null
  fi
}

count=0
if [ -n "$items" ]; then
  while IFS=$'\t' read -r id title link; do
    [ -n "$title" ] || continue
    key="${adapter}:${arg}:${id}"
    emit_signal "$key" "$title" "$link" "$kind_label"
    count=$((count + 1))
  done <<< "$items"
fi

# Missing/empty feed → emit a fallback note instead of failing silently. This
# is not itself a candidate; it flags that a WebSearch/WebFetch fallback pass
# may be needed for this feedless source.
if [ "$count" -eq 0 ]; then
  fallback_key="${adapter}:${arg}:feed-unavailable"
  fallback_note="no structured feed items found for ${adapter} ${arg}; WebSearch/WebFetch fallback recommended"
  emit_signal "$fallback_key" "$fallback_note" "https://${domain}/${arg}" "feed-unavailable"
fi

exit 0
