#!/usr/bin/env bash
# validate-growth-candidate.sh — validate one growth candidate JSON object.
# Usage: validate-growth-candidate.sh <candidate.json>   (exit 0 valid)
set -euo pipefail

C="${1:?usage: validate-growth-candidate.sh <candidate.json>}"
if [ ! -f "$C" ]; then echo "candidate not found: $C" >&2; exit 2; fi
if ! jq -e . "$C" >/dev/null 2>&1; then echo "not valid JSON: $C" >&2; exit 2; fi
jq -e 'type=="object"' "$C" >/dev/null 2>&1 || { echo "candidate invalid: candidate must be a JSON object" >&2; exit 1; }

fail() { echo "candidate invalid: $1" >&2; exit 1; }

jq -e '.lens as $l | ["technical-seo","keyword-gap","content-opportunity","community","directory","backlink"] | index($l)' "$C" >/dev/null \
  || fail "lens must be one of the 6 known lenses"
jq -e '.channel as $c | ["technical_seo","content","outreach","directories"] | index($c)' "$C" >/dev/null \
  || fail "channel must be one of technical_seo|content|outreach|directories"
jq -e '.kind as $k | ["artifact","outbound"] | index($k)' "$C" >/dev/null \
  || fail "kind must be artifact|outbound"
jq -e '(.title|type)=="string" and (.title|length>0)' "$C" >/dev/null || fail "title is required (non-empty string)"
jq -e '(.norm_title|type)=="string" and (.norm_title|length>0)' "$C" >/dev/null || fail "norm_title is required (non-empty string)"
jq -e '.roi | numbers and . == floor and . >= 1 and . <= 5' "$C" >/dev/null \
  || fail "roi must be an integer 1..5"
jq -e '.severity | numbers and . == floor and . >= 1 and . <= 5' "$C" >/dev/null \
  || fail "severity must be an integer 1..5"
jq -e '.effort as $e | ["small","medium","large"] | index($e)' "$C" >/dev/null \
  || fail "effort must be small|medium|large"
jq -e '.confidence | numbers and . >= 0 and . <= 1' "$C" >/dev/null \
  || fail "confidence must be a number 0..1"

exit 0
