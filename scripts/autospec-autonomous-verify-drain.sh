#!/usr/bin/env bash
# autospec-autonomous-verify-drain.sh — adversarial-skeptic verify bridge for
# explore's fail-closed verify stage (the AUTOSPEC_EXPLORE_VERIFY_CMD seam),
# bridged through the LLM harness (omx).
#
# explore (scripts/autospec-explore.sh) runs a fail-closed adversarial verify:
# an autonomous `--once` run with NO skeptic verdicts files ZERO proposals
# (reason:verify-unavailable-failclosed). In the detached conductor no verifier
# is wired, so explore generates proposals every idle cycle but files nothing.
# This wrapper IS that verifier: it reads the deduped proposals from
# $AUTOSPEC_EXPLORE_DEDUPED_IN, runs an omx-backed adversarial skeptic
# (default-to-REFUTED), and writes the {norm_title -> {verdict, reason}} map to
# $AUTOSPEC_EXPLORE_VERDICTS_OUT — the exact seam contract explore consumes
# (see tests/explore/test_explore_orchestrator_verify.bats write_skeptic).
#
# Consumer success gate: exit 0 AND a non-empty verdicts file. This wrapper
# INVERTS the sibling drains' fail-safe-to-dry: all failures (omx absent/
# non-zero, unparseable output, stall/timeout) is FAIL-CLOSED — exit non-zero
# and write no all-survived map. explore then refutes-by-default and files
# nothing. NEVER emit all-"survived" on error: that would push unverified junk
# into the admin-auto-merge loop.
#
# Empty deduped -> write {} and exit 0 (nothing to verify).
#
# All harness chatter goes to stdout/stderr (explore captures it into the
# iteration research.log); the verdict map goes only to the file path in
# $AUTOSPEC_EXPLORE_VERDICTS_OUT.
set -eu

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [ -f "$SCRIPT_DIR/autospec-runtime-config.sh" ]; then
    # shellcheck source=/dev/null
    . "$SCRIPT_DIR/autospec-runtime-config.sh"
elif [ -f "$HOME/.autospec/scripts/autospec-runtime-config.sh" ]; then
    # shellcheck source=/dev/null
    . "$HOME/.autospec/scripts/autospec-runtime-config.sh"
fi

DEFAULT_REPO_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
if command -v autospec_runtime_config_path >/dev/null 2>&1; then
    REPO_DIR="$(autospec_runtime_config_path autonomous.repo_dir AUTOSPEC_REPO_DIR "$DEFAULT_REPO_DIR")"
else
    REPO_DIR="${AUTOSPEC_REPO_DIR:-$DEFAULT_REPO_DIR}"
fi
if command -v autospec_runtime_config_int >/dev/null 2>&1; then
    VERIFY_STALL_SECS="$(autospec_runtime_config_int autonomous.verify.stall_secs AUTOSPEC_AUTONOMOUS_VERIFY_STALL_SECS 120)"
    VERIFY_POLL_SECS="$(autospec_runtime_config_int autonomous.verify.poll_secs AUTOSPEC_AUTONOMOUS_VERIFY_POLL_SECS 15)"
    VERIFY_MAX_SECS="$(autospec_runtime_config_int autonomous.verify.max_secs AUTOSPEC_AUTONOMOUS_VERIFY_MAX_SECS 300)"
else
    VERIFY_STALL_SECS="${AUTOSPEC_AUTONOMOUS_VERIFY_STALL_SECS:-120}"
    VERIFY_POLL_SECS="${AUTOSPEC_AUTONOMOUS_VERIFY_POLL_SECS:-15}"
    VERIFY_MAX_SECS="${AUTOSPEC_AUTONOMOUS_VERIFY_MAX_SECS:-300}"
fi

DEDUPED_IN="${AUTOSPEC_EXPLORE_DEDUPED_IN:-}"
VERDICTS_OUT="${AUTOSPEC_EXPLORE_VERDICTS_OUT:-}"

fail_closed() {
    # $1 = reason. Fail-closed: non-zero exit, no all-survived map written.
    printf 'autospec-autonomous-verify-drain: fail-closed: %s\n' "${1:-verify-error}" >&2
    exit 1
}

PROCESS_TREE_HELPER="$SCRIPT_DIR/lib/autospec-process-tree.sh"
if [ ! -f "$PROCESS_TREE_HELPER" ]; then
    fail_closed "process-tree helper missing: $PROCESS_TREE_HELPER"
fi
# shellcheck source=/dev/null
. "$PROCESS_TREE_HELPER"

