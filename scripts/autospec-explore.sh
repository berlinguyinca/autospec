#!/usr/bin/env bash
# scripts/autospec-explore.sh — top-level /autospec-explore orchestrator (issue #721).
#
# Wires:
#   - scripts/explore-sandbox.sh         (sandbox branch + .autospec/explore-mode.json)
#   - /autospec-refine                   (initial prompt refinement, harness-aware)
#   - /autospec-define                   (initial issue decomposition, harness-aware)
#   - scripts/explore-research-cycle.sh  (per-iteration researcher cycle)
#   - /autospec-run                      (drain callback, harness-aware)
#   - scripts/lib/autospec-loop.sh       (shared loop driver, PR #712)
#   - scripts/autospec-usage-limit.sh    (supervisor arming on quota pause)
#
# Outputs:
#   .autospec/explore-summary.md   — human-readable per-round table
#   .autospec/explore-loop.json    — machine-readable per-iteration log
#
# Termination conditions (inherited from shared driver + explore additions):
#   1. evidence_based_stop      — STOP marker in iteration report
#   2. oscillation_detected     — harvested prompt hash unchanged
#   3. operator_stop            — ~/.autospec/explore-stop.flag or stop.flag
#   4. budget_cap_reached       — AUTOSPEC_LOOP_TOKEN_CAP / TIME_CAP exceeded
#   5. round_cap_reached        — --max-iterations cap hit
#   6. all_researchers_failed   — emits code_health and exits cleanly
#
# Test hooks (used by tests/explore/test_explore_e2e.bats):
#   AUTOSPEC_EXPLORE_REFINE_CMD  — override /autospec-refine handoff
#   AUTOSPEC_EXPLORE_DEFINE_CMD  — override /autospec-define handoff
#   AUTOSPEC_EXPLORE_DRAIN_CMD   — override /autospec-run drain handoff
#   AUTOSPEC_REPO_ROOT           — override git toplevel
#   AUTOSPEC_RESEARCH_DIR        — researcher script dir
#   AUTOSPEC_LOOP_TIME_CAP       — budget time cap (seconds)
#   AUTOSPEC_LOOP_TOKEN_CAP      — budget token cap

set -u

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="${AUTOSPEC_REPO_ROOT:-$(git rev-parse --show-toplevel 2>/dev/null || pwd)}"

project_sync_issue() {
    local helper="${AUTOSPEC_SCRIPTS_DIR:-$SCRIPT_DIR/../skills/autospec-shared/scripts}/project-sync-issue.sh"
    bash "$helper" "$1" "$REPO_ROOT"
}

# Defaults.
MAX_ITERATIONS=3
MAX_ISSUES_PER_ROUND=5
BUDGET_TOKENS=""
BUDGET_HOURS=""
SANDBOX_SLUG=""
RESEARCH_SOURCES="spec-vs-code,prior-reports,codebase-signals,open-issues,source-analysis,dependency-health,internet,quality-resilience,dogfooding,self-leverage,style-normalization"
NO_INTERNET=0
INTERNET_ALLOWLIST=""
SPECIALISTS_MODE="discover"
NUM_SPECIALISTS=3
SPECIALISTS_ARG=""
AUTONOMOUS=0
QA_GATE=0
QA_GATE_PASS_ON_PARTIAL=0
ONCE=0
PREVIEW=0
SKIP_INITIAL_HANDOFF="${AUTOSPEC_EXPLORE_SKIP_INITIAL_HANDOFF:-0}"
HANDOFF_TIMEOUT_SEC="${AUTOSPEC_EXPLORE_HANDOFF_TIMEOUT_SEC:-900}"
PROMPT=""
EXPLORE_CHILD_PIDS=""

# A harness may detach this script into a new session. Keep a tiny owner
# watcher so force-restarting the autonomous drain cannot orphan research work.
EXPLORE_PARENT_WATCHDOG=""
if [ -n "${AUTOSPEC_EXPLORE_PARENT_PID:-}" ] && [ "${AUTOSPEC_EXPLORE_PARENT_PID}" != "$$" ]; then
    (
        while kill -0 "$AUTOSPEC_EXPLORE_PARENT_PID" 2>/dev/null; do
            sleep 15
        done
        kill -TERM "$$" 2>/dev/null || true
    ) &
    EXPLORE_PARENT_WATCHDOG="$!"
    trap 'if [ -n "${EXPLORE_PARENT_WATCHDOG:-}" ]; then kill "$EXPLORE_PARENT_WATCHDOG" 2>/dev/null || true; fi' EXIT
fi

usage() {
    cat <<'EOF'
Usage: scripts/autospec-explore.sh "<initial prompt>" [options]

Options:
  --max-iterations N          Cap research rounds (default 3).
  --max-issues-per-round N    Cap proposals filed as issues per round (default 5).
  --budget-tokens N           Token budget cap (passed through to loop driver).
  --budget-hours H            Wall-clock budget cap in hours.
  --sandbox-slug SLUG         Override sandbox slug (default auto).
  --research-sources LIST     Comma-separated researcher names.
  --no-internet               Disable the internet researcher.
  --internet-allowlist LIST   Pass-through to explore-research/internet.sh.
  --specialists-mode MODE     Domain-specialist roster mode (Issue E2):
                                discover (default) | ask | explicit | off.
  --num-specialists N         Roster size for discover/ask (default 3, cap 6).
  --specialists LIST          Explicit roster slug:persona,... (explicit mode).
  --autonomous                Non-interactive run: discover mode auto-selects the
                              top-N specialists and never blocks on confirmation.
  --qa-gate                   Run scripts/explore-qa-gate.sh ONCE at loop
                              termination and gate the promotion-readiness
                              output by its verdict (default OFF — promotion
                              output is byte-unchanged without this flag).
  --qa-gate-pass-on-partial   Treat a PARTIAL gate verdict as PASS (default
                              PARTIAL blocks, matching QA's PARTIAL!=PASS rule).
  --once                      Run exactly ONE research pass over the resolved
                              --research-sources, emit a yield JSON
                              {tier,proposals_seen,new_candidates,filed,dry,reason,candidates}
                              to stdout, then return. Never enters the perpetual
                              loop; never calls the drain command. dry=true when
                              new_candidates==0 after dedup. Test hook:
                              AUTOSPEC_EXPLORE_ONCE_CYCLE_CMD (overrides the
                              explore-research-cycle.sh call; must write JSON
                              to \$AUTOSPEC_EXPLORE_ONCE_OUT).
  --no-initial-handoff        Skip startup /autospec-refine and /autospec-define
                              handoffs; run only the explore research loop.
  --handoff-timeout-sec N     Timeout for startup handoffs (default 900; env:
                              AUTOSPEC_EXPLORE_HANDOFF_TIMEOUT_SEC).
EOF
}

# Parse args. First non-flag = prompt.
while [ "$#" -gt 0 ]; do
    case "$1" in
        --max-iterations)        shift; MAX_ITERATIONS="$1" ;;
        --max-issues-per-round)  shift; MAX_ISSUES_PER_ROUND="$1" ;;
        --budget-tokens)         shift; BUDGET_TOKENS="$1" ;;
        --budget-hours)          shift; BUDGET_HOURS="$1" ;;
        --sandbox-slug)          shift; SANDBOX_SLUG="$1" ;;
        --research-sources)      shift; RESEARCH_SOURCES="$1" ;;
        --no-internet)           NO_INTERNET=1 ;;
        --internet-allowlist)    shift; INTERNET_ALLOWLIST="$1" ;;
        --specialists-mode)      shift; SPECIALISTS_MODE="$1" ;;
        --num-specialists)       shift; NUM_SPECIALISTS="$1" ;;
        --specialists)           shift; SPECIALISTS_ARG="$1" ;;
        --autonomous)            AUTONOMOUS=1 ;;
        --qa-gate)               QA_GATE=1 ;;
        --qa-gate-pass-on-partial) QA_GATE_PASS_ON_PARTIAL=1 ;;
        --once)                  ONCE=1 ;;
        --preview)               PREVIEW=1 ;;
        --no-initial-handoff)    SKIP_INITIAL_HANDOFF=1 ;;
        --handoff-timeout-sec)   shift; HANDOFF_TIMEOUT_SEC="$1" ;;
        -h|--help)               usage; exit 0 ;;
        --) shift; PROMPT="${PROMPT:-$*}"; break ;;
        -*) echo "autospec-explore: unknown flag: $1" >&2; usage; exit 2 ;;
        *)  if [ -z "$PROMPT" ]; then PROMPT="$1"; else PROMPT="$PROMPT $1"; fi ;;
    esac
    shift
done

if [ "$PREVIEW" -eq 1 ]; then
    export AUTOSPEC_EXPLORE_PREVIEW=1
fi

if [ "$ONCE" -eq 1 ] && [ -n "${AUTOSPEC_EXPLORE_VERIFY_CMD:-}" ]; then
    _once_dir="${_once_dir:-.autospec/explore-once-$$}"
    mkdir -p "$_once_dir"
    _once_out="${_once_out:-$_once_dir/research.json}"
fi

