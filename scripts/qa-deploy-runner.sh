#!/usr/bin/env bash
# qa-deploy-runner.sh — autospec-qa deploy contract runner (CORE: parse +
# safety floor + no-op). Part of #694; this is child #1293.
#
# SCOPE (this child): contract load + ajv validation, exit-0 no-op when the
# contract is absent, and the three non-negotiable safety-floor rules. It does
# NOT execute stages, run health probes, or write the verdict deploy block —
# those land in #1294/#1295.
#
# Usage:
#   qa-deploy-runner.sh --repo-dir <dir> [--verdict <path>]
#
# Reads <repo-dir>/.autospec/qa-deploy.yml. The file's PRESENCE makes deploy
# mandatory; its ABSENCE is a no-op (today's behavior, byte-for-byte).
#
# Exit codes (shared with the spec's Error handling table):
#   0 = ok / no-op (contract absent, or — in later children — deploy passed)
#   2 = usage / contract invalid (ajv fail, malformed YAML, missing required,
#       or a missing yq/jq/ajv tool) -> category code_health:qa_deploy_invalid_contract
#   3 = safety-floor violation -> category one of:
#         qa_deploy_forbidden_target | qa_deploy_prod_pattern | qa_deploy_missing_records_cap
#
# Per the project memory:
#   * set -u, NOT set -e — failures are handled explicitly so the right exit
#     code/category is emitted (no `[ test ] && action` one-sided conditionals
#     under set -e; we use if/then/fi throughout).
#   * bash 3.2-safe: no `[ -f <(...) ]` process substitution against a test;
#     all intermediate data is written to real temp files first.
#   * injection-safe matching: contract-supplied values (forbidden tokens) are
#     matched as LITERAL substrings via `grep -iF -e "$token"` — never
#     interpolated into a regex test(). The production-pattern and data-clone
#     regexes are FIXED (compiled into this script), with the command/name as
#     the searched input, so no user value reaches a regex.

set -u

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
SCHEMA_FILE="$REPO_ROOT/schemas/autospec-qa-deploy.schema.json"

REPO_DIR=""
VERDICT_PATH=""

# ── Emit a category + message, return the matching exit code ──────────────────
# emit_and_exit <exit_code> <category> <message>
emit_and_exit() {
    local code="$1" category="$2" message="$3"
    printf 'qa-deploy-runner: %s: %s\n' "$category" "$message" >&2
    # Also print the bare category to stdout so callers/tests can scrape it
    # without parsing stderr formatting.
    printf '%s\n' "$category"
    exit "$code"
}

# ── Arg parsing ──────────────────────────────────────────────────────────────
while [ $# -gt 0 ]; do
    case "$1" in
        --repo-dir)
            REPO_DIR="${2:-}"
            shift 2
            ;;
        --verdict)
            VERDICT_PATH="${2:-}"
            shift 2
            ;;
        -h|--help)
            sed -n '2,30p' "$0"
            exit 0
            ;;
        *)
            emit_and_exit 2 "code_health:qa_deploy_invalid_contract" "unknown argument: $1"
            ;;
    esac
done

if [ -z "$REPO_DIR" ]; then
    emit_and_exit 2 "code_health:qa_deploy_invalid_contract" "--repo-dir <dir> is required"
fi
if [ ! -d "$REPO_DIR" ]; then
    emit_and_exit 2 "code_health:qa_deploy_invalid_contract" "repo dir not found: $REPO_DIR"
fi

CONTRACT_FILE="$REPO_DIR/.autospec/qa-deploy.yml"

# ── No-op: contract absent ───────────────────────────────────────────────────
# Absence is byte-for-byte today's behavior. Write nothing, touch no verdict.
if [ ! -f "$CONTRACT_FILE" ]; then
    exit 0
fi

# ── Dependency checks (missing tool -> exit 2 + actionable brew message) ──────
if ! command -v yq >/dev/null 2>&1; then
    emit_and_exit 2 "code_health:qa_deploy_invalid_contract" \
        "yq not found. Install with: brew install yq"
fi
if ! command -v jq >/dev/null 2>&1; then
    emit_and_exit 2 "code_health:qa_deploy_invalid_contract" \
        "jq not found. Install with: brew install jq"
