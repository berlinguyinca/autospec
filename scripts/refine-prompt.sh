#!/usr/bin/env bash
# scripts/refine-prompt.sh — autospec-refine orchestrator (issue #670).
#
# N-round prompt refinement with repo-grounded lenses. Each round applies one
# named lens (deterministic text transformation in v1) and is recorded to a
# JSON artifact at .autospec/refinements/<slug>-<ISO-timestamp>.json.
#
# Lenses (default order): repo-grounding → clarity-ac → sizing → adversarial.
# If --rounds exceeds the lens list length, the adversarial lens repeats.
#
# Termination — round loop exits when one of:
#   - converged          round N == round N-1 byte-identical
#   - round_cap_reached  --rounds N capped at AUTOSPEC_REFINE_MAX_ROUNDS (10)
#   - completed          all requested rounds executed cleanly
#
# Path allowlist — refuses paths matching .env, *credential*, *secret*, *.pem,
# *.key, .git/, node_modules/. Violation surfaces as
# `code_health:refine_path_violation` and exits 3.
#
# Exit codes:
#   0  — happy path (completed | converged | round_cap_reached)
#   2  — usage / bad args
#   3  — code_health:refine_path_violation
#   4  — empty prompt; code_health:refine_artifact_write_failed
#        (artifact dir unwritable, or artifact missing/empty after write)

set -u

usage() {
    cat <<'EOF'
Usage: refine-prompt.sh "<initial prompt>" [flags]
       refine-prompt.sh --from-file <path>  [flags]

Flags:
  --rounds N             Number of refinement passes (default 3, cap 10).
  --lenses <list>        Comma-separated lens order. Names:
                         repo-grounding, clarity-ac, sizing, adversarial.
  --from-file <path>     Read the initial prompt from a file.
  --output <path>        Also write the final refined prompt to this file.
  --dry-run              Skip handoff, write artifact only.
  --interactive          After refinement, hand off to /autospec (interactive).
  --autonomous           After refinement, hand off to /autospec --autonomous (default).
  --artifact-dir DIR     Override .autospec/refinements (test hook).
  --repo-root DIR        Override repo root (test hook).
  --memory-root DIR      Override ~/.autospec/projects memory root (test hook).
  --help                 Show this and exit.

Lens dispatch:
  --lens-mode MODE       deterministic | llm | auto (overrides the env hatch).
  --llm-binary PATH      Explicit LLM dispatcher for the lens path (test hook).

Environment:
  AUTOSPEC_REFINE_MAX_ROUNDS    default 10
  AUTOSPEC_REFINE_LENS_MODE     deterministic | llm | auto (default auto).
                                auto = LLM-first: each lens round dispatches the
                                LLM path first; the deterministic template lens
                                is the fallback ONLY when the LLM path is
                                unavailable/fails (round tagged
                                degraded_fallback=true). llm = LLM-only (fails
                                loudly if no dispatcher). deterministic = legacy
                                template lenses (offline/tests). The --lens-mode
                                flag wins over this env var.
EOF
}

# ── arg parsing ───────────────────────────────────────────────────
PROMPT=""
FROM_FILE=""
ROUNDS=3
LENSES_RAW=""
OUTPUT=""
DRY_RUN=0
HANDOFF_MODE="autonomous"
ARTIFACT_DIR=".autospec/refinements"
REPO_ROOT="."
MEMORY_ROOT="${HOME}/.autospec/projects"
MAX_ROUNDS="${AUTOSPEC_REFINE_MAX_ROUNDS:-10}"
CONTINUE_MODE=0
MAX_ITERATIONS="${AUTOSPEC_REFINE_LOOP_MAX_ITERATIONS:-5}"
SIM_ITER_DIR=""
SIM_TOKENS=""
TOKEN_CAP="${AUTOSPEC_REFINE_LOOP_TOKEN_CAP:-2000000}"
TIME_CAP="${AUTOSPEC_REFINE_LOOP_TIME_CAP:-21600}"
LENS_MODE=""
LLM_BINARY=""

while [ $# -gt 0 ]; do
    case "$1" in
        --rounds) ROUNDS="$2"; shift ;;
        --lenses) LENSES_RAW="$2"; shift ;;
        --from-file) FROM_FILE="$2"; shift ;;
        --output) OUTPUT="$2"; shift ;;
        --dry-run) DRY_RUN=1 ;;
        --interactive) HANDOFF_MODE="interactive" ;;
        --autonomous) HANDOFF_MODE="autonomous" ;;
        --artifact-dir) ARTIFACT_DIR="$2"; shift ;;
        --repo-root) REPO_ROOT="$2"; shift ;;
        --memory-root) MEMORY_ROOT="$2"; shift ;;
        --continue) CONTINUE_MODE=1 ;;
        --max-iterations) MAX_ITERATIONS="$2"; shift ;;
        --simulate-iterations) SIM_ITER_DIR="$2"; shift ;;
        --simulate-tokens) SIM_TOKENS="$2"; shift ;;
        --lens-mode) LENS_MODE="$2"; shift ;;
        --llm-binary) LLM_BINARY="$2"; shift ;;
        --help|-h) usage; exit 0 ;;
        --*) echo "refine-prompt: unknown flag: $1" >&2; usage >&2; exit 2 ;;
        *)
            if [ -z "$PROMPT" ]; then
                PROMPT="$1"
            else
                echo "refine-prompt: extra positional arg: $1" >&2
                exit 2
            fi
            ;;
    esac
    shift
done

if [ -n "$FROM_FILE" ] && [ -n "$PROMPT" ]; then
    echo "refine-prompt: --from-file and positional prompt are mutually exclusive" >&2
    exit 2
fi

if [ -n "$FROM_FILE" ]; then
    if ! check_path_allowed "$FROM_FILE" 2>/dev/null; then : ; fi
    # Path-allowlist check runs after function defs; defer to below.
    FROM_FILE_RESOLVED="$FROM_FILE"
fi

# ── path allowlist ────────────────────────────────────────────────
# Forbidden patterns: .env (exact basename or .env.*), *credential*, *secret*,
# *.pem, *.key, .git/, node_modules/.
#
# Path-security hardening (issue #680):
#   1. Resolve via `realpath -m` (or `readlink -f` fallback) to follow
#      symlinks and canonicalize `..` segments.
#   2. Check BOTH the literal input and the resolved target against the
#      forbidden patterns — a safe-looking symlink to .env must reject.
#   3. Reject post-canonicalization `..` segments (defense in depth).
_match_forbidden() {
    local p="$1"
    case "$p" in
        *.env|*.env.*|*/.env|.env) return 0 ;;
        *credential*|*Credential*|*CREDENTIAL*) return 0 ;;
        *secret*|*Secret*|*SECRET*) return 0 ;;
        *.pem|*.key) return 0 ;;
        */.git/*|.git/*|*/.git|.git) return 0 ;;
        */node_modules/*|node_modules/*|*/node_modules|node_modules) return 0 ;;
    esac
    return 1
}