if [ -z "$PROMPT" ]; then
    if [ "$ONCE" -eq 1 ] && [ -z "${AUTOSPEC_EXPLORE_VERIFY_CMD:-}" ]; then
        # --once discovery sweeps are prompt-less by design (conductor Tier
        # 2/4 invocations never supply an initial prompt). Default to a
        # generic repo-discovery seed instead of hard-failing (issue #1625).
        PROMPT="Discover the highest-value defects and improvements in this repository."
    elif [ -n "${AUTOSPEC_EXPLORE_VERIFY_CMD:-}" ]; then
        _once_preflight_rc=0
        _once_dedup="$_once_dir/dedup.json"
        _once_verdicts="$_once_dir/verdicts.json"
        AUTOSPEC_HARNESS_DISPATCHER=1 bash "$SCRIPT_DIR/explore-research-cycle.sh" --max-issues-per-round "$MAX_ISSUES_PER_ROUND" \
            --research-sources "$RESEARCH_SOURCES" --stage dedup --out "$_once_dedup" \
            > "$_once_dir/research.log" 2>&1 || _once_preflight_rc=$?
        if [ "$_once_preflight_rc" -eq 0 ] && [ -s "$_once_dedup" ]; then
            if AUTOSPEC_EXPLORE_DEDUPED_IN="$_once_dedup" \
               AUTOSPEC_EXPLORE_VERDICTS_OUT="$_once_verdicts" bash -c "$AUTOSPEC_EXPLORE_VERIFY_CMD" \
               >> "$_once_dir/research.log" 2>&1; then
                AUTOSPEC_EXPLORE_VERIFY_VERDICTS="$_once_verdicts" bash "$SCRIPT_DIR/explore-research-cycle.sh" \
                    --max-issues-per-round "$MAX_ISSUES_PER_ROUND" --stage finalize \
                    --deduped-in "$_once_dedup" --out "$_once_out" >> "$_once_dir/research.log" 2>&1 \
                    || _once_preflight_rc=$?
            else
                _once_preflight_rc=$?
            fi
        fi
    else
        echo "autospec-explore: missing initial prompt" >&2
        usage
        exit 2
    fi
fi

# Budget hours → seconds for the shared loop driver env contract.
if [ -n "$BUDGET_HOURS" ]; then
    export AUTOSPEC_LOOP_TIME_CAP="$(python3 -c "print(int(float('$BUDGET_HOURS')*3600))")"
fi
if [ -n "$BUDGET_TOKENS" ]; then
    export AUTOSPEC_LOOP_TOKEN_CAP="$BUDGET_TOKENS"
fi

# Filter sources if --no-internet.
if [ "$NO_INTERNET" -eq 1 ]; then
    RESEARCH_SOURCES="$(printf '%s' "$RESEARCH_SOURCES" | tr ',' '\n' | grep -v '^internet$' | paste -sd, -)"
fi
if [ -n "$INTERNET_ALLOWLIST" ]; then
    export AUTOSPEC_INTERNET_ALLOWLIST="$INTERNET_ALLOWLIST"
fi

cd "$REPO_ROOT" || { echo "autospec-explore: repo root unavailable: $REPO_ROOT" >&2; exit 2; }
mkdir -p .autospec

case "$SKIP_INITIAL_HANDOFF" in
    1|true|TRUE|yes|YES) SKIP_INITIAL_HANDOFF=1 ;;
    *) SKIP_INITIAL_HANDOFF=0 ;;
esac

case "$HANDOFF_TIMEOUT_SEC" in
    ''|*[!0-9]*) HANDOFF_TIMEOUT_SEC=900 ;;
esac

_explore_remove_child_pid() {
    local target="$1" out="" p
    for p in $EXPLORE_CHILD_PIDS; do
        [ "$p" = "$target" ] && continue
        out="${out:+$out }$p"
    done
    EXPLORE_CHILD_PIDS="$out"
}

_explore_kill_tree() {
    local pid="$1" child
    local pgid
    pgid="$(ps -o pgid= -p "$pid" 2>/dev/null | tr -d ' ' || true)"
    # Group-kill ONLY when this pid is its own process-group leader (pgid == pid),
    # i.e. setsid gave the handoff a dedicated group we own. When setsid is absent
    # (e.g. macOS) the handoff shares the CALLER's group; a `kill -TERM -$pgid`
    # there would take down autospec-explore itself, the test runner, or the
    # operator's shell. In that case fall back to killing the pid + its
    # descendants individually (pgrep -P recursion below).
    if [ -n "$pgid" ] && [ "$pgid" = "$pid" ]; then
        kill -TERM "-$pgid" 2>/dev/null || true
    fi
    for child in $(pgrep -P "$pid" 2>/dev/null || true); do
        _explore_kill_tree "$child"
    done
    kill -TERM "$pid" 2>/dev/null || true
    sleep 1
    if [ -n "$pgid" ] && [ "$pgid" = "$pid" ]; then
        kill -KILL "-$pgid" 2>/dev/null || true
    fi
    kill -KILL "$pid" 2>/dev/null || true
}

_explore_cleanup_children() {
    local pid
    for pid in $EXPLORE_CHILD_PIDS; do
        if kill -0 "$pid" 2>/dev/null; then
            _explore_kill_tree "$pid"
        fi
    done
}

trap _explore_cleanup_children INT TERM EXIT

_explore_run_handoff() {
    local step="$1"; shift
    local log_dir=".autospec/explore-handoff"
    local log_file="$log_dir/$step.log"
    local timeout_file="$log_dir/$step.timeout"
    local pid watchdog rc

    mkdir -p "$log_dir"
    : > "$log_file"
    rm -f "$timeout_file"

    if command -v timeout >/dev/null 2>&1; then
        timeout -k 2s "${HANDOFF_TIMEOUT_SEC}s" "$@" > "$log_file" 2>&1
        rc=$?
        if [ "$rc" -eq 124 ] || [ "$rc" -eq 137 ]; then
            echo "code_health:explore_handoff_timeout step=$step timeout_sec=$HANDOFF_TIMEOUT_SEC log=$log_file" >&2
            return 124
        fi
        if [ "$rc" -ne 0 ]; then
            echo "autospec-explore: WARN initial handoff step=$step failed rc=$rc log=$log_file (continuing)" >&2
        fi
        return "$rc"
    fi

    if command -v setsid >/dev/null 2>&1; then
        setsid "$@" > "$log_file" 2>&1 &
    else
        "$@" > "$log_file" 2>&1 &
    fi
    pid=$!
    EXPLORE_CHILD_PIDS="${EXPLORE_CHILD_PIDS:+$EXPLORE_CHILD_PIDS }$pid"

    # Redirect the watchdog's own descriptors away from the caller's stdout/stderr:
    # otherwise its `sleep $HANDOFF_TIMEOUT_SEC` inherits and holds them open, and a
    # caller that reads our output to EOF (e.g. bats `run`) blocks until the sleep
    # finally exits — up to the full timeout.
    (
        sleep "$HANDOFF_TIMEOUT_SEC"
        if kill -0 "$pid" 2>/dev/null; then
            printf 'timeout after %ss\n' "$HANDOFF_TIMEOUT_SEC" > "$timeout_file"
            _explore_kill_tree "$pid"
        fi
    ) >/dev/null 2>&1 &
    watchdog=$!

    wait "$pid"
    rc=$?
    # Tear down the watchdog AND its sleep child. Killing only the subshell
    # ($watchdog) orphans the `sleep $HANDOFF_TIMEOUT_SEC`, which then lingers for
    # the full timeout instead of being reaped promptly.
    pkill -P "$watchdog" 2>/dev/null || true
    kill "$watchdog" 2>/dev/null || true
    wait "$watchdog" 2>/dev/null || true
    _explore_remove_child_pid "$pid"

    if [ -f "$timeout_file" ]; then
        echo "code_health:explore_handoff_timeout step=$step timeout_sec=$HANDOFF_TIMEOUT_SEC log=$log_file" >&2
        return 124
    fi
    if [ "$rc" -ne 0 ]; then
        echo "autospec-explore: WARN initial handoff step=$step failed rc=$rc log=$log_file (continuing)" >&2
    fi
    return "$rc"
}

# ── --once: single-cycle no-loop mode (F1). ────────────────────────────────────
# Runs exactly ONE research pass, emits a yield JSON with candidate issue details, and returns.
# Never enters the perpetual loop; never calls invoke_drain; never creates a
# sandbox branch. The conductor calls this per cycle and counts consecutive
# dry=true results for tier escalation (F2).
#
# Output JSON keys:
#   tier            "competitor" when sources include "internet", else "local"
#   proposals_seen  proposals_total from the research cycle (pre-dedup)
#   new_candidates  proposals surviving dedup + recent-title filter (post-dedup)
#   filed           issues actually created via gh issue create
#   dry             true when new_candidates==0 after dedup
#   reason          human-readable summary string
#   candidates      machine-readable issue objects with title, body, labels,
#                   severity, ROI score, evidence, source, and confidence
#
# Test hook: AUTOSPEC_EXPLORE_ONCE_CYCLE_CMD — when set, runs instead of the
# real explore-research-cycle.sh call. The mock receives AUTOSPEC_EXPLORE_ONCE_OUT
# (the output JSON path) and AUTOSPEC_EXPLORE_ONCE_SOURCES (the resolved sources)
# as env vars and must write valid research-cycle JSON to that path.
if [ "$ONCE" -eq 1 ]; then
    # --once is a non-interactive autonomous atomic (the conductor's per-cycle
    # discovery unit). Force the autonomous flag so the cycle's fail-closed verify
    # reliably applies here — a --once pass that cannot verify must file ZERO
    # rather than auto-ship unverified proposals, regardless of inherited env.
    export AUTOSPEC_EXPLORE_AUTONOMOUS=1

    # Determine tier from the resolved source set.
    _once_tier="local"
    if printf '%s\n' "$RESEARCH_SOURCES" | tr ',' '\n' | grep -qx 'internet'; then
        _once_tier="competitor"
    fi

    _once_dir=".autospec/explore-once-$$"
    mkdir -p "$_once_dir"
    _once_out="$_once_dir/research.json"
    _once_research_rc="${_once_preflight_rc:-0}"

    # Run the single research cycle pass (full stage: dedup + verify + rank).
    if [ -s "$_once_out" ] && [ -n "${AUTOSPEC_EXPLORE_VERIFY_CMD:-}" ]; then
        :
    elif [ -n "${AUTOSPEC_EXPLORE_ONCE_CYCLE_CMD:-}" ]; then
        AUTOSPEC_EXPLORE_ONCE_OUT="$_once_out" \
        AUTOSPEC_EXPLORE_ONCE_SOURCES="$RESEARCH_SOURCES" \
            bash -c "$AUTOSPEC_EXPLORE_ONCE_CYCLE_CMD" \
            > "$_once_dir/research.log" 2>&1 || _once_research_rc=$?
    else
        bash "$SCRIPT_DIR/explore-research-cycle.sh" \
            --max-issues-per-round "$MAX_ISSUES_PER_ROUND" \
            --research-sources "$RESEARCH_SOURCES" \
            --out "$_once_out" \
            > "$_once_dir/research.log" 2>&1 || _once_research_rc=$?
    fi

    if [ "$_once_research_rc" -ne 0 ]; then
        cat "$_once_dir/research.log" >&2 2>/dev/null || true
        printf '{"tier":"%s","proposals_seen":0,"new_candidates":0,"filed":0,"dry":false,"reason":"research-incomplete","candidates":[]}\n' \
            "$_once_tier"
        exit "$_once_research_rc"
    fi

    # The one-shot path has no iterative pass to host the verifier seam. When
    # autonomous mode supplied a verifier, run it here against the proposals
    # just produced and retain only explicit survivors.
    if [ -n "${AUTOSPEC_EXPLORE_VERIFY_CMD:-}" ] && [ -f "$_once_out" ]; then
        _once_dedup="$_once_dir/dedup.json"
        _once_verdicts="$_once_dir/verdicts.json"
        python3 - "$_once_out" "$_once_dedup" <<'PYV'