deterministic_fallback() {
    # Preserve progress when the optional skeptic harness is unavailable, but
    # only allow candidates whose evidence names a real repository path and
    # line. Everything else remains refuted by default.
    [ "${AUTOSPEC_AUTONOMOUS_DETERMINISTIC_VERIFY:-1}" = "1" ] || return 1
    python3 - "$DEDUPED_IN" "$VERDICTS_OUT" <<'PY'
import json, os, re, sys
dedup_path, out_path = sys.argv[1:]
dd = json.load(open(dedup_path))
out = {}
for item in dd.get("deduped", []) or []:
    key = str(item.get("norm_title", ""))
    evidence = str(item.get("evidence", ""))
    match = re.search(r"([A-Za-z0-9_.-]+(?:/[A-Za-z0-9_.-]+)+|[A-Za-z0-9_.-]+\.(?:md|sh|rs|py|js|ts)):(\d+)", evidence)
    if not key or not match:
        continue
    path = match.group(1)
    if not os.path.isfile(path):
        continue
    out[key] = {"verdict": "survived", "reason": "deterministic repository evidence confirmed the referenced path and line"}
if not out:
    raise SystemExit(1)
json.dump(out, open(out_path, "w"))
PY
}

[ -n "$DEDUPED_IN" ] || fail_closed "AUTOSPEC_EXPLORE_DEDUPED_IN unset"
[ -f "$DEDUPED_IN" ] || fail_closed "deduped input not found: $DEDUPED_IN"
[ -n "$VERDICTS_OUT" ] || fail_closed "AUTOSPEC_EXPLORE_VERDICTS_OUT unset"
command -v python3 >/dev/null 2>&1 || fail_closed "python3 not found on PATH"

# ── Count deduped proposals; empty -> {} + exit 0 (nothing to verify). ────────
DEDUP_COUNT="$(python3 - "$DEDUPED_IN" <<'PY' 2>/dev/null || printf 'ERR'
import json, sys
try:
    dd = json.load(open(sys.argv[1]))
    items = [p for p in (dd.get("deduped") or []) if p.get("norm_title")]
    print(len(items))
except Exception:
    print("ERR")
PY
)"
case "$DEDUP_COUNT" in
    ''|*[!0-9]*) fail_closed "deduped input did not parse: $DEDUPED_IN" ;;
esac
if [ "$DEDUP_COUNT" -eq 0 ]; then
    printf '{}' > "$VERDICTS_OUT"
    printf 'autospec-autonomous-verify-drain: no deduped proposals to verify; wrote {}\n' >&2
    exit 0
fi

# ── Harness absence is FAIL-CLOSED here (unlike the run/explore drains). ──────
command -v omx >/dev/null 2>&1 || fail_closed "omx not found on PATH"

# ── Build the adversarial-skeptic prompt (embeds the exact norm_title keys). ──
PROMPT_FILE="$(mktemp "${TMPDIR:-/tmp}/autospec-verify-prompt.XXXXXX" 2>/dev/null || printf '/tmp/autospec-verify-prompt.%s' "$$")"
if ! python3 - "$DEDUPED_IN" > "$PROMPT_FILE" <<'PY'; then
import json, sys
dd = json.load(open(sys.argv[1]))
items = [p for p in (dd.get("deduped") or []) if p.get("norm_title")]
lines = []
lines.append("You are an ADVERSARIAL SKEPTIC verifying candidate improvements for a")
lines.append("software repository before they are filed as work. Your bias is to REFUTE.")
lines.append("")
lines.append("DEFAULT TO REFUTED. Only return \"survived\" for a proposal when its evidence")
lines.append("clearly justifies a real, durable, non-duplicate improvement. If the evidence")
lines.append("is thin, speculative, a restatement of the title, or a likely duplicate of")
lines.append("existing behavior, refute it.")
lines.append("")
lines.append("Proposals to judge:")
lines.append("")
for i, p in enumerate(items, 1):
    lines.append("Proposal %d:" % i)
    lines.append("  norm_title (use this EXACT string as the JSON key): %s" % json.dumps(p.get("norm_title", "")))
    lines.append("  title: %s" % json.dumps(p.get("title", "")))
    lines.append("  evidence: %s" % json.dumps(p.get("evidence", "")))
    lines.append("  estimated_complexity: %s" % json.dumps(p.get("estimated_complexity", "")))
    lines.append("  confidence: %s" % json.dumps(p.get("confidence", "")))
    lines.append("")
