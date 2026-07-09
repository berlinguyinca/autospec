#!/usr/bin/env bash
# scripts/lib/autospec-loop.sh — shared continuous-iteration loop driver
# (issue #708).
#
# Single source of truth for the autospec continuous-iteration loop. Sourced by:
#   - scripts/refine-prompt.sh::run_continue_loop()  (/autospec-refine --continue)
#   - scripts/autospec-continue.sh                   (/autospec-continue --loop)
#   - skills/autospec (/autospec --loop)             via the same callers
#
# Six termination conditions (priority order):
#   1. evidence_based_stop  — STOP: <reason> marker in iteration report
#   2. convergence_clean    — harvest returns empty / (none — converged)
#   3. oscillation_detected — iter N+1 harvested-prompt hash == iter N
#   4. operator_stop        — ~/.autospec/stop.flag or refine-loop-stop.flag present
#   5. budget_cap_reached   — AUTOSPEC_LOOP_TOKEN_CAP or _TIME_CAP exceeded
#   6. round_cap_reached    — --max-iterations cap hit without other termination
#
# Aliases AUTOSPEC_REFINE_LOOP_* env vars to AUTOSPEC_LOOP_* (the latter is the
# canonical name introduced in #708; the former is preserved so PR #678/#693
# refine-continue tests keep passing).
#
# Public entry point: autospec_loop_run
#
# Required globals set by caller before invocation:
#   PROMPT, ARTIFACT_DIR, REPO_ROOT, MEMORY_ROOT, MAX_ITERATIONS, ROUNDS,
#   SIM_ITER_DIR (optional test hook), SIM_TOKENS (optional), TOKEN_CAP,
#   TIME_CAP, SCRIPT_PATH (path to refine-prompt.sh for per-iter dispatch)

# Guard against double-sourcing.
if [ -n "${_AUTOSPEC_LOOP_LIB_LOADED:-}" ]; then return 0 2>/dev/null || true; fi
_AUTOSPEC_LOOP_LIB_LOADED=1

# Source the shared matcher library if not already loaded.
_AUTOSPEC_LOOP_LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if ! declare -F extract_next_prefix_continuations >/dev/null 2>&1; then
    if [ -f "$_AUTOSPEC_LOOP_LIB_DIR/extract-matchers.sh" ]; then
        # shellcheck source=extract-matchers.sh
        . "$_AUTOSPEC_LOOP_LIB_DIR/extract-matchers.sh"
    fi
fi

# slug-from-prompt: produce a stable lowercase-dashed slug suitable for
# artifact filenames.
autospec_loop_slug_from_prompt() {
    printf '%s' "$1" | tr '[:upper:]' '[:lower:]' \
        | sed -E 's/[^a-z0-9]+/-/g; s/^-+//; s/-+$//' \
        | cut -c1-40
}

# harvest_next_prompt: read an iteration report and emit the next prompt to
# feed into the loop. Empty stdout means "converged"; "STOP::<reason>"
# means evidence-based stop.
autospec_loop_harvest_next_prompt() {
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
        if printf '%s' "$section" | grep -qiE '^\s*-\s*\(?none\b|no further work|^\s*done\s*$|converged'; then
            printf ''
            return 0
        fi
        local first_bullet
        first_bullet="$(printf '%s\n' "$section" | grep -E '^\s*[-*]\s+' | head -1 | sed -E 's/^\s*[-*]\s+//')"
        if [ -n "$first_bullet" ]; then
            printf '%s' "$first_bullet"
            return 0
        fi
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
    if declare -F extract_next_prefix_continuations >/dev/null 2>&1; then
        local next_pref
        MSG="$(cat "$report")" next_pref="$(MSG="$(cat "$report")" extract_next_prefix_continuations)"
        if [ -n "$next_pref" ]; then
            local first_line
            first_line="$(printf '%s\n' "$next_pref" | head -1)"
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

# autospec_loop_run: the canonical multi-iteration driver. Reads globals
# (PROMPT, ARTIFACT_DIR, REPO_ROOT, MEMORY_ROOT, MAX_ITERATIONS, ROUNDS,
# SIM_ITER_DIR, SIM_TOKENS, TOKEN_CAP, TIME_CAP, SCRIPT_PATH) and writes:
#   - $ARTIFACT_DIR/<slug>-loop.json
#   - $ARTIFACT_DIR/<slug>-loop-summary.md
# Also writes a copy of the summary to .autospec/loop-summary.md (when not
# in simulate mode) so /autospec --loop has a canonical output location.
autospec_loop_run() {
    local loop_slug
    loop_slug="$(autospec_loop_slug_from_prompt "$PROMPT")"
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
    local row_status=""

    # Resolve caps with both canonical and legacy env names.
    local _token_cap="${TOKEN_CAP:-${AUTOSPEC_LOOP_TOKEN_CAP:-${AUTOSPEC_REFINE_LOOP_TOKEN_CAP:-2000000}}}"
    local _time_cap="${TIME_CAP:-${AUTOSPEC_LOOP_TIME_CAP:-${AUTOSPEC_REFINE_LOOP_TIME_CAP:-21600}}}"
    local _max_iter="${MAX_ITERATIONS:-${AUTOSPEC_LOOP_MAX_ITERATIONS:-${AUTOSPEC_REFINE_LOOP_MAX_ITERATIONS:-5}}}"
    local _script_path="${SCRIPT_PATH:-$0}"

    while [ "$iter" -lt "$_max_iter" ]; do
        iter=$((iter + 1))

        # Operator escape — checked at iteration boundary.
        if [ -f "${HOME}/.autospec/refine-loop-stop.flag" ] \
            || [ -f "${HOME}/.autospec/stop.flag" ]; then
            status="operator_stop"
            break
        fi

        local iter_artifact_subdir="$ARTIFACT_DIR/iter-${iter}"
        mkdir -p "$iter_artifact_subdir"
        local refine_log="$iter_artifact_subdir/refine.log"
        local refine_status=0

        # Staleness guard: capture mtime of any pre-existing run-summary.md
        # and move it aside so this iteration must produce a fresh, non-empty
        # file with newer mtime. Real-mode only.
        local run_summary="$REPO_ROOT/.autospec/run-summary.md"
        local mtime_before=0
        if [ -z "${SIM_ITER_DIR:-}" ] && [ -f "$run_summary" ]; then
            mtime_before="$(stat -c%Y "$run_summary" 2>/dev/null || stat -f%m "$run_summary" 2>/dev/null || echo 0)"
            mv "$run_summary" "$run_summary.prev-iter${iter}" 2>/dev/null || true
        fi

        if [ -n "${SIM_ITER_DIR:-}" ]; then
            bash "$_script_path" "$cur_prompt" --rounds "${ROUNDS:-3}" --dry-run \
                --artifact-dir "$iter_artifact_subdir" \
                --repo-root "$REPO_ROOT" \
                --memory-root "$MEMORY_ROOT" \
                > "$refine_log" 2>&1 || refine_status=$?
        else
            bash "$_script_path" "$cur_prompt" --rounds "${ROUNDS:-3}" \
                --artifact-dir "$iter_artifact_subdir" \
                --repo-root "$REPO_ROOT" \
                --memory-root "$MEMORY_ROOT" \
                > "$refine_log" 2>&1 || refine_status=$?
        fi

        local refinement_artifact
        refinement_artifact="$(ls "$iter_artifact_subdir"/*.json 2>/dev/null | head -1)"
        [ -n "$refinement_artifact" ] || refinement_artifact=""

        if [ -z "${SIM_ITER_DIR:-}" ] && [ "$refine_status" -ne 0 ]; then
            status="iteration_error"
            row_status="iteration_error"
            local row
            row="$(printf '| %4d | %-21s | %-60s | %10s | %4s | %-20s |' \
                "$iter" "$(printf '%s' "$cur_source" | head -c 21)" \
                "handoff failed rc=$refine_status" "0" "-" "iteration_error")"
            if [ -z "$table_rows" ]; then table_rows="$row"; else table_rows="$table_rows"$'\n'"$row"; fi
            break
        fi

        local report_path=""
        if [ -n "${SIM_ITER_DIR:-}" ]; then
            report_path="$SIM_ITER_DIR/iter-${iter}-report.md"
        else
            report_path="$REPO_ROOT/.autospec/run-summary.md"
        fi

        if [ -z "${SIM_ITER_DIR:-}" ]; then
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

        local harvested
        harvested="$(autospec_loop_harvest_next_prompt "$report_path")"

        if [ -n "${SIM_TOKENS:-}" ]; then
            tokens_used=$((tokens_used + SIM_TOKENS))
        fi

        local stop_reason="null"
        row_status="next-steps found"
        local row_harvested="${harvested:-(empty)}"
        local row_harvested_short
        row_harvested_short="$(printf '%s' "$row_harvested" | head -c 60)"

        if [ -n "$harvested" ] && [ "${harvested#STOP::}" != "$harvested" ]; then
            stop_reason="\"$(printf '%s' "${harvested#STOP::}" | python3 -c 'import json,sys; sys.stdout.write(json.dumps(sys.stdin.read())[1:-1])')\""
            row_status="evidence_based_stop"
            row_harvested_short="STOP: ${harvested#STOP::}"
        fi

        local converged=0
        if [ -z "$harvested" ]; then
            converged=1
            row_status="convergence_clean"
        fi

        local oscillation=0
        local cur_hash=""
        if [ -n "$harvested" ] && [ "${harvested#STOP::}" = "$harvested" ]; then
            cur_hash="$(printf '%s' "$harvested" | shasum -a 256 | awk '{print $1}')"
            if [ -n "$prev_hash" ] && [ "$cur_hash" = "$prev_hash" ]; then
                oscillation=1
                row_status="oscillation_detected"
            fi
        fi

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

        local row
        row="$(printf '| %4d | %-21s | %-60s | %10s | %4s | %-20s |' \
            "$iter" "$(printf '%s' "$cur_source" | head -c 21)" \
            "$row_harvested_short" "0" "-" "$row_status")"
        if [ -z "$table_rows" ]; then
            table_rows="$row"
        else
            table_rows="$table_rows"$'\n'"$row"
        fi

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

        if [ "$tokens_used" -gt "$_token_cap" ] 2>/dev/null; then
            status="budget_cap_reached"
            break
        fi
        local now
        now="$(date +%s)"
        if [ $((now - start_ts)) -gt "$_time_cap" ]; then
            status="budget_cap_reached"
            break
        fi

        prev_hash="$cur_hash"
        cur_prompt="$harvested"
        cur_source="$report_path"
    done

    iter_records="$iter_records]"

    if [ -z "$status" ]; then
        status="round_cap_reached"
    fi

    cat > "$loop_json" <<EOF
{
  "slug": "$loop_slug",
  "status": "$status",
  "iterations_executed": $iter,
  "max_iterations": $_max_iter,
  "tokens_used": $tokens_used,
  "iterations": $iter_records
}
EOF

    # Validate loop JSON against loop schema (issue #682). Non-fatal warn
    # if jsonschema is unavailable; fatal if validation actually fails.
    local _loop_schema
    _loop_schema="$(cd "$_AUTOSPEC_LOOP_LIB_DIR/../.." 2>/dev/null && pwd)/schemas/autospec-refinement-loop.schema.json"
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
    sys.stderr.write(f"autospec-loop: loop schema validation failed: {e.message}\n")
    sys.exit(1)
PY
            echo "autospec-loop: WARN — loop artifact failed schema validation: $loop_json" >&2
        }
    fi

    {
        printf '## /autospec continuous loop summary\n\n'
        printf '| Iter | Harvested from        | Refined prompt (first 60 chars)                              | PRs merged | Time | Status               |\n'
        printf '|------|-----------------------|--------------------------------------------------------------|-----------:|------|----------------------|\n'
        printf '%s\n' "$table_rows"
        printf '\nFinal status: %s\n' "$status"
        printf 'Iterations executed: %s / %s\n' "$iter" "$_max_iter"
    } > "$loop_md"

    # In real mode, also mirror the summary to .autospec/loop-summary.md so
    # /autospec --loop has a canonical, stable output location.
    if [ -z "${SIM_ITER_DIR:-}" ] && [ -d "$REPO_ROOT" ]; then
        mkdir -p "$REPO_ROOT/.autospec" 2>/dev/null || true
        cp "$loop_md" "$REPO_ROOT/.autospec/loop-summary.md" 2>/dev/null || true
    fi

    printf '%s\n' "## /autospec continuous loop summary"
    printf '%s\n' "Final status: $status (iterations=$iter, artifact=$loop_json)"
}

# ── Conductor orchestrator (issue #1378) ─────────────────────────────────────
#
# autospec_conductor_run: Phase-1 perpetual conductor loop entry point for
# /autospec-autonomous.  Each cycle walks:
#   1. Resilience heartbeat / lock (autonomous-resilience.sh)
#   2. Tier-0 control-channel poll (autonomous-control-channel.sh) → preempt
#   3. Waterfall tier selection (autonomous-waterfall.sh)
#   4. Tier-1 drain: check premerge gate (autonomous-premerge-gate.sh)
#      MUST emit merge-ok before any autospec-run invocation
#   4b. Main-health gate (autonomous-resilience.sh main-health): red main →
#      halt Tier-1 merges; pending → skip drain this cycle (never drain onto red)
#   5. Drain via AUTOSPEC_RUN_CMD (inherits autospec-run path)
#   5b. Outcome-ledger wiring (F5): after drain, record per-source ship/fail
#       outcome for explore-filed issues (is_discovery=true in last-outcome.json).
#       Tier-1 backlog issues do not write the file; recording is silently skipped.
#   6a. Usage-governor soft-park (F6): autonomous-usage-governor.sh parks at
#       AUTOSPEC_USAGE_SOFT_PCT (default 90%) BEFORE the Step-6 hard cap; layers
#       on top of the spend-ledger backstop (fail-open continue on any error).
#   6. Spend-ledger tally (autonomous-spend-ledger.sh); park on cap
#   7. Once-per-UTC-day digest to .autospec/autonomous-digest.md + pinned issue
#   8. On park (spend/usage): arm ScheduleWakeup/cron via autospec-usage-limit.sh
#   9. Tier 1.5+ never-idle waterfall: promote existing open issues, run
#      local discovery, generate architecture/test-coverage work, then run
#      internet/operator-polish discovery. Park only when every tier is dry.
#      Discovery consults explore-source-weights.sh before ranking proposals.
#
# Tiers 1.5-4 are enabled by default; AUTOSPEC_DISABLE_DISCOVERY_TIERS=1 is
# an emergency fail-closed park at the Tier-1 dry threshold.
#
# Globals (caller sets before invoking):
#   CONDUCTOR_SCRIPTS_DIR   path to scripts/ dir containing helper scripts
#   CONDUCTOR_REPO          owner/repo slug (optional; auto-detected)
#   CONDUCTOR_MAX_CYCLES    cycle cap; 0 = unlimited (default 0)
#   CONDUCTOR_POLL_INTERVAL seconds to sleep between cycles (default 60)
#   CONDUCTOR_DRY_RUN       1 = log only; no autospec-run invocations
#   CONDUCTOR_NO_DIGEST     1 = skip daily digest writes
#   AUTOSPEC_RUN_CMD              override autospec-run invocation (for tests/dry-run)
#   AUTOSPEC_SCRIPTS_DIR          installed scripts path (fallback for helpers)
#   AUTOSPEC_SESSION_ID           stable session identifier (default: conductor-$$)
#   AUTOSPEC_EXPLORE_LEDGER       outcome ledger path (default: .autospec/explore-ledger.jsonl)
#   AUTOSPEC_EXPLORE_LEDGER_BIN   explicit path to explore-ledger.sh (for tests)
#   AUTOSPEC_EXPLORE_WEIGHTS_BIN  explicit path to explore-source-weights.sh (for tests)
#   AUTOSPEC_LAST_OUTCOME_FILE    path the run command writes discovery outcomes to
#
# Safety rules (AGENTS.md):
#   set -eu; if/then/fi for one-sided conditionals; no RETURN traps;
#   jq: use capture()/== never interpolated test() for dynamic values.

# _autospec_conductor_record_stop: emit one terminal marker and persist terminal
# state.  Uses globals because POSIX signal traps cannot receive Bash locals
# safely while the conductor may be interrupted inside a child command.
_autospec_conductor_record_stop() {
    local reason="${1:-unknown}"
    local cycle="${2:-${_AUTOSPEC_CONDUCTOR_CYCLE:-0}}"
    local shape="${3:-normal}"
    if [ "${_AUTOSPEC_CONDUCTOR_STOP_RECORDED:-0}" = "1" ]; then
        return 0
    fi
    _AUTOSPEC_CONDUCTOR_STOP_RECORDED=1
    if [ "$shape" = "signal" ]; then
        printf '[conductor] stopped: %s (cycle=%s)\n' "$reason" "$cycle" >&2
    else
        printf '[conductor] stopped: %s (cycles=%s)\n' "$reason" "$cycle" >&2
    fi
    if [ -n "${_AUTOSPEC_CONDUCTOR_RESILIENCE:-}" ] \
        && [ -f "$_AUTOSPEC_CONDUCTOR_RESILIENCE" ] \
        && [ -n "${_AUTOSPEC_CONDUCTOR_REPO:-}" ]; then
        bash "$_AUTOSPEC_CONDUCTOR_RESILIENCE" state write \
            --repo "$_AUTOSPEC_CONDUCTOR_REPO" \
            --status "stopped:${reason}:cycle-${cycle}" \
            --session "${_AUTOSPEC_CONDUCTOR_SESSION:-}" \
            2>/dev/null || true
    fi
}

_autospec_conductor_release_lock() {
    if [ "${_AUTOSPEC_CONDUCTOR_LOCK_HELD:-0}" != "1" ]; then
        return 0
    fi
    _AUTOSPEC_CONDUCTOR_LOCK_HELD=0
    if [ -n "${_AUTOSPEC_CONDUCTOR_RESILIENCE:-}" ] \
        && [ -f "$_AUTOSPEC_CONDUCTOR_RESILIENCE" ] \
        && [ -n "${_AUTOSPEC_CONDUCTOR_REPO:-}" ]; then
        bash "$_AUTOSPEC_CONDUCTOR_RESILIENCE" lock release \
            --repo "$_AUTOSPEC_CONDUCTOR_REPO" \
            --session "${_AUTOSPEC_CONDUCTOR_SESSION:-}" \
            2>/dev/null || true
    fi
}

_autospec_conductor_signal_exit_code() {
    case "$1" in
        HUP) printf '129' ;;
        INT) printf '130' ;;
        QUIT) printf '131' ;;
        TERM) printf '143' ;;
        *) printf '128' ;;
    esac
}