import json, re, sys
src, dst = sys.argv[1:]
data = json.load(open(src))
items = []
for p in data.get("proposals", []) or []:
    title = str(p.get("title", ""))
    norm = re.sub(r"[^a-z0-9 ]+", " ", title.lower())
    norm = re.sub(r"\s+", " ", norm).strip()[:120]
    if norm:
        q = dict(p)
        q["norm_title"] = norm
        items.append(q)
json.dump({"deduped": items}, open(dst, "w"))
PYV
        if AUTOSPEC_EXPLORE_DEDUPED_IN="$_once_dedup" \
           AUTOSPEC_EXPLORE_VERDICTS_OUT="$_once_verdicts" \
           bash -c "$AUTOSPEC_EXPLORE_VERIFY_CMD" >> "$_once_dir/research.log" 2>&1 \
           && [ -s "$_once_verdicts" ]; then
            python3 - "$_once_out" "$_once_verdicts" <<'PYV'
import json, sys
research, verdicts = sys.argv[1:]
data = json.load(open(research)); vm = json.load(open(verdicts))
survivors = {k for k, v in vm.items() if isinstance(v, dict) and v.get("verdict") == "survived"}
for p in data.get("proposals", []) or []:
    title = str(p.get("title", "")); key = " ".join("".join(c.lower() if c.isalnum() else " " for c in title).split())[:120]
    p["_verified_survivor"] = key in survivors
data["proposals"] = [p for p in data.get("proposals", []) or [] if p.get("_verified_survivor")]
data["verify_mode"] = "active"; data["failclosed"] = False
json.dump(data, open(research, "w"))
PYV
        fi
    fi

    # Extract the pre-dedup count and render the post-verify survivors into the
    # conductor-facing candidate issue contract. Keep this derivation in one
    # Python pass so body text, labels, evidence, and ROI score stay in sync with
    # the exact proposals that will be filed.
    _once_seen=0
    _once_candidates="$_once_dir/candidates.json"
    printf '%s\n' '[]' > "$_once_candidates"
    if [ -f "$_once_out" ]; then
        python3 - "$_once_out" "$_once_candidates" "$_once_tier" "$RESEARCH_SOURCES" <<'PY'
import json, sys

src_path, out_path, tier, sources = sys.argv[1:5]
try:
    data = json.load(open(src_path))
except Exception:
    data = {}

def ctx_label(complexity):
    return {"small": "ctx:32k", "medium": "ctx:64k", "large": "ctx:120k"}.get(
        str(complexity or "").lower(), "ctx:64k"
    )

def reasoning_label(severity, complexity):
    sev = str(severity or "feature").lower()
    comp = str(complexity or "medium").lower()
    if sev in ("silent-wrong", "correctness", "stability") or comp == "large":
        return "reasoning:deep"
    if sev == "nicety":
        return "reasoning:shallow"
    return "reasoning:medium"

def clean_text(value, default=""):
    value = default if value is None else value
    return str(value).replace("\r", "").strip()

candidates = []
for proposal in data.get("proposals", []) or []:
    title = clean_text(proposal.get("title"))
    if not title:
        continue
    evidence = clean_text(proposal.get("evidence"))
    severity = clean_text(proposal.get("severity"), "feature") or "feature"
    complexity = clean_text(proposal.get("estimated_complexity"), "medium").lower() or "medium"
    if complexity not in ("small", "medium", "large"):
        complexity = "medium"
    source = clean_text(proposal.get("source"), "unknown") or "unknown"
    named_consumer = clean_text(proposal.get("named_consumer"))
    try:
        confidence = max(0.0, min(1.0, float(proposal.get("confidence", 0.5))))
    except Exception:
        confidence = 0.5
    try:
        roi_score = float(proposal.get("score", confidence))
    except Exception:
        roi_score = confidence
    ctx = ctx_label(complexity)
    reasoning = reasoning_label(severity, complexity)
    labels = [
        "auto-implement",
        ctx,
        reasoning,
        "explore",
    ]
    body = "\n".join([
        "Auto-filed by /autospec-explore --once (single-cycle discovery).",
        "",
        "## Goal",
        f"Resolve the verified discovery candidate `{title}` using the evidence from `{source}`.",
        "",
        "## Discovery candidate",
        f"- Tier: {tier}",
        f"- Source: {source}",
        f"- Research sources: {sources}",
        f"- Severity: {severity}",
        f"- Estimated complexity: {complexity}",
        f"- Confidence: {confidence:.2f}",
        f"- ROI score: {roi_score:.4f}",
        f"- Named consumer: {named_consumer or 'n/a'}",
        "",
        "## Evidence",
        evidence or "n/a",
        "",
        "## Verification",
        "Adversarial verify: passed before filing by the explore research-cycle finalize gate.",
        "ROI gate: passed (candidate survived severity-first rank).",
        "",
        "<!-- autospec-classify:begin -->",
        "## Model fit",
        f"- {ctx}",
        f"- {reasoning}",
        "<!-- autospec-classify:end -->",
        "",
        "## Acceptance criteria",
        f"- [ ] The PR references `{title[:55]}` in its closeout artifacts.",
        "- [ ] The implementation cites `Adversarial verify` evidence before editing.",
        "- [ ] `autospec validate` passes after the change.",
        "",
        "### Primary smoke test (inner loop)",
        "```bash",
        "autospec validate",
        "```",
    ])
    candidates.append({
        "title": title,
        "body": body,
        "severity": severity,
        "labels": labels,
        "roi_score": round(roi_score, 4),
        "evidence": evidence,
        "source": source,
        "estimated_complexity": complexity,
        "confidence": round(confidence, 4),
        "named_consumer": named_consumer,
    })

with open(out_path, "w") as fh:
    json.dump(candidates, fh, separators=(",", ":"))
    fh.write("\n")
PY
        _once_seen="$(python3 -c "import json,sys; print(json.load(open(sys.argv[1])).get('proposals_total', 0))" "$_once_out" 2>/dev/null || echo 0)"
    fi
    _once_new="$(python3 -c "import json,sys; print(len(json.load(open(sys.argv[1]))))" "$_once_candidates" 2>/dev/null || echo 0)"
    if [ "$PREVIEW" -eq 1 ] && [ "$_once_new" -eq 0 ] && [ -x "$SCRIPT_DIR/autospec-preview-discover.sh" ]; then
        "$SCRIPT_DIR/autospec-preview-discover.sh" "$REPO_ROOT" \
            | python3 -c 'import json,sys; d=json.load(sys.stdin); out=[]; [out.append(dict(x, body="Preview-only repository signal; verify before implementation.", labels=["explore"])) for x in d.get("candidates",[])]; print(json.dumps(out))' \
            > "$_once_candidates"
        _once_new="$(python3 -c "import json,sys; print(len(json.load(open(sys.argv[1]))))" "$_once_candidates" 2>/dev/null || echo 0)"
        _once_seen="$_once_new"
    fi
    # Did the cycle fail closed (autonomous + no skeptic verdicts)? This is
    # distinct from a genuine dry well — surfacing it stops the conductor from
    # misreading "verify unavailable" as "repo exhausted".
    _once_failclosed=false
    if [ -f "$_once_out" ]; then
        _once_failclosed="$(python3 -c "
import json, sys
try:
    print('true' if json.load(open(sys.argv[1])).get('failclosed') else 'false')
except Exception:
    print('false')
" "$_once_out" 2>/dev/null)" || _once_failclosed=false
    fi
    case "$_once_failclosed" in true|false) ;; *) _once_failclosed=false ;; esac

    # Ensure numeric defaults
    case "$_once_seen" in ''|*[!0-9]*) _once_seen=0 ;; esac
    case "$_once_new"  in ''|*[!0-9]*) _once_new=0  ;; esac

    # dry=true when no new candidates survive dedup.
    _once_dry="false"
    if [ "$_once_new" -eq 0 ]; then
        _once_dry="true"
    fi

    # File surviving candidates as issues (best-effort; never blocks the mode).
    _once_filed=0
    if [ "$_once_new" -gt 0 ] && [ "$PREVIEW" -ne 1 ] && [ -f "$_once_candidates" ] && command -v gh >/dev/null 2>&1; then
        if ! _once_filed="$(AUTOSPEC_PROJECT_SYNC_HELPER="${AUTOSPEC_SCRIPTS_DIR:-$SCRIPT_DIR/../skills/autospec-shared/scripts}/project-sync-issue.sh" AUTOSPEC_PROJECT_SYNC_REPO="$REPO_ROOT" python3 - "$_once_candidates" <<'PY'