_canonicalize() {
    local p="$1"
    local r=""
    # Try GNU realpath -m (handles non-existent paths).
    if command -v realpath >/dev/null 2>&1; then
        r="$(realpath -m "$p" 2>/dev/null)" || r=""
        if [ -n "$r" ]; then printf '%s' "$r"; return; fi
        # BSD realpath (macOS) — only resolves existing paths.
        r="$(realpath "$p" 2>/dev/null)" || r=""
        if [ -n "$r" ]; then printf '%s' "$r"; return; fi
    fi
    if command -v readlink >/dev/null 2>&1; then
        r="$(readlink -f "$p" 2>/dev/null)" || r=""
        if [ -n "$r" ]; then printf '%s' "$r"; return; fi
        # Single-level symlink fallback (BSD readlink).
        if [ -L "$p" ]; then
            local target
            target="$(readlink "$p" 2>/dev/null)" || target=""
            if [ -n "$target" ]; then
                case "$target" in
                    /*) printf '%s' "$target"; return ;;
                    *)  printf '%s/%s' "$(dirname "$p")" "$target"; return ;;
                esac
            fi
        fi
    fi
    # Python fallback.
    if command -v python3 >/dev/null 2>&1; then
        r="$(python3 -c 'import os,sys; print(os.path.realpath(sys.argv[1]))' "$p" 2>/dev/null)" || r=""
        if [ -n "$r" ]; then printf '%s' "$r"; return; fi
    fi
    printf '%s' "$p"
}

check_path_allowed() {
    local p="$1"
    # Literal pattern check first.
    if _match_forbidden "$p"; then return 1; fi
    # Resolve (follows symlinks, canonicalizes ..) and re-check.
    local resolved
    resolved="$(_canonicalize "$p")"
    if [ -n "$resolved" ] && [ "$resolved" != "$p" ]; then
        if _match_forbidden "$resolved"; then return 1; fi
    fi
    # Reject residual .. segments.
    case "$resolved" in
        *..*) return 1 ;;
    esac
    return 0
}

safe_read() {
    # Reads a file if allowed; otherwise emits the violation and exits 3.
    local p="$1"
    if ! check_path_allowed "$p"; then
        echo "code_health:refine_path_violation path=$p" >&2
        exit 3
    fi
    [ -f "$p" ] && cat "$p"
}

# Resolve from-file after function def.
if [ -n "$FROM_FILE" ]; then
    if ! check_path_allowed "$FROM_FILE"; then
        echo "code_health:refine_path_violation path=$FROM_FILE" >&2
        exit 3
    fi
    if [ ! -f "$FROM_FILE" ]; then
        echo "refine-prompt: --from-file not found: $FROM_FILE" >&2
        exit 2
    fi
    PROMPT="$(cat "$FROM_FILE")"
fi

if [ -z "$PROMPT" ]; then
    echo "refine-prompt: empty prompt (provide a positional arg or --from-file)" >&2
    exit 4
fi

# Extended allowlist (issue #680): artifact-dir and simulate-iterations dir
# must also pass the forbidden-path check.
if ! check_path_allowed "$ARTIFACT_DIR"; then
    echo "code_health:refine_path_violation path=$ARTIFACT_DIR" >&2
    exit 3
fi
if [ -n "$SIM_ITER_DIR" ]; then
    if ! check_path_allowed "$SIM_ITER_DIR"; then
        echo "code_health:refine_path_violation path=$SIM_ITER_DIR" >&2
        exit 3
    fi
fi

# ── slug helper (single definition — needed by --continue and the artifact step) ──
# slug_from_prompt: stable lowercase-dashed slug for artifact filenames.
# Whitespace (including newlines) collapses to single spaces before
# dashification so a multi-line prompt yields one single-line filename.
slug_from_prompt() {
    printf '%s' "$1" | tr '[:upper:]' '[:lower:]' \
        | tr -s '[:space:]' ' ' \
        | sed -E 's/[^a-z0-9]+/-/g; s/^-+//; s/-+$//' \
        | cut -c1-40
}

# ── continuous-iteration mode (--continue, issue #673) ────────────
#
# Wraps the single-pass refine + handoff loop. After each iteration the
# orchestrator reads the autospec run report (either DIR/iter-<N>-report.md
# from --simulate-iterations test hook, or .autospec/run-summary.md in real
# operation) and applies the harvest contract:
#
#   1. `## Next steps` / `## What to do next` / `## Remaining work` /
#      `## Open blockers` section (case-insensitive).
#   2. Fenced ```autospec-next or ```next-prompt blocks.
#   3. `STOP: <reason>` markers — terminates with evidence_based_stop.
#   4. Empty / "(none — converged)" / missing — convergence_clean.
#
# Termination:
#   convergence_clean | oscillation_detected | round_cap_reached |
#   evidence_based_stop | operator_stop | budget_cap_reached |
#   iteration_error (real handoff returned non-zero — issue #681).
#
# Real-mode vs simulated test mode:
#   --simulate-iterations DIR  — test-only path; per-iteration refine runs
#                                with --dry-run and reports are read from
#                                pre-staged DIR/iter-<N>-report.md fixtures.
#                                No real /autospec dispatch happens.
#   (default real mode)        — each iteration runs the FULL refine +
#                                handoff path; the report is harvested from
#                                $REPO_ROOT/.autospec/run-summary.md and
#                                handoff failure stops the loop with
#                                status=iteration_error.
#
# Per-iteration record + summary table per spec
# (docs/specs/2026-05-28-autospec-refine-design.md §Continuous-iteration mode).

# Shared matcher library (issue #707). Sourced once so harvest_next_prompt
# can invoke the same matchers as scripts/extract-conversational-recommendation.sh.
_REFINE_MATCHER_LIB="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib/extract-matchers.sh"
if [ -f "$_REFINE_MATCHER_LIB" ]; then
    # shellcheck source=lib/extract-matchers.sh
    . "$_REFINE_MATCHER_LIB"
fi

# Shared loop driver (issue #708). Single source of truth for the
# continuous-iteration loop used by /autospec-refine --continue,
# /autospec-continue, and /autospec --loop. When the lib is available,
# run_continue_loop below delegates to autospec_loop_run.
_AUTOSPEC_LOOP_LIB="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib/autospec-loop.sh"
if [ -f "$_AUTOSPEC_LOOP_LIB" ]; then
    # shellcheck source=lib/autospec-loop.sh
    . "$_AUTOSPEC_LOOP_LIB"
fi

harvest_next_prompt() {
    local report="$1"
    [ -f "$report" ] || { printf ''; return 0; }
    # 1. STOP marker takes precedence.
    if grep -qE '^STOP:[[:space:]]' "$report"; then
        local reason
        reason="$(grep -E '^STOP:[[:space:]]' "$report" | head -1 | sed -E 's/^STOP:[[:space:]]*//')"
        printf 'STOP::%s' "$reason"
        return 0
    fi
    # 2. Header section harvest.
    local section
    section="$(awk '
        BEGIN{IGNORECASE=1; inblock=0}
        /^##[[:space:]]+(Next steps|What to do next|Remaining work|Open blockers)/ {inblock=1; next}
        inblock && /^##[[:space:]]/ {inblock=0}
        inblock {print}
    ' "$report")"
    if [ -n "$section" ]; then
        # Detect explicit convergence phrases.
        if printf '%s' "$section" | grep -qiE '^\s*-\s*\(?none\b|no further work|^\s*done\s*$|converged'; then
            # Convergence sentinel: empty harvest.
            printf ''
            return 0
        fi
        # Take the first non-empty bullet line as the canonical prompt.
        local first_bullet
        first_bullet="$(printf '%s\n' "$section" | grep -E '^\s*[-*]\s+' | head -1 | sed -E 's/^\s*[-*]\s+//')"
        if [ -n "$first_bullet" ]; then
            printf '%s' "$first_bullet"
            return 0
        fi
        # Fallback: first non-blank line of section.
        local first_line
        first_line="$(printf '%s\n' "$section" | grep -v '^\s*$' | head -1)"
        printf '%s' "$first_line"
        return 0
    fi
    # 3. Fenced autospec-next / next-prompt block.
    local fenced
    fenced="$(awk '
        BEGIN{infence=0}
        /^```(autospec-next|next-prompt)/ {infence=1; next}
        infence && /^```/ {infence=0; exit}
        infence {print}
    ' "$report")"
    if [ -n "$fenced" ]; then
        printf '%s' "$fenced" | head -1
        return 0
    fi
    # 3.5 Next-prefix / continuation prefixes (issue #707).
    # Recognises "Next best slice:", "Next step:", "Continue with:",
    # "Proceed with:", "Move on to:", "Then:", "Up next:", "Suggested next:",
    # etc. Extracts the matched line + following paragraph, then strips the
    # leading prefix so the harvested prompt is the actionable body.
    if declare -F extract_next_prefix_continuations >/dev/null 2>&1; then
        local next_pref
        MSG="$(cat "$report")" next_pref="$(MSG="$(cat "$report")" extract_next_prefix_continuations)"
        if [ -n "$next_pref" ]; then
            # Use the first line; strip the leading "Next best slice:" / etc.
            local first_line
            first_line="$(printf '%s\n' "$next_pref" | head -1)"
            # Case-insensitive strip of recognised prefixes.
            local stripped
            stripped="$(printf '%s' "$first_line" | sed -E 's/^[[:space:]]*([Nn]ext best slice|[Nn]ext best step|[Nn]ext slice|[Nn]ext step|[Cc]ontinue with|[Pp]roceed with|[Mm]ove on to|[Mm]ove to|[Uu]p next is|[Uu]p next|[Ss]uggested next|[Tt]hen|[Nn]ext):[[:space:]]*//')"
            if [ -n "$stripped" ]; then
                printf '%s' "$stripped"
            else
                printf '%s' "$first_line"
            fi
            return 0
        fi
    fi
    # 4. Nothing found — convergence.
    printf ''
}