_autospec_conductor_on_signal() {
    local sig="$1"
    local cycle="${_AUTOSPEC_CONDUCTOR_CYCLE:-0}"
    _autospec_conductor_record_stop "signal:${sig}" "$cycle" signal
    _autospec_conductor_release_lock
    trap - EXIT HUP INT QUIT TERM
    exit "$(_autospec_conductor_signal_exit_code "$sig")"
}

_autospec_conductor_on_exit() {
    local rc="$?"
    if [ "${_AUTOSPEC_CONDUCTOR_STOP_RECORDED:-0}" != "1" ]; then
        local reason="abnormal-exit:${rc}"
        if [ "$rc" -eq 0 ]; then
            reason="unknown"
        fi
        _autospec_conductor_record_stop "$reason" "${_AUTOSPEC_CONDUCTOR_CYCLE:-0}" normal
    fi
    _autospec_conductor_release_lock
}

_autospec_conductor_all_blocked_refs() {
    printf '%s' "$1" \
        | jq -r '[.blocked[]? | "#\(.number) reason=\(.reason // "blocked") deps=\((.unmet_dependencies // []) | join(","))"] | join("; ")' \
        2>/dev/null || true
}

_autospec_conductor_queue_count() {
    printf '%s' "$1" | jq -r "$2" 2>/dev/null || true
}

_autospec_conductor_escalate_all_blocked() {
    local repo="$1"
    local queue_json="$2"
    local count="$3"
    local refs="$4"
    local no_digest="$5"

    printf '[conductor] autospec:needs-human — Tier-1 all-blocked unresolved after Tier 1.5 promotion (%s issues)%s\n' \
        "$count" "${refs:+: ${refs}}" >&2
    gh label create autospec:needs-human --repo "$repo" --color d73a4a --force >/dev/null 2>&1 || true
    printf '%s\n' "$queue_json" | jq -r '.blocked[]?.number' 2>/dev/null \
        | while IFS= read -r _blocked_issue; do
            [ -n "$_blocked_issue" ] || continue
            gh issue edit "$_blocked_issue" --repo "$repo" --add-label autospec:needs-human >/dev/null 2>&1 || true
        done
    if [ "$no_digest" = "1" ]; then
        return 0
    fi
    mkdir -p "${HOME}/.autospec" 2>/dev/null || true
    printf '%s Tier-1 all-blocked unresolved (%s issues): %s\n' \
        "$(date -u +'%Y-%m-%dT%H:%M:%SZ' 2>/dev/null || echo unknown)" \
        "$count" "${refs:-unknown blockers}" \
        >> "${HOME}/.autospec/autonomous-digest.md" 2>/dev/null || true
}

