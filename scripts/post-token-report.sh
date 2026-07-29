#!/usr/bin/env bash
# skills/autospec-run/scripts/post-token-report.sh — Post one idempotent token-usage comment.
#
# Usage:
#   post-token-report.sh --issue N --repo OWNER/REPO [--tokens-json FILE]
#
# Reads token counts from FILE (JSON with per-role sub-objects for
# implementer/reviewer/recovery, each with: input_tokens, output_tokens,
# cache_creation_input_tokens, cache_read_input_tokens, model; plus top-level
# "pr" field). Composes a ## Token usage comment and posts it to the issue.
#
# Idempotency: if a comment with the marker block
#   <!-- autospec-tokens:begin --> … <!-- autospec-tokens:end -->
# already exists on the issue, the script edits it in place rather than
# creating a new comment.
#
# Fallback: when --tokens-json is not provided, the file does not exist, or
# parsing fails, the script posts "tokens: unavailable on <harness>" ONCE and
# exits 0. The run NEVER fails due to missing token data.
#
# Exit codes:
#   0  Success (or best-effort fallback)
#   1  Usage error (missing required --issue or --repo argument)
#
# Compatible with bash 3.2+

set -eu

ISSUE=""
REPO=""
TOKENS_JSON=""

while [ $# -gt 0 ]; do
  case "$1" in
    --issue)
      if [ $# -lt 2 ]; then
        printf 'post-token-report.sh: --issue requires an argument\n' >&2
        exit 1
      fi
      ISSUE="$2"; shift 2 ;;
    --repo)
      if [ $# -lt 2 ]; then
        printf 'post-token-report.sh: --repo requires an argument\n' >&2
        exit 1
      fi
      REPO="$2"; shift 2 ;;
    --tokens-json)
      if [ $# -lt 2 ]; then
        printf 'post-token-report.sh: --tokens-json requires an argument\n' >&2
        exit 1
      fi
      TOKENS_JSON="$2"; shift 2 ;;
    -*)
      printf 'post-token-report.sh: unknown option: %s\n' "$1" >&2
      exit 1 ;;
    *)
      printf 'post-token-report.sh: unexpected argument: %s\n' "$1" >&2
      exit 1 ;;
  esac
done

if [ -z "$ISSUE" ] || [ -z "$REPO" ]; then
  printf 'Usage: post-token-report.sh --issue N --repo OWNER/REPO [--tokens-json FILE]\n' >&2
  exit 1
fi

MARKER_BEGIN="<!-- autospec-tokens:begin -->"
MARKER_END="<!-- autospec-tokens:end -->"

# ── Compose comment body ──────────────────────────────────────────────────────