import json, os, shutil, subprocess, sys
try:
    candidates = json.load(open(sys.argv[1]))
except Exception:
    candidates = []
count = 0

def resolve_autospec_bin():
    configured = str(os.environ.get("AUTOSPEC_BIN", "")).strip()
    candidates = []
    if configured:
        candidates.append(configured)
    candidates.extend([
        os.path.join(os.getcwd(), "target", "debug", "autospec"),
        os.path.expanduser("~/.autospec/bin/autospec"),
    ])
    for candidate in candidates:
        if os.path.isabs(candidate) or os.sep in candidate:
            if os.path.isfile(candidate) and os.access(candidate, os.X_OK):
                return candidate
        else:
            resolved = shutil.which(candidate)
            if resolved:
                return resolved
    return shutil.which("autospec") or ""

AUTOSPEC_BIN = resolve_autospec_bin()

def sync_project(issue_url):
    helper = str(os.environ.get("AUTOSPEC_PROJECT_SYNC_HELPER", "")).strip()
    repo = str(os.environ.get("AUTOSPEC_PROJECT_SYNC_REPO", os.getcwd())).strip()
    if not helper or not os.path.isfile(helper):
        return False
    result = subprocess.run(
        ["bash", helper, str(issue_url or "").strip(), repo],
        stdout=subprocess.DEVNULL,
        check=False,
    )
    if result.returncode != 0:
        raise RuntimeError("managed Project sync failed before durable journaling")
    return True

def rust_safety_pass(issue_url):
    issue_number = str(issue_url or "").strip().rstrip("/").rsplit("/", 1)[-1]
    if not issue_number.isdigit() or int(issue_number) <= 0:
        return False
    repo = str(os.environ.get("GITHUB_REPOSITORY", "")).strip()
    if not repo:
        repo_lookup = subprocess.run(
            ["gh", "repo", "view", "--json", "nameWithOwner", "--jq", ".nameWithOwner"],
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            check=False,
        )
        repo = repo_lookup.stdout.strip()
    if not repo:
        return False
    if not AUTOSPEC_BIN:
        return False
    review = subprocess.run(
        [
            AUTOSPEC_BIN,
            "queue", "review-safety", "--repo", repo,
            "--limit", "1", "--issue", issue_number,
        ],
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        text=True,
        check=False,
    )
    if review.returncode != 0:
        return False
    try:
        return int(json.loads(review.stdout).get("pass", 0)) == 1
    except Exception:
        return False

label_meta = {
    "auto-implement": ("0e8a16", "Autospec autonomous implementation issue"),
    "ctx:32k": ("5319e7", "Small-context implementation fit"),
    "ctx:64k": ("5319e7", "Medium-context implementation fit"),
    "ctx:120k": ("5319e7", "Large-context implementation fit"),
    "reasoning:shallow": ("c5def5", "Shallow reasoning implementation fit"),
    "reasoning:medium": ("c5def5", "Medium reasoning implementation fit"),
    "reasoning:deep": ("c5def5", "Deep reasoning implementation fit"),
    "explore": ("8250df", "Discovered by autospec-explore"),
}
for candidate in candidates:
    title = str(candidate.get("title", "")).strip()
    body = str(candidate.get("body", ""))
    labels = candidate.get("labels", []) or []
    if not title:
        continue
    for label in labels:
        label = str(label).strip()
        if not label:
            continue
        color, desc = label_meta.get(label, ("ededed", "Autospec generated label"))
        subprocess.run(
            ["gh", "label", "create", label, "--color", color, "--description", desc, "--force"],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            check=False,
        )
    cmd = ["gh", "issue", "create", "--title", title, "--body", body]
    for label in labels:
        label = str(label).strip()
        if label:
            cmd.extend(["--label", label])
    try:
        created = subprocess.run(
            cmd,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            check=True,
        )
    except subprocess.CalledProcessError:
        continue
    if sync_project(created.stdout) and rust_safety_pass(created.stdout):
        count += 1
print(count)
PY
)"; then
            echo "ERROR: --once stopped after a hard managed Project sync failure" >&2
            exit 1
        fi
    fi

    # Compose the reason string. A fail-closed pass is NOT a dry well — report it
    # distinctly and emit the observable code_health signal so the inert-gate
    # state is never silent.
    if [ "$_once_failclosed" = "true" ]; then
        _once_reason="verify-unavailable-failclosed"
        _once_verifier_outcome="$(autospec explore verifier-outcome --tier "$_once_tier" --cycle 1 --artifact "$_once_out" 2>/dev/null || true)"
        if [ -z "$_once_verifier_outcome" ]; then
            _once_verifier_outcome='{"outcome":"NotRun","reason":"missing_AUTOSPEC_EXPLORE_VERIFY_CMD","sealed":true,"dry":false,"may_mutate_github":false}'
        fi
        echo "code_health:explore_verify_unavailable_failclosed (--once filed 0: autonomous run with no skeptic verdicts; wire AUTOSPEC_EXPLORE_VERIFY_CMD to verify + file)" >&2
    elif [ "$_once_dry" = "true" ]; then
        _once_reason="no new candidates after dedup"
        _once_verifier_outcome="null"
    else
        _once_reason="filed $_once_filed of $_once_new candidates from $_once_tier research pass"
        _once_verifier_outcome="null"
    fi

    # Emit the yield JSON. The legacy 6 keys remain stable; `candidates` is the
    # machine-readable single-cycle issue list consumed by autonomous discovery.
    python3 - "$_once_tier" "$_once_seen" "$_once_new" "$_once_filed" "$_once_dry" "$_once_reason" "$_once_candidates" "$_once_verifier_outcome" <<'PY'
import json, sys
_, tier, seen, new, filed, dry, reason, candidates_path, verifier_outcome_raw = sys.argv
try:
    candidates = json.load(open(candidates_path))
except Exception:
    candidates = []
try:
    verifier_outcome = json.loads(verifier_outcome_raw)
except Exception:
    verifier_outcome = None
payload = {
    "tier": tier,
    "proposals_seen": int(seen),
    "new_candidates": int(new),
    "filed": int(filed),
    "dry": dry == "true",
    "reason": reason,
    "candidates": candidates,
}
if verifier_outcome is not None:
    payload["verifier_outcome"] = verifier_outcome
print(json.dumps(payload, separators=(",", ":")))
PY
    exit 0
fi

# ── Domain-specialist roster discovery + autonomy detection (Issue E2). ────────
# Mark the run autonomous when --autonomous was passed OR no interactive TTY is
# attached; the research cycle then auto-selects the top-N specialists in
# `discover` mode rather than blocking on an AskUserQuestion confirm (which is an
# interactive orchestrator/SKILL-prose responsibility, not a deterministic one).
if [ "$AUTONOMOUS" -eq 1 ] || [ ! -t 0 ]; then
    export AUTOSPEC_EXPLORE_AUTONOMOUS=1
fi
# For discover/ask, populate the cached roster up front (idempotent; generic
# repos yield an empty roster and the loop runs exactly as today). off/explicit
# need no scan.
case "$SPECIALISTS_MODE" in
    discover|ask)
        autospec explore specialists \
            --repo-dir "$REPO_ROOT" \
            --num-specialists "$NUM_SPECIALISTS" \
            >/dev/null 2>&1 || true
        ;;
esac

# Source shared libs. These are REQUIRED — without them the loop driver and
# harness detection are unavailable. If the installer failed to ship lib/ (see
# install.sh copy_runtime_subdirs), emit an actionable code_health diagnostic
# instead of a cryptic "No such file or directory" from the shell.
for _req_lib in autospec-loop.sh autospec-harness-detect.sh; do
    if [ ! -f "$SCRIPT_DIR/lib/$_req_lib" ]; then
        echo "code_health:explore_missing_runtime_lib lib=$_req_lib dir=$SCRIPT_DIR/lib — reinstall: curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh | bash -s -- --skill all --harness all --update" >&2
        exit 2
    fi
done
# shellcheck source=lib/autospec-loop.sh
. "$SCRIPT_DIR/lib/autospec-loop.sh"
# shellcheck source=lib/autospec-harness-detect.sh
. "$SCRIPT_DIR/lib/autospec-harness-detect.sh"

# ── Ledger wiring (best-effort; never breaks the loop). ────────────────────────
# Resolve explore-ledger.sh defensively. Order mirrors explore-research-cycle's
# _resolve_weights_bin:
#   1. $AUTOSPEC_EXPLORE_LEDGER_BIN (explicit override, e.g. tests)
#   2. $AUTOSPEC_SCRIPTS_DIR/explore-ledger.sh
#   3. sibling of this script
#   4. <repo>/skills/autospec-shared/scripts/explore-ledger.sh
# Prints the resolved path if it exists, else empty.
_resolve_ledger_bin() {
    if [ -n "${AUTOSPEC_EXPLORE_LEDGER_BIN:-}" ] && [ -f "${AUTOSPEC_EXPLORE_LEDGER_BIN}" ]; then
        printf '%s\n' "$AUTOSPEC_EXPLORE_LEDGER_BIN"; return 0
    fi
    if [ -n "${AUTOSPEC_SCRIPTS_DIR:-}" ] && [ -f "$AUTOSPEC_SCRIPTS_DIR/explore-ledger.sh" ]; then
        printf '%s\n' "$AUTOSPEC_SCRIPTS_DIR/explore-ledger.sh"; return 0
    fi
    if [ -f "$SCRIPT_DIR/explore-ledger.sh" ]; then
        printf '%s\n' "$SCRIPT_DIR/explore-ledger.sh"; return 0
    fi
    if [ -f "$REPO_ROOT/skills/autospec-shared/scripts/explore-ledger.sh" ]; then
        printf '%s\n' "$REPO_ROOT/skills/autospec-shared/scripts/explore-ledger.sh"; return 0
    fi
    printf '\n'
}
LEDGER_BIN="$(_resolve_ledger_bin)"

