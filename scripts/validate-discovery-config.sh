#!/usr/bin/env bash
# validate-discovery-config.sh — fail-closed validator for the `discovery.*` block of
# .autospec/autospec.yml (policy: auto). Mirrors validate-growth-config.sh.
# Usage: validate-discovery-config.sh <autospec.yml>   (exit 0 valid; non-zero invalid)
# Exit codes: 0 valid, 1 invalid value/type, 2 unreadable input / not parseable YAML or JSON.
set -euo pipefail

CONFIG="${1:?usage: validate-discovery-config.sh <autospec.yml>}"
if [ ! -f "$CONFIG" ]; then echo "config not found: $CONFIG" >&2; exit 2; fi

if ! command -v yq >/dev/null 2>&1; then
  echo "validate-discovery-config: yq is required (brew install yq)" >&2
  exit 2
fi

JSON_TMP="$(mktemp -t discovery-config-json.XXXXXX)"
trap 'rm -f "$JSON_TMP"' EXIT

if ! yq -o=json '.' "$CONFIG" > "$JSON_TMP" 2>/dev/null; then
  echo "not valid YAML/JSON: $CONFIG" >&2
  exit 2
fi

fail() { echo "discovery config invalid: $1" >&2; exit 1; }

# No discovery block at all is fine — discovery defaults to disabled/absent.
if ! jq -e '.discovery // empty' "$JSON_TMP" >/dev/null 2>&1; then
  exit 0
fi

D='.discovery'

# enabled: bool
if jq -e "${D}.enabled != null" "$JSON_TMP" >/dev/null 2>&1; then
  jq -e "(${D}.enabled | type) == \"boolean\"" "$JSON_TMP" >/dev/null 2>&1 \
    || fail "discovery.enabled must be a boolean"
fi

# seed_sources: array
if jq -e "${D}.seed_sources != null" "$JSON_TMP" >/dev/null 2>&1; then
  jq -e "(${D}.seed_sources | type) == \"array\"" "$JSON_TMP" >/dev/null 2>&1 \
    || fail "discovery.seed_sources must be an array"
fi

# forbidden_classes: array (extend-only — see discovery-blocklist.sh for the builtin-union
# guarantee; here we only validate shape).
if jq -e "${D}.forbidden_classes != null" "$JSON_TMP" >/dev/null 2>&1; then
  jq -e "(${D}.forbidden_classes | type) == \"array\"" "$JSON_TMP" >/dev/null 2>&1 \
    || fail "discovery.forbidden_classes must be an array"
fi

# max_new_sources_per_round: integer >= 0
if jq -e "${D}.max_new_sources_per_round != null" "$JSON_TMP" >/dev/null 2>&1; then
  jq -e "(${D}.max_new_sources_per_round | type) == \"number\" and (${D}.max_new_sources_per_round == (${D}.max_new_sources_per_round | floor))" \
    "$JSON_TMP" >/dev/null 2>&1 \
    || fail "discovery.max_new_sources_per_round must be an integer"
  jq -e "${D}.max_new_sources_per_round >= 0" "$JSON_TMP" >/dev/null 2>&1 \
    || fail "discovery.max_new_sources_per_round must be >= 0"
fi

# userspace.opt_out: bool
if jq -e "${D}.userspace.opt_out != null" "$JSON_TMP" >/dev/null 2>&1; then
  jq -e "(${D}.userspace.opt_out | type) == \"boolean\"" "$JSON_TMP" >/dev/null 2>&1 \
    || fail "discovery.userspace.opt_out must be a boolean"
fi

# rate_limits: object
if jq -e "${D}.rate_limits != null" "$JSON_TMP" >/dev/null 2>&1; then
  jq -e "(${D}.rate_limits | type) == \"object\"" "$JSON_TMP" >/dev/null 2>&1 \
    || fail "discovery.rate_limits must be an object"
fi

# Secrets must be env-var-name-only: reject any inline-looking token/key/secret values
# anywhere under discovery.rate_limits (the only place per-source credentials would land).
if jq -e "${D}.rate_limits != null" "$JSON_TMP" >/dev/null 2>&1; then
  jq -e "[${D}.rate_limits | .. | objects | keys[]? | select(test(\"(?i)^(token|secret|api_key|apikey|password)$\"))] | length == 0" \
    "$JSON_TMP" >/dev/null 2>&1 \
    || fail "inline secret detected under discovery.rate_limits: use a *_env key (env-var name) instead"
fi

exit 0
