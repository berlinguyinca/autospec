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
#   6. Spend-ledger tally (autonomous-spend-ledger.sh); park on cap
#   7. Once-per-UTC-day digest to .autospec/autonomous-digest.md + pinned issue
#   8. On park (spend/usage): arm ScheduleWakeup/cron via autospec-usage-limit.sh
#   9. Tier 2+ discovery (not yet enabled in Phase 1): consult explore-source-
#      weights.sh before the discovery cycle ranks proposals, then park.
#      When F2 enables Tier 2, the cycle runs here with AUTOSPEC_EXPLORE_LEDGER set.
#
# Tiers 2-4 are not enabled in Phase 1; a dry Tier-1 parks and notifies.
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
autospec_conductor_run() {
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

    local _cycle=0
    local _dry_cycles=0
    local _last_digest_day=""
    local _stop_reason=""
    local _conductor_session="${AUTOSPEC_SESSION_ID:-conductor-$$}"

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
            return 1
        fi
    fi

    # ── Main cycle loop ───────────────────────────────────────────────────────
    while true; do
        _cycle=$((_cycle + 1))
        printf '[conductor] cycle %s starting\n' "$_cycle" >&2

        # ── Step 1: Resilience heartbeat ──────────────────────────────────────
        if [ -f "$_resilience" ] && [ -n "$_repo" ]; then
            bash "$_resilience" state write \
                --repo "$_repo" \
                --status "running:cycle-${_cycle}" \
                --session "$_conductor_session" \
                2>/dev/null || true
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
            esac
        fi

        # ── Step 3: Waterfall tier selection ──────────────────────────────────
        local _tier_json
        _tier_json="$(bash "$_waterfall" \
            --dry-cycles "$_dry_cycles" \
            ${_repo:+--repo "$_repo"} \
            2>/dev/null \
            || printf '{"tier":1,"action":"run-backlog","reason":"waterfall-unavailable"}')"
        local _tier
        _tier="$(printf '%s' "$_tier_json" \
            | jq -r '.tier // 1' 2>/dev/null || echo 1)"
        local _action
        _action="$(printf '%s' "$_tier_json" \
            | jq -r '.action // "run-backlog"' 2>/dev/null || echo "run-backlog")"

        printf '[conductor] tier=%s action=%s\n' "$_tier" "$_action" >&2

        # ── Step 4 + 5: Tier-1 drain gated on premerge check ─────────────────
        local _work_done=0
        if [ "$_tier" = "1" ] && [ "$_action" = "run-backlog" ]; then

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
                                # Always consume the outcome file so a later cycle never
                                # re-processes a stale outcome (LOW review fix).
                                rm -f "$_outcome_file" 2>/dev/null || true
                            fi
                        fi
                        _work_done=1
                        _dry_cycles=0
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

        else
            # Tier 2+ are not enabled in Phase 1.
            # Consult source weights before discovery ranking (best-effort).
            # The ranking already reads them via explore-research-cycle.sh;
            # the conductor passes AUTOSPEC_EXPLORE_LEDGER so the cycle picks
            # up the right ledger when F2 enables Tier 2.
            if [ -n "$_weights_bin" ] && [ -x "$_weights_bin" ]; then
                printf '[conductor] consulting source weights for discovery cycle (Tier %s)\n' \
                    "$_tier" >&2
                "$_weights_bin" --json --ledger "$_ledger_path" \
                    >/dev/null 2>&1 || true
            fi
            export AUTOSPEC_EXPLORE_LEDGER="$_ledger_path"
            # Park and notify.
            printf '[conductor] Tier %s not enabled in Phase 1; dry-cycles=%s — parking\n' \
                "$_tier" "$_dry_cycles" >&2
            if [ -n "$_notify_sh" ]; then
                bash "$_notify_sh" "autospec-autonomous" \
                    "conductor parked: backlog dry (tier $((${_tier} - 1)) exhausted); Tier 2+ not enabled" \
                    || true
            fi
            _stop_reason="phase1:tier${_tier}-not-enabled"
            break
        fi

        # ── Step 6: Spend-ledger tally (autonomous-spend-ledger.sh) ──────────
        if [ -f "$_spend" ]; then
            bash "$_spend" add \
                --tokens 0 \
                --issues "$_work_done" \
                2>/dev/null || true
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

    # Release conductor lock on exit.
    if [ -f "$_resilience" ] && [ -n "$_repo" ]; then
        bash "$_resilience" lock release \
            --repo "$_repo" \
            --session "$_conductor_session" \
            2>/dev/null || true
    fi

    printf '[conductor] stopped: %s (cycles=%s)\n' \
        "${_stop_reason:-unknown}" "$_cycle" >&2
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

    {
        printf '## autospec-autonomous daily digest — %s\n\n' "$today"
        printf 'Conductor: `autospec_conductor_run` (scripts/lib/autospec-loop.sh)\n\n'
        if [ -n "$repo" ]; then
            printf 'Repo: %s\n' "$repo"
        fi
        if [ -n "$_drift_section" ]; then
            printf '\n%s\n' "$_drift_section"
        fi
        printf '\n_Generated by autospec-autonomous Phase-1 conductor._\n'
    } > "$digest_file" || true
    printf '[conductor] daily digest written to %s\n' "$digest_file" >&2
    printf '%s' "$today"
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