autospec_conductor_run() {
    if [ -n "${HOME:-}" ]; then
        case ":${PATH:-}:" in
            *":$HOME/.autospec/bin:"*) ;;
            *) PATH="$HOME/.autospec/bin:${PATH:-}"; export PATH ;;
        esac
    fi

    # Resolve scripts directory: explicit override > AUTOSPEC_SCRIPTS_DIR > sibling of lib/.
    local _sdir
    _sdir="${CONDUCTOR_SCRIPTS_DIR:-${AUTOSPEC_SCRIPTS_DIR:-$(cd "$_AUTOSPEC_LOOP_LIB_DIR/.." && pwd)}}"

    local _repo="${CONDUCTOR_REPO:-${AUTOSPEC_REPO:-}}"
    local _max="${CONDUCTOR_MAX_CYCLES:-0}"
    local _poll="${CONDUCTOR_POLL_INTERVAL:-60}"
    local _dry="${CONDUCTOR_DRY_RUN:-0}"
    local _no_digest="${CONDUCTOR_NO_DIGEST:-0}"

    # Resolve helper script paths.
    local _control_ch="${_sdir}/autonomous-control-channel.sh"
    local _waterfall="${_sdir}/autonomous-waterfall.sh"
    local _spend="${_sdir}/autonomous-spend-ledger.sh"
    local _gate="${_sdir}/autonomous-premerge-gate.sh"
    local _resilience="${_sdir}/autonomous-resilience.sh"
    local _usage_limit="${_sdir}/autospec-usage-limit.sh"
    local _governor="${_sdir}/autonomous-usage-governor.sh"
    local _list_ready="${AUTOSPEC_LIST_READY_BIN:-}"

    # ── Ledger wiring (F5) ─────────────────────────────────────────────────────
    # Resolve repo root (parent of scripts/ dir) for ledger data file path.
    local _repo_root
    _repo_root="$(cd "${_sdir}/.." 2>/dev/null && pwd || printf '.')"

    # Resolve outcome ledger data path (env override > default under repo root).
    local _ledger_path="${AUTOSPEC_EXPLORE_LEDGER:-${_repo_root}/.autospec/explore-ledger.jsonl}"

    # Resolve explore-ledger.sh (fail-open: missing script is benign).
    local _ledger_sh=""
    if [ -n "${AUTOSPEC_EXPLORE_LEDGER_BIN:-}" ] && [ -x "$AUTOSPEC_EXPLORE_LEDGER_BIN" ]; then
        _ledger_sh="$AUTOSPEC_EXPLORE_LEDGER_BIN"
    elif [ -n "${AUTOSPEC_SCRIPTS_DIR:-}" ] && [ -f "${AUTOSPEC_SCRIPTS_DIR}/explore-ledger.sh" ]; then
        _ledger_sh="${AUTOSPEC_SCRIPTS_DIR}/explore-ledger.sh"
    elif [ -f "${_sdir}/explore-ledger.sh" ]; then
        _ledger_sh="${_sdir}/explore-ledger.sh"
    elif [ -f "${_sdir}/../skills/autospec-shared/scripts/explore-ledger.sh" ]; then
        _ledger_sh="${_sdir}/../skills/autospec-shared/scripts/explore-ledger.sh"
    fi

    # Resolve explore-source-weights.sh (same resolution order as explore-research-cycle.sh).
    local _weights_bin=""
    if [ -n "${AUTOSPEC_EXPLORE_WEIGHTS_BIN:-}" ] && [ -x "$AUTOSPEC_EXPLORE_WEIGHTS_BIN" ]; then
        _weights_bin="$AUTOSPEC_EXPLORE_WEIGHTS_BIN"
    elif [ -n "${AUTOSPEC_SCRIPTS_DIR:-}" ] && [ -f "${AUTOSPEC_SCRIPTS_DIR}/explore-source-weights.sh" ]; then
        _weights_bin="${AUTOSPEC_SCRIPTS_DIR}/explore-source-weights.sh"
    elif [ -f "${_sdir}/explore-source-weights.sh" ]; then
        _weights_bin="${_sdir}/explore-source-weights.sh"
    elif [ -f "${_sdir}/../skills/autospec-shared/scripts/explore-source-weights.sh" ]; then
        _weights_bin="${_sdir}/../skills/autospec-shared/scripts/explore-source-weights.sh"
    fi

    # Locate notify.sh: script-relative, then PATH.
    local _notify_sh=""
    if [ -f "${_sdir}/../skills/autospec-shared/scripts/notify.sh" ]; then
        _notify_sh="${_sdir}/../skills/autospec-shared/scripts/notify.sh"
    elif command -v notify.sh >/dev/null 2>&1; then
        _notify_sh="notify.sh"
    fi

    # ── Sandbox wiring (F3) ────────────────────────────────────────────────────
    # Resolve explore-sandbox.sh: env override > sibling of scripts/.
    # Called idempotently at Tier 2/3 entry to ensure .autospec/explore-mode.json
    # exists so implementer PRs target the sandbox base.
    local _sandbox_sh=""
    if [ -n "${AUTOSPEC_SANDBOX_BIN:-}" ] && [ -x "$AUTOSPEC_SANDBOX_BIN" ]; then
        _sandbox_sh="$AUTOSPEC_SANDBOX_BIN"
    elif [ -f "${_sdir}/explore-sandbox.sh" ]; then
        _sandbox_sh="${_sdir}/explore-sandbox.sh"
    fi

    # ── Persona synthesis wiring (F2) ──────────────────────────────────────────
    # Resolve autonomous-persona-synth.sh: env override > sibling of scripts/.
    # The script self-gates on AUTOSPEC_PERSONA_REFRESH_DAYS staleness, so the
    # actual Tier-A synthesis only runs when the global persona is stale — the
    # per-cycle invocation is a no-op (fast exit 0) while the persona is fresh.
    local _persona_synth_sh=""
    if [ -n "${AUTOSPEC_PERSONA_SYNTH_BIN:-}" ] && [ -x "$AUTOSPEC_PERSONA_SYNTH_BIN" ]; then
        _persona_synth_sh="$AUTOSPEC_PERSONA_SYNTH_BIN"
    elif [ -f "${_sdir}/autonomous-persona-synth.sh" ]; then
        _persona_synth_sh="${_sdir}/autonomous-persona-synth.sh"
    fi

    # ── Persona mining wiring (F3) ─────────────────────────────────────────────
    # Resolve autonomous-persona-mine.sh: env override > sibling of scripts/.
    # The script self-gates on AUTOSPEC_PERSONA_REFRESH_DAYS staleness of its
    # mined digest, so the per-cycle invocation is a fast no-op while fresh. It
    # MUST run before persona synthesis below: the mined-decision digest is the
    # F1 precedence-3 source the synthesizer reads. Without this wiring F3 ships
    # but never runs and precedence-3 is permanently empty.
    local _persona_mine_sh=""
    if [ -n "${AUTOSPEC_PERSONA_MINE_BIN:-}" ] && [ -x "$AUTOSPEC_PERSONA_MINE_BIN" ]; then
        _persona_mine_sh="$AUTOSPEC_PERSONA_MINE_BIN"
    elif [ -f "${_sdir}/autonomous-persona-mine.sh" ]; then
        _persona_mine_sh="${_sdir}/autonomous-persona-mine.sh"
    fi

    # ── Persona recalibrate flag (F6 control channel → conductor) ──────────────
    # autospec:recalibrate-persona drops this flag via autonomous-control-channel.sh;
    # the conductor consumes it in step 1b to force a persona/mine refresh on the
    # next cycle, then clears it. Path mirrors the control channel's FLAG_DIR.
    local _persona_recal_flag="${AUTOSPEC_CONTROL_STATE_DIR:-${HOME}/.autospec}/persona-recalibrate.flag"

    # ── F4: Priority match script resolution ───────────────────────────────────
    # Resolve autonomous-priority-match.sh: env override > sibling of scripts/.
    local _priority_match_sh=""
    if [ -n "${AUTOSPEC_PRIORITY_MATCH_BIN:-}" ] && [ -x "$AUTOSPEC_PRIORITY_MATCH_BIN" ]; then
        _priority_match_sh="$AUTOSPEC_PRIORITY_MATCH_BIN"
    elif [ -f "${_sdir}/autonomous-priority-match.sh" ]; then
        _priority_match_sh="${_sdir}/autonomous-priority-match.sh"
    fi

    # ── F4: Priority intake ────────────────────────────────────────────────────
    # Persist priorities to ~/.autospec/autonomous-priorities.md.
    # Sources (in priority order):
    #   1. CONDUCTOR_PRIORITIES env (--priorities "a; b; c" style, ";" delimited)
    #   2. Existing priorities file (subsequent runs — no re-ask)
    #   3. First run with neither: ONE AskUserQuestion startup gate via
    #      AUTOSPEC_ASK_PRIORITIES_CMD seam, then proceed.
    # fail-open: all errors produce a warning but never abort the conductor.
    local _priorities_file="${AUTOSPEC_PRIORITIES_FILE:-${HOME}/.autospec/autonomous-priorities.md}"
    local _priorities_dir
    _priorities_dir="$(dirname "$_priorities_file")"

    if [ -n "${CONDUCTOR_PRIORITIES:-}" ]; then
        # --priorities supplied: parse semicolon-delimited items and append.
        mkdir -p "$_priorities_dir" 2>/dev/null || true
        printf '%s\n' "$CONDUCTOR_PRIORITIES" | tr ';' '\n' | while IFS= read -r _pri; do
            _pri="$(printf '%s' "$_pri" | sed 's/^[[:space:]]*//; s/[[:space:]]*$//')"
            if [ -n "$_pri" ]; then
                printf -- '- %s\n' "$_pri"
            fi
        done >> "$_priorities_file" 2>/dev/null || true
        printf '[conductor] F4: persisted priorities from CONDUCTOR_PRIORITIES to %s\n' \
            "$_priorities_file" >&2
    elif [ ! -f "$_priorities_file" ]; then
        # First run with no priorities file: one AskUserQuestion startup gate.
        local _ask_priorities_cmd="${AUTOSPEC_ASK_PRIORITIES_CMD:-}"
        if [ -n "$_ask_priorities_cmd" ]; then
            printf '[conductor] F4: no priorities file — invoking startup gate\n' >&2
            local _asked_priorities
            _asked_priorities="$(bash -c "$_ask_priorities_cmd" 2>/dev/null || true)"
            mkdir -p "$_priorities_dir" 2>/dev/null || true
            if [ -n "$_asked_priorities" ]; then
                printf '%s\n' "$_asked_priorities" | tr ';' '\n' | while IFS= read -r _pri; do
                    _pri="$(printf '%s' "$_pri" | sed 's/^[[:space:]]*//; s/[[:space:]]*$//')"
                    if [ -n "$_pri" ]; then
                        printf -- '- %s\n' "$_pri"
                    fi
                done >> "$_priorities_file" 2>/dev/null || true
            else
                # User declined — create the file to suppress future asks.
                : > "$_priorities_file"
            fi
        else
            # No ask seam available: proceed without priorities (fail-open).
            printf '[conductor] F4: no priorities and no AUTOSPEC_ASK_PRIORITIES_CMD seam; proceeding without priorities\n' >&2
        fi
    fi

    # Pre-compute the explore cycle wrapper for Tier 2/3 priority marking.
    # When _priority_match_sh and _priorities_file both exist, the conductor
    # wraps the explore-research-cycle.sh call with a two-stage approach:
    #   1. --stage dedup (get raw deduped proposals)
    #   2. autonomous-priority-match.sh on each proposal (sets priority:true)
    #   3. --stage finalize (rank with the clamped boost consuming the flag)
    # The wrapper is set as AUTOSPEC_EXPLORE_ONCE_CYCLE_CMD inline on the bash -c
    # invocation so it never leaks into Tier-1 drain calls.
    # Fail-open: if cycle script or match script is absent, fall back to the
    # original non-wrapped explore call.
    local _priority_cycle_cmd=""
    local _cycle_sh="${_sdir}/explore-research-cycle.sh"
    local _eff_persona="${HOME}/.autospec/operator-persona.md"
    if [ -f "${_repo_root}/.autospec/operator-persona.effective.md" ]; then
        _eff_persona="${_repo_root}/.autospec/operator-persona.effective.md"
    fi
    if [ -z "$_list_ready" ]; then
        if [ -f "${_sdir}/list-ready-issues.sh" ]; then
            _list_ready="${_sdir}/list-ready-issues.sh"
        elif [ -f "${_repo_root}/skills/autospec-run/scripts/list-ready-issues.sh" ]; then
            _list_ready="${_repo_root}/skills/autospec-run/scripts/list-ready-issues.sh"
        fi
    fi
    if [ -f "$_priority_match_sh" ] && [ -f "$_priorities_file" ] \
        && [ -s "$_priorities_file" ] && [ -f "$_cycle_sh" ]; then
        # Single-quoted so variables expand when bash -c runs the string.
        # Inner python3 -c uses env vars (PRIORITY_DEDUP/MATCH/MARKED) to avoid
        # quoting conflicts between the outer bash string and python string literals.
        _priority_cycle_cmd='set -eu
