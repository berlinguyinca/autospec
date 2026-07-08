#!/usr/bin/env bash
# validate-growth-config.sh — validate a JSON rendering of .autospec/growth.yml.
# Usage: validate-growth-config.sh <config.json>   (exit 0 valid; non-zero invalid)
set -euo pipefail

CONFIG="${1:?usage: validate-growth-config.sh <config.json>}"
if [ ! -f "$CONFIG" ]; then echo "config not found: $CONFIG" >&2; exit 2; fi
if ! jq -e . "$CONFIG" >/dev/null 2>&1; then echo "not valid JSON: $CONFIG" >&2; exit 2; fi

fail() { echo "growth.yml invalid: $1" >&2; exit 1; }

# Required fields (jq-driven; keeps zero external schema-validator dependency).
jq -e '.product.name // empty | select(. != "")' "$CONFIG" >/dev/null || fail "product.name is required"
jq -e '.site.url // empty | test("^https?://")'  "$CONFIG" >/dev/null || fail "site.url must be an http(s) URL"
jq -e '.site.repo_path // empty'                 "$CONFIG" >/dev/null || fail "site.repo_path is required"
jq -e '.measurement'                             "$CONFIG" >/dev/null || fail "measurement block is required"
jq -e '.approval.control_repo // empty'          "$CONFIG" >/dev/null || fail "approval.control_repo is required"

# Secrets must be env-var references only: reject any inline-looking token value.
if jq -e '.measurement.analytics.token // empty' "$CONFIG" >/dev/null 2>&1; then
  fail "inline secret detected: use measurement.analytics.token_env (env-var name), not token"
fi

exit 0