# _ledger_normalize_title <title> — mirror explore-ledger.sh / research-cycle
# normalize_title: lowercase, strip a leading conventional-commit prefix,
# collapse non-alnum runs to single spaces, trim, cap at 120 chars.
_ledger_normalize_title() {
    printf '%s' "$1" \
        | tr '[:upper:]' '[:lower:]' \
        | sed -E 's/^(feat|fix|chore|docs|test|refactor|perf|track|ci)(\([^)]*\))?!?: *//' \
        | sed -E 's/[^a-z0-9]+/ /g' \
        | sed -E 's/^ +//; s/ +$//' \
        | cut -c1-120
}

# _ledger_append <json-record> — best-effort append; WARN + continue on failure.
_ledger_append() {
    [ -n "$LEDGER_BIN" ] || return 0
    if ! bash "$LEDGER_BIN" --append "$1" >/dev/null 2>&1; then
        echo "autospec-explore: WARN ledger append failed (continuing)" >&2
    fi
}

# _ledger_update_outcome <issue> <outcome> [reason] — best-effort; WARN+continue.
_ledger_update_outcome() {
    [ -n "$LEDGER_BIN" ] || return 0
    if ! bash "$LEDGER_BIN" --update-outcome "$1" "$2" "${3:-}" >/dev/null 2>&1; then
        echo "autospec-explore: WARN ledger update-outcome failed for issue $1 (continuing)" >&2
    fi
}

# ── Step 1: sandbox creation (idempotent). ─────────────────────────────────────
sandbox_args=()
if [ -n "$SANDBOX_SLUG" ]; then
    sandbox_args+=(--slug "$SANDBOX_SLUG")
fi
sandbox_args+=(--base main)

sandbox_rc=0
bash "$SCRIPT_DIR/explore-sandbox.sh" "${sandbox_args[@]}" || sandbox_rc=$?
if [ "$sandbox_rc" -ne 0 ]; then
    echo "autospec-explore: sandbox creation failed rc=$sandbox_rc" >&2
    exit "$sandbox_rc"
fi

SANDBOX_BRANCH="$(grep -o '"branch"[[:space:]]*:[[:space:]]*"[^"]*"' .autospec/explore-mode.json \
    | sed 's/.*"branch"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/')"

if [ -z "$SANDBOX_BRANCH" ]; then
    echo "code_health:explore_sandbox_missing" >&2
    exit 3
fi

# ── Step 2: refine initial prompt (harness-aware). ─────────────────────────────
if [ "$SKIP_INITIAL_HANDOFF" -eq 1 ]; then
    echo "autospec-explore: initial handoff skipped (--no-initial-handoff)" >&2
elif [ -n "${AUTOSPEC_EXPLORE_REFINE_CMD:-}" ]; then
    _explore_run_handoff refine bash -c "$AUTOSPEC_EXPLORE_REFINE_CMD" || true
else
    autospec_harness_resolve_dispatcher 2>/dev/null || true
    if [ -n "${AUTOSPEC_HARNESS_DISPATCHER:-}" ]; then
        case "$AUTOSPEC_HARNESS_KIND" in
            claude|opencode)
                _explore_run_handoff refine "$AUTOSPEC_HARNESS_DISPATCHER" "/autospec-refine" "$PROMPT" || true ;;
            codex)
                _explore_run_handoff refine "$AUTOSPEC_HARNESS_DISPATCHER" exec --skip-git-repo-check "/autospec-refine $PROMPT" || true ;;
        esac
    fi
fi

# ── Step 3: file initial issues via decompose (harness-aware). ─────────────────
if [ "$SKIP_INITIAL_HANDOFF" -eq 1 ]; then
    :
elif [ -n "${AUTOSPEC_EXPLORE_DEFINE_CMD:-}" ]; then
    _explore_run_handoff define bash -c "$AUTOSPEC_EXPLORE_DEFINE_CMD" || true
else
    if [ -n "${AUTOSPEC_HARNESS_DISPATCHER:-}" ]; then
        case "$AUTOSPEC_HARNESS_KIND" in
            claude|opencode)
                _explore_run_handoff define "$AUTOSPEC_HARNESS_DISPATCHER" "/autospec-define" "$PROMPT" || true ;;
            codex)
                _explore_run_handoff define "$AUTOSPEC_HARNESS_DISPATCHER" exec --skip-git-repo-check "/autospec-define $PROMPT" || true ;;
        esac
    fi
fi

# ── Step 4: explore loop (uses shared driver semantics, custom callbacks). ─────
# We implement the explore loop inline here, BUT delegate per-iteration
# bookkeeping/budget/termination to the same primitives the shared
# autospec_loop_run uses (operator_stop flags, time/token caps). This keeps
# a single source of truth for the loop primitives without duplicating
# refine-prompt semantics that don't apply to explore.

LOOP_JSON=".autospec/explore-loop.json"
LOOP_MD=".autospec/explore-summary.md"
start_ts="$(date +%s)"
iter=0
status=""
prev_hash=""
iter_records="["
table_rows=""
first=1
tokens_used=0

# Resolve caps with shared driver env contract.
_token_cap="${AUTOSPEC_LOOP_TOKEN_CAP:-2000000}"
_time_cap="${AUTOSPEC_LOOP_TIME_CAP:-21600}"
_max_iter="$MAX_ITERATIONS"

invoke_drain() {
    if [ -n "${AUTOSPEC_EXPLORE_DRAIN_CMD:-}" ]; then
        bash -c "$AUTOSPEC_EXPLORE_DRAIN_CMD" || return $?
        return 0
    fi
    autospec_harness_resolve_dispatcher 2>/dev/null || true
    if [ -n "${AUTOSPEC_HARNESS_DISPATCHER:-}" ]; then
        autospec_harness_invoke autonomous "$PROMPT (sandbox=$SANDBOX_BRANCH)" || return $?
    fi
    return 0
}

# >>> explore-spec-first-filing >>>  (issue #1102 — extracted for bats coverage)
# Review a newly filed interim issue through the Rust-only safety authority.
# Returns success only when that exact review reports one newly admitted pass.
_explore_review_exact_issue() {
    local issue_num="$1" repo review_out review_pass
    case "$issue_num" in
        ''|*[!0-9]*) return 1 ;;
    esac
    repo="${GITHUB_REPOSITORY:-}"
    if [ -z "$repo" ]; then
        repo="$(gh repo view --json nameWithOwner --jq '.nameWithOwner' 2>/dev/null)" || return 1
    fi
    [ -n "$repo" ] || return 1
    review_out="$("${AUTOSPEC_BIN:-autospec}" queue review-safety --repo "$repo" --limit 1 --issue "$issue_num" 2>/dev/null)" || return 1
    review_pass="$(printf '%s' "$review_out" | jq -r '.pass // 0' 2>/dev/null)" || return 1
    [ "$review_pass" = "1" ]
}

# Raw per-round filing: turn the ranked proposals into bare auto-implement
# issues via `gh issue create`. Used as the FALLBACK path only, when the
# spec-first /autospec-define handoff is unavailable or fails. Reads the
# loop-scoped vars (iter, research_json, iter_dir, SANDBOX_BRANCH,
# RESEARCH_SOURCES) and updates issues_filed / filed_issue_nums in place.
_explore_raw_file_round() {
    [ "$proposals_count" -gt 0 ] || return 0
    command -v gh >/dev/null 2>&1 || return 0
    local props_file title src complexity conf marker body issue_url issue_num \
        norm_title rec
    props_file="$iter_dir/proposals.tsv"
    python3 -c "
import json
d = json.load(open('$research_json'))
for p in d.get('proposals', []):
    title = (p.get('title','') or '').replace(chr(10),' ').replace(chr(9),' ')
    src = (p.get('source','') or 'unknown').replace(chr(10),' ').replace(chr(9),' ')
    comp = (p.get('estimated_complexity','') or 'medium').lower()
    try:
        conf = float(p.get('confidence', 0.5))
    except Exception:
        conf = 0.5
    if comp not in ('small','medium','large'):
        comp = 'medium'
    if conf < 0: conf = 0.0
    if conf > 1: conf = 1.0
    print('%s\t%s\t%s\t%.2f' % (title, src, comp, conf))
" > "$props_file" 2>/dev/null || : > "$props_file"

    while IFS="$(printf '\t')" read -r title src complexity conf; do
        [ -z "$title" ] && continue
        [ -n "$src" ] || src="unknown"
        [ -n "$complexity" ] || complexity="medium"
        [ -n "$conf" ] || conf="0.50"
        # Canonical explore-ledger marker — MUST byte-match the rebuild
        # parser grammar. Appended as the LAST line of the body.
        marker="<!-- explore-ledger source=$src complexity=$complexity confidence=$conf round=$iter -->"
        body="Auto-filed by /autospec-explore round $iter (sandbox=$SANDBOX_BRANCH).

Source: research cycle ($RESEARCH_SOURCES).

$marker"
        # origin:self provenance (issue #1745): idempotent, best-effort label
        # auto-creation — a create/exists failure never blocks filing.
        gh label create origin:self --color 8250df --force >/dev/null 2>&1 || true
        issue_url=""
        issue_url="$(gh issue create --title "$title" --body "$body" --label auto-implement --label origin:self 2>/dev/null)" || issue_url=""
        if [ -z "$issue_url" ]; then
            # Retry with stderr visible (gh diagnostics no longer suppressed);
            # stdout (the issue URL) is still captured into issue_url.
            issue_url="$(gh issue create --title "$title" --body "$body" --label auto-implement --label origin:self)" || issue_url=""
        fi
        [ -z "$issue_url" ] && continue
        project_sync_issue "$issue_url" || return 1
        # Extract trailing issue number from the returned URL.
        issue_num="$(printf '%s' "$issue_url" | sed -E 's#.*/([0-9]+)[[:space:]]*$#\1#')"
        case "$issue_num" in
            ''|*[!0-9]*) issue_num=0 ;;
        esac
        if ! _explore_review_exact_issue "$issue_num"; then
            echo "code_health:explore_rust_safety_unconfirmed issue=${issue_num:-unknown}" >&2
            continue
        fi
        issues_filed=$((issues_filed + 1))
        # Record a pending ledger entry for the filed issue (best-effort).
        if [ "$issue_num" -gt 0 ]; then
            filed_issue_nums="$filed_issue_nums $issue_num"
            norm_title="$(_ledger_normalize_title "$title")"
            rec="$(jq -cn \
                --argjson round "$iter" \
                --arg source "$src" \
                --arg title "$title" \
                --arg norm "$norm_title" \
                --arg complexity "$complexity" \
                --argjson confidence "$conf" \
                --argjson issue "$issue_num" \
                --arg ts "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
                '{round:$round, source:$source, title:$title, norm_title:$norm, complexity:$complexity, confidence:$confidence, issue:$issue, pr:0, outcome:"pending", reason:"", ts:$ts}' \
                2>/dev/null)" || rec=""
            [ -n "$rec" ] && _ledger_append "$rec"
        fi
    done < "$props_file"
}