lines.append("Output a single JSON object keyed by the EXACT norm_title strings above")
lines.append("(verbatim — do not rephrase or normalize them). Each value is")
lines.append("{\"verdict\":\"survived\"|\"refuted\",\"reason\":\"<one short sentence>\"}.")
lines.append("Every proposal must appear exactly once.")
lines.append("")
lines.append("Wrap the JSON object between these two sentinel lines, each ALONE on its")
lines.append("own line, with NOTHING else on those lines, so it can be recovered from")
lines.append("surrounding harness output:")
lines.append("===AUTOSPEC_VERDICTS_BEGIN===")
lines.append('{"<norm_title>":{"verdict":"refuted","reason":"..."}}')
lines.append("===AUTOSPEC_VERDICTS_END===")
sys.stdout.write("\n".join(lines))
PY
    rm -f "$PROMPT_FILE"
    fail_closed "failed to build skeptic prompt"
fi
PROMPT="$(cat "$PROMPT_FILE")"
rm -f "$PROMPT_FILE"

# ── Run the skeptic through omx, with a stall watchdog. ───────────────────────
HARNESS_LOG="$(mktemp "${TMPDIR:-/tmp}/autospec-verify-drain.XXXXXX" 2>/dev/null || printf '/tmp/autospec-verify-drain.%s' "$$")"

# The background job must be the new-session process itself: backgrounding a
# function wraps it in a subshell that stays in our own process group, which
# the shared helper refuses to signal and which would leak the harness tree.
NEW_SESSION_CMD=(python3 -c 'import os, sys; os.setsid(); os.execvp(sys.argv[1], sys.argv[1:])')
if command -v setsid >/dev/null 2>&1; then
    NEW_SESSION_CMD=(setsid)
fi
"${NEW_SESSION_CMD[@]}" omx exec \
    --cd "$REPO_DIR" \
    --dangerously-bypass-approvals-and-sandbox \
    "$PROMPT" > "$HARNESS_LOG" 2>&1 &
child_pid="$!"

omx_rc=0
verify_started_epoch="$(date +%s)"
if [ "${VERIFY_STALL_SECS:-0}" -le 0 ] 2>/dev/null; then
    set +e
    wait "$child_pid"
    omx_rc="$?"
    set -e
else
    last_size=0
    last_progress_epoch="$(date +%s)"
    while kill -0 "$child_pid" 2>/dev/null; do
        sleep "$VERIFY_POLL_SECS"
        now_epoch="$(date +%s)"
        elapsed_secs=$((now_epoch - verify_started_epoch))
        if [ "${VERIFY_MAX_SECS:-0}" -gt 0 ] 2>/dev/null && [ "$elapsed_secs" -ge "$VERIFY_MAX_SECS" ]; then
            printf 'autospec-autonomous-verify-drain: absolute timeout after %ss; terminating skeptic child pid %s\n' \
                "$VERIFY_MAX_SECS" "$child_pid" >&2
            autospec_kill_tree "$child_pid" separate-recursive
            wait "$child_pid" 2>/dev/null || true
            omx_rc=124
            break
        fi
        current_size="$(stat -c '%s' "$HARNESS_LOG" 2>/dev/null || stat -f '%z' "$HARNESS_LOG" 2>/dev/null || printf '0')"
        if [ "$current_size" != "$last_size" ]; then
            last_size="$current_size"
            last_progress_epoch="$(date +%s)"
            continue
        fi
        idle_secs=$((now_epoch - last_progress_epoch))
        if [ "$idle_secs" -ge "$VERIFY_STALL_SECS" ]; then
            printf 'autospec-autonomous-verify-drain: stalled after %ss with no output; terminating skeptic child pid %s\n' \
                "$VERIFY_STALL_SECS" "$child_pid" >&2
            autospec_kill_tree "$child_pid" separate-recursive
            wait "$child_pid" 2>/dev/null || true
            omx_rc=124
            break
        fi
    done
    if [ "$omx_rc" -eq 0 ]; then
        set +e
        wait "$child_pid"
        omx_rc="$?"
        set -e
    fi
fi

# Surface harness output for observability (goes to explore's research.log).
cat "$HARNESS_LOG" >&2 2>/dev/null || true

# A non-zero harness exit is FAIL-CLOSED — even if the log looks like a valid
# verdict object, an errored omx run is not trustworthy.
if [ "$omx_rc" -ne 0 ]; then
    if deterministic_fallback; then
        printf 'autospec-autonomous-verify-drain: skeptic unavailable (rc=%s); deterministic evidence fallback accepted %s candidate(s)\n' \
            "$omx_rc" "$(python3 -c 'import json,sys; print(len(json.load(open(sys.argv[1]))))' "$VERDICTS_OUT" 2>/dev/null || printf 0)" >&2
        rm -f "$HARNESS_LOG" 2>/dev/null || true
        exit 0
    fi
    rm -f "$HARNESS_LOG" 2>/dev/null || true
    fail_closed "omx skeptic exited $omx_rc"
