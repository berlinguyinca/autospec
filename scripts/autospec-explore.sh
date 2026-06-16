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
SPECIALISTS_MODE="discover"
NUM_SPECIALISTS=3
SPECIALISTS_ARG=""
AUTONOMOUS=0
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
  --specialists-mode MODE     Domain-specialist roster mode (Issue E2):
                                discover (default) | ask | explicit | off.
  --num-specialists N         Roster size for discover/ask (default 3, cap 6).
  --specialists LIST          Explicit roster slug:persona,... (explicit mode).
  --autonomous                Non-interactive run: discover mode auto-selects the
                              top-N specialists and never blocks on confirmation.
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
        if [ -x "$SCRIPT_DIR/explore-specialist-scan.sh" ]; then
            AUTOSPEC_NUM_SPECIALISTS="$NUM_SPECIALISTS" \
                bash "$SCRIPT_DIR/explore-specialist-scan.sh" >/dev/null 2>&1 || true
        fi
        ;;
esac

# Source shared libs.
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

# >>> explore-spec-first-filing >>>  (issue #1102 — extracted for bats coverage)
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
        issue_url=""
        issue_url="$(gh issue create --title "$title" --body "$body" --label auto-implement 2>/dev/null)" || issue_url=""
        if [ -z "$issue_url" ]; then
            # Retry with stderr visible (gh diagnostics no longer suppressed);
            # stdout (the issue URL) is still captured into issue_url.
            issue_url="$(gh issue create --title "$title" --body "$body" --label auto-implement)" || issue_url=""
        fi
        [ -z "$issue_url" ] && continue
        issues_filed=$((issues_filed + 1))
        # Extract trailing issue number from the returned URL.
        issue_num="$(printf '%s' "$issue_url" | sed -E 's#.*/([0-9]+)[[:space:]]*$#\1#')"
        case "$issue_num" in
            ''|*[!0-9]*) issue_num=0 ;;
        esac
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

    # 3. Decompose via /autospec-define --base <sandbox> (never targets main).
    define_rc=0
    _explore_round_decompose "$spec_path" || define_rc=$?

    if [ "$define_rc" -ne 0 ]; then
        # 4. Fallback — keep the committed spec, raw-file this round, continue.
        echo "code_health:explore_define_unavailable round=$iter rc=$define_rc spec=$spec_path" >&2
        _explore_raw_file_round
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

    if [ "$proposals_count" -eq 0 ] && [ "$research_rc" -ne 0 ]; then
        echo "code_health:explore_all_researchers_failed iter=$iter" >&2
        status="explore_all_researchers_failed"
        break
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