run_continue_loop() {
    # Delegate to the shared driver (issue #708) when available. The shared
    # driver is identical to the original implementation below — kept inline
    # only as a safety net if the lib failed to source.
    if declare -F autospec_loop_run >/dev/null 2>&1; then
        SCRIPT_PATH="$0" autospec_loop_run
        return $?
    fi
    local loop_slug
    loop_slug="$(slug_from_prompt "$PROMPT")"
    [ -n "$loop_slug" ] || loop_slug="prompt"
    mkdir -p "$ARTIFACT_DIR"
    local loop_json="$ARTIFACT_DIR/${loop_slug}-loop.json"
    local loop_md="$ARTIFACT_DIR/${loop_slug}-loop-summary.md"
    local start_ts
    start_ts="$(date +%s)"
    local cur_prompt="$PROMPT"
    local cur_source="(operator input)"
    local iter=0
    local status=""
    local prev_hash=""
    local iter_records="["
    local table_rows=""
    local first=1
    local tokens_used=0

    while [ "$iter" -lt "$MAX_ITERATIONS" ]; do
        iter=$((iter + 1))

        # Operator escape — checked at iteration boundary.
        if [ -f "${HOME}/.autospec/refine-loop-stop.flag" ] \
            || [ -f "${HOME}/.autospec/stop.flag" ]; then
            status="operator_stop"
            break
        fi

        # Run refine on current prompt — inline by re-invoking the same script
        # WITHOUT --continue so the single-pass path executes. In real
        # operation we invoke the FULL handoff path (refine + dispatch to
        # /autospec) and harvest the resulting .autospec/run-summary.md.
        # The --simulate-iterations test hook short-circuits to --dry-run and
        # reads pre-staged iter-<N>-report.md fixtures.
        local iter_artifact_subdir="$ARTIFACT_DIR/iter-${iter}"
        mkdir -p "$iter_artifact_subdir"
        local refine_log="$iter_artifact_subdir/refine.log"
        local refine_status=0

        # Staleness guard (issue #692 fix 2): capture mtime of each pre-existing
        # run-summary.md and move it aside so this iteration must produce a
        # fresh, non-empty file with newer mtime. Real-mode only.
        local run_summary="$REPO_ROOT/.autospec/run-summary.md"
        local mtime_before=0
        if [ -z "$SIM_ITER_DIR" ] && [ -f "$run_summary" ]; then
            mtime_before="$(stat -c%Y "$run_summary" 2>/dev/null || stat -f%m "$run_summary" 2>/dev/null || echo 0)"
            mv "$run_summary" "$run_summary.prev-iter${iter}" 2>/dev/null || true
        fi

        if [ -n "$SIM_ITER_DIR" ]; then
            # Test-only path: dry-run, no real /autospec dispatch.
            bash "$0" "$cur_prompt" --rounds "$ROUNDS" --dry-run \
                --artifact-dir "$iter_artifact_subdir" \
                --repo-root "$REPO_ROOT" \
                --memory-root "$MEMORY_ROOT" \
                > "$refine_log" 2>&1 || refine_status=$?
        else
            # Real path: refine + handoff. handoff_exit_code in the artifact
            # reflects whether /autospec ran cleanly.
            bash "$0" "$cur_prompt" --rounds "$ROUNDS" \
                --artifact-dir "$iter_artifact_subdir" \
                --repo-root "$REPO_ROOT" \
                --memory-root "$MEMORY_ROOT" \
                > "$refine_log" 2>&1 || refine_status=$?
        fi

        local refinement_artifact
        refinement_artifact="$(ls "$iter_artifact_subdir"/*.json 2>/dev/null | head -1)"
        [ -n "$refinement_artifact" ] || refinement_artifact=""

        # If real handoff failed, stop the loop with iteration_error.
        if [ -z "$SIM_ITER_DIR" ] && [ "$refine_status" -ne 0 ]; then
            status="iteration_error"
            row_status="iteration_error"
            local row
            row="$(printf '| %4d | %-21s | %-60s | %10s | %4s | %-20s |' \
                "$iter" "$(printf '%s' "$cur_source" | head -c 21)" \
                "handoff failed rc=$refine_status" "0" "-" "iteration_error")"
            if [ -z "$table_rows" ]; then table_rows="$row"; else table_rows="$table_rows"$'\n'"$row"; fi
            break
        fi

        # Determine where the iteration report lives.
        local report_path=""
        if [ -n "$SIM_ITER_DIR" ]; then
            report_path="$SIM_ITER_DIR/iter-${iter}-report.md"
        else
            report_path="$REPO_ROOT/.autospec/run-summary.md"
        fi

        # Staleness post-check (issue #692 fix 2): in real mode, the handoff
        # must produce a fresh, non-empty run-summary.md with mtime > mtime_before.
        # Missing / empty / stale → iteration_error.
        if [ -z "$SIM_ITER_DIR" ]; then
            local stale_reason=""
            if [ ! -f "$report_path" ]; then
                stale_reason="run_summary_missing_after_handoff"
            elif [ ! -s "$report_path" ]; then
                stale_reason="run_summary_empty_after_handoff"
            else
                local mtime_after
                mtime_after="$(stat -c%Y "$report_path" 2>/dev/null || stat -f%m "$report_path" 2>/dev/null || echo 0)"
                if [ "$mtime_after" -le "$mtime_before" ] 2>/dev/null; then
                    stale_reason="run_summary_stale_after_handoff"
                fi
            fi
            if [ -n "$stale_reason" ]; then
                echo "code_health:$stale_reason path=$report_path" >&2
                status="iteration_error"
                local row
                row="$(printf '| %4d | %-21s | %-60s | %10s | %4s | %-20s |' \
                    "$iter" "$(printf '%s' "$cur_source" | head -c 21)" \
                    "$stale_reason" "0" "-" "iteration_error")"
                if [ -z "$table_rows" ]; then table_rows="$row"; else table_rows="$table_rows"$'\n'"$row"; fi
                break
            fi
        fi

        # Harvest next prompt from the report.
        local harvested
        harvested="$(harvest_next_prompt "$report_path")"

        # Token budget tracking (simulated for tests).
        if [ -n "$SIM_TOKENS" ]; then
            tokens_used=$((tokens_used + SIM_TOKENS))
        fi

        # Build per-iteration JSON record.
        local stop_reason="null"
        local row_status="next-steps found"
        local row_harvested="${harvested:-(empty)}"
        local row_harvested_short
        row_harvested_short="$(printf '%s' "$row_harvested" | head -c 60)"

        # Evidence-based stop check.
        if [ -n "$harvested" ] && [ "${harvested#STOP::}" != "$harvested" ]; then
            stop_reason="\"$(printf '%s' "${harvested#STOP::}" | python3 -c 'import json,sys; sys.stdout.write(json.dumps(sys.stdin.read())[1:-1])')\""
            row_status="evidence_based_stop"
            row_harvested_short="STOP: ${harvested#STOP::}"
        fi

        # Convergence — empty harvested.
        local converged=0
        if [ -z "$harvested" ]; then
            converged=1
            row_status="convergence_clean"
        fi

        # Oscillation — hash matches previous iteration's harvested prompt.
        local oscillation=0
        local cur_hash=""
        if [ -n "$harvested" ] && [ "${harvested#STOP::}" = "$harvested" ]; then
            cur_hash="$(printf '%s' "$harvested" | shasum -a 256 | awk '{print $1}')"
            if [ -n "$prev_hash" ] && [ "$cur_hash" = "$prev_hash" ]; then
                oscillation=1
                row_status="oscillation_detected"
            fi
        fi

        # Append iteration record.
        local harvested_json
        harvested_json="$(printf '%s' "${harvested:-}" | python3 -c 'import json,sys; sys.stdout.write(json.dumps(sys.stdin.read()))')"
        local source_json
        source_json="$(printf '%s' "$cur_source" | python3 -c 'import json,sys; sys.stdout.write(json.dumps(sys.stdin.read()))')"
        local refart_json
        refart_json="$(printf '%s' "$refinement_artifact" | python3 -c 'import json,sys; sys.stdout.write(json.dumps(sys.stdin.read()))')"
        local report_json
        report_json="$(printf '%s' "$report_path" | python3 -c 'import json,sys; sys.stdout.write(json.dumps(sys.stdin.read()))')"
        local record
        record=$(cat <<EOF
{"iteration":$iter,"harvested_from_report":$report_json,"source":$source_json,"harvested_prompt":$harvested_json,"refinement_artifact":$refart_json,"handoff_pr_count":0,"handoff_pr_numbers":[],"stop_reason":$stop_reason,"status":"$row_status"}
EOF
)
        if [ "$first" = 1 ]; then
            iter_records="$iter_records$record"
            first=0
        else
            iter_records="$iter_records,$record"
        fi

        # Build table row.
        local row
        row="$(printf '| %4d | %-21s | %-60s | %10s | %4s | %-20s |' \
            "$iter" "$(printf '%s' "$cur_source" | head -c 21)" \
            "$row_harvested_short" "0" "-" "$row_status")"
        if [ -z "$table_rows" ]; then
            table_rows="$row"
        else
            table_rows="$table_rows"$'\n'"$row"
        fi

        # Termination decisions (in priority order).
        if [ "$row_status" = "evidence_based_stop" ]; then
            status="evidence_based_stop"
            break
        fi
        if [ "$converged" = 1 ]; then
            status="convergence_clean"
            break
        fi
        if [ "$oscillation" = 1 ]; then
            status="oscillation_detected"
            break
        fi

        # Budget caps.
        if [ "$tokens_used" -gt "$TOKEN_CAP" ] 2>/dev/null; then
            status="budget_cap_reached"
            break
        fi
        local now
        now="$(date +%s)"
        if [ $((now - start_ts)) -gt "$TIME_CAP" ]; then
            status="budget_cap_reached"
            break
        fi

        # Continue: harvested becomes next iteration's prompt.
        prev_hash="$cur_hash"
        cur_prompt="$harvested"
        cur_source="$report_path"
    done

    iter_records="$iter_records]"

    if [ -z "$status" ]; then
        status="round_cap_reached"
    fi

    # Write JSON record.
    cat > "$loop_json" <<EOF
{
  "slug": "$loop_slug",
  "status": "$status",
  "iterations_executed": $iter,
  "max_iterations": $MAX_ITERATIONS,
  "tokens_used": $tokens_used,
  "iterations": $iter_records
}
EOF

    # Validate loop JSON against loop schema (issue #682). Non-fatal warn
    # if jsonschema is unavailable; fatal if validation actually fails.
    _loop_schema="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/schemas/autospec-refinement-loop.schema.json"
    if [ -f "$_loop_schema" ] && command -v python3 >/dev/null 2>&1; then
        python3 - "$loop_json" "$_loop_schema" <<'PY' || {
import json, sys
try:
    import jsonschema
except ImportError:
    sys.exit(0)
with open(sys.argv[1]) as f: doc = json.load(f)
with open(sys.argv[2]) as f: sch = json.load(f)
try:
    jsonschema.validate(doc, sch)
except jsonschema.ValidationError as e:
    sys.stderr.write(f"refine-prompt: loop schema validation failed: {e.message}\n")
    sys.exit(1)
PY
            echo "refine-prompt: WARN — loop artifact failed schema validation: $loop_json" >&2
        }
    fi

    # Write markdown summary.
    {
        printf '## /autospec-refine continuous loop summary\n\n'
        printf '| Iter | Harvested from        | Refined prompt (first 60 chars)                              | PRs merged | Time | Status               |\n'
        printf '|------|-----------------------|--------------------------------------------------------------|-----------:|------|----------------------|\n'
        printf '%s\n' "$table_rows"
        printf '\nFinal status: %s\n' "$status"
        printf 'Iterations executed: %s / %s\n' "$iter" "$MAX_ITERATIONS"
    } > "$loop_md"

    printf '%s\n' "## /autospec-refine continuous loop summary"
    printf '%s\n' "Final status: $status (iterations=$iter, artifact=$loop_json)"
}

if [ "$CONTINUE_MODE" = 1 ]; then
    run_continue_loop
    exit 0
fi

# ── lens registry ─────────────────────────────────────────────────
DEFAULT_LENSES="repo-grounding,clarity-ac,sizing,adversarial"
if [ -z "$LENSES_RAW" ]; then
    LENSES_RAW="$DEFAULT_LENSES"
fi

# Validate lens names.
KNOWN_LENSES="repo-grounding clarity-ac sizing adversarial"
IFS=',' read -r -a REQUESTED_LENSES <<< "$LENSES_RAW"
for l in "${REQUESTED_LENSES[@]}"; do
    case " $KNOWN_LENSES " in
        *" $l "*) ;;
        *) echo "refine-prompt: unknown lens: $l" >&2; exit 2 ;;
    esac