fi
if ! command -v ajv >/dev/null 2>&1; then
    emit_and_exit 2 "code_health:qa_deploy_invalid_contract" \
        "ajv CLI not found. Install with: npm install -g ajv-cli"
fi

# ── Step 1: YAML -> JSON via yq (malformed YAML fails here) ───────────────────
YQ_ERR="$(mktemp -t qa-deploy-yq-err.XXXXXX)"
CONTRACT_JSON_FILE="$(mktemp -t qa-deploy-contract.XXXXXX)"
cleanup() { rm -f "$YQ_ERR" "$CONTRACT_JSON_FILE"; }
trap cleanup EXIT

if ! yq -o=json '.' "$CONTRACT_FILE" > "$CONTRACT_JSON_FILE" 2>"$YQ_ERR"; then
    emit_and_exit 2 "code_health:qa_deploy_invalid_contract" \
        "failed to parse $CONTRACT_FILE: $(cat "$YQ_ERR")"
fi

CONTRACT_JSON="$(cat "$CONTRACT_JSON_FILE")"
if [ "$CONTRACT_JSON" = "null" ] || [ -z "$CONTRACT_JSON" ]; then
    emit_and_exit 2 "code_health:qa_deploy_invalid_contract" \
        "$CONTRACT_FILE parsed to empty/null"
fi

# ── Step 2: ajv schema validation (missing required / wrong shape -> exit 2) ──
if [ ! -f "$SCHEMA_FILE" ]; then
    emit_and_exit 2 "code_health:qa_deploy_invalid_contract" \
        "schema not found: $SCHEMA_FILE"
fi

# Use Node/Ajv2020 directly (same approach as validate-contract.sh: ajv-cli's
# --spec flag has a known parsing bug for draft2020).
AJV_OUT="$(node -e "
try {
  var fs = require('fs');
  var Ajv2020;
  try { Ajv2020 = require('ajv/dist/2020'); }
  catch(e) {
    var paths = [
      '/opt/homebrew/lib/node_modules/ajv-cli/node_modules/ajv/dist/2020',
      '/usr/local/lib/node_modules/ajv-cli/node_modules/ajv/dist/2020'
    ];
    for (var i = 0; i < paths.length; i++) {
      try { Ajv2020 = require(paths[i]); break; } catch(e2) {}
    }
  }
  if (!Ajv2020) { console.error('ajv/dist/2020 not found'); process.exit(1); }
  var schema = JSON.parse(fs.readFileSync('$SCHEMA_FILE', 'utf8'));
  var data   = JSON.parse(fs.readFileSync('$CONTRACT_JSON_FILE', 'utf8'));
  var ajv    = new Ajv2020({strict: false, allErrors: true});
  var valid  = ajv.validate(schema, data);
  if (valid) { console.log('valid'); process.exit(0); }
  else { console.error(ajv.errorsText()); process.exit(2); }
} catch(e) { console.error(e.message); process.exit(1); }
" 2>&1)"
AJV_RC=$?
if [ "$AJV_RC" -ne 0 ]; then
    emit_and_exit 2 "code_health:qa_deploy_invalid_contract" \
        "schema validation failed: $AJV_OUT"
fi

# ── Safety floor ─────────────────────────────────────────────────────────────
# Enforced BEFORE any stage execution; each violation aborts immediately with
# exit 3 and the named category. No stage runs once any rule trips.

# Collect the values we need as newline-delimited lists via jq (jq reads the
# JSON; no contract value is interpolated into any pattern).
FORBIDDEN_LIST="$(printf '%s' "$CONTRACT_JSON" | jq -r '.target_envs.forbidden[]? // empty')"

# All stage commands (deploy stages).
STAGE_COMMANDS="$(printf '%s' "$CONTRACT_JSON" | jq -r '.stages[]?.command // empty')"
# All teardown commands.
TEARDOWN_COMMANDS="$(printf '%s' "$CONTRACT_JSON" | jq -r '.teardown[]?.command // empty')"
# All health_check URLs.
HEALTHCHECK_URLS="$(printf '%s' "$CONTRACT_JSON" | jq -r '.stages[]?.health_check?.url // empty')"

