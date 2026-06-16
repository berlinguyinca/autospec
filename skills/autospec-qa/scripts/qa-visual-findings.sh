#!/usr/bin/env bash
# qa-visual-findings.sh — shape LLM-vision verdicts into qa-verdict findings.
#
# The visual-fidelity loop captures route screenshots (gen-screenshots.mjs), a
# TIER_A vision model judges each against DESIGN.md tokens + the issue's
# interaction states, and emits per-route verdicts. This script turns those
# verdicts into the `category:"visual_fidelity"` findings the autospec-qa heal
# loop consumes from .autospec/qa-verdict.json.
#
# Input (stdin): JSON array of vision verdicts, each:
#   {"route":"/checkout","viewport":"desktop","status":"PASS|PARTIAL|FAIL","issues":["spacing != 8px token", ...]}
#
# Output (stdout): JSON array of qa-verdict findings (one per non-PASS verdict):
#   {"category":"visual_fidelity","release_blocking":<bool>,"status":...,"summary":...,"evidence":...}
#
# Usage:
#   qa-visual-findings.sh [--blocking-on FAIL|PARTIAL]   # default FAIL
#
# release_blocking: FAIL verdicts block by default; PARTIAL is advisory unless
# --blocking-on PARTIAL is given. PASS verdicts are dropped (no finding).
#
# Exit codes:
#   0  ok ([] when no input / all PASS)
#   2  jq missing / usage error
#
# Requires: bash 3.2+, jq

set +e

BLOCK_ON="FAIL"
while [ $# -gt 0 ]; do
  case "$1" in
    --blocking-on) shift; BLOCK_ON="${1:-FAIL}" ;;
    -h|--help) grep '^#' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "qa-visual-findings: unknown arg $1" >&2; exit 2 ;;
  esac
  shift
done
case "$BLOCK_ON" in FAIL|PARTIAL) ;; *) echo "qa-visual-findings: --blocking-on must be FAIL or PARTIAL" >&2; exit 2 ;; esac

command -v jq >/dev/null 2>&1 || { echo "qa-visual-findings: FATAL jq missing" >&2; exit 2; }

input="$(cat)"
[ -n "$input" ] || { printf '[]'; exit 0; }

out="$(printf '%s' "$input" | jq -c --arg block "$BLOCK_ON" '
  def slug(r): (r | ascii_downcase | gsub("^/";"") | gsub("[^a-z0-9]+";"_") | gsub("^_|_$";"")) as $s
               | (if $s == "" then "root" else $s end);
  [ .[]
    | select((.status // "PASS") != "PASS")
    | {
        category: "visual_fidelity",
        release_blocking: (.status == "FAIL" or ($block == "PARTIAL" and .status == "PARTIAL")),
        status: .status,
        summary: ("\(.route // "?") (\(.viewport // "desktop")): " + (((.issues // []) | join("; ")) | if . == "" then "visual fidelity deviates from DESIGN.md" else . end)),
        evidence: ("docs/assets/screenshots/" + slug(.route // "/") + "__" + (.viewport // "desktop") + ".png")
      }
  ]' 2>/dev/null)"

[ -n "$out" ] || { echo "qa-visual-findings: WARN malformed input — emitting []" >&2; printf '[]'; exit 0; }
printf '%s' "$out"