done

# ── round-cap enforcement ─────────────────────────────────────────
ROUNDS_REQUESTED="$ROUNDS"
CAPPED=0
if [ "$ROUNDS" -gt "$MAX_ROUNDS" ] 2>/dev/null; then
    ROUNDS="$MAX_ROUNDS"
    CAPPED=1
fi
if [ "$ROUNDS" -lt 1 ] 2>/dev/null; then
    echo "refine-prompt: --rounds must be >= 1" >&2
    exit 2
fi

# ── repo context loader (with allowlist) ──────────────────────────
CONTEXT_SPARSE=true
SOURCES_USED=()

load_agents_md() {
    local p="$REPO_ROOT/AGENTS.md"
    if check_path_allowed "$p" && [ -f "$p" ]; then
        SOURCES_USED+=("AGENTS.md")
        CONTEXT_SPARSE=false
        cat "$p"
    fi
}

load_recent_specs() {
    local dir="$REPO_ROOT/docs/specs"
    [ -d "$dir" ] || return 0
    local count=0
    # Last 30 days, cap 5. Fall back to mtime-sorted listing.
    while IFS= read -r f; do
        [ -n "$f" ] || continue
        check_path_allowed "$f" || continue
        SOURCES_USED+=("$(basename "$f")")
        CONTEXT_SPARSE=false
        count=$((count + 1))
        [ "$count" -ge 5 ] && break
    done < <(find "$dir" -name '*.md' -type f -mtime -30 2>/dev/null | sort -r | head -n 5)
}