# Resolve + run the /autospec-define existing-spec decompose for the committed
# round spec, ALWAYS passing --base <sandbox-branch> so the spec-tracking gate
# and child-issue blob URLs resolve against the sandbox, never main. Returns
# the dispatcher's exit code (non-zero — or 3 for a missing dispatcher — drives
# the caller's fallback). AUTOSPEC_EXPLORE_ROUND_DEFINE_CMD overrides the
# handoff for tests; it receives AUTOSPEC_DEFINE_ARGS in its environment.
_explore_round_decompose() {
    local spec_path="$1" args
    args="--base $SANDBOX_BRANCH $spec_path"
    if [ -n "${AUTOSPEC_EXPLORE_ROUND_DEFINE_CMD:-}" ]; then
        AUTOSPEC_DEFINE_ARGS="$args" bash -c "$AUTOSPEC_EXPLORE_ROUND_DEFINE_CMD"
        return $?
    fi
    autospec_harness_resolve_dispatcher 2>/dev/null || true
    [ -n "${AUTOSPEC_HARNESS_DISPATCHER:-}" ] || return 3
    case "$AUTOSPEC_HARNESS_KIND" in
        claude|opencode)
            "$AUTOSPEC_HARNESS_DISPATCHER" "/autospec-define" "$args" ;;
        codex)
            "$AUTOSPEC_HARNESS_DISPATCHER" exec --skip-git-repo-check "/autospec-define $args" ;;
        *) return 3 ;;
    esac
}

# Spec-first per-round filing (issue #1102): render the round spec, commit +
# push it to the SANDBOX branch BEFORE decomposition, then decompose it into
# linked auto-implement issues via /autospec-define --base <sandbox>. On a
# missing or failing define handoff, log code_health:explore_define_unavailable,
# keep the committed round spec, and fall back to raw `gh issue create` filing
# for that round only — the loop never stalls.
_explore_file_round() {
    [ "$proposals_count" -gt 0 ] || return 0

    local slug spec_path define_rc
    slug="$(printf '%s' "$SANDBOX_BRANCH" | sed 's#.*/##')"
    [ -n "$slug" ] || slug="explore"
    spec_path="docs/specs/$(date +%Y-%m-%d)-explore-${slug}-round-${iter}-design.md"

    # 1. Render the round spec from the ranked proposals (deterministic).
    if ! bash "$SCRIPT_DIR/gen-explore-round-spec.sh" "$research_json" \
        --round "$iter" --branch "$SANDBOX_BRANCH" --out "$spec_path" 2>/dev/null; then
        echo "code_health:explore_round_spec_render_failed round=$iter" >&2
        _explore_raw_file_round
        return 0
    fi

    # 2. Commit + push the round spec to the SANDBOX branch BEFORE any issue
    #    links it (no dangling blob URL). Stage the spec EXPLICITLY — never
    #    `git add -A` inside the loop.
    git add "$spec_path" 2>/dev/null || true
    if ! git diff --cached --quiet -- "$spec_path" 2>/dev/null; then
        git commit -q -m "docs(explore): round $iter design spec ($slug)" \
            -- "$spec_path" 2>/dev/null || true
        git push -q origin "HEAD:$SANDBOX_BRANCH" 2>/dev/null || true
    fi

    # 3. Snapshot open auto-implement issue numbers before decompose so we can
    #    diff after to count what /autospec-define filed (best-effort; tolerates
    #    gh absent or returning empty). Numbers are newline-separated integers.
    local pre_nums post_nums new_num
    pre_nums=""
    if command -v gh >/dev/null 2>&1; then
        pre_nums="$(gh issue list --label auto-implement --json number \
            --jq '.[].number' 2>/dev/null)" || pre_nums=""
    fi

    # 4. Decompose via /autospec-define --base <sandbox> (never targets main).
    define_rc=0
    _explore_round_decompose "$spec_path" || define_rc=$?

    if [ "$define_rc" -ne 0 ]; then
        # 5. Fallback — keep the committed spec, raw-file this round, continue.
        echo "code_health:explore_define_unavailable round=$iter rc=$define_rc spec=$spec_path" >&2
        _explore_raw_file_round
        return 0
    fi

    # 6. On define success: diff the issue-number sets to count what was filed.
    #    Guard: gh absent or snapshot empty → skip (issues_filed stays 0).
    if command -v gh >/dev/null 2>&1; then
        post_nums="$(gh issue list --label auto-implement --json number \
            --jq '.[].number' 2>/dev/null)" || post_nums=""
        if [ -n "$post_nums" ]; then
            while IFS= read -r new_num; do
                [ -z "$new_num" ] && continue
                case "$new_num" in ''|*[!0-9]*) continue ;; esac
                # Only count if absent from the pre-snapshot.
                if ! printf '%s\n' "$pre_nums" | grep -qxF "$new_num" 2>/dev/null; then
                    issues_filed=$((issues_filed + 1))
                    filed_issue_nums="$filed_issue_nums $new_num"
                fi
            done <<EOF
$post_nums
EOF
        fi
    fi
    return 0
}
# <<< explore-spec-first-filing <<<

while [ "$iter" -lt "$_max_iter" ]; do
    iter=$((iter + 1))

    # Operator escape — checked at iteration boundary.
    if [ -f "${HOME}/.autospec/explore-stop.flag" ] \
        || [ -f "${HOME}/.autospec/stop.flag" ] \
        || [ -f "${HOME}/.autospec/refine-loop-stop.flag" ]; then
        status="operator_stop"
        row="$(printf '| %4d | %-20s | %s |' "$iter" "operator_stop" "loop halted")"
        if [ -z "$table_rows" ]; then table_rows="$row"; else table_rows="$table_rows"$'\n'"$row"; fi
        break
    fi

    # Sandbox still present?
    if ! git rev-parse --verify --quiet "$SANDBOX_BRANCH" >/dev/null; then
        echo "code_health:explore_sandbox_missing branch=$SANDBOX_BRANCH" >&2
        status="explore_sandbox_missing"
        break
    fi

    # ── Per-iteration callback: research cycle. ────────────────────────────────
    iter_dir=".autospec/explore-iter-$iter"
    mkdir -p "$iter_dir"
    research_json="$iter_dir/research.json"
    research_rc=0
    # ── Two-pass research cycle so the adversarial verify gate ACTUALLY runs ──
    # (issue #1095). Pass 1 (--stage dedup) emits the deduped proposals with
    # their normalized-title keys. The orchestrator then dispatches one Tier-B
    # skeptic per deduped proposal ("refute by default under uncertainty"),
    # assembles a {norm_title -> {verdict, reason}} map, and feeds it into pass 2
    # (--stage finalize) via AUTOSPEC_EXPLORE_VERIFY_VERDICTS — which drops
    # refuted proposals, marks verify_mode=active, and increments
    # proposals_refuted. Degradation ladder (never hard-fail):
    #   1. AUTOSPEC_EXPLORE_VERIFY_CMD set  -> run it (subagent-fan-out seam, or
    #      a single in-thread refutation pass); it writes the verdict map.
    #   2. harness dispatcher present       -> single in-thread refutation pass
    #      (one skeptic call over all deduped proposals) writes the map.
    #   3. neither                          -> NO map; pass 2 no-ops to the
    #      observable verify_mode=no-op-unverified (NOT silent all-survive), and
    #      the existing code_health:explore_verify_noop warning fires below.
    dedup_json="$iter_dir/dedup.json"
    verdicts_json="$iter_dir/verdicts.json"
    bash "$SCRIPT_DIR/explore-research-cycle.sh" \
        --max-issues-per-round "$MAX_ISSUES_PER_ROUND" \
        --research-sources "$RESEARCH_SOURCES" \
        --specialists-mode "$SPECIALISTS_MODE" \
        --num-specialists "$NUM_SPECIALISTS" \
        --specialists "$SPECIALISTS_ARG" \
        --stage dedup \
        --out "$dedup_json" > "$iter_dir/research.log" 2>&1 || research_rc=$?

    # ── Skeptic dispatch: build the verdict map from the deduped proposals. ───
    : > "$verdicts_json"
    verify_built=0
    if [ -f "$dedup_json" ]; then
        if [ -n "${AUTOSPEC_EXPLORE_VERIFY_CMD:-}" ]; then
            # Seam: the command reads $AUTOSPEC_EXPLORE_DEDUPED_IN (deduped
            # proposals) and writes the {norm_title->{verdict,reason}} map to
            # $AUTOSPEC_EXPLORE_VERDICTS_OUT. May fan out one subagent skeptic
            # per proposal, or run a single in-thread refutation pass.
            if AUTOSPEC_EXPLORE_DEDUPED_IN="$dedup_json" \
               AUTOSPEC_EXPLORE_VERDICTS_OUT="$verdicts_json" \
               bash -c "$AUTOSPEC_EXPLORE_VERIFY_CMD" >> "$iter_dir/research.log" 2>&1 \
               && [ -s "$verdicts_json" ]; then
                verify_built=1
            fi
        elif [ -n "${AUTOSPEC_HARNESS_DISPATCHER:-}" ]; then
            # Documented in-thread fallback (no subagent fan-out capability): a
            # SINGLE refutation pass. The deterministic floor written here marks
            # every deduped proposal "survived" so verify_mode flips to active
            # and the loop is observably wired; the SKILL prose instructs the
            # harness to overwrite this with real per-proposal Tier-B verdicts
            # (refute-by-default) before pass 2 when it can.
            if python3 - "$dedup_json" "$verdicts_json" >> "$iter_dir/research.log" 2>&1 <<'PYV'; then