_csh="${AUTOSPEC_CYCLE_SH}"
_match="${AUTOSPEC_PRIORITY_MATCH_BIN}"
_out="${AUTOSPEC_EXPLORE_ONCE_OUT}"
_src="${AUTOSPEC_EXPLORE_ONCE_SOURCES:-spec-vs-code,prior-reports,codebase-signals,open-issues,source-analysis,dependency-health}"
_dedup="${_out}.dedup.$$"
bash "$_csh" --stage dedup --out "$_dedup" --research-sources "$_src" 2>/dev/null || true
_marked="${_dedup}.marked"
if [ -f "$_dedup" ] && [ -f "$_match" ]; then
    PRIORITY_DEDUP="$_dedup" PRIORITY_MATCH="$_match" PRIORITY_MARKED="$_marked" \
    python3 -c "
import json,subprocess,os
d=json.load(open(os.environ[\"PRIORITY_DEDUP\"]))
match=os.environ[\"PRIORITY_MATCH\"]
props=d.get(\"deduped\") or d.get(\"proposals\") or []
out=[]
for p in props:
    try:
        r=subprocess.run([\"bash\",match],input=json.dumps(p),capture_output=True,text=True,timeout=30)
        out.append(json.loads(r.stdout) if r.returncode==0 and r.stdout.strip().startswith(\"{\") else p)
    except Exception:
        out.append(p)
d[\"deduped\"]=out
json.dump(d,open(os.environ[\"PRIORITY_MARKED\"],\"w\"))
" 2>/dev/null || cp "$_dedup" "$_marked"
    bash "$_csh" --stage finalize --deduped-in "$_marked" --out "$_out" --research-sources "$_src" 2>/dev/null || true
    rm -f "$_dedup" "$_marked" 2>/dev/null || true
else
    bash "$_csh" --out "$_out" --research-sources "$_src" 2>/dev/null || true
    rm -f "$_dedup" 2>/dev/null || true
fi'
    fi

    local _cycle=0
    local _dry_cycles=0
    local _tier15_dry_cycles=0
    local _tier2_dry_cycles=0
    local _tier3_dry_cycles=0
    local _tier4_dry_cycles=0
    # F3: count of discovery issues filed by Tier 2/3 cycles that have not yet
    # been consumed by F5 outcome processing.  Non-zero → refuse Tier-1 main merge.
    local _inflight_discovery=0
    local _last_digest_day=""
    local _stop_reason=""
    local _conductor_session="${AUTOSPEC_SESSION_ID:-conductor-$$}"

    _AUTOSPEC_CONDUCTOR_CYCLE=0
    _AUTOSPEC_CONDUCTOR_STOP_RECORDED=0
    _AUTOSPEC_CONDUCTOR_LOCK_HELD=0
    _AUTOSPEC_CONDUCTOR_RESILIENCE="$_resilience"
    _AUTOSPEC_CONDUCTOR_REPO="$_repo"
    _AUTOSPEC_CONDUCTOR_SESSION="$_conductor_session"
    trap '_autospec_conductor_on_signal HUP' HUP
    trap '_autospec_conductor_on_signal INT' INT
    trap '_autospec_conductor_on_signal QUIT' QUIT
    trap '_autospec_conductor_on_signal TERM' TERM
    trap '_autospec_conductor_on_exit' EXIT

    # Acquire single-instance conductor lock (fail-open: errors never block).
    # Reuses resume's 300s/10800s staleness thresholds — never contradicts resume.
    if [ -f "$_resilience" ] && [ -n "$_repo" ]; then
        local _lock_out
        _lock_out="$(bash "$_resilience" lock acquire \
            --repo "$_repo" \
            --session "$_conductor_session" \
            2>/dev/null || true)"
        if printf '%s' "$_lock_out" | grep -q 'DECISION:lock-held'; then
            printf '[conductor] WARN: another conductor holds the lock — exiting\n' >&2
            _autospec_conductor_record_stop "lock-held" "$_cycle" normal
            trap - EXIT HUP INT QUIT TERM
            return 1
        fi
        if printf '%s' "$_lock_out" | grep -q 'DECISION:lock-acquired'; then
            _AUTOSPEC_CONDUCTOR_LOCK_HELD=1
        fi
    fi

    # ── Main cycle loop ───────────────────────────────────────────────────────
    while true; do
        if [ -f "${AUTOSPEC_STOP_FLAG_FILE:-${HOME}/.autospec/stop.flag}" ]; then
            printf '[conductor] operator stop flag detected: %s\n' \
                "${AUTOSPEC_STOP_FLAG_FILE:-${HOME}/.autospec/stop.flag}" >&2
            _stop_reason="operator:stop-flag"
            break
        fi
        _cycle=$((_cycle + 1))
        _AUTOSPEC_CONDUCTOR_CYCLE="$_cycle"
        printf '[conductor] cycle %s starting\n' "$_cycle" >&2

        # ── Step 1: Resilience heartbeat ──────────────────────────────────────
        if [ -f "$_resilience" ] && [ -n "$_repo" ]; then
            bash "$_resilience" state write \
                --repo "$_repo" \
                --status "running:cycle-${_cycle}" \
                --session "$_conductor_session" \
                2>/dev/null || true
        fi

        # ── Step 1b: Persona mining + synthesis on cadence (F2/F3) ────────────
        # Fail-open: errors (incl. synth lock-held exit 2) never block the loop.
        # Both helpers self-gate on staleness so this is a fast no-op when fresh.
        # An autospec:recalibrate-persona control signal (F6) drops a flag that
        # forces a refresh; consume + clear it here so the refresh happens once.
        local _persona_force=""
        if [ "$_dry" != "1" ] && [ -f "$_persona_recal_flag" ]; then
            printf '[conductor] F6: persona-recalibrate flag found — forcing persona refresh\n' >&2
            _persona_force="--force"
            rm -f "$_persona_recal_flag" 2>/dev/null || true
        fi
        # F3: refresh the mined-decision digest (F1 precedence-3 input) BEFORE
        # synthesis so the fresh digest feeds the source bundle.
        if [ -f "$_persona_mine_sh" ] && [ -n "$_repo_root" ] && [ "$_dry" != "1" ]; then
            # shellcheck disable=SC2086
            bash "$_persona_mine_sh" --repo-root "$_repo_root" $_persona_force \
                >/dev/null 2>&1 || true
        fi
        # F2: synthesize the persona (self-gates on staleness; forced on recal).
        if [ -f "$_persona_synth_sh" ] && [ -n "$_repo_root" ]; then
            if [ "$_dry" = "1" ]; then
                bash "$_persona_synth_sh" --repo-root "$_repo_root" --dry-run \
                    >/dev/null 2>&1 || true
            else
                # shellcheck disable=SC2086
                bash "$_persona_synth_sh" --repo-root "$_repo_root" $_persona_force \
                    >/dev/null 2>&1 || true
            fi
        fi

        # ── Step 2: Tier-0 control-channel poll (preempts everything) ─────────
        local _ctrl_decision=""
        if [ -f "$_control_ch" ]; then
            local _ctrl_repo_flag=""
            if [ -n "$_repo" ]; then
                _ctrl_repo_flag="--repo ${_repo}"
            fi
            local _ctrl_out
            # shellcheck disable=SC2086
            _ctrl_out="$(bash "$_control_ch" ${_ctrl_repo_flag} 2>/dev/null || true)"
            _ctrl_decision="$(printf '%s' "$_ctrl_out" \
                | grep '^DECISION:' | head -1 \
                | sed 's/^DECISION://' || true)"

            # ── F4: control-payload capture ────────────────────────────────────
            # The conductor currently discards DIRECTIVE:/PRIORITY_ISSUE: lines
            # after printf.  F4 persists them into the priorities file so steer
            # directives survive across cycles.  Fail-open: errors never abort.
            local _ctrl_payload
            _ctrl_payload="$(printf '%s' "$_ctrl_out" \
                | grep -E '^(DIRECTIVE:|PRIORITY_ISSUE:)' || true)"
            if [ -n "$_ctrl_payload" ]; then
                mkdir -p "$_priorities_dir" 2>/dev/null || true
                {
                    printf '\n## Steer directive captured %s\n' \
                        "$(date -u +'%Y-%m-%dT%H:%M:%SZ' 2>/dev/null || echo 'unknown')"
                    printf '%s\n' "$_ctrl_payload"
                } >> "$_priorities_file" 2>/dev/null || true
                printf '[conductor] F4: control-payload captured into %s\n' \
                    "$_priorities_file" >&2
            fi
        fi

        if [ -n "$_ctrl_decision" ]; then
            case "$_ctrl_decision" in
                graceful-stop)
                    printf '[conductor] DECISION:graceful-stop received — finishing cycle %s then exiting\n' \
                        "$_cycle" >&2
                    _stop_reason="control:graceful-stop"
                    break
                    ;;
                pause)
                    printf '[conductor] DECISION:pause received — parking loop\n' >&2
                    _conductor_maybe_write_digest \
                        "$_no_digest" "$_last_digest_day" "$_sdir" "$_repo" "$_dry" \
                        >/dev/null 2>&1 || true
                    _stop_reason="control:pause"
                    break
                    ;;
                steer|priority)
                    # Handled by control-channel.sh side-effects; conductor continues.
                    printf '[conductor] DECISION:%s handled by control-channel\n' \
                        "$_ctrl_decision" >&2
                    ;;
                persona-recalibrate)
                    # control-channel.sh wrote persona-recalibrate.flag; step 1b of
                    # the next cycle consumes it and forces a persona refresh.
                    printf '[conductor] DECISION:persona-recalibrate — refresh forced next cycle\n' >&2
                    ;;
            esac
        fi

        # ── Step 2b: Dependency-aware ready count (#1632) ─────────────────────
        # Compute the same readiness definition the Tier-1 drain (Step 4/5) uses
        # BEFORE calling the waterfall, and inject it as the waterfall's Tier-1
        # gate — one source of truth, so a fully-blocked backlog (or a
        # worker-cap-reached cycle) does not pin Tier-1 forever. Cached here and
        # reused by the drain step below instead of re-querying.
        local _queue_ready_len=""
        local _queue_batch_len=""
        local _queue_blocked_len=""
        local _queue_cap_reached="false"
        local _ready_count=""
        local _all_blocked_count=0
        local _all_blocked_refs=""
        if [ -n "$_list_ready" ] && [ -f "$_list_ready" ] && [ -n "$_repo" ]; then
            local _queue_json
            local _runtime_config_sh=""
            local _queue_batch_request=""
            local _queue_max_workers=""
            if [ -f "${_repo_root}/scripts/autospec-runtime-config.sh" ]; then
                _runtime_config_sh="${_repo_root}/scripts/autospec-runtime-config.sh"
            elif [ -f "${_sdir}/autospec-runtime-config.sh" ]; then
                _runtime_config_sh="${_sdir}/autospec-runtime-config.sh"
            elif [ -f "$HOME/.autospec/scripts/autospec-runtime-config.sh" ]; then
                _runtime_config_sh="$HOME/.autospec/scripts/autospec-runtime-config.sh"
            fi
            if [ -n "$_runtime_config_sh" ]; then
                # shellcheck source=/dev/null
                . "$_runtime_config_sh"
            fi
            if command -v autospec_runtime_batch_size >/dev/null 2>&1; then
                _queue_batch_request="$(autospec_runtime_batch_size)"
            else
                _queue_batch_request="${AUTOSPEC_BATCH_SIZE:-1}"
            fi
            case "$_queue_batch_request" in *[!0-9]*|'') _queue_batch_request=1 ;; esac
            [ "$_queue_batch_request" -gt 0 ] || _queue_batch_request=1
            if command -v autospec_runtime_repo_workers >/dev/null 2>&1; then
                _queue_max_workers="$(autospec_runtime_repo_workers)"
            else
                _queue_max_workers="${AUTOSPEC_MAX_CONCURRENT_REPO_WORKERS:-0}"
            fi
            if [ "${_queue_max_workers:-0}" -gt "$_queue_batch_request" ] 2>/dev/null; then
                _queue_batch_request="${_queue_max_workers:-0}"
            fi
            _queue_json="$(bash "$_list_ready" --repo "$_repo" --batch-size "$_queue_batch_request" 2>/dev/null || true)"
            # Only trust the reading when the helper produced parseable JSON with
            # a .ready array. A transient helper/GitHub failure must NOT
            # masquerade as an empty backlog: leaving _ready_count empty omits
            # --backlog-count below, so the waterfall runs its OWN
            # readiness-aware count (which itself falls back to the naive gh
            # count) rather than being forced to 0 on a blip.
            if printf '%s' "$_queue_json" | jq -e 'has("ready")' >/dev/null 2>&1; then
                _queue_ready_len="$(_autospec_conductor_queue_count "$_queue_json" '.ready | length')"
                _queue_batch_len="$(_autospec_conductor_queue_count "$_queue_json" '.batch | length')"
                _queue_blocked_len="$(_autospec_conductor_queue_count "$_queue_json" '.blocked | length')"
                _queue_cap_reached="$(_autospec_conductor_queue_count "$_queue_json" '.worker_cap.reached // false')"
                [ -n "$_queue_cap_reached" ] || _queue_cap_reached="false"
                case "$_queue_ready_len" in *[!0-9]*|'') _queue_ready_len="" ;; esac
                case "$_queue_batch_len" in *[!0-9]*|'') _queue_batch_len="" ;; esac
                case "$_queue_blocked_len" in *[!0-9]*|'') _queue_blocked_len="" ;; esac
                if [ "$_queue_cap_reached" = "true" ]; then
                    # A capped cycle is not drainable work — never pins Tier-1.
                    _ready_count=0
                else
                    _ready_count=$(( ${_queue_ready_len:-0} + ${_queue_batch_len:-0} ))
                fi
                if [ "${_ready_count:-0}" -eq 0 ] && [ "${_queue_blocked_len:-0}" -gt 0 ]; then
                    _all_blocked_count="${_queue_blocked_len:-0}"
                    _all_blocked_refs="$(_autospec_conductor_all_blocked_refs "$_queue_json")"
                fi
            fi
        fi

        # ── Step 3: Waterfall tier selection ──────────────────────────────────
        local _tier_json
        _tier_json="$(bash "$_waterfall" \
            --dry-cycles "$_dry_cycles" \
            --tier15-dry-cycles "$_tier15_dry_cycles" \
            --tier2-dry-cycles "$_tier2_dry_cycles" \
            --tier3-dry-cycles "$_tier3_dry_cycles" \
            --tier4-dry-cycles "$_tier4_dry_cycles" \
            ${_repo:+--repo "$_repo"} \
            ${_ready_count:+--backlog-count "$_ready_count"} \
            2>/dev/null \
            || printf '{"tier":1,"action":"run-backlog","reason":"waterfall-unavailable"}')"
        local _tier
        _tier="$(printf '%s' "$_tier_json" \
            | jq -r '.tier // 1' 2>/dev/null || echo 1)"
        local _action
        _action="$(printf '%s' "$_tier_json" \
            | jq -r '.action // "run-backlog"' 2>/dev/null || echo "run-backlog")"
        local _reason
        _reason="$(printf '%s' "$_tier_json" \
            | jq -r '.reason // ""' 2>/dev/null || echo "")"

        printf '[conductor] tier=%s action=%s\n' "$_tier" "$_action" >&2

        # ── Step 4 + 5: Tier-1 drain gated on premerge check ─────────────────
        local _work_done=0
        if [ "$_action" = "park" ]; then
            printf '[conductor] parking: %s\n' "$_reason" >&2
            _stop_reason="waterfall:park:${_reason}"
            break
        elif [ "$_action" = "promote-open-issues" ]; then
            # ── Tier 1.5: promote/decompose/classify existing open issues ─────
            # Auto-detect the promotion command when the operator hasn't pinned
            # one via AUTOSPEC_PROMOTE_OPEN_ISSUES_CMD. Preference order:
            #   1. autospec-classify skill on PATH (rare as a real binary);
            #   2. the real promote-open-issues script (scripts/autonomous-
            #      promote-open-issues.sh) — passed --apply, which is SAFE
            #      because that script is double-gated and still no-ops (report-
            #      only) unless AUTOSPEC_PROMOTE_OPEN_ISSUES_APPLY=1 is also set;
            #   3. the classify-model-fit --help no-op placeholder (final
            #      fallback so the tier is never left command-less).
            local _promote_cmd="${AUTOSPEC_PROMOTE_OPEN_ISSUES_CMD:-}"
            if [ -z "$_promote_cmd" ]; then
                if command -v autospec-classify >/dev/null 2>&1; then
                    _promote_cmd="autospec-classify --apply-boards"
                elif [ -f "${_sdir}/autonomous-promote-open-issues.sh" ]; then
                    _promote_cmd="bash ${_sdir}/autonomous-promote-open-issues.sh --apply"
                elif [ -f "${_sdir}/classify-model-fit.sh" ]; then
                    _promote_cmd="bash ${_sdir}/classify-model-fit.sh --help >/dev/null"
                fi
            fi

            local _promote_out
            if [ "$_dry" = "1" ]; then
                printf '[conductor] [dry-run] would promote/decompose/classify open issues for Tier 1.5
' >&2
                _promote_out='{"dry":true,"filed":0,"reason":"dry-run"}'
            elif [ -n "$_promote_cmd" ]; then
                printf '[conductor] Tier 1.5: promoting/decomposing/classifying open issues
' >&2
                _promote_out="$(bash -c "$_promote_cmd" 2>/dev/null || printf '{"dry":true,"filed":0,"reason":"promotion-error"}')"
            else
                printf '[conductor] WARN: no Tier 1.5 promotion command available — treating promotion as dry
' >&2
                _promote_out='{"dry":true,"filed":0,"reason":"promotion-command-missing"}'
            fi

            local _promote_dry _promote_filed
            _promote_dry="$(printf '%s' "$_promote_out" | jq -r 'if has("dry") then .dry else true end' 2>/dev/null || echo 'true')"
            _promote_filed="$(printf '%s' "$_promote_out" | jq -r '.filed // .promoted // 0' 2>/dev/null || echo 0)"
            printf '[conductor] Tier 1.5 promotion result: dry=%s filed=%s
'                 "$_promote_dry" "$_promote_filed" >&2
            if [ "$_promote_dry" = "false" ] || { [ "$_promote_filed" -gt 0 ] 2>/dev/null; }; then
                _work_done=1
            else
                _tier15_dry_cycles=$((_tier15_dry_cycles + 1))
                printf '[conductor] Tier 1.5 dry (tier15-dry-cycles=%s)
'                     "$_tier15_dry_cycles" >&2
                if [ "${_all_blocked_count:-0}" -gt 0 ] && [ "$_dry" != "1" ]; then
                    _autospec_conductor_escalate_all_blocked \
                        "$_repo" "$_queue_json" "$_all_blocked_count" "$_all_blocked_refs" "$_no_digest"
                fi
            fi

        elif [ "$_tier" = "1" ] && [ "$_action" = "run-backlog" ]; then
            local _skip_tier1_cycle=0
            # Reuse the readiness snapshot already computed in Step 2b (same
            # list-ready-issues.sh call the waterfall's --backlog-count used) —
            # do not re-query.
            if [ -n "$_list_ready" ] && [ -f "$_list_ready" ] && [ -n "$_repo" ]; then
                if [ "$_queue_cap_reached" = "true" ]; then
                    printf '[conductor] Tier-1 worker cap reached — skipping drain this cycle\n' >&2
                    _work_done=0
                    _queue_batch_len=0
                    _queue_ready_len="${_queue_ready_len:-1}"
                    _skip_tier1_cycle=1
                fi
                if [ "${_queue_ready_len:-0}" -eq 0 ] && [ "${_queue_batch_len:-0}" -eq 0 ]; then
                    if [ "${_queue_blocked_len:-0}" -gt 0 ]; then
                        printf '[conductor] Tier-1 all-blocked (%s issues) — dry cycle%s\n' \
                            "$_queue_blocked_len" \
                            "${_all_blocked_refs:+: ${_all_blocked_refs}}" >&2
                    else
                        printf '[conductor] Tier-1 queue empty — dry cycle\n' >&2
                    fi
                    _dry_cycles=$((_dry_cycles + 1))
                    _skip_tier1_cycle=1
                fi
            fi

            if [ "$_skip_tier1_cycle" != "1" ]; then
            # Pre-merge gate MUST be present and emit merge-ok (fail-closed).
            if [ ! -f "$_gate" ]; then
                printf '[conductor] HALT: autonomous-premerge-gate.sh missing at %s\n' \
                    "$_gate" >&2
                printf 'code_health:autonomous_gate_missing\n' >&2
                if [ -n "$_notify_sh" ]; then
                    bash "$_notify_sh" "autospec-autonomous" \
                        "conductor halted: premerge gate script missing" || true
                fi
                _stop_reason="code_health:autonomous_gate_missing"
                break
            fi

            # Run gate; capture last line as verdict.
            local _gate_output
            _gate_output="$(bash "$_gate" \
                ${_repo:+--repo "$_repo"} \
                ${_dry:+--dry-run} \
                2>/dev/null || true)"
            local _gate_verdict
            _gate_verdict="$(printf '%s' "$_gate_output" | tail -1 || true)"

            case "$_gate_verdict" in
                merge-ok)
                    # ── Main-health gate (spec Phase-1 invariant: red main → halt
                    # Tier-1 merges).  Never drain onto a red main.  Poll
                    # autonomous-resilience.sh main-health and honor its DECISION:
                    #   continue  → proceed with drain
                    #   wait      → skip drain this cycle (re-poll next cycle)
                    #   halt      → stop Tier-1 merges, notify, exit loop
                    # Indeterminate/absent (no resilience or no repo) → proceed:
                    # the deterministic conservatism (pending on gh failure) lives
                    # in autonomous-resilience.sh main-health itself.
                    local _main_health="continue"
                    if [ -f "$_resilience" ] && [ -n "$_repo" ]; then
                        local _mh_out
                        _mh_out="$(bash "$_resilience" main-health \
                            --repo "$_repo" 2>/dev/null || true)"
                        local _mh_decision
                        _mh_decision="$(printf '%s' "$_mh_out" \
                            | grep '^DECISION:' | head -1 \
                            | sed 's/^DECISION://' || true)"
                        if [ -n "$_mh_decision" ]; then
                            _main_health="$_mh_decision"
                        fi
                    fi

                    if [ "$_main_health" = "halt" ]; then
                        printf '[conductor] HALT: main-health red — halting Tier-1 merges\n' >&2
                        if [ -n "$_notify_sh" ]; then
                            bash "$_notify_sh" "autospec-autonomous" \
                                "conductor halted: main branch CI red — Tier-1 merges stopped" || true
                        fi
                        _stop_reason="main-health:red"
                        break
                    fi

                    if [ "$_main_health" = "wait" ]; then
                        printf '[conductor] main-health pending — skipping drain this cycle\n' >&2
                        _dry_cycles=$((_dry_cycles + 1))
                    else
                        # F3: while discovery issues are in flight, main merges are
                        # refused — but that refusal is enforced PER-PR by the phase4
                        # implementer via .autospec/explore-mode.json (PRs target the
                        # sandbox base; gh pr merge against main is refused). The
                        # conductor MUST still run the drain so discovery issues are
                        # implemented onto the sandbox and the in-flight counter clears
                        # as their outcomes are consumed below. Skipping the drain here
                        # deadlocks: the decrement lives in this same branch, so a
                        # skipped drain could never lower the counter — once raised,
                        # the counter would block every future drain forever and the
                        # discovery tier (the whole point of Phase 2) would never make
                        # progress (Phase 5.5 integration finding).
                        if [ "$_inflight_discovery" -gt 0 ]; then
                            printf 'code_health:autonomous_main_merge_refused\n' >&2
                            printf '[conductor] F3: %s discovery issue(s) in-flight — main merges refused (phase4 sandbox routing); draining onto sandbox\n' \
                                "$_inflight_discovery" >&2
                        fi
                        # Gate + main-health green — invoke drain.
                        if [ "$_dry" = "1" ]; then
                            printf '[conductor] [dry-run] would invoke autospec-run for Tier-1 drain\n' >&2
                        else
                            local _run_cmd="${AUTOSPEC_RUN_CMD:-}"
                            if [ -n "$_run_cmd" ]; then
                                bash -c "$_run_cmd" 2>&1 || true
                            else
                                printf '[conductor] WARN: AUTOSPEC_RUN_CMD not set; skipping drain\n' >&2
                            fi
                            # ── Step 5b: Record per-source outcome for discovery issues ──
                            # Tier-1 backlog issues do not write this file; recording is
                            # silently skipped when absent.  All ledger calls are best-effort
                            # and never abort the cycle.
                            local _outcome_file
                            _outcome_file="${AUTOSPEC_LAST_OUTCOME_FILE:-${_repo_root}/.autospec/last-outcome.json}"
                            if [ -f "$_outcome_file" ]; then
                                if [ -n "$_ledger_sh" ]; then
                                    local _is_disc _issue_num _source_val _outcome_val
                                    _is_disc="$(jq -r '.is_discovery // false' "$_outcome_file" 2>/dev/null || true)"
                                    if [ "$_is_disc" = "true" ]; then
                                        _issue_num="$(jq -r '.issue // 0' "$_outcome_file" 2>/dev/null || true)"
                                        _source_val="$(jq -r '.source // ""' "$_outcome_file" 2>/dev/null || true)"
                                        _outcome_val="$(jq -r '.outcome // "stalled"' "$_outcome_file" 2>/dev/null || true)"
                                        if [ -n "$_issue_num" ] && [ "$_issue_num" != "0" ] \
                                            && [ -n "$_source_val" ]; then
                                            printf '[conductor] recording discovery outcome: issue=%s source=%s outcome=%s\n' \
                                                "$_issue_num" "$_source_val" "$_outcome_val" >&2
                                            bash "$_ledger_sh" \
                                                --ledger "$_ledger_path" \
                                                --update-outcome "$_issue_num" "$_outcome_val" \
                                                2>/dev/null || true
                                        fi
                                    fi
                                else
                                    printf '[conductor] WARN: explore-ledger.sh unresolved; discarding outcome file\n' >&2
                                fi
                                # F3: decrement in-flight discovery counter when a
                                # discovery outcome is consumed, so the main-merge
                                # refusal gate clears once all filed issues are resolved.
                                local _f3_is_disc
                                _f3_is_disc="$(jq -r '.is_discovery // false' \
                                    "$_outcome_file" 2>/dev/null || echo 'false')"
                                if [ "$_f3_is_disc" = "true" ] && [ "$_inflight_discovery" -gt 0 ]; then
                                    _inflight_discovery=$((_inflight_discovery - 1))
                                    printf '[conductor] F3: discovery outcome consumed; in-flight=%s\n' \
                                        "$_inflight_discovery" >&2
                                fi
                                # Always consume the outcome file so a later cycle never
                                # re-processes a stale outcome (LOW review fix).
                                rm -f "$_outcome_file" 2>/dev/null || true
                            fi
                        fi
                        _work_done=1
                    fi
                    ;;
                block*)
                    printf '[conductor] premerge-gate blocked: %s — skipping drain this cycle\n' \
                        "$_gate_verdict" >&2
                    _dry_cycles=$((_dry_cycles + 1))
                    ;;
                halt*)
                    printf '[conductor] HALT: premerge-gate returned: %s\n' "$_gate_verdict" >&2
                    _stop_reason="premerge-gate:${_gate_verdict}"
                    break
                    ;;
                *)
                    # Unknown or empty verdict — treat as dry cycle.
                    _dry_cycles=$((_dry_cycles + 1))
                    ;;
            esac
            fi

        elif [ "$_action" = "run-architecture-improvement" ]; then
            # ── Tier 3: architecture/test-coverage/technical-debt improvement ─
            local _arch_cmd="${AUTOSPEC_ARCHITECTURE_IMPROVEMENT_CMD:-}"
            if [ -z "$_arch_cmd" ]; then
                local _explore_script="${_sdir}/autospec-explore.sh"
                if [ -f "$_explore_script" ]; then
                    _arch_cmd="bash $_explore_script --once --research-sources architecture,test-coverage,technical-debt"
                elif command -v autospec-explore >/dev/null 2>&1; then
                    _arch_cmd="autospec-explore --once --research-sources architecture,test-coverage,technical-debt"
                fi
            fi

            local _arch_out
            if [ "$_dry" = "1" ]; then
                printf '[conductor] [dry-run] would generate Tier 3 architecture/test-coverage work
' >&2
                _arch_out='{"dry":true,"filed":0,"reason":"dry-run"}'
            elif [ -n "$_arch_cmd" ]; then
                printf '[conductor] Tier 3: generating architecture/test-coverage improvement work
' >&2
                _arch_out="$(bash -c "$_arch_cmd" 2>/dev/null || printf '{"dry":true,"filed":0,"reason":"architecture-error"}')"
            else
                printf '[conductor] WARN: no Tier 3 architecture improvement command available — treating Tier 3 as dry
' >&2
                _arch_out='{"dry":true,"filed":0,"reason":"architecture-command-missing"}'
            fi

            local _arch_dry _arch_filed
            _arch_dry="$(printf '%s' "$_arch_out" | jq -r 'if has("dry") then .dry else true end' 2>/dev/null || echo 'true')"
            _arch_filed="$(printf '%s' "$_arch_out" | jq -r '.filed // 0' 2>/dev/null || echo 0)"
            printf '[conductor] Tier 3 architecture result: dry=%s filed=%s
'                 "$_arch_dry" "$_arch_filed" >&2
            if [ "$_arch_dry" = "false" ] || { [ "$_arch_filed" -gt 0 ] 2>/dev/null; }; then
                _work_done=1
            else
                _tier3_dry_cycles=$((_tier3_dry_cycles + 1))
                printf '[conductor] Tier 3 dry (tier3-dry-cycles=%s)
'                     "$_tier3_dry_cycles" >&2
            fi

        elif [ "$_action" = "run-explore-once" ] || [ "$_action" = "run-explore-once-internet" ]; then
            # ── Tier 2/4: discovery via autospec-explore --once ───────────────
            # F3: ensure explore sandbox exists before filing discovery issues.
            # Idempotent — explore-sandbox.sh is a no-op when the branch already
            # exists.  The implementer's phase4 contract reads explore-mode.json
            # and targets the sandbox base; conductor does not duplicate that logic.
            if [ -n "$_sandbox_sh" ]; then
                printf '[conductor] Tier %s: ensuring explore sandbox (F3)\n' "$_tier" >&2
                bash "$_sandbox_sh" --base main 2>/dev/null || true
            fi
            # Consult source weights before discovery ranking (best-effort, F5).
            # The ranking reads them via explore-research-cycle.sh; the conductor
            # passes AUTOSPEC_EXPLORE_LEDGER so the cycle picks up the right ledger.
            if [ -n "$_weights_bin" ] && [ -x "$_weights_bin" ]; then
                printf '[conductor] consulting source weights for discovery cycle (Tier %s)\n' \
                    "$_tier" >&2
                "$_weights_bin" --json --ledger "$_ledger_path" \
                    >/dev/null 2>&1 || true
            fi
            export AUTOSPEC_EXPLORE_LEDGER="$_ledger_path"

            # Resolve the explore command: AUTOSPEC_EXPLORE_CMD override for
            # tests, else the explore script sibling of this scripts/ dir.
            local _explore_cmd="${AUTOSPEC_EXPLORE_CMD:-}"
            if [ -z "$_explore_cmd" ]; then
                local _explore_script="${_sdir}/autospec-explore.sh"
                if [ -f "$_explore_script" ]; then
                    _explore_cmd="bash $_explore_script"
                elif command -v autospec-explore >/dev/null 2>&1; then
                    _explore_cmd="autospec-explore"
                fi
            fi

            if [ -z "$_explore_cmd" ]; then
                printf '[conductor] WARN: no explore command available for Tier %s — skipping\n' \
                    "$_tier" >&2
                if [ "$_tier" = "2" ]; then
                    _tier2_dry_cycles=$((_tier2_dry_cycles + 1))
                fi
            else
                # Build the --once invocation for the selected tier.
                local _explore_out _explore_rc _explore_err_file
                _explore_err_file="$(mktemp "${TMPDIR:-/tmp}/autospec-explore-once.XXXXXX" 2>/dev/null || echo /tmp/autospec-explore-once.$$)"
                if [ "$_dry" = "1" ]; then
                    printf '[conductor] [dry-run] would invoke explore --once for Tier %s\n' \
                        "$_tier" >&2
                    # Simulate a dry yield in dry-run mode.
                    _explore_out='{"tier":"local","proposals_seen":0,"new_candidates":0,"filed":0,"dry":true,"reason":"dry-run"}'
                    _explore_rc=0
                elif [ "$_action" = "run-explore-once-internet" ]; then
                    printf '[conductor] Tier 4: invoking explore --once --research-sources internet\n' >&2
                    # F4: wrap with priority-aware cycle when priorities are available.
                    # NOTE: capture rc via if/then/else — a bare `var="$(cmd)"`
                    # assignment aborts the whole conductor under `set -e`
                    # (autospec-autonomous.sh runs `set -eu`) when explore
                    # exits non-zero, skipping the explore_error path entirely.
                    if [ -n "$_priority_cycle_cmd" ]; then
                        if _explore_out="$(AUTOSPEC_EXPLORE_ONCE_CYCLE_CMD="$_priority_cycle_cmd" \
                            AUTOSPEC_CYCLE_SH="$_cycle_sh" \
                            AUTOSPEC_PRIORITY_MATCH_BIN="$_priority_match_sh" \
                            AUTOSPEC_PRIORITIES_FILE="$_priorities_file" \
                            AUTOSPEC_PERSONA_FILE="$_eff_persona" \
                            bash -c "$_explore_cmd --once --research-sources internet" \
                            2>"$_explore_err_file")"; then _explore_rc=0; else _explore_rc=$?; fi
                    else
                        if _explore_out="$(bash -c "$_explore_cmd --once --research-sources internet" \
                            2>"$_explore_err_file")"; then _explore_rc=0; else _explore_rc=$?; fi
                    fi
                else
                    printf '[conductor] Tier 2: invoking explore --once (local sources)\n' >&2
                    # F4: wrap with priority-aware cycle when priorities are available.
                    # (if/then/else rc capture — see note above; set -e safe.)
                    if [ -n "$_priority_cycle_cmd" ]; then
                        if _explore_out="$(AUTOSPEC_EXPLORE_ONCE_CYCLE_CMD="$_priority_cycle_cmd" \
                            AUTOSPEC_CYCLE_SH="$_cycle_sh" \
                            AUTOSPEC_PRIORITY_MATCH_BIN="$_priority_match_sh" \
                            AUTOSPEC_PRIORITIES_FILE="$_priorities_file" \
                            AUTOSPEC_PERSONA_FILE="$_eff_persona" \
                            bash -c "$_explore_cmd --once" \
                            2>"$_explore_err_file")"; then _explore_rc=0; else _explore_rc=$?; fi
                    else
                        if _explore_out="$(bash -c "$_explore_cmd --once" \
                            2>"$_explore_err_file")"; then _explore_rc=0; else _explore_rc=$?; fi
                    fi
                fi

                # A non-zero exit means explore crashed or never started (bad
                # invocation, missing dep, misconfig) — this is a VISIBLE
                # health signal, not a silent dry (issue #1625). Only a clean
                # exit (rc=0) reporting dry:true is a genuine "no candidates".
                local _explore_is_error=0
                if [ "$_explore_rc" -ne 0 ]; then
                    _explore_is_error=1
                    local _explore_err_tail
                    _explore_err_tail="$(tail -n 5 "$_explore_err_file" 2>/dev/null || true)"
                    printf '[conductor] Tier %s explore ERROR: exit=%s code_health:explore_error\n' \
                        "$_tier" "$_explore_rc" >&2
                    if [ -n "$_explore_err_tail" ]; then
                        printf '[conductor] Tier %s explore stderr:\n%s\n' \
                            "$_tier" "$_explore_err_tail" >&2
                    fi
                    _explore_out='{"dry":true,"filed":0,"reason":"explore-error"}'
                fi
                rm -f "$_explore_err_file" 2>/dev/null || true

                # Parse yield JSON for dryness.
                local _explore_dry
                _explore_dry="$(printf '%s' "$_explore_out" \
                    | jq -r 'if has("dry") then .dry else true end' 2>/dev/null || echo 'true')"
                local _explore_filed
                _explore_filed="$(printf '%s' "$_explore_out" \
                    | jq -r '.filed // 0' 2>/dev/null || echo 0)"

                if [ "$_explore_is_error" -eq 1 ]; then
                    printf '[conductor] Tier %s explore result: ERROR (exit=%s) — not a clean dry\n' \
                        "$_tier" "$_explore_rc" >&2
                else
                    printf '[conductor] Tier %s explore result: dry=%s filed=%s\n' \
                        "$_tier" "$_explore_dry" "$_explore_filed" >&2
                fi

                if [ "$_explore_dry" = "false" ]; then
                    # New candidates were filed — they become Tier-1 backlog.
                    # Float selection back up: reset both dry counters so the
                    # waterfall selects Tier 1 next cycle.
                    printf '[conductor] Tier %s filed %s candidate(s) — floating back to Tier 1\n' \
                        "$_tier" "$_explore_filed" >&2
                    _work_done=1
                    # F3: track in-flight discovery issues so the main-merge
                    # refusal gate blocks Tier-1 drain until they are consumed.
                    if [ "$_explore_filed" -gt 0 ] 2>/dev/null; then
                        _inflight_discovery=$((_inflight_discovery + _explore_filed))
                        printf '[conductor] F3: %s discovery issue(s) now in-flight (total=%s)\n' \
                            "$_explore_filed" "$_inflight_discovery" >&2
                    fi
                else
                    if [ "$_explore_is_error" -eq 1 ]; then
                        # Crash/misconfig, not a genuine "no candidates" — still
                        # counts as non-productive for waterfall progression,
                        # but logged distinctly from a clean dry (issue #1625).
                        if [ "$_tier" = "2" ]; then
                            _tier2_dry_cycles=$((_tier2_dry_cycles + 1))
                            printf '[conductor] Tier 2 explore_error (tier2-dry-cycles=%s)\n' \
                                "$_tier2_dry_cycles" >&2
                        else
                            _tier4_dry_cycles=$((_tier4_dry_cycles + 1))
                            printf '[conductor] Tier 4 explore_error (tier4-dry-cycles=%s)\n' \
                                "$_tier4_dry_cycles" >&2
                        fi
                    else
                        # Explore was genuinely dry for this tier.
                        if [ "$_tier" = "2" ]; then
                            _tier2_dry_cycles=$((_tier2_dry_cycles + 1))
                            printf '[conductor] Tier 2 dry (tier2-dry-cycles=%s)\n' \
                                "$_tier2_dry_cycles" >&2
                        else
                            _tier4_dry_cycles=$((_tier4_dry_cycles + 1))
                            printf '[conductor] Tier 4 dry (tier4-dry-cycles=%s)\n' \
                                "$_tier4_dry_cycles" >&2
                        fi
                    fi
                fi
            fi
        else
            # Unknown action — log and treat as dry cycle.
            printf '[conductor] unknown waterfall action=%s for tier=%s — skipping\n' \
                "$_action" "$_tier" >&2
            _dry_cycles=$((_dry_cycles + 1))
        fi

        # Any successful tier work floats the next selection back to Tier 1.
        if [ "$_work_done" -eq 1 ]; then
            _dry_cycles=0
            _tier15_dry_cycles=0
            _tier2_dry_cycles=0
            _tier3_dry_cycles=0
            _tier4_dry_cycles=0
        fi

        # ── Step 6a: Usage governor soft-park (F6) ───────────────────────────
        # Soft-park at AUTOSPEC_USAGE_SOFT_PCT (default 90%) BEFORE the Phase-1
        # hard-quota backstop (Step 6) fires. The governor prefers a live usage
        # fraction (usage-observe.sh) and falls back to the spend-ledger token
        # tally. It is fail-open ("continue" on any error), so a missing/older
        # install never blocks the loop; it only ever ADDS an earlier park.
        if [ -f "$_governor" ]; then
            local _gov_harness="${AUTOSPEC_GOVERNOR_HARNESS:-${AUTOSPEC_HARNESS:-claude}}"
            case "$_gov_harness" in
                claude|codex|opencode) ;;
                *) _gov_harness="claude" ;;
            esac
            local _gov_out
            _gov_out="$(bash "$_governor" "$_gov_harness" \
                --repo-dir "$_repo_root" 2>/dev/null || printf 'continue')"
            case "$_gov_out" in
                park*)
                    printf '[conductor] usage-governor: %s — arming resume and exiting\n' \
                        "$_gov_out" >&2
                    _conductor_arm_resume \
                        "$_sdir" "$_repo" "$_conductor_session" \
                        "$_notify_sh" "usage-governor:${_gov_out}"
                    _stop_reason="usage-governor:park"
                    break
                    ;;
            esac
        fi

        # ── Step 6: Spend-ledger tally (autonomous-spend-ledger.sh) ──────────
        if [ -f "$_spend" ]; then
            if [ "$_dry" != "1" ]; then
                bash "$_spend" add \
                    --tokens 0 \
                    --issues "$_work_done" \
                    2>/dev/null || true
            fi
            local _spend_check
            _spend_check="$(bash "$_spend" check 2>/dev/null || echo "continue")"
            case "$_spend_check" in
                park*)
                    printf '[conductor] spend-ledger: %s — arming resume and exiting\n' \
                        "$_spend_check" >&2
                    _conductor_arm_resume \
                        "$_sdir" "$_repo" "$_conductor_session" \
                        "$_notify_sh" "$_spend_check"
                    _stop_reason="spend-ledger:park"
                    break
                    ;;
            esac
        fi

        # ── Step 7: Once-per-UTC-day digest ───────────────────────────────────
        local _new_day
        _new_day="$(_conductor_maybe_write_digest \
            "$_no_digest" "$_last_digest_day" "$_sdir" "$_repo" "$_dry" 2>&1 \
            | tail -1 || printf '%s' "$_last_digest_day")"
        # _new_day stdout is the updated day; progress log went to stderr.
        # Re-capture cleanly by calling the helper with stderr suppressed.
        _last_digest_day="$(_conductor_maybe_write_digest \
            "$_no_digest" "$_last_digest_day" "$_sdir" "$_repo" "$_dry" \
            2>/dev/null || printf '%s' "$_last_digest_day")"

        # ── Cycle cap ─────────────────────────────────────────────────────────
        if [ "$_max" -gt 0 ] 2>/dev/null; then
            if [ "$_cycle" -ge "$_max" ]; then
                _stop_reason="max-cycles-reached"
                break
            fi
        fi

        # ── Poll interval ─────────────────────────────────────────────────────
        if [ "$_dry" != "1" ] && [ "$_poll" -gt 0 ] 2>/dev/null; then
            sleep "$_poll" || true
        fi
    done

    _autospec_conductor_record_stop "${_stop_reason:-unknown}" "$_cycle" normal
    _autospec_conductor_release_lock
    trap - EXIT HUP INT QUIT TERM
}

