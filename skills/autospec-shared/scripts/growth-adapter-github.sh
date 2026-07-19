#!/usr/bin/env bash
# growth-adapter-github.sh — fetch GitHub repo signals, emit normalized envelope.
# Fetch goes through GROWTH_FETCH_CMD (default curl) so tests inject fixtures.
# Missing credentials -> non-zero + empty (fail-closed).
set -euo pipefail

cfg="${1:-}"
[ -n "$cfg" ] || { echo "usage: growth-adapter-github.sh <config.json>" >&2; exit 2; }
[ -f "$cfg" ] || { echo "config not found: $cfg" >&2; exit 2; }
here="$(cd "$(dirname "$0")" && pwd)"

repo="$(jq -r '.measurement.github.repo // ""' "$cfg")"
tok_env="$(jq -r '.measurement.github.token_env // "GITHUB_TOKEN"' "$cfg")"
[ -n "$repo" ] || { echo "github adapter: .measurement.github.repo missing" >&2; exit 1; }
# Indirect env lookup, bash 3.2 safe.
tok="${!tok_env:-}"
[ -n "$tok" ] || { echo "github adapter: \$$tok_env unset (fail-closed)" >&2; exit 1; }

fetch="${GROWTH_FETCH_CMD:-curl -fsSL}"
raw="$($fetch "https://api.github.com/repos/$repo")" || { echo "github adapter: fetch failed" >&2; exit 1; }
trap 'rm -f "${tmp:-}"' EXIT
tmp="$(mktemp)"; printf '%s' "$raw" > "$tmp"
bash "$here/growth-measure.sh" --normalize github "$tmp"
rm -f "$tmp"