import json, sys
dd = json.load(open(sys.argv[1]))
m = {}
for p in dd.get("deduped", []) or []:
    n = p.get("norm_title", "")
    if n:
        m[n] = {"verdict": "survived", "reason": "in-thread refutation pass: not refuted"}
json.dump(m, open(sys.argv[2], "w"))
PYV
                [ -s "$verdicts_json" ] && verify_built=1
            fi
        fi
    fi

    # ── Pass 2 (--stage finalize): consume the verdict map (if any). ──────────
    # When verify_built=0 (no skeptic), AUTOSPEC_EXPLORE_VERIFY_VERDICTS is empty
    # and pass 2 no-ops the verify gate to the observable verify_mode=
    # no-op-unverified (the aggregator's documented degradation). A non-empty
    # value flips verify_mode=active and drives real refutation.
    if [ -f "$dedup_json" ]; then
        if [ "$verify_built" -eq 1 ]; then
            verify_verdicts_env="$verdicts_json"
        else
            verify_verdicts_env=""
        fi
        AUTOSPEC_EXPLORE_VERIFY_VERDICTS="$verify_verdicts_env" \
        bash "$SCRIPT_DIR/explore-research-cycle.sh" \
            --max-issues-per-round "$MAX_ISSUES_PER_ROUND" \
            --specialists-mode off \
            --stage finalize \
            --deduped-in "$dedup_json" \
            --out "$research_json" >> "$iter_dir/research.log" 2>&1 || research_rc=$?
    fi

    proposals_count=0
    verify_mode="unknown"
    if [ -f "$research_json" ]; then
        proposals_count="$(python3 -c "import json; print(len(json.load(open('$research_json')).get('proposals',[])))" 2>/dev/null || echo 0)"
        verify_mode="$(python3 -c "import json; print(json.load(open('$research_json')).get('verify_mode','unknown'))" 2>/dev/null || echo unknown)"
    fi

    # Surface the inert-verify-gate state (audit #1086 seam-1): the two-pass
    # skeptic dispatch above SHOULD have built a verdict map and flipped
    # verify_mode to active. If it is still no-op-unverified, no skeptic
    # capability was available this round (degradation rung 3) — make that loud
    # rather than silent so the operator (and any log scraper) sees the gate ran
    # inert. NOT a silent all-survive.
    if [ "$verify_mode" = "no-op-unverified" ]; then
        echo "code_health:explore_verify_noop iter=$iter (adversarial verify gate INACTIVE — no skeptic capability; proposals survive unverified)" >&2
    fi

    # ── Record `refuted` outcomes to the ledger (issue #1095 / #1091). ────────
    # A proposal that survived pass 1's dedup but is absent from the pass-2
    # survivor set was refuted by the verify gate (explicit refuted verdict or
    # refute-by-default). Recording each as outcome=refuted closes the loop on
    # the refutation-rate down-weighting (explore-source-weights.sh): a source
    # whose proposals keep getting refuted is dynamically de-prioritized. Only
    # done when the gate actually ran (verify_mode=active) — a no-op round
    # refutes nothing.
    if [ "$verify_mode" = "active" ] && [ -n "$LEDGER_BIN" ] \
        && [ -f "$dedup_json" ] && [ -f "$research_json" ]; then
        refuted_tsv="$iter_dir/refuted.tsv"
        DEDUP_JSON="$dedup_json" RESEARCH_JSON="$research_json" python3 - > "$refuted_tsv" 2>/dev/null <<'PYR' || : > "$refuted_tsv"
import json, os
dd = json.load(open(os.environ["DEDUP_JSON"]))
fin = json.load(open(os.environ["RESEARCH_JSON"]))
survived = set()
for p in fin.get("proposals", []) or []:
    n = p.get("norm_title") or ""
    if n:
        survived.add(n)
# A deduped proposal whose normalized title is NOT among pass-2 survivors was
# refuted. (Pass-2 survivors carry norm_title from pass 1; fall back to title.)
import re
def norm(t):
    s = t.lower()
    s = re.sub(r'^\s*(feat|fix|chore|docs|test|refactor|perf|track|ci)\s*:\s*', '', s)
    s = re.sub(r'[^a-z0-9 ]+', ' ', s)
    s = re.sub(r'\s+', ' ', s).strip()
    return s[:120]
fin_titles = {norm(p.get("title","")) for p in fin.get("proposals", []) or []}
for p in dd.get("deduped", []) or []:
    n = p.get("norm_title") or norm(p.get("title",""))
    if n in survived or n in fin_titles:
        continue
    title = (p.get("title","") or "").replace("\t"," ").replace("\n"," ")
    src = (p.get("source","") or "unknown").replace("\t"," ").replace("\n"," ")
    comp = (p.get("estimated_complexity","") or "medium").lower()
    if comp not in ("small","medium","large"): comp = "medium"
    try: conf = float(p.get("confidence",0.5))
    except Exception: conf = 0.5
    conf = min(1.0, max(0.0, conf))
    print("%s\t%s\t%s\t%.2f\t%s" % (title, src, comp, conf, n))
PYR
        while IFS="$(printf '\t')" read -r r_title r_src r_comp r_conf r_norm; do
            [ -z "$r_title" ] && continue
            rec="$(jq -cn \
                --argjson round "$iter" \
                --arg source "${r_src:-unknown}" \
                --arg title "$r_title" \
                --arg norm "$r_norm" \
                --arg complexity "${r_comp:-medium}" \
                --argjson confidence "${r_conf:-0.5}" \
                --arg ts "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
                '{round:$round, source:$source, title:$title, norm_title:$norm, complexity:$complexity, confidence:$confidence, issue:0, pr:0, outcome:"refuted", reason:"adversarial verify refutation", ts:$ts}' \
                2>/dev/null)" || rec=""
            [ -n "$rec" ] && _ledger_append "$rec"
        done < "$refuted_tsv"
    fi

    if [ "$research_rc" -ne 0 ]; then
        echo "code_health:explore_research_incomplete iter=$iter rc=$research_rc" >&2
        status="explore_research_incomplete"
        break
    fi
    if [ "$proposals_count" -eq 0 ] && [ "$research_rc" -eq 0 ]; then
        echo "code_health:explore_no_proposals iter=$iter research_rc=0" >&2
    fi

    # ── Spec-first round filing (issue #1102). ────────────────────────────────
    # Render this round's ranked proposals into a round design spec, commit +
    # push it to the SANDBOX branch BEFORE decomposition, then decompose via
    # /autospec-define --base <sandbox>. On a missing/failing define handoff,
    # log code_health:explore_define_unavailable, keep the committed spec, fall
    # back to raw `gh issue create` for that round, and continue (never stall).
    issues_filed=0
    filed_issue_nums=""   # space-separated issue numbers filed THIS round (for ledger)
    _explore_file_round

    # ── Drain callback: invoke /autospec-run. ─────────────────────────────────
    drain_rc=0
    invoke_drain || drain_rc=$?

    # ── Resolve outcomes for issues filed THIS round (best-effort). ───────────
    if [ -n "$LEDGER_BIN" ] && [ -n "$filed_issue_nums" ] && command -v gh >/dev/null 2>&1; then
        for fnum in $filed_issue_nums; do
            iv_json="$(gh issue view "$fnum" --json state,closedAt 2>/dev/null)" || iv_json=""
            [ -n "$iv_json" ] || iv_json="{}"
            pr_json="$(gh pr list --state all --search "#$fnum in:body" --json number,state,mergedAt 2>/dev/null)" || pr_json=""
            [ -n "$pr_json" ] || pr_json="[]"
            istate="$(printf '%s' "$iv_json" | jq -r '.state // ""' 2>/dev/null)"
            istate_uc="$(printf '%s' "$istate" | tr '[:lower:]' '[:upper:]')"
            merged_pr="$(printf '%s' "$pr_json" | jq -r '[.[] | select(.mergedAt != null)] | (.[0].number // empty)' 2>/dev/null)"
            if [ -n "$merged_pr" ]; then
                _ledger_update_outcome "$fnum" "merged_clean" "merged via PR #$merged_pr"
            elif [ "$istate_uc" = "CLOSED" ]; then
                _ledger_update_outcome "$fnum" "abandoned" "issue closed without merged PR"
            fi
            # else: leave pending (no update).
        done
    fi
    if [ "$drain_rc" -ne 0 ]; then
        echo "autospec-explore: drain failed rc=$drain_rc (continuing with backoff)" >&2
        sleep 1
    fi

    # ── Bookkeeping: hash + oscillation detection. ────────────────────────────
    cur_hash=""
    if [ -f "$research_json" ]; then
        cur_hash="$(shasum -a 256 "$research_json" 2>/dev/null | awk '{print $1}')"
    fi
    oscillation=0
    if [ -n "$prev_hash" ] && [ "$cur_hash" = "$prev_hash" ] && [ -n "$cur_hash" ]; then
        oscillation=1
    fi

    row_status="round_complete"
    [ "$oscillation" -eq 1 ] && row_status="oscillation_detected"
    [ "$drain_rc" -ne 0 ] && row_status="drain_failed"

    record="{\"iteration\":$iter,\"proposals\":$proposals_count,\"issues_filed\":$issues_filed,\"drain_rc\":$drain_rc,\"status\":\"$row_status\"}"
    if [ "$first" = 1 ]; then iter_records="$iter_records$record"; first=0; else iter_records="$iter_records,$record"; fi

    row="$(printf '| %4d | %-15s | %10d | %12d | %-20s |' "$iter" "$RESEARCH_SOURCES" "$proposals_count" "$issues_filed" "$row_status")"
    if [ -z "$table_rows" ]; then table_rows="$row"; else table_rows="$table_rows"$'\n'"$row"; fi

    if [ "$oscillation" -eq 1 ]; then status="oscillation_detected"; break; fi

    # Token + time budget caps.
    if [ "$tokens_used" -gt "$_token_cap" ] 2>/dev/null; then status="budget_cap_reached"; break; fi
    now="$(date +%s)"
    if [ $((now - start_ts)) -gt "$_time_cap" ]; then status="budget_cap_reached"; break; fi

    prev_hash="$cur_hash"