# _conductor_maybe_write_digest: write the daily digest when the UTC day changes.
# Prints the current UTC day on stdout (new or unchanged) so the caller can
# track the last-written day.  Log lines go to stderr.
_conductor_maybe_write_digest() {
    local no_digest="$1"
    local last_day="$2"
    local sdir="$3"
    local repo="$4"
    local dry="$5"

    if [ "$no_digest" = "1" ]; then
        printf '%s' "$last_day"
        return 0
    fi

    local today
    today="$(date -u +'%Y-%m-%d' 2>/dev/null || echo "unknown")"

    if [ "$today" = "$last_day" ]; then
        # Same UTC day — no-op.
        printf '%s' "$last_day"
        return 0
    fi

    if [ "$dry" = "1" ]; then
        printf '[conductor] [dry-run] would write digest for UTC day %s\n' "$today" >&2
        printf '%s' "$today"
        return 0
    fi

    # Resolve repo root as the parent of sdir (scripts/).
    local repo_root
    repo_root="$(cd "${sdir}/.." 2>/dev/null && pwd || printf '.')"
    local digest_file="${repo_root}/.autospec/autonomous-digest.md"
    mkdir -p "$(dirname "$digest_file")" 2>/dev/null || true

    # Compute sandbox-to-main drift (reporting only — never rebases).
    local _drift_section=""
    _drift_section="$(_conductor_sandbox_drift_section "$repo_root" 2>/dev/null || true)"

    # Resolve persona and priorities files (env overrides for testing).
    local _persona_file="${AUTOSPEC_PERSONA_FILE:-}"
    if [ -z "$_persona_file" ]; then
        if [ -f "${repo_root}/.autospec/operator-persona.effective.md" ]; then
            _persona_file="${repo_root}/.autospec/operator-persona.effective.md"
        else
            _persona_file="${HOME}/.autospec/operator-persona.md"
        fi
    fi
    local _priorities_file="${AUTOSPEC_PRIORITIES_FILE:-${HOME}/.autospec/autonomous-priorities.md}"

    # Build persona block (fail-soft: missing file → "not yet built").
    local _persona_section=""
    _persona_section="$(_conductor_digest_persona_section "$_persona_file" 2>/dev/null || true)"

    # Build priorities block (fail-soft: missing/empty file → omit block).
    local _priorities_section=""
    _priorities_section="$(_conductor_digest_priorities_section "$_priorities_file" 2>/dev/null || true)"

    {
        printf '## autospec-autonomous daily digest — %s\n\n' "$today"
        printf 'Conductor: `autospec_conductor_run` (scripts/lib/autospec-loop.sh)\n\n'
        if [ -n "$repo" ]; then
            printf 'Repo: %s\n' "$repo"
        fi
        if [ -n "$_drift_section" ]; then
            printf '\n%s\n' "$_drift_section"
        fi
        if [ -n "$_persona_section" ]; then
            printf '\n%s\n' "$_persona_section"
        fi
        if [ -n "$_priorities_section" ]; then
            printf '\n%s\n' "$_priorities_section"
        fi
        printf '\n_Generated by autospec-autonomous Phase-1 conductor._\n'
    } > "$digest_file" || true
    printf '[conductor] daily digest written to %s\n' "$digest_file" >&2
    printf '%s' "$today"
}