fi

# ── Extract + normalize the verdict map from the (chatty) omx stdout. ─────────
# Real codex/omx output is NOT a clean fenced block — the verdict arrives as raw
# JSON buried in brace-heavy chatter (shell-var braces, hook telemetry, the final
# message printed twice, a `tokens used` block, JSON-ish footers). Recovery is:
#   1. PRIMARY — the JSON between the LAST ===AUTOSPEC_VERDICTS_BEGIN/END=== pair.
#   2. FALLBACK — among ALL candidate dicts (fenced + balanced-brace spans), pick
#      the one with the GREATEST key-overlap with the known norm_titles (NOT just
#      the first parseable dict, which is often a telemetry object → empty map).
# If proposals existed but ZERO verdicts overlapping `known` are recovered, that
# is an extraction FAILURE (not "all refuted"): exit non-zero WITHOUT writing a
# map, so the consumer's `exit 0 && [ -s ... ]` gate fails closed OBSERVABLY.
if python3 - "$DEDUPED_IN" "$HARNESS_LOG" "$VERDICTS_OUT" <<'PY'; then
import json, re, sys

dedup_path, log_path, out_path = sys.argv[1], sys.argv[2], sys.argv[3]

dd = json.load(open(dedup_path))
known = {p["norm_title"] for p in (dd.get("deduped") or []) if p.get("norm_title")}

text = open(log_path, "r", errors="replace").read()

BEGIN = "===AUTOSPEC_VERDICTS_BEGIN==="
END = "===AUTOSPEC_VERDICTS_END==="


def candidates(s):
    # 1. Fenced ```json ... ``` (or bare ```) blocks, first to last.
    for m in re.finditer(r"```(?:json)?\s*(.*?)```", s, re.DOTALL | re.IGNORECASE):
        yield m.group(1)
    # 2. Balanced brace scan, last to first (LLMs put the answer last).
    spans = []
    depth = 0
    start = None
    for i, ch in enumerate(s):
        if ch == "{":
            if depth == 0:
                start = i
            depth += 1
        elif ch == "}":
            if depth > 0:
                depth -= 1
                if depth == 0 and start is not None:
                    spans.append(s[start:i + 1])
    for span in reversed(spans):
        yield span


parsed = None

# 1. PRIMARY: text between the LAST BEGIN marker and the first END after it.
begins = [m.end() for m in re.finditer(re.escape(BEGIN), text)]
ends = [m.start() for m in re.finditer(re.escape(END), text)]
if begins and ends:
    b = begins[-1]
    after = [e for e in ends if e > b]
    if after:
        try:
            obj = json.loads(text[b:after[0]])
            if isinstance(obj, dict):
                parsed = obj
        except Exception:
            parsed = None

# 2. FALLBACK: best key-overlap with `known` across all candidate dicts.
if parsed is None:
    best = None
    best_overlap = -1
    for cand in candidates(text):
        try:
            obj = json.loads(cand)
        except Exception:
            continue
        if not isinstance(obj, dict):
            continue
        overlap = len(set(obj.keys()) & known)
        if overlap > best_overlap:
            best_overlap = overlap
            best = obj
    parsed = best

out = {}
if isinstance(parsed, dict):
    for k, v in parsed.items():
        if k not in known or not isinstance(v, dict):
            continue
        verdict = "survived" if v.get("verdict") == "survived" else "refuted"
        reason = str(v.get("reason") or "")
        out[k] = {"verdict": verdict, "reason": reason}

# Fail LOUD on extraction failure: proposals existed (known is non-empty here —
# the N==0 case already returned {}+exit0 in bash) but nothing was recovered.
# Do NOT write a map; exit non-zero so explore records the code_health warning
# rather than silently filing nothing while looking successful.
if not out:
    sys.stderr.write(
        "verify-drain: recovered zero verdicts overlapping known proposals; "
        "extraction failure, failing closed\n"
    )
    sys.exit(3)

json.dump(out, open(out_path, "w"))
PY
    parse_rc=0
else
    parse_rc=$?
fi
rm -f "$HARNESS_LOG" 2>/dev/null || true

if [ "$parse_rc" -ne 0 ]; then
    if deterministic_fallback; then
        printf 'autospec-autonomous-verify-drain: verdict extraction failed; deterministic evidence fallback accepted %s candidate(s)\n' \
            "$(python3 -c 'import json,sys; print(len(json.load(open(sys.argv[1]))))' "$VERDICTS_OUT" 2>/dev/null || printf 0)" >&2
        exit 0
    fi
    fail_closed "could not extract a verdict map from skeptic output"
fi

exit 0