compose_comment() {
  local tokens_json="$1"
  local harness="${AUTOSPEC_HARNESS:-unknown}"

  # If no tokens file or it doesn't exist, post fallback
  if [ -z "$tokens_json" ] || [ ! -f "$tokens_json" ]; then
    printf '%s\n' "## Token usage"
    printf '%s\n' "tokens: unavailable on ${harness}"
    printf '%s\n' "$MARKER_BEGIN"
    printf '%s\n' "$MARKER_END"
    return
  fi

  # Extract per-role totals (best-effort — absent fields default to 0/null)
  local impl_input impl_output impl_cache_c impl_cache_r impl_model
  local rev_input rev_output rev_model
  local rec_input rec_output
  local pr_num total_input total_output

  impl_input=$(jq '.implementer.input_tokens // 0' "$tokens_json" 2>/dev/null || printf '0')
  impl_output=$(jq '.implementer.output_tokens // 0' "$tokens_json" 2>/dev/null || printf '0')
  impl_model=$(jq -r '.implementer.model // ""' "$tokens_json" 2>/dev/null || printf '')
  rev_input=$(jq '.reviewer.input_tokens // 0' "$tokens_json" 2>/dev/null || printf '0')
  rev_output=$(jq '.reviewer.output_tokens // 0' "$tokens_json" 2>/dev/null || printf '0')
  rev_model=$(jq -r '.reviewer.model // ""' "$tokens_json" 2>/dev/null || printf '')
  rec_input=$(jq '.recovery.input_tokens // 0' "$tokens_json" 2>/dev/null || printf '0')
  rec_output=$(jq '.recovery.output_tokens // 0' "$tokens_json" 2>/dev/null || printf '0')
  pr_num=$(jq '.pr // empty' "$tokens_json" 2>/dev/null || printf '')

  # Compute per-role totals and grand total
  local impl_total rev_total rec_total
  impl_total=$(( impl_input + impl_output ))
  rev_total=$(( rev_input + rev_output ))
  rec_total=$(( rec_input + rec_output ))
  total_input=$(( impl_input + rev_input + rec_input ))
  total_output=$(( impl_output + rev_output + rec_output ))
  local grand_total=$(( total_input + total_output ))

  # Format numbers with commas (portable awk)
  fmt_num() {
    printf '%s' "$1" | awk '{
      n = $0 + 0
      s = sprintf("%d", n)
      result = ""
      len = length(s)
      for (i = 1; i <= len; i++) {
        if (i > 1 && (len - i + 1) % 3 == 0) result = result ","
        result = result substr(s, i, 1)
      }
      print result
    }'
  }

  local impl_fmt rev_fmt grand_fmt
  impl_fmt=$(fmt_num "$impl_total")
  rev_fmt=$(fmt_num "$rev_total")
  grand_fmt=$(fmt_num "$grand_total")

  # Build model suffix for implementer
  local impl_model_sfx=""
  if [ -n "$impl_model" ]; then
    impl_model_sfx=" ($impl_model)"
  fi

  # Build PR suffix
  local pr_sfx=""
  if [ -n "$pr_num" ]; then
    pr_sfx=" — PR #${pr_num}"
  fi

  # Build reviewer model suffix
  local rev_model_sfx=""
  if [ -n "$rev_model" ]; then
    rev_model_sfx=" ($rev_model)"
  fi

  # Build recovery line
  local rec_line="- recovery: —"
  if [ "$rec_total" -gt 0 ] 2>/dev/null; then
    rec_line="- recovery: $(fmt_num "$rec_total")"
  fi

  local impl_line rev_line total_line
  impl_line="- implementer: ${impl_fmt}${impl_model_sfx}${pr_sfx}"
  rev_line="- reviewer: ${rev_fmt}${rev_model_sfx}"
  total_line="- total: ${grand_fmt}"
  printf '%s\n' "## Token usage"
  printf '%s\n' "$impl_line"
  printf '%s\n' "$rev_line"
  printf '%s\n' "$rec_line"
  printf '%s\n' "$total_line"
  printf '%s\n' "$MARKER_BEGIN"
  printf '%s\n' "$MARKER_END"
}

# ── Check for existing marker comment ────────────────────────────────────────

find_existing_comment_id() {
  # Returns the numeric comment ID if a comment with our marker exists, else empty
  gh api "repos/${REPO}/issues/${ISSUE}/comments" --method GET \
    --paginate 2>/dev/null \
    | jq -r --arg marker "$MARKER_BEGIN" \
      '.[] | select(.body | contains($marker)) | .id' 2>/dev/null \
    | head -1 || true
}

# ── Main ──────────────────────────────────────────────────────────────────────

# Compose the comment body (best-effort; never fails)
comment_body=$(compose_comment "${TOKENS_JSON:-}") || {
  comment_body="$(printf '%s\n%s\n%s\n%s\n' \
    "## Token usage" \
    "tokens: unavailable on ${AUTOSPEC_HARNESS:-unknown}" \
    "$MARKER_BEGIN" \
    "$MARKER_END")"
}

# Print the composed body to stdout for visibility/testing
printf '%s\n' "$comment_body"

# Find existing comment to edit in place
existing_id=$(find_existing_comment_id 2>/dev/null || true)

if [ -n "$existing_id" ]; then
  # Edit in place
  gh api "repos/${REPO}/issues/comments/${existing_id}" \
    --method PATCH \
    --field "body=${comment_body}" >/dev/null 2>&1 || true
else
  # Post new comment
  gh issue comment "$ISSUE" --repo "$REPO" --body "$comment_body" >/dev/null 2>&1 || true
fi

exit 0