# _conductor_digest_persona_section PERSONA_FILE
#   Emit a markdown persona block for the daily digest.
#   Fail-soft: if the persona file is absent, emit a "not yet built" notice.
#   Reads: last mtime (last refresh), per-dimension confidence lines,
#   and calibration-agreement % (from the confidence section).
_conductor_digest_persona_section() {
    local _pfile="$1"

    printf '### Operator persona\n\n'

    if [ ! -f "$_pfile" ]; then
        printf '_Persona not yet built. Run `/autospec-persona` to calibrate._\n'
        return 0
    fi

    # Last refresh: file mtime formatted as UTC date.
    local _last_refresh
    _last_refresh="$(date -u -r "$_pfile" +'%Y-%m-%dT%H:%M:%SZ' 2>/dev/null \
        || stat -c '%y' "$_pfile" 2>/dev/null | sed 's/ .*//' \
        || echo 'unknown')"
    printf -- '- **Last refresh:** %s\n' "$_last_refresh"

    # Per-dimension confidence: lines matching "- <Dim>: <level>" inside the
    # "## Confidence (per dimension)" section.
    local _in_conf=0 _conf_lines="" _total_dims=0 _high_dims=0
    while IFS= read -r _line; do
        if printf '%s' "$_line" | grep -q '^## Confidence (per dimension)'; then
            _in_conf=1
            continue
        fi
        if [ "$_in_conf" = "1" ]; then
            # Stop at the next ## heading.
            if printf '%s' "$_line" | grep -q '^## '; then
                _in_conf=0
                continue
            fi
            if printf '%s' "$_line" | grep -q '^- '; then
                _total_dims=$(( _total_dims + 1 ))
                if printf '%s' "$_line" | grep -q ': high$'; then
                    _high_dims=$(( _high_dims + 1 ))
                fi
                _conf_lines="${_conf_lines}  ${_line}
"
            fi
        fi
    done < "$_pfile"

    if [ -n "$_conf_lines" ]; then
        printf -- '- **Per-dimension confidence:**\n'
        printf '%s' "$_conf_lines"
    else
        printf -- '- **Per-dimension confidence:** _unavailable (persona not yet synthesized with confidence notes)_\n'
    fi

    # Calibration-agreement %: fraction of dimensions rated "high".
    local _cal_pct="n/a"
    if [ "$_total_dims" -gt 0 ]; then
        _cal_pct="$(( _high_dims * 100 / _total_dims ))%"
    fi
    printf -- '- **Calibration-agreement:** %s\n' "$_cal_pct"
}