load_git_log() {
    ( cd "$REPO_ROOT" && git log --since='7 days ago' --oneline 2>/dev/null | head -n 20 ) || true
}

load_memory_feedback() {
    # ~/.autospec/projects/*/memory/feedback_*.md, keyword-match prompt tokens.
    [ -d "$MEMORY_ROOT" ] || return 0
    local count=0
    while IFS= read -r f; do
        [ -n "$f" ] || continue
        check_path_allowed "$f" || continue
        # Keyword match: each whitespace token from prompt ≥4 chars present in file.
        local matched=0
        for tok in $PROMPT; do
            [ "${#tok}" -ge 4 ] || continue
            if grep -qi "$tok" "$f" 2>/dev/null; then
                matched=1
                break
            fi
        done
        if [ "$matched" = 1 ]; then
            SOURCES_USED+=("memory:$(basename "$f")")
            CONTEXT_SPARSE=false
            count=$((count + 1))
            [ "$count" -ge 5 ] && break
        fi
    done < <(find "$MEMORY_ROOT" -name 'feedback_*.md' -type f 2>/dev/null)
}

AGENTS_CONTENT="$(load_agents_md)"
load_recent_specs   # populates SOURCES_USED
GIT_LOG_CONTENT="$(load_git_log)"
load_memory_feedback

# ── lens implementations (deterministic v1) ───────────────────────
# Each lens takes the previous prompt on stdin and emits the refined prompt on
# stdout. They MUST apply a named, measurable change so bats can assert.

lens_repo_grounding() {
    # Inject concrete file paths and conventions discovered in AGENTS.md / specs.
    local prev
    prev="$(cat)"
    local appended=""
    appended+=$'\n\n## Repo grounding (autospec-refine repo-grounding lens)\n'
    if [ -n "$AGENTS_CONTENT" ]; then
        appended+=$'- AGENTS.md present — follow lockstep, TDD, conventional commits, sizing caps.\n'
        # Extract up to 5 file path-like tokens from AGENTS.md.
        local paths
        paths="$(printf '%s\n' "$AGENTS_CONTENT" | grep -oE '[a-zA-Z0-9_./-]+\.(sh|md|json|yml|yaml|py|js|ts)' | sort -u | head -n 5 || true)"
        if [ -n "$paths" ]; then
            appended+=$'- Project-specific paths to ground against:\n'
            while IFS= read -r p; do
                [ -n "$p" ] || continue
                appended+="  - \`$p\`"$'\n'
            done <<< "$paths"
        fi
    fi
    if [ -n "$GIT_LOG_CONTENT" ]; then
        appended+=$'- Recently-changed scope (last 7 days):\n'
        while IFS= read -r line; do
            [ -n "$line" ] || continue
            appended+="  - $line"$'\n'
        done <<< "$(printf '%s\n' "$GIT_LOG_CONTENT" | head -n 3)"
    fi
    if [ "${#SOURCES_USED[@]}" -gt 0 ]; then
        appended+=$'- Sources consulted: '"${SOURCES_USED[*]}"$'\n'
    fi
    printf '%s%s' "$prev" "$appended"
}

lens_clarity_ac() {
    # Disambiguate hedging language and emit explicit AC checkbox list.
    local prev
    prev="$(cat)"
    # Disambiguate hedges in-place.
    local body="$prev"
    body="$(printf '%s' "$body" | sed -E 's/should probably/MUST/g; s/might/MUST/g; s/could try/MUST/g; s/maybe/MUST/g')"
    body+=$'\n\n## Acceptance criteria (autospec-refine clarity-ac lens)\n'
    body+=$'- [ ] Implementation matches the disambiguated prompt above.\n'
    body+=$'- [ ] Tests cover happy path + at least one adversarial scenario.\n'
    body+=$'- [ ] `autospec validate` passes locally.\n'
    body+=$'- [ ] PR description names the test command operators should run.\n'
    printf '%s' "$body"
}

lens_sizing() {
    # Enforce small-LLM caps: warn if body > 400 words, suggest split.
    local prev
    prev="$(cat)"
    local wc
    wc=$(printf '%s' "$prev" | wc -w | tr -d ' ')
    local appended=""
    appended+=$'\n\n## Sizing (autospec-refine sizing lens)\n'
    appended+="- Current word count: $wc"$'\n'
    appended+=$'- Cap per child issue: 400 words / 3 files / 30 LOC outline.\n'
    if [ "$wc" -gt 400 ]; then
        appended+=$'- ACTION: split into parent + child sequence; this prompt exceeds the 400-word cap.\n'
    else
        appended+=$'- Within cap — no split required.\n'
    fi
    printf '%s%s' "$prev" "$appended"
}

lens_adversarial() {
    # Critical-question pass; add risk-driven test requirements.
    local prev
    prev="$(cat)"
    local appended=""
    appended+=$'\n\n## Adversarial review (autospec-refine adversarial lens)\n'
    appended+=$'- What happens on empty input? Add a test.\n'
    appended+=$'- What happens on malformed input? Add a test.\n'
    appended+=$'- What happens when the environment lacks the expected dependency? Fail loudly.\n'
    appended+=$'- What happens on partial network failure? Retry once, then surface.\n'
    appended+=$'- What forbidden paths could the change accidentally touch? Enforce allowlist.\n'
    printf '%s%s' "$prev" "$appended"
}

apply_lens() {
    local name="$1"
    case "$name" in
        repo-grounding)  lens_repo_grounding ;;
        clarity-ac)      lens_clarity_ac ;;
        sizing)          lens_sizing ;;
        adversarial)     lens_adversarial ;;
        *) echo "refine-prompt: unknown lens at apply: $name" >&2; exit 2 ;;
    esac
}

# ── LLM-driven lens dispatch (issue #684) ─────────────────────────
# Per-round lens routing flows through scripts/refine-prompt-lens-llm.sh when
# --lens-mode llm is active (or auto-detected). On LLM failure for a lens,
# falls back to the deterministic implementation for THAT lens and records
# `lens_implementation=deterministic` with `degraded_fallback=true` so the
# round JSON reflects the actual code path taken.
LENS_LLM_SH="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/refine-prompt-lens-llm.sh"