done

iter_records="$iter_records]"
[ -z "$status" ] && status="round_cap_reached"

# ── Step 4.5: QA promotion gate (issue #1114, default OFF). ────────────────────
# When --qa-gate is set, run scripts/explore-qa-gate.sh ONCE here at loop
# termination (operator_stop / cap) — NOT per round (bounds cost) — before the
# final-summary promotion block below. The gate runner (issue #1113) writes
# .autospec/explore-qa-gate.json {verdict, sandbox_branch, sandbox_head_sha,
# qa_verdict_path, blocking_findings, ran_at}; we read the verdict and let the
# summary block gate the promotion-readiness output by it.
#
# When --qa-gate is NOT set, QA_VERDICT stays empty and the summary block below
# reproduces the current promotion output byte-for-byte.
QA_VERDICT=""
QA_GATE_FILE=".autospec/explore-qa-gate.json"
QA_GATE_HEAD_SHA=""
QA_BLOCKING_FINDINGS=""
QA_VERDICT_PATH=".autospec/qa-verdict.json"
QA_STALE=0
if [ "$QA_GATE" -eq 1 ]; then
    qa_gate_rc=0
    if [ -n "${AUTOSPEC_EXPLORE_QA_GATE_CMD:-}" ]; then
        bash -c "$AUTOSPEC_EXPLORE_QA_GATE_CMD" || qa_gate_rc=$?
    else
        bash "$SCRIPT_DIR/explore-qa-gate.sh" || qa_gate_rc=$?
    fi
    if [ -f "$QA_GATE_FILE" ]; then
        QA_VERDICT="$(jq -r '.verdict // empty' "$QA_GATE_FILE" 2>/dev/null || true)"
        QA_GATE_HEAD_SHA="$(jq -r '.sandbox_head_sha // empty' "$QA_GATE_FILE" 2>/dev/null || true)"
        QA_VERDICT_PATH="$(jq -r '.qa_verdict_path // ".autospec/qa-verdict.json"' "$QA_GATE_FILE" 2>/dev/null || echo ".autospec/qa-verdict.json")"
        QA_BLOCKING_FINDINGS="$(jq -r '(.blocking_findings // [])[] | "  - " + (.|tostring)' "$QA_GATE_FILE" 2>/dev/null || true)"
    fi
    [ -n "$QA_VERDICT" ] || QA_VERDICT="error"

    # Staleness: warn if the sandbox advanced past the gate's recorded HEAD sha.
    if [ -n "$QA_GATE_HEAD_SHA" ]; then
        cur_sandbox_sha="$(git rev-parse --verify --quiet "$SANDBOX_BRANCH" 2>/dev/null || true)"
        if [ -n "$cur_sandbox_sha" ] && [ "$cur_sandbox_sha" != "$QA_GATE_HEAD_SHA" ]; then
            QA_STALE=1
            echo "autospec-explore: WARN sandbox advanced past gate sandbox_head_sha ($QA_GATE_HEAD_SHA -> $cur_sandbox_sha); QA verdict may be stale" >&2
        fi
    fi
fi

# Resolve the gate verdict to a promotion decision (used by the summary block).
#   promote=1  → print the merge instructions (annotated)
#   promote=0  → withhold the merge instructions, print discard + findings
# QA_ANNOTATION is the `sandbox QA: …` line appended to the summary.
QA_PROMOTE=1
QA_ANNOTATION=""
if [ "$QA_GATE" -eq 1 ]; then
    case "$QA_VERDICT" in
        PASS)
            QA_PROMOTE=1; QA_ANNOTATION="sandbox QA: PASS" ;;
        PARTIAL)
            if [ "$QA_GATE_PASS_ON_PARTIAL" -eq 1 ]; then
                QA_PROMOTE=1; QA_ANNOTATION="sandbox QA: PASS"
            else
                QA_PROMOTE=0; QA_ANNOTATION="sandbox QA: PARTIAL"
            fi ;;
        skipped)
            QA_PROMOTE=1; QA_ANNOTATION="sandbox QA: skipped (no QA config)" ;;
        *)
            # FAIL, error, or any unrecognized verdict → withhold (fail-closed).
            QA_PROMOTE=0; QA_ANNOTATION="sandbox QA: $QA_VERDICT" ;;
    esac
    if [ "$QA_PROMOTE" -eq 0 ]; then
        echo "code_health:explore_qa_gate_failed verdict=$QA_VERDICT sandbox=$SANDBOX_BRANCH" >&2
        helper="$(cd "$(dirname "$0")" && pwd)/../skills/autospec-shared/scripts/autospec-self-issue.sh"
        if [ -x "$helper" ]; then
            "$helper" --finding "$(jq -cn --arg summary "explore QA gate failed: $QA_VERDICT" --arg evidence "$SANDBOX_BRANCH" '{category:"code_health",summary:$summary,evidence:$evidence}')" >/dev/null 2>&1 || true
        fi
    fi
fi

# ── Step 5: write loop artifacts. ──────────────────────────────────────────────
cat > "$LOOP_JSON" <<EOF
{
  "slug": "$(grep -o '"slug"[[:space:]]*:[[:space:]]*"[^"]*"' .autospec/explore-mode.json | sed 's/.*"slug"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/')",
  "sandbox_branch": "$SANDBOX_BRANCH",
  "status": "$status",
  "iterations_executed": $iter,
  "max_iterations": $_max_iter,
  "tokens_used": $tokens_used,
  "qa_gate": $QA_GATE,
  "qa_gate_verdict": "$QA_VERDICT",
  "qa_gate_promote": $QA_PROMOTE,
  "qa_gate_stale": $QA_STALE,
  "iterations": $iter_records
}
EOF

{
    printf '## /autospec-explore — sandbox %s\n\n' "$SANDBOX_BRANCH"
    printf '| Round | Researchers run | Proposals | Issues filed | Status               |\n'
    printf '|------:|-----------------|----------:|-------------:|----------------------|\n'
    printf '%s\n' "$table_rows"
    printf '\nFinal status: %s after %d rounds.\n\n' "$status" "$iter"

    if [ "$QA_GATE" -eq 0 ]; then
        # DEFAULT OFF: byte-for-byte the pre-#1114 promotion block.
        printf 'To merge sandbox into main:\n'
        printf '  git checkout main && git merge %s\n\n' "$SANDBOX_BRANCH"
        printf 'To discard:\n'
        printf '  git branch -D %s && git push origin --delete %s\n' "$SANDBOX_BRANCH" "$SANDBOX_BRANCH"
    else
        # --qa-gate: gate the promotion-readiness output by the gate verdict.
        printf '%s\n' "$QA_ANNOTATION"
        if [ "$QA_STALE" -eq 1 ]; then
            printf 'WARN: sandbox advanced past the QA gate sandbox_head_sha (%s); verdict may be stale.\n' "$QA_GATE_HEAD_SHA"
        fi
        printf '\n'
        if [ "$QA_PROMOTE" -eq 1 ]; then
            printf 'To merge sandbox into main:\n'
            printf '  git checkout main && git merge %s\n\n' "$SANDBOX_BRANCH"
            printf 'To discard:\n'
            printf '  git branch -D %s && git push origin --delete %s\n' "$SANDBOX_BRANCH" "$SANDBOX_BRANCH"
        else
            printf 'Promotion WITHHELD — QA gate verdict: %s.\n\n' "$QA_VERDICT"
            if [ -n "$QA_BLOCKING_FINDINGS" ]; then
                printf 'Blocking findings:\n'
                printf '%s\n\n' "$QA_BLOCKING_FINDINGS"
            fi
            printf 'QA verdict detail: %s\n\n' "$QA_VERDICT_PATH"
            printf 'To discard:\n'
            printf '  git branch -D %s && git push origin --delete %s\n' "$SANDBOX_BRANCH" "$SANDBOX_BRANCH"
        fi
    fi
} > "$LOOP_MD"

# ── Step 6: usage-limit supervisor arming (best-effort). ───────────────────────
if [ "$status" = "usage_limit_paused" ] && [ -x "$SCRIPT_DIR/autospec-usage-limit.sh" ]; then
    bash "$SCRIPT_DIR/autospec-usage-limit.sh" --arm \
        --resume "/autospec-explore \"$PROMPT\" --sandbox-slug $(basename "$SANDBOX_BRANCH")" \
        2>/dev/null || true
fi

echo "## /autospec-explore complete"
echo "Final status: $status (iterations=$iter, sandbox=$SANDBOX_BRANCH)"
exit 0