# _conductor_digest_priorities_section PRIORITIES_FILE
#   Emit a markdown priorities block for the daily digest.
#   Lists active priorities and any biased filed work captured by F4.
#   Fail-soft: missing or empty file → empty output (caller skips the block).
_conductor_digest_priorities_section() {
    local _pfile="$1"

    if [ ! -f "$_pfile" ] || [ ! -s "$_pfile" ]; then
        return 0
    fi

    printf '### Active priorities\n\n'

    # Emit bullet priorities (lines starting with "- ").
    local _has_priorities=0
    while IFS= read -r _line; do
        if printf '%s' "$_line" | grep -q '^- '; then
            printf '%s\n' "$_line"
            _has_priorities=1
        fi
    done < "$_pfile"

    if [ "$_has_priorities" = "0" ]; then
        printf '_No active operator priorities._\n'
    fi

    # Biased filed work: PRIORITY_ISSUE: and DIRECTIVE: entries captured by F4.
    local _bias_lines=""
    while IFS= read -r _line; do
        case "$_line" in
            PRIORITY_ISSUE:*|DIRECTIVE:*)
                _bias_lines="${_bias_lines}  - ${_line}
"
                ;;
        esac
    done < "$_pfile"

    if [ -n "$_bias_lines" ]; then
        printf '\n**Biased filed work (captured control-channel payloads):**\n'
        printf '%s' "$_bias_lines"
    fi
}