# ── Rule 1: Forbidden-target match ───────────────────────────────────────────
# Each target_envs.forbidden token matched LITERAL, case-insensitive substring
# against each stage command, each health_check.url. (stdout matching happens at
# stage run time in #1294.) `grep -iF -e "$token"` keeps the token literal — no
# regex interpolation, so a token like "a.b.c" or "x|y" can never act as a
# pattern. The searched haystack is the command/url text on stdin.
HAYSTACK_FOR_FORBIDDEN="$(printf '%s\n%s\n' "$STAGE_COMMANDS" "$HEALTHCHECK_URLS")"
if [ -n "$FORBIDDEN_LIST" ]; then
    while IFS= read -r token; do
        if [ -z "$token" ]; then
            continue
        fi
        if printf '%s' "$HAYSTACK_FOR_FORBIDDEN" | grep -iF -e "$token" >/dev/null 2>&1; then
            emit_and_exit 3 "qa_deploy_forbidden_target" \
                "forbidden target token '$token' appears in a stage command or health_check.url"
        fi
    done <<EOF
$FORBIDDEN_LIST
EOF
fi

# ── Rule 2: Production-pattern rejection ──────────────────────────────────────
# Each command (deploy + teardown) word-boundary case-insensitive matched
# against a FIXED set of production patterns. The regex is hardcoded here; the
# command text is the searched input (stdin), so no contract value reaches the
# pattern. Patterns: --prod, --production, " prod ", " production ",
# production-only, live-prod. `--prod(uction)?` is anchored so it does not match
# inside an unrelated longer flag (e.g. --prod-dry-run-disabled would still
# match --prod as a token, which is the intended over-block).
PROD_REGEX='(--prod([^a-z0-9]|$)|--production([^a-z0-9]|$)| prod | production |production-only|live-prod)'
ALL_COMMANDS="$(printf '%s\n%s\n' "$STAGE_COMMANDS" "$TEARDOWN_COMMANDS")"
if [ -n "$ALL_COMMANDS" ]; then
    while IFS= read -r cmd; do
        if [ -z "$cmd" ]; then
            continue
        fi
        if printf '%s' "$cmd" | grep -iE "$PROD_REGEX" >/dev/null 2>&1; then
            emit_and_exit 3 "qa_deploy_prod_pattern" \
                "production pattern detected in command: $cmd"
        fi
    done <<EOF
$ALL_COMMANDS
EOF
fi

# ── Rule 3: max_records required for data-clone stages ────────────────────────
# Any stage whose name OR command matches (case-insensitive) clone|copy|
# replicate|sync MUST declare safety.max_records. The clone regex is FIXED; the
# name/command is the searched input. We resolve per-stage with jq so a missing
# max_records is detected precisely (null/absent -> violation).
CLONE_REGEX='(clone|copy|replicate|sync)'
STAGE_COUNT="$(printf '%s' "$CONTRACT_JSON" | jq -r '.stages | length // 0')"
i=0
while [ "$i" -lt "$STAGE_COUNT" ]; do
    s_name="$(printf '%s' "$CONTRACT_JSON" | jq -r ".stages[$i].name // \"\"")"
    s_cmd="$(printf '%s' "$CONTRACT_JSON" | jq -r ".stages[$i].command // \"\"")"
    s_max="$(printf '%s' "$CONTRACT_JSON" | jq -r ".stages[$i].safety.max_records // \"\"")"
    is_clone=0
    if printf '%s' "$s_name" | grep -iE "$CLONE_REGEX" >/dev/null 2>&1; then
        is_clone=1
    elif printf '%s' "$s_cmd" | grep -iE "$CLONE_REGEX" >/dev/null 2>&1; then
        is_clone=1
    fi
    if [ "$is_clone" -eq 1 ] && [ -z "$s_max" ]; then
        emit_and_exit 3 "qa_deploy_missing_records_cap" \
            "data-clone stage '$s_name' is missing required safety.max_records"
    fi
    i=$((i + 1))
done

# ── Parse + safety floor passed ──────────────────────────────────────────────
# Stage execution, health probes, and verdict-write are deferred to #1294/#1295.
# A valid, safe contract that this core cannot yet execute exits 0 (no-op for
# the execution half) so the gate does not block on the unimplemented portion.
exit 0