_resolve_lens_mode() {
    # Resolve the lens dispatch mode to one of: deterministic | llm | auto.
    #
    # Precedence (issue #1024): the --lens-mode flag wins over the
    # AUTOSPEC_REFINE_LENS_MODE env hatch, which wins over the default (auto).
    # All three values are allow-listed; an out-of-list value is fatal so a
    # typo never silently degrades to a different code path.
    #
    #   deterministic — legacy template lenses only; never dispatch the LLM
    #                   (offline / test / billable-API-free path).
    #   llm           — LLM-only; fail loudly if the dispatcher is unavailable.
    #   auto (default)— LLM-first: dispatch the LLM path each round; the
    #                   deterministic template lens is the fallback ONLY when
    #                   the LLM path is unavailable/fails, tagging
    #                   degraded_fallback=true on that round.
    #
    # Flag wins over env (flag-over-env). Flag is allow-listed.
    if [ -n "$LENS_MODE" ]; then
        case "$LENS_MODE" in
            deterministic|llm|auto) printf '%s' "$LENS_MODE"; return ;;
            *) echo "refine-prompt: --lens-mode must be deterministic|llm|auto" >&2; exit 2 ;;
        esac
    fi
    # Env hatch — allow-listed; an invalid value is fatal (allow-list).
    if [ -n "${AUTOSPEC_REFINE_LENS_MODE:-}" ]; then
        case "$AUTOSPEC_REFINE_LENS_MODE" in
            deterministic|llm|auto) printf '%s' "$AUTOSPEC_REFINE_LENS_MODE"; return ;;
            *) echo "refine-prompt: AUTOSPEC_REFINE_LENS_MODE must be deterministic|llm|auto (got: $AUTOSPEC_REFINE_LENS_MODE)" >&2; exit 2 ;;
        esac
    fi
    # Default: LLM-first auto.
    printf '%s' "auto"
}

# Availability probe — is an LLM dispatcher reachable for the lens path?
# Mirrors refine-prompt-lens-llm.sh's own resolution: an explicit
# --llm-binary, or a claude/codex on PATH. Used to decide llm-vs-fallback
# without spending a real dispatch.
_lens_llm_available() {
    if [ -n "$LLM_BINARY" ]; then return 0; fi
    if command -v claude >/dev/null 2>&1; then return 0; fi
    if command -v codex  >/dev/null 2>&1; then return 0; fi
    return 1
}

# Validate the allow-listed inputs in the MAIN shell (not the $() subshell
# below, where an exit would only terminate the subshell and leak an empty
# LENS_MODE_RESOLVED). Flag-over-env: the flag is validated first.
if [ -n "$LENS_MODE" ]; then
    case "$LENS_MODE" in
        deterministic|llm|auto) ;;
        *) echo "refine-prompt: --lens-mode must be deterministic|llm|auto" >&2; exit 2 ;;
    esac
elif [ -n "${AUTOSPEC_REFINE_LENS_MODE:-}" ]; then
    case "$AUTOSPEC_REFINE_LENS_MODE" in
        deterministic|llm|auto) ;;
        *) echo "refine-prompt: AUTOSPEC_REFINE_LENS_MODE must be deterministic|llm|auto (got: $AUTOSPEC_REFINE_LENS_MODE)" >&2; exit 2 ;;
    esac
fi

LENS_MODE_RESOLVED="$(_resolve_lens_mode)"

# llm mode is LLM-only: if no dispatcher is reachable, fail loudly rather than
# silently degrading to the deterministic template lens (issue #1024).
if [ "$LENS_MODE_RESOLVED" = "llm" ] && ! _lens_llm_available; then
    echo "refine-prompt: ERROR — lens-mode=llm requires an LLM dispatcher (claude/codex or --llm-binary); none available" >&2
    exit 2
fi

# auto mode is LLM-first with a deterministic fallback. When no dispatcher is
# reachable up front, collapse to the deterministic template lens but mark the
# fallback so every round's artifact carries degraded_fallback=true. The flag
# below is consulted by apply_lens_routed.
AUTO_DEGRADED_FALLBACK=0
if [ "$LENS_MODE_RESOLVED" = "auto" ] && ! _lens_llm_available; then
    AUTO_DEGRADED_FALLBACK=1
    echo "refine-prompt: WARN — lens-mode=auto: no LLM dispatcher available; using deterministic fallback (degraded_fallback=true)" >&2
fi
LAST_LENS_IMPL=""
LAST_DEGRADED_FALLBACK="false"

apply_lens_routed() {
    # Routes through the LLM dispatcher when LENS_MODE_RESOLVED=llm; falls back
    # to deterministic per-lens on dispatcher failure. Writes the impl marker
    # ("llm" or "deterministic") and degraded flag ("true"/"false") to
    # $LENS_IMPL_FILE so the (subshell-captured) caller can still read them
    # — bash $() runs in a subshell, so plain variable side-effects would be
    # lost otherwise.
    local name="$1"
    local input="$2"
    local impl="deterministic"
    local degraded="false"
    local impl_file="${LENS_IMPL_FILE:-}"

    # auto mode with no dispatcher available up front: deterministic fallback,
    # tagged degraded (issue #1024). No LLM dispatch is attempted.
    if [ "$LENS_MODE_RESOLVED" = "auto" ] && [ "${AUTO_DEGRADED_FALLBACK:-0}" = 1 ]; then
        impl="deterministic"
        degraded="true"
        if [ -n "$impl_file" ]; then printf '%s|%s' "$impl" "$degraded" > "$impl_file"; fi
        printf '%s' "$input" | apply_lens "$name"
        return 0
    fi

    # llm and auto both dispatch the LLM path first. In auto mode a per-lens
    # dispatch failure degrades to the deterministic template lens for THAT
    # lens (degraded_fallback=true). In llm mode the dispatcher is known
    # reachable (checked up front); a per-lens failure still degrades that
    # single lens rather than aborting the whole run.
    if { [ "$LENS_MODE_RESOLVED" = "llm" ] || [ "$LENS_MODE_RESOLVED" = "auto" ]; } && [ -x "$LENS_LLM_SH" ]; then
        # Build a tiny context file from already-loaded sources.
        local ctx
        ctx="$(mktemp -t refine-lens-ctx.XXXXXX)"
        {
            if [ -n "$AGENTS_CONTENT" ]; then
                printf '%s\n' "## AGENTS.md"
                printf '%s\n' "$AGENTS_CONTENT"
            fi
            if [ -n "$GIT_LOG_CONTENT" ]; then
                printf '%s\n' "## Recent git log"
                printf '%s\n' "$GIT_LOG_CONTENT"
            fi
        } > "$ctx" 2>/dev/null || true
        local llm_args=( --lens "$name" --prompt "$input" --context-file "$ctx" )
        if [ -n "$LLM_BINARY" ]; then
            llm_args+=( --llm-binary "$LLM_BINARY" )
        fi
        local llm_out
        local llm_rc=0
        llm_out="$(bash "$LENS_LLM_SH" "${llm_args[@]}" 2>/dev/null)" || llm_rc=$?
        rm -f "$ctx" 2>/dev/null || true
        if [ "$llm_rc" = 0 ] && [ -n "$llm_out" ]; then
            impl="llm"
            [ -n "$impl_file" ] && printf '%s|%s' "$impl" "$degraded" > "$impl_file"
            printf '%s' "$llm_out"
            return 0
        fi
        # Fallback for THIS lens.
        impl="deterministic"
        degraded="true"
        echo "refine-prompt: WARN — LLM lens '$name' failed; falling back to deterministic" >&2
    fi

    [ -n "$impl_file" ] && printf '%s|%s' "$impl" "$degraded" > "$impl_file"
    printf '%s' "$input" | apply_lens "$name"
}