# _conductor_sandbox_drift_section: compute sandbox-to-main merge-base distance
# and a conflict-risk estimate.  Outputs a markdown section; never rebases.
# Fail-open: any git error produces a minimal "unavailable" line.
_conductor_sandbox_drift_section() {
    local repo_root="$1"
    local mode_file="${repo_root}/.autospec/explore-mode.json"

    # If no explore-mode.json, nothing to report.
    if [ ! -f "$mode_file" ]; then
        return 0
    fi

    local sandbox_branch base_branch
    sandbox_branch="$(grep -o '"branch"[[:space:]]*:[[:space:]]*"[^"]*"' "$mode_file" \
        | sed 's/.*"branch"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/' | head -1)"
    base_branch="$(grep -o '"base"[[:space:]]*:[[:space:]]*"[^"]*"' "$mode_file" \
        | sed 's/.*"base"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/' | head -1)"
    base_branch="${base_branch:-main}"

    if [ -z "$sandbox_branch" ]; then
        return 0
    fi

    # Run git commands from the repo root.
    local merge_base commits_behind sandbox_files conflict_risk

    merge_base="$(git -C "$repo_root" merge-base \
        "origin/${base_branch}" "$sandbox_branch" 2>/dev/null || true)"

    if [ -z "$merge_base" ]; then
        printf '### Sandbox drift\n\n_Merge-base unavailable (sandbox branch not fetched)._\n'
        return 0
    fi

    commits_behind="$(git -C "$repo_root" rev-list \
        --count "${merge_base}..origin/${base_branch}" 2>/dev/null || echo "?")"

    # Files changed in main since merge-base.
    local tmp_base tmp_sandbox
    tmp_base="$(mktemp)"
    tmp_sandbox="$(mktemp)"
    git -C "$repo_root" diff --name-only \
        "${merge_base}" "origin/${base_branch}" 2>/dev/null \
        | sort > "$tmp_base" || true
    # Files changed in sandbox since merge-base.
    git -C "$repo_root" diff --name-only \
        "${merge_base}" "$sandbox_branch" 2>/dev/null \
        | sort > "$tmp_sandbox" || true

    sandbox_files="$(wc -l < "$tmp_sandbox" | tr -d ' ')"
    # Overlap = potential conflict risk.
    local overlap
    overlap="$(comm -12 "$tmp_base" "$tmp_sandbox" 2>/dev/null | wc -l | tr -d ' ')"
    rm -f "$tmp_base" "$tmp_sandbox"

    if [ "$overlap" -gt 0 ] 2>/dev/null; then
        if [ "$overlap" -gt 5 ] 2>/dev/null; then
            conflict_risk="high (${overlap} overlapping files)"
        elif [ "$overlap" -gt 0 ] 2>/dev/null; then
            conflict_risk="medium (${overlap} overlapping files)"
        fi
    else
        conflict_risk="low (0 overlapping files)"
    fi

    printf '### Sandbox drift (sandbox → %s)\n\n' "$base_branch"
    printf '| Metric | Value |\n'
    printf '|--------|-------|\n'
    printf '| Sandbox branch | `%s` |\n' "$sandbox_branch"
    printf '| Commits behind `%s` | %s |\n' "$base_branch" "$commits_behind"
    printf '| Files changed in sandbox | %s |\n' "$sandbox_files"
    printf '| Conflict-risk estimate | %s |\n' "$conflict_risk"
    printf '\n> Reporting only — no auto-rebase. Operator action required to promote.\n'
}

# _conductor_arm_resume: write resume context and arm ScheduleWakeup/cron via
# autospec-usage-limit.sh.  Fail-open: errors must never block the exit path.
_conductor_arm_resume() {
    local sdir="$1"
    local repo="$2"
    local session="$3"
    local notify_sh="$4"
    local reason="$5"

    printf '[conductor] arming resume context for: %s\n' "$reason" >&2

    local usage_limit="${sdir}/autospec-usage-limit.sh"
    if [ -f "$usage_limit" ]; then
        local run_id="conductor-${session}"
        local repo_root
        repo_root="$(cd "${sdir}/.." 2>/dev/null && pwd || printf '.')"
        local resume_cmd="bash ${sdir}/autospec-autonomous.sh --resume --run-id ${run_id}"
        local wait_secs="${AUTOSPEC_RESUME_WAIT_SECS:-3600}"
        bash "$usage_limit" arm \
            --harness "autonomous" \
            --repo-dir "$repo_root" \
            --command "$resume_cmd" \
            --wait-seconds "$wait_secs" \
            --run-id "$run_id" \
            --no-daemon \
            2>/dev/null || true
    fi

    if [ -n "$notify_sh" ]; then
        bash "$notify_sh" "autospec-autonomous" \
            "conductor parked: ${reason} — resume armed" || true
    fi
}
