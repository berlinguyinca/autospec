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

# Defaults.
MAX_ITERATIONS=3
MAX_ISSUES_PER_ROUND=5
BUDGET_TOKENS=""
BUDGET_HOURS=""
SANDBOX_SLUG=""
RESEARCH_SOURCES="spec-vs-code,prior-reports,codebase-signals,open-issues,source-analysis,internet"
NO_INTERNET=0
INTERNET_ALLOWLIST=""
PROMPT=""

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
        -h|--help)               usage; exit 0 ;;
        --) shift; PROMPT="${PROMPT:-$*}"; break ;;
        -*) echo "autospec-explore: unknown flag: $1" >&2; usage; exit 2 ;;
        *)  if [ -z "$PROMPT" ]; then PROMPT="$1"; else PROMPT="$PROMPT $1"; fi ;;
    esac
    shift
done

if [ -z "$PROMPT" ]; then
    echo "autospec-explore: missing initial prompt" >&2
    usage
    exit 2
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

# Source shared libs.
# shellcheck source=lib/autospec-loop.sh
. "$SCRIPT_DIR/lib/autospec-loop.sh"
# shellcheck source=lib/autospec-harness-detect.sh
. "$SCRIPT_DIR/lib/autospec-harness-detect.sh"

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
if [ -n "${AUTOSPEC_EXPLORE_REFINE_CMD:-}" ]; then
    bash -c "$AUTOSPEC_EXPLORE_REFINE_CMD" || true
else
    autospec_harness_resolve_dispatcher 2>/dev/null || true
    if [ -n "${AUTOSPEC_HARNESS_DISPATCHER:-}" ]; then
        case "$AUTOSPEC_HARNESS_KIND" in
            claude|opencode)
                "$AUTOSPEC_HARNESS_DISPATCHER" "/autospec-refine" "$PROMPT" >/dev/null 2>&1 || true ;;
            codex)
                "$AUTOSPEC_HARNESS_DISPATCHER" exec --skip-git-repo-check "/autospec-refine $PROMPT" >/dev/null 2>&1 || true ;;
        esac
    fi
fi

# ── Step 3: file initial issues via decompose (harness-aware). ─────────────────
if [ -n "${AUTOSPEC_EXPLORE_DEFINE_CMD:-}" ]; then
    bash -c "$AUTOSPEC_EXPLORE_DEFINE_CMD" || true
else
    if [ -n "${AUTOSPEC_HARNESS_DISPATCHER:-}" ]; then
        case "$AUTOSPEC_HARNESS_KIND" in
            claude|opencode)
                "$AUTOSPEC_HARNESS_DISPATCHER" "/autospec-define" "$PROMPT" >/dev/null 2>&1 || true ;;
            codex)
                "$AUTOSPEC_HARNESS_DISPATCHER" exec --skip-git-repo-check "/autospec-define $PROMPT" >/dev/null 2>&1 || true ;;
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
    bash "$SCRIPT_DIR/explore-research-cycle.sh" \
        --max-issues-per-round "$MAX_ISSUES_PER_ROUND" \
        --research-sources "$RESEARCH_SOURCES" \
        --out "$research_json" > "$iter_dir/research.log" 2>&1 || research_rc=$?

    proposals_count=0
    if [ -f "$research_json" ]; then
        proposals_count="$(python3 -c "import json; print(len(json.load(open('$research_json')).get('proposals',[])))" 2>/dev/null || echo 0)"
    fi

    if [ "$proposals_count" -eq 0 ] && [ "$research_rc" -ne 0 ]; then
        echo "code_health:explore_all_researchers_failed iter=$iter" >&2
        status="explore_all_researchers_failed"
        break
    fi

    # ── File top-N proposals as auto-implement issues. ────────────────────────
    issues_filed=0
    if [ "$proposals_count" -gt 0 ] && command -v gh >/dev/null 2>&1; then
        # Iterate proposal titles via python.
        titles_file="$iter_dir/titles.txt"
        python3 -c "
import json
d = json.load(open('$research_json'))
for p in d.get('proposals', []):
    print(p.get('title','').replace(chr(10),' '))
" > "$titles_file" 2>/dev/null || : > "$titles_file"

        while IFS= read -r title; do
            [ -z "$title" ] && continue
            body="Auto-filed by /autospec-explore round $iter (sandbox=$SANDBOX_BRANCH).

Source: research cycle ($RESEARCH_SOURCES)."
            if gh issue create --title "$title" --body "$body" --label auto-implement >/dev/null 2>&1; then
                issues_filed=$((issues_filed + 1))
            elif gh issue create --title "$title" --body "$body" --label auto-implement; then
                issues_filed=$((issues_filed + 1))
            fi
        done < "$titles_file"
    fi

    # ── Drain callback: invoke /autospec-run. ─────────────────────────────────
    drain_rc=0
    invoke_drain || drain_rc=$?
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

# ── Step 5: write loop artifacts. ──────────────────────────────────────────────
cat > "$LOOP_JSON" <<EOF
{
  "slug": "$(grep -o '"slug"[[:space:]]*:[[:space:]]*"[^"]*"' .autospec/explore-mode.json | sed 's/.*"slug"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/')",
  "sandbox_branch": "$SANDBOX_BRANCH",
  "status": "$status",
  "iterations_executed": $iter,
  "max_iterations": $_max_iter,
  "tokens_used": $tokens_used,
  "iterations": $iter_records
}
EOF

{
    printf '## /autospec-explore — sandbox %s\n\n' "$SANDBOX_BRANCH"
    printf '| Round | Researchers run | Proposals | Issues filed | Status               |\n'
    printf '|------:|-----------------|----------:|-------------:|----------------------|\n'
    printf '%s\n' "$table_rows"
    printf '\nFinal status: %s after %d rounds.\n\n' "$status" "$iter"
    printf 'To merge sandbox into main:\n'
    printf '  git checkout main && git merge %s\n\n' "$SANDBOX_BRANCH"
    printf 'To discard:\n'
    printf '  git branch -D %s && git push origin --delete %s\n' "$SANDBOX_BRANCH" "$SANDBOX_BRANCH"
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