# ── helpers ───────────────────────────────────────────────────────
word_count() { printf '%s' "$1" | wc -w | tr -d ' '; }

iso_ts() { date -u +'%Y-%m-%dT%H-%M-%SZ'; }

json_escape() {
    # Stream-safe JSON string escape for a bash string. Reads stdin.
    python3 -c 'import json,sys; sys.stdout.write(json.dumps(sys.stdin.read()))'
}

# ── main loop ─────────────────────────────────────────────────────
SLUG="$(slug_from_prompt "$PROMPT")"
[ -n "$SLUG" ] || SLUG="prompt"
TS="$(iso_ts)"
ARTIFACT="$ARTIFACT_DIR/${SLUG}-${TS}.json"
if ! mkdir -p "$ARTIFACT_DIR"; then
    echo "code_health:refine_artifact_write_failed path=$ARTIFACT" >&2
    exit 4
fi
if [ ! -w "$ARTIFACT_DIR" ]; then
    echo "code_health:refine_artifact_write_failed path=$ARTIFACT" >&2
    exit 4
fi

PREV_PROMPT="$PROMPT"
ROUNDS_JSON=""
STATUS="completed"
DEGRADED_ROUNDS=()
ROUNDS_EXECUTED=0
CONVERGED_EARLY=false

# If round cap triggered, status is round_cap_reached but we still run the
# capped number of rounds.
if [ "$CAPPED" = 1 ]; then
    STATUS="round_cap_reached"
fi

NUM_LENSES="${#REQUESTED_LENSES[@]}"

for ((i=1; i<=ROUNDS; i++)); do
    # Lens picker: if i <= NUM_LENSES, use REQUESTED_LENSES[i-1].
    # Otherwise repeat 'adversarial'.
    if [ "$i" -le "$NUM_LENSES" ]; then
        LENS="${REQUESTED_LENSES[$((i-1))]}"
    else
        LENS="adversarial"
    fi

    LENS_IMPL_FILE="$(mktemp -t refine-lens-impl.XXXXXX)"
    export LENS_IMPL_FILE
    REFINED="$(apply_lens_routed "$LENS" "$PREV_PROMPT")"
    ROUND_IMPL="deterministic"
    ROUND_DEGRADED="false"
    if [ -f "$LENS_IMPL_FILE" ]; then
        IFS='|' read -r ROUND_IMPL ROUND_DEGRADED < "$LENS_IMPL_FILE"
        rm -f "$LENS_IMPL_FILE"
    fi
    [ -n "$ROUND_IMPL" ] || ROUND_IMPL="deterministic"
    [ -n "$ROUND_DEGRADED" ] || ROUND_DEGRADED="false"

    PREV_WC="$(word_count "$PREV_PROMPT")"
    NEW_WC="$(word_count "$REFINED")"
    DELTA=$((NEW_WC - PREV_WC))

    # Degradation check (round N word count < 75% of round N-1).
    if [ "$PREV_WC" -gt 0 ] && [ "$i" -gt 1 ]; then
        # NEW_WC * 100 < 75 * PREV_WC  =>  NEW_WC * 4 < 3 * PREV_WC
        if [ $((NEW_WC * 4)) -lt $((PREV_WC * 3)) ]; then
            DEGRADED_ROUNDS+=("$i:$LENS")
        fi
    fi

    # Build round JSON object.
    REFINED_JSON="$(printf '%s' "$REFINED" | json_escape)"
    SOURCES_JSON="$(printf '%s\n' "${SOURCES_USED[@]:-}" | python3 -c 'import json,sys; arr=[l for l in sys.stdin.read().splitlines() if l]; sys.stdout.write(json.dumps(arr))')"
    ROUND_OBJ=$(cat <<EOF
{"round_number":$i,"lens":"$LENS","sources_used":$SOURCES_JSON,"refined_prompt":$REFINED_JSON,"diff_summary":"lens=$LENS applied","word_count_delta":$DELTA,"reasoning":"lens_impl=$ROUND_IMPL","lens_implementation":"$ROUND_IMPL","degraded_fallback":$ROUND_DEGRADED}
EOF
)
    if [ -n "$ROUNDS_JSON" ]; then
        ROUNDS_JSON="$ROUNDS_JSON,$ROUND_OBJ"
    else
        ROUNDS_JSON="$ROUND_OBJ"
    fi

    ROUNDS_EXECUTED="$i"

    # Convergence check (round N byte-identical to round N-1).
    if [ "$REFINED" = "$PREV_PROMPT" ]; then
        STATUS="converged"
        CONVERGED_EARLY=true
        break
    fi

    PREV_PROMPT="$REFINED"
done

FINAL_PROMPT="$PREV_PROMPT"

# Degraded status: when word-count-delta degradation was detected and no
# other terminating condition (converged / round_cap_reached) already set
# the status. Issue #682 — `degraded` was previously dead in the schema.
if [ "$STATUS" = "completed" ] && [ "${#DEGRADED_ROUNDS[@]}" -gt 0 ]; then
    STATUS="degraded"
fi

# ── write artifact ────────────────────────────────────────────────
ORIG_JSON="$(printf '%s' "$PROMPT" | json_escape)"
FINAL_JSON="$(printf '%s' "$FINAL_PROMPT" | json_escape)"
HEAD_SHA="$( ( cd "$REPO_ROOT" && git rev-parse HEAD 2>/dev/null ) || echo unknown )"

# Degraded rounds array.
DEGRADED_JSON="$(printf '%s\n' "${DEGRADED_ROUNDS[@]:-}" | python3 -c 'import json,sys; arr=[l for l in sys.stdin.read().splitlines() if l]; sys.stdout.write(json.dumps(arr))')"

HANDOFF_EXECUTED=false
HANDOFF_EXIT_CODE="null"
if [ "$DRY_RUN" = 1 ]; then
    HANDOFF_TARGET="dry-run"
elif [ "$HANDOFF_MODE" = "interactive" ]; then
    HANDOFF_TARGET="/autospec"
else
    HANDOFF_TARGET="/autospec --autonomous"
fi
# handoff_executed flips to true ONLY after a real handoff returns 0 — see
# the dispatch block below (issue #681 Finding 6). The artifact is written
# once here with the pessimistic default, then rewritten post-dispatch if
# the handoff succeeded.

CONTEXT_SPARSE_JSON="$CONTEXT_SPARSE"

write_artifact() {
    cat > "$ARTIFACT" <<EOF
{
  "original_prompt": $ORIG_JSON,
  "rounds": [$ROUNDS_JSON],
  "final_prompt": $FINAL_JSON,
  "status": "$STATUS",
  "metadata": {
    "head_sha": "$HEAD_SHA",
    "timestamp": "$TS",
    "rounds_requested": $ROUNDS_REQUESTED,
    "rounds_executed": $ROUNDS_EXECUTED,
    "converged_early": $CONVERGED_EARLY,
    "degraded_rounds": $DEGRADED_JSON,
    "context_sparse": $CONTEXT_SPARSE_JSON,
    "handoff_target": "$HANDOFF_TARGET",
    "handoff_executed": $HANDOFF_EXECUTED,
    "handoff_exit_code": $HANDOFF_EXIT_CODE
  }
}
EOF
}
write_artifact
if [ ! -s "$ARTIFACT" ]; then
    echo "code_health:refine_artifact_write_failed path=$ARTIFACT" >&2
    exit 4
fi

if [ -n "$OUTPUT" ]; then
    if ! check_path_allowed "$OUTPUT"; then
        echo "code_health:refine_path_violation path=$OUTPUT" >&2
        exit 3
    fi
    printf '%s' "$FINAL_PROMPT" > "$OUTPUT"
