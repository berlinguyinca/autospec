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
#   qa-deploy-runner.sh --repo-dir <dir> [--verdict <path>] [--skip-teardown]
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
SKIP_TEARDOWN=0

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
        --skip-teardown)
            SKIP_TEARDOWN=1
            shift
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

if ! yq -o=json '.' "$CONTRACT_FILE" > "$CONTRACT_JSON_FILE" 2>"$YQ_ERR" \
    && ! yq '.' "$CONTRACT_FILE" > "$CONTRACT_JSON_FILE" 2>"$YQ_ERR"; then
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
AJV_BIN="$(command -v ajv 2>/dev/null || true)"
AJV_OUT="$(AJV_BIN="$AJV_BIN" node -e "
try {
  var fs = require('fs');
  var path = require('path');
  var Ajv2020;
  try { Ajv2020 = require('ajv/dist/2020'); }
  catch(e) {
    var paths = [
      '/opt/homebrew/lib/node_modules/ajv-cli/node_modules/ajv/dist/2020',
      '/usr/local/lib/node_modules/ajv-cli/node_modules/ajv/dist/2020'
    ];
    var ajvBin = process.env.AJV_BIN || '';
    if (ajvBin) {
      try {
        var real = fs.realpathSync(ajvBin);
        paths.unshift(path.join(path.dirname(path.dirname(real)), 'node_modules', 'ajv', 'dist', '2020'));
      } catch(e0) {}
    }
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
# #1294: execute stages in order, run health probes, apply the run-time
# stdout forbidden-target check, and write the deploy: block atomically.
# (Teardown — #1295 — is intentionally NOT implemented here.)

# Resolve the verdict path (default: <repo-dir>/.autospec/qa-verdict.json).
if [ -z "$VERDICT_PATH" ]; then
    VERDICT_PATH="$REPO_DIR/.autospec/qa-verdict.json"
fi

# ── Portable hard wall-clock timeout ─────────────────────────────────────────
# macOS bash 3.2 has no `timeout`/`gtimeout` binary, so we run the command in
# the background and a sibling watchdog that kills it after N seconds. We
# group-kill ONLY when the child is its own process-group leader (pgid==pid) so
# we never nuke the caller's process group (per the explore-handoff kill-tree
# memory); otherwise we kill the single pid. The watchdog's own output is
# redirected and it is reaped, so no stray `sleep` orphans hold a pipe open.
#
# run_stage_with_timeout <timeout_secs> <command_string> <stdout_file>
# Returns: 0 ok, 124 timed out, other = command's own non-zero exit.
run_stage_with_timeout() {
    local _timeout="$1" _cmd="$2" _outfile="$3"
    local _cpid _wpid _rc

    # Launch the command in its own session if setsid is available so the
    # watchdog can group-kill safely; fall back to a plain background job.
    if command -v setsid >/dev/null 2>&1; then
        setsid bash -c "$_cmd" >"$_outfile" 2>&1 &
    else
        bash -c "$_cmd" >"$_outfile" 2>&1 &
    fi
    _cpid=$!

    # Watchdog: sleep then signal the child. Its stdout/stderr are discarded.
    (
        sleep "$_timeout"
        # Try a group kill only when the child leads its own group.
        _pgid="$(ps -o pgid= -p "$_cpid" 2>/dev/null | tr -d ' ')"
        if [ -n "$_pgid" ] && [ "$_pgid" = "$_cpid" ]; then
            kill -TERM "-$_pgid" 2>/dev/null
            sleep 1
            kill -KILL "-$_pgid" 2>/dev/null
        else
            kill -TERM "$_cpid" 2>/dev/null
            sleep 1
            kill -KILL "$_cpid" 2>/dev/null
        fi
    ) >/dev/null 2>&1 &
    _wpid=$!

    # Wait for the command; capture its exit status.
    if wait "$_cpid" 2>/dev/null; then
        _rc=0
    else
        _rc=$?
    fi

    # Reap the watchdog: if the command finished first, kill the watchdog (and
    # its sleep) so nothing lingers. pkill -P reaps the watchdog's sleep child.
    if kill -0 "$_wpid" 2>/dev/null; then
        pkill -P "$_wpid" 2>/dev/null
        kill -TERM "$_wpid" 2>/dev/null
    fi
    wait "$_wpid" 2>/dev/null || true

    # A command killed by SIGTERM/SIGKILL reports 143/137 (or 124 from us). If
    # the watchdog is what killed it (it is no longer alive AND rc indicates a
    # signal), normalize to 124 (timed out). We detect "timed out" by checking
    # whether the elapsed wall-clock exceeded the cap; simpler: a signal exit
    # (>128) after the watchdog fired is treated as a timeout.
    if [ "$_rc" -gt 128 ]; then
        return 124
    fi
    return "$_rc"
}

# ── Monotonic-ish millisecond clock ──────────────────────────────────────────
# Tests assert KEY presence + integer type, not exact ms. We use the wall clock
# (date) since a true monotonic source is not portable across bash 3.2/macOS;
# durations are advisory.
now_ms() {
    # date +%s%N is GNU-only; macOS date lacks %N. Use python3 (always present
    # for the verdict write) for a portable millisecond timestamp.
    python3 -c 'import time; print(int(time.time()*1000))'
}

# ── Write a qa_deploy_failed verdict + finding, then exit 1 ───────────────────
# write_failure_and_exit <stage_name> <stdout_tail>
write_failure_and_exit() {
    local _stage="$1" _tail="$2"
    # stages_run so far is in $STAGES_RUN_JSON / durations in $DURATIONS_JSON.
    # NOTE: do NOT use ${VAR:-{}} brace defaults here — bash parses `:-{}}` as
    # default `{` plus a literal trailing `}`, corrupting valid JSON. These
    # accumulators are always initialized before any stage runs, so a plain
    # expansion is correct.
    VERDICT_PATH="$VERDICT_PATH" \
    DEPLOY_STAGE="$_stage" \
    DEPLOY_TAIL="$_tail" \
    STAGES_RUN_JSON="$STAGES_RUN_JSON" \
    DURATIONS_JSON="$DURATIONS_JSON" \
    TARGET_ENV="$TARGET_ENV" \
    HEAD_SHA="$HEAD_SHA_DEPLOYED" \
    python3 - <<'PY'
import json, os, sys

path   = os.environ["VERDICT_PATH"]
stage  = os.environ["DEPLOY_STAGE"]
tail   = os.environ["DEPLOY_TAIL"]
target = os.environ.get("TARGET_ENV", "")
head   = os.environ.get("HEAD_SHA", "")

verdict = {}
if os.path.isfile(path):
    try:
        with open(path) as f:
            verdict = json.load(f) or {}
    except Exception:
        verdict = {}

deploy = dict(verdict.get("deploy", {}) or {})
deploy["stages_run"]        = json.loads(os.environ.get("STAGES_RUN_JSON", "[]"))
deploy["stage_durations_ms"] = json.loads(os.environ.get("DURATIONS_JSON", "{}"))
if target:
    deploy["target_env"] = target
if head:
    deploy["head_sha_deployed"] = head
deploy["failed_stage"] = stage
verdict["deploy"] = deploy

findings = list(verdict.get("findings", []) or [])
findings.append({
    "category": "code_health:qa_deploy_failed",
    "summary":  "deploy stage '%s' failed; stdout tail: %s" % (stage, tail),
})
verdict["findings"] = findings

os.makedirs(os.path.dirname(path) or ".", exist_ok=True)
tmp = path + ".tmp"
with open(tmp, "w") as out:
    json.dump(verdict, out, indent=2)
    out.write("\n")
os.replace(tmp, path)
PY
    emit_and_exit 1 "code_health:qa_deploy_failed" \
        "deploy stage '$_stage' failed"
}

# ── Derive target_env: scheme+host of the first stage's health_check.url, ─────
# else the first target_envs.allowed entry, else "".
TARGET_ENV="$(printf '%s' "$CONTRACT_JSON" | jq -r '
    ([.stages[]?.health_check?.url // empty] | .[0] // "") as $u
    | if $u != "" then ($u | capture("^(?<o>[a-zA-Z][a-zA-Z0-9+.-]*://[^/]+)").o // $u)
      else (.target_envs.allowed[0]? // "") end
' 2>/dev/null)"
if [ -z "$TARGET_ENV" ] || [ "$TARGET_ENV" = "null" ]; then
    TARGET_ENV=""
fi

# ── head_sha_deployed: short HEAD of the TARGET repo dir (guard non-git) ──────
HEAD_SHA_DEPLOYED="$(git -C "$REPO_DIR" rev-parse --short HEAD 2>/dev/null || echo "")"
if [ -z "$HEAD_SHA_DEPLOYED" ]; then
    HEAD_SHA_DEPLOYED="unknown-not-a-git-repo"
fi

# ── Accumulators for the deploy: block (built as JSON via jq) ─────────────────
STAGES_RUN_JSON="[]"
DURATIONS_JSON="{}"
DATA_CLONE_JSON=""   # set if any stage emits records_copied=<int>

STAGE_OUT="$(mktemp -t qa-deploy-stage-out.XXXXXX)"
cleanup_stage() { rm -f "$STAGE_OUT"; }
# Extend the existing EXIT trap behavior (cleanup() removes YQ_ERR + contract).
trap 'cleanup; cleanup_stage' EXIT

# ── Execute each stage in order ───────────────────────────────────────────────
i=0
while [ "$i" -lt "$STAGE_COUNT" ]; do
    s_name="$(printf '%s' "$CONTRACT_JSON" | jq -r ".stages[$i].name // \"\"")"
    s_cmd="$(printf '%s' "$CONTRACT_JSON" | jq -r ".stages[$i].command // \"\"")"
    s_timeout="$(printf '%s' "$CONTRACT_JSON" | jq -r ".stages[$i].timeout // 300")"
    s_max="$(printf '%s' "$CONTRACT_JSON" | jq -r ".stages[$i].safety.max_records // \"\"")"

    # Build the env-prefix for the command. Pass env values VERBATIM but never
    # log/record VALUES — only key names are ever printed. We export each pair
    # into the bash -c subshell via a leading assignment string built with jq
    # (so values with spaces survive). Key NAMES only go to stderr.
    ENV_ASSIGNS="$(printf '%s' "$CONTRACT_JSON" | jq -r "
        (.stages[$i].env // {}) | to_entries
        | map(\"export \" + .key + \"=\" + (.value|tostring|@sh) + \"; \")
        | join(\"\")
    ")"
    ENV_KEYS="$(printf '%s' "$CONTRACT_JSON" | jq -r "(.stages[$i].env // {}) | keys | join(\",\")")"
    if [ -n "$ENV_KEYS" ]; then
        printf 'qa-deploy-runner: stage %s env keys: %s\n' "$s_name" "$ENV_KEYS" >&2
    fi

    printf 'qa-deploy-runner: running stage %s\n' "$s_name" >&2

    _start_ms="$(now_ms)"
    run_stage_with_timeout "$s_timeout" "$ENV_ASSIGNS$s_cmd" "$STAGE_OUT"
    _stage_rc=$?
    _end_ms="$(now_ms)"
    _dur_ms=$((_end_ms - _start_ms))
    if [ "$_dur_ms" -lt 0 ]; then _dur_ms=0; fi

    # Record this stage as run + its duration (regardless of pass/fail so the
    # failure verdict reflects what was attempted).
    STAGES_RUN_JSON="$(printf '%s' "$STAGES_RUN_JSON" | jq -c --arg n "$s_name" '. + [$n]')"
    DURATIONS_JSON="$(printf '%s' "$DURATIONS_JSON" | jq -c --arg n "$s_name" --argjson d "$_dur_ms" '. + {($n): $d}')"

    # stdout tail (last 20 lines) for diagnostics — never includes env values
    # (we never echoed them; the command's own output is captured).
    _tail="$(tail -n 20 "$STAGE_OUT" 2>/dev/null | tr '\n' ' ' | cut -c1-500)"

    # ── Run-time forbidden-target check against captured STDOUT (rule 1 ext) ──
    # Same literal, case-insensitive grep -iF matching as the static floor;
    # contract tokens are never interpolated into a regex.
    if [ -n "$FORBIDDEN_LIST" ]; then
        while IFS= read -r token; do
            if [ -z "$token" ]; then
                continue
            fi
            if grep -iF -e "$token" "$STAGE_OUT" >/dev/null 2>&1; then
                emit_and_exit 3 "qa_deploy_forbidden_target" \
                    "forbidden target token '$token' appeared in stdout of stage '$s_name'"
            fi
        done <<EOF
$FORBIDDEN_LIST
EOF
    fi

    # ── Stage command failure / timeout -> exit 1 ────────────────────────────
    if [ "$_stage_rc" -eq 124 ]; then
        write_failure_and_exit "$s_name" "stage timed out after ${s_timeout}s; $_tail"
    fi
    if [ "$_stage_rc" -ne 0 ]; then
        write_failure_and_exit "$s_name" "$_tail"
    fi

    # ── Parse data_clone (records_copied=<int>) from this stage's stdout ──────
    # FIXED regex; the stdout is the searched input. Take the last match.
    _rec="$(grep -E '^records_copied=[0-9]+$' "$STAGE_OUT" 2>/dev/null \
        | tail -n 1 | sed -E 's/^records_copied=([0-9]+)$/\1/')"
    if [ -n "$_rec" ]; then
        if [ -n "$s_max" ]; then
            DATA_CLONE_JSON="$(jq -nc --argjson r "$_rec" --argjson m "$s_max" \
                '{records_copied: $r, max_records: $m}')"
        else
            DATA_CLONE_JSON="$(jq -nc --argjson r "$_rec" '{records_copied: $r}')"
        fi
    fi

    # ── health_check (if present): poll url up to retry x retry_interval ──────
    HC_URL="$(printf '%s' "$CONTRACT_JSON" | jq -r ".stages[$i].health_check?.url // \"\"")"
    if [ -n "$HC_URL" ]; then
        HC_STATUS="$(printf '%s' "$CONTRACT_JSON" | jq -r ".stages[$i].health_check?.expect_status // 200")"
        HC_RETRY="$(printf '%s' "$CONTRACT_JSON" | jq -r ".stages[$i].health_check?.retry // 30")"
        HC_INTERVAL="$(printf '%s' "$CONTRACT_JSON" | jq -r ".stages[$i].health_check?.retry_interval // 10")"

        printf 'qa-deploy-runner: probing health for stage %s (%s attempts)\n' \
            "$s_name" "$HC_RETRY" >&2
        _attempt=0
        _ready=0
        while [ "$_attempt" -lt "$HC_RETRY" ]; do
            _code="$(curl -s -o /dev/null -w '%{http_code}' "$HC_URL" 2>/dev/null || echo "000")"
            if [ "$_code" = "$HC_STATUS" ]; then
                _ready=1
                break
            fi
            _attempt=$((_attempt + 1))
            if [ "$_attempt" -lt "$HC_RETRY" ]; then
                sleep "$HC_INTERVAL"
            fi
        done
        if [ "$_ready" -ne 1 ]; then
            write_failure_and_exit "$s_name" \
                "health_check exhausted ${HC_RETRY} retries against ${HC_URL} (expected ${HC_STATUS}); $_tail"
        fi
    fi

    i=$((i + 1))
done

# ── Success: write the deploy: block atomically (.tmp + mv) ───────────────────
# Additive — merge into any existing verdict; create minimal {"deploy":{...}}
# if none exists yet (deploy may run before the audit).
VERDICT_PATH="$VERDICT_PATH" \
STAGES_RUN_JSON="$STAGES_RUN_JSON" \
DURATIONS_JSON="$DURATIONS_JSON" \
TARGET_ENV="$TARGET_ENV" \
HEAD_SHA_DEPLOYED="$HEAD_SHA_DEPLOYED" \
DATA_CLONE_JSON="$DATA_CLONE_JSON" \
python3 - <<'PY'
import json, os

path = os.environ["VERDICT_PATH"]

verdict = {}
if os.path.isfile(path):
    try:
        with open(path) as f:
            verdict = json.load(f) or {}
    except Exception:
        verdict = {}

deploy = {}
deploy["stages_run"]         = json.loads(os.environ.get("STAGES_RUN_JSON", "[]"))
deploy["stage_durations_ms"] = json.loads(os.environ.get("DURATIONS_JSON", "{}"))
target = os.environ.get("TARGET_ENV", "")
if target:
    deploy["target_env"] = target
deploy["head_sha_deployed"]  = os.environ.get("HEAD_SHA_DEPLOYED", "")
dc = os.environ.get("DATA_CLONE_JSON", "")
if dc:
    deploy["data_clone"] = json.loads(dc)

# Additive: replace ONLY the deploy block, leave every other verdict key intact.
verdict["deploy"] = deploy

os.makedirs(os.path.dirname(path) or ".", exist_ok=True)
tmp = path + ".tmp"
with open(tmp, "w") as out:
    json.dump(verdict, out, indent=2)
    out.write("\n")
os.replace(tmp, path)
PY

# ── Teardown phase (#1295) ────────────────────────────────────────────────────
# Runs AFTER the deploy: block (the verdict is final). Each teardown command is
# subject to the SAME safety floor as deploy stages — the static prod-pattern
# rule already covered teardown commands above; here we add the run-time
# forbidden-target check (literal, case-insensitive grep -iF — no regex
# interpolation, per the jq/regex-injection memory) against each teardown
# command before running it. Failures of an `on_failure: warn` step log only;
# an `on_failure: fail` step downgrades a PASS verdict to PARTIAL (NEVER a FAIL
# or already-PARTIAL verdict). All verdict updates are atomic (.tmp+os.replace)
# and additive (only deploy.teardown_run / deploy.teardown_failures and the
# verdict downgrade are touched).

# Record teardown_run up front (false when skipped, true otherwise). This is an
# atomic, additive update — it preserves every other verdict + deploy key.
write_teardown_run() {
    local _run="$1"   # "true" or "false"
    VERDICT_PATH="$VERDICT_PATH" TEARDOWN_RUN="$_run" python3 - <<'PY'
import json, os
path = os.environ["VERDICT_PATH"]
run  = os.environ["TEARDOWN_RUN"] == "true"
verdict = {}
if os.path.isfile(path):
    try:
        with open(path) as f:
            verdict = json.load(f) or {}
    except Exception:
        verdict = {}
deploy = dict(verdict.get("deploy", {}) or {})
deploy["teardown_run"] = run
verdict["deploy"] = deploy
os.makedirs(os.path.dirname(path) or ".", exist_ok=True)
tmp = path + ".tmp"
with open(tmp, "w") as out:
    json.dump(verdict, out, indent=2)
    out.write("\n")
os.replace(tmp, path)
PY
}

# Append one failing teardown name and (if downgrade=1) rewrite PASS->PARTIAL.
# Atomic + additive: only deploy.teardown_failures and (conditionally) the
# top-level verdict are changed; a FAIL / already-PARTIAL verdict is untouched.
record_teardown_failure() {
    local _name="$1" _downgrade="$2"   # _downgrade: "1" to attempt PASS->PARTIAL
    VERDICT_PATH="$VERDICT_PATH" TEARDOWN_NAME="$_name" TEARDOWN_DOWNGRADE="$_downgrade" \
    python3 - <<'PY'
import json, os
path      = os.environ["VERDICT_PATH"]
name      = os.environ["TEARDOWN_NAME"]
downgrade = os.environ["TEARDOWN_DOWNGRADE"] == "1"
verdict = {}
if os.path.isfile(path):
    try:
        with open(path) as f:
            verdict = json.load(f) or {}
    except Exception:
        verdict = {}
deploy = dict(verdict.get("deploy", {}) or {})
failures = list(deploy.get("teardown_failures", []) or [])
failures.append(name)
deploy["teardown_failures"] = failures
verdict["deploy"] = deploy
# Downgrade ONLY a PASS verdict to PARTIAL. Never touch FAIL or PARTIAL.
if downgrade and verdict.get("verdict") == "PASS":
    verdict["verdict"] = "PARTIAL"
os.makedirs(os.path.dirname(path) or ".", exist_ok=True)
tmp = path + ".tmp"
with open(tmp, "w") as out:
    json.dump(verdict, out, indent=2)
    out.write("\n")
os.replace(tmp, path)
PY
}

# Seed an empty teardown_failures list so the key is always present when
# teardown runs. Atomic + additive (touches only deploy.teardown_failures).
seed_teardown_failures() {
    VERDICT_PATH="$VERDICT_PATH" python3 - <<'PY'
import json, os
path = os.environ["VERDICT_PATH"]
verdict = {}
if os.path.isfile(path):
    try:
        with open(path) as f:
            verdict = json.load(f) or {}
    except Exception:
        verdict = {}
deploy = dict(verdict.get("deploy", {}) or {})
if "teardown_failures" not in deploy:
    deploy["teardown_failures"] = []
verdict["deploy"] = deploy
os.makedirs(os.path.dirname(path) or ".", exist_ok=True)
tmp = path + ".tmp"
with open(tmp, "w") as out:
    json.dump(verdict, out, indent=2)
    out.write("\n")
os.replace(tmp, path)
PY
}

TEARDOWN_COUNT="$(printf '%s' "$CONTRACT_JSON" | jq -r '.teardown | length // 0')"

if [ "$SKIP_TEARDOWN" -eq 1 ]; then
    printf 'qa-deploy-runner: --skip-teardown set; skipping all teardown\n' >&2
    write_teardown_run "false"
else
    write_teardown_run "true"
    seed_teardown_failures

    t=0
    while [ "$t" -lt "$TEARDOWN_COUNT" ]; do
        td_name="$(printf '%s' "$CONTRACT_JSON" | jq -r ".teardown[$t].name // \"\"")"
        td_cmd="$(printf '%s' "$CONTRACT_JSON" | jq -r ".teardown[$t].command // \"\"")"
        td_on_failure="$(printf '%s' "$CONTRACT_JSON" | jq -r ".teardown[$t].on_failure // \"warn\"")"

        # ── Run-time forbidden-target check on the teardown COMMAND (rule 1) ──
        # Literal, case-insensitive; the command text is the searched input — no
        # contract value is interpolated into a regex.
        if [ -n "$FORBIDDEN_LIST" ]; then
            while IFS= read -r token; do
                if [ -z "$token" ]; then
                    continue
                fi
                if printf '%s' "$td_cmd" | grep -iF -e "$token" >/dev/null 2>&1; then
                    emit_and_exit 3 "qa_deploy_forbidden_target" \
                        "forbidden target token '$token' appears in teardown command '$td_name'"
                fi
            done <<EOF
$FORBIDDEN_LIST
EOF
        fi

        printf 'qa-deploy-runner: running teardown %s (on_failure=%s)\n' \
            "$td_name" "$td_on_failure" >&2

        # Run the teardown command with the same portable timeout wrapper as
        # deploy stages (default 300s — teardown has no per-step timeout field).
        run_stage_with_timeout 300 "$td_cmd" "$STAGE_OUT"
        td_rc=$?

        if [ "$td_rc" -ne 0 ]; then
            if [ "$td_on_failure" = "fail" ]; then
                printf 'qa-deploy-runner: teardown %s failed (on_failure=fail); downgrading PASS->PARTIAL\n' \
                    "$td_name" >&2
                record_teardown_failure "$td_name" "1"
            else
                printf 'qa-deploy-runner: teardown %s failed (on_failure=warn); logging only\n' \
                    "$td_name" >&2
                record_teardown_failure "$td_name" "0"
            fi
        fi

        t=$((t + 1))
    done
fi

exit 0