fi

# Produce the human-readable markdown sibling next to the JSON. The
# renderer reuses the JSON's timestamp so the .md and .json share a
# base path; missing the .md (#671 shipped the renderer but didn't wire
# it into the orchestrator's artifact step).
RENDER_SH="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/refine-render-overview.sh"
if [ -x "$RENDER_SH" ]; then
    "$RENDER_SH" --json "$ARTIFACT" --slug "$SLUG" \
        --output-dir "$ARTIFACT_DIR" >/dev/null 2>&1 || \
        echo "refine-prompt: WARN — overview render failed for $ARTIFACT" >&2
fi
if [ ! -s "$ARTIFACT" ]; then
    echo "code_health:refine_artifact_write_failed path=$ARTIFACT" >&2
    exit 4
fi

echo "refine-prompt: status=$STATUS rounds_executed=$ROUNDS_EXECUTED artifact=$ARTIFACT"

# ── handoff dispatch ──────────────────────────────────────────────
# --dry-run skips handoff. Otherwise resolve the autospec entry point via
# `claude` (slash-command dispatcher) if available, falling back to the
# `autospec` binary on PATH. Test harnesses stub both.
#
# Path-safety (issue #681 Finding 6): reject each resolved dispatcher that
# lives under /tmp/ or the operator's home tmpdir unless explicitly
# overridden via AUTOSPEC_HANDOFF_DISPATCHER=1. This blocks the common
# PATH-stub attack where a writable temp directory shadows a system binary.
#
# Bookkeeping (issue #681 Finding 6): handoff_executed=true ONLY after the
# binary returns 0; capture exit code as metadata.handoff_exit_code; drop
# `|| true` so failures propagate. Re-write the artifact at the end.

_handoff_dispatcher_safe() {
    local resolved="$1"
    [ -n "$resolved" ] || return 1
    if [ -n "${AUTOSPEC_HANDOFF_DISPATCHER:-}" ]; then
        # Even under override, reject relative paths — they're ambiguous and
        # can be hijacked by cwd shadowing.
        case "$resolved" in /*) return 0 ;; *) return 1 ;; esac
    fi
    # Reject relative paths outright (issue #692).
    case "$resolved" in
        /*) ;;
        *) return 1 ;;
    esac
    # Check raw path against tmpdir denylist.
    case "$resolved" in
        /tmp/*|/private/tmp/*|/var/tmp/*|/var/folders/*) return 1 ;;
    esac
    if [ -n "${TMPDIR:-}" ]; then
        case "$resolved" in
            "$TMPDIR"*|"${TMPDIR%/}"/*) return 1 ;;
        esac
    fi
    # Canonicalize (follows symlinks) and re-check — symlink at safe path
    # pointing into tmpdir must be rejected (issue #692).
    local canon
    canon="$(_canonicalize "$resolved")"
    if [ -n "$canon" ] && [ "$canon" != "$resolved" ]; then
        case "$canon" in
            /tmp/*|/private/tmp/*|/var/tmp/*|/var/folders/*) return 1 ;;
        esac
        if [ -n "${TMPDIR:-}" ]; then
            case "$canon" in
                "$TMPDIR"*|"${TMPDIR%/}"/*) return 1 ;;
            esac
        fi
    fi
    return 0
}

# Harness-aware loop dispatcher (issue #723). Source the shared helper so
# Claude Code / Codex CLI / OpenCode each get their canonical invocation
# form. Legacy `autospec` binary fallback is preserved when the helper can
# detect neither the AI harness nor a per-harness binary on PATH.
_AUTOSPEC_HARNESS_DETECT_LIB="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib/autospec-harness-detect.sh"
if [ -f "$_AUTOSPEC_HARNESS_DETECT_LIB" ]; then
    # shellcheck source=lib/autospec-harness-detect.sh
    . "$_AUTOSPEC_HARNESS_DETECT_LIB"
fi

run_handoff() {
    local rc=0
    if declare -F autospec_harness_resolve_dispatcher >/dev/null 2>&1; then
        # Detect harness first; only fall back to legacy `autospec` binary if
        # no harness is detected AND no harness binary is on PATH.
        local kind=""
        if declare -F autospec_harness_detect >/dev/null 2>&1; then
            kind="$(autospec_harness_detect 2>/dev/null || true)"
        fi
        # Try harness-aware resolve; on exit 3 (no dispatcher), fall through
        # to autospec legacy binary. On exit 5 (path-safety refused), surface
        # the refusal directly so callers see the rc=5 contract (issue #692).
        local resolve_rc=0
        local resolve_err
        resolve_err="$(mktemp -t refine-harness-resolve.XXXXXX)"
        ( autospec_harness_resolve_dispatcher ) >/dev/null 2>"$resolve_err" || resolve_rc=$?
        if [ "$resolve_rc" = 0 ]; then
            autospec_harness_resolve_dispatcher
            rm -f "$resolve_err"
            echo "refine-prompt: handoff harness=$AUTOSPEC_HARNESS_KIND dispatcher=$AUTOSPEC_HARNESS_DISPATCHER" >&2
            autospec_harness_invoke "$HANDOFF_MODE" "$FINAL_PROMPT"
            return $?
        fi
        if [ "$resolve_rc" = 5 ]; then
            cat "$resolve_err" >&2
            rm -f "$resolve_err"
            return 5
        fi
        rm -f "$resolve_err"
        # Resolve failed with rc=3 (code_health:loop_handoff_no_dispatcher_for_harness).
        # Try the legacy `autospec` binary fallback before surfacing the error.
        if command -v autospec >/dev/null 2>&1; then
            local dispatcher
            dispatcher="$(command -v autospec)"
            if ! _handoff_dispatcher_safe "$dispatcher"; then
                echo "refine-prompt: ERROR — refusing handoff: dispatcher in tmpdir: $dispatcher (set AUTOSPEC_HANDOFF_DISPATCHER=1 to override)" >&2
                return 5
            fi
            echo "refine-prompt: handoff dispatcher=$dispatcher (legacy autospec binary)" >&2
            if [ "$HANDOFF_MODE" = "interactive" ]; then
                "$dispatcher" "$FINAL_PROMPT"
                rc=$?
            else
                "$dispatcher" --autonomous "$FINAL_PROMPT"
                rc=$?
            fi
            return $rc
        fi
        echo "refine-prompt: WARN — no handoff dispatcher for harness=$kind; artifact retained" >&2
        return 127
    fi
    # Helper missing — preserve original behavior.
    echo "refine-prompt: WARN — no handoff dispatcher (claude/autospec) on PATH; artifact retained" >&2
    return 127
}

if [ "$DRY_RUN" != 1 ]; then
    set +e
    run_handoff
    HANDOFF_RC=$?
    set -e
    HANDOFF_EXIT_CODE="$HANDOFF_RC"
    if [ "$HANDOFF_RC" = "0" ]; then
        HANDOFF_EXECUTED=true
    else
        HANDOFF_EXECUTED=false
        STATUS="handoff_failed"
    fi
    write_artifact
    if [ "$HANDOFF_RC" != "0" ] && [ "$HANDOFF_RC" != "127" ]; then
        # 127 = no dispatcher on PATH — preserve legacy behavior (warn,
        # exit 0). Every other non-zero is a real failure and must propagate.
        echo "refine-prompt: handoff failed rc=$HANDOFF_RC" >&2
        exit "$HANDOFF_RC"
    fi
fi

exit 0
