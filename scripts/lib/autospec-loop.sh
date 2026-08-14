#!/usr/bin/env bash
# linter:allow-COMPLEXITY existing conductor monolith; this fix is a narrow state-boundary repair
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

# Keep loop-summary row formatting identical across success and error paths.
_autospec_loop_append_table_row() {
    local rows="$1" iteration="$2" source="$3" harvested="$4" merged_prs="$5" status="$6" row
    row="$(printf '| %4d | %-21s | %-60s | %10s | %4s | %-20s |' \
        "$iteration" "$(printf '%s' "$source" | head -c 21)" \
        "$(printf '%s' "$harvested" | head -c 60)" "$merged_prs" "-" "$status")"
    if [ -z "$rows" ]; then printf '%s' "$row"; else printf '%s\n%s' "$rows" "$row"; fi
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
            table_rows="$(_autospec_loop_append_table_row "$table_rows" "$iter" "$cur_source" \
                "handoff failed rc=$refine_status" "0" "iteration_error")"
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
                table_rows="$(_autospec_loop_append_table_row "$table_rows" "$iter" "$cur_source" \
                    "$stale_reason" "0" "iteration_error")"
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

        table_rows="$(_autospec_loop_append_table_row "$table_rows" "$iter" "$cur_source" \
            "$row_harvested_short" "0" "$row_status")"

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
#   AUTOSPEC_PROVENANCE_BIN         explicit path to autonomous-provenance.sh (for tests)
#   AUTOSPEC_INTEGRATION_BRANCH_BIN explicit path to autonomous-integration-branch.sh (for tests)
#
# Dispatch-time provenance split (integration-branch design §Architecture 5):
# when autonomous-provenance.sh + autonomous-integration-branch.sh resolve and
# a repo slug is known, the Tier-1 drain resolves each batch issue's provenance
# from GitHub labels (re-derived every cycle; no session memory) and dispatches
# the operator subset (mode file parked; PRs target the parent) separately from
# the self subset (kind=integration mode file active; PRs target the
# integration branch). Each dispatch exports AUTOSPEC_RUN_ONLY_ISSUES — a
# space-separated issue-number list scoping that run invocation.
#
# Safety rules (AGENTS.md):
#   set -eu; if/then/fi for one-sided conditionals; no RETURN traps;
#   jq: use capture()/== never interpolated test() for dynamic values.

# _autospec_conductor_accountability_event: append through the Rust-owned private journal.
# A journal failure is returned to the caller; remote projection failures are degraded
# inside the Rust command after the local append succeeds.
_autospec_conductor_accountability_event() {
    local kind="$1" what="$2" why="$3" evidence="$4" project="${5:-0}"
    local repo="${_AUTOSPEC_CONDUCTOR_REPO:-}"
    local bin="${_AUTOSPEC_CONDUCTOR_ACCOUNTABILITY_BIN:-}"
    [ -n "$repo" ] || return 0
    [ -n "$bin" ] || return 0
    if [ ! -x "$bin" ] && ! command -v "$bin" >/dev/null 2>&1; then
        return 0
    fi
    local slug state_root launch
    slug="$(printf '%s' "$repo" | tr '/:' '__')"
    state_root="${AUTOSPEC_AUTONOMOUS_OPERATOR_DIR:-$HOME/.autospec/autonomous-operator}"
    launch="$state_root/$slug/launch.json"
    if [ ! -f "$launch" ] || ! command -v jq >/dev/null 2>&1 \
        || ! jq -e '.accountability.run_id | strings | length > 0' "$launch" >/dev/null 2>&1; then
        return 0
    fi
    local args=(autonomous accountability-event --repo "$repo" --kind "$kind" \
        --what "$what" --why "$why" --evidence "$evidence")
    [ "$project" = "1" ] && args+=(--project)
    "$bin" "${args[@]}" >/dev/null
}

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
    local accountability_kind="stopped"
    case "$reason" in
        *park*) accountability_kind="parked" ;;
        all-done|completed) accountability_kind="completed" ;;
    esac
    _autospec_conductor_accountability_event "$accountability_kind" \
        "Conductor stopped after ${cycle} cycle(s)" \
        "The terminal boundary records why autonomous mutation ended" \
        "$reason" 1 || printf '[conductor] WARN: accountability terminal event journal failed\n' >&2
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

_autospec_conductor_all_blocked_single_reason() {
    printf '%s' "$1" \
        | jq -r '
            [ .blocked[]? | (.reason // "blocked") ] as $reasons
            | if (($reasons | length) > 0 and ($reasons | unique | length) == 1)
              then $reasons[0]
              else ""
              end
        ' 2>/dev/null || true
}

_autospec_conductor_queue_count() {
    printf '%s' "$1" | jq -r "$2" 2>/dev/null || true
}

# ── Control-channel promote/discard consumers (integration-branch design §8) ──
# Trust for `promote` AND `discard` is already vetted upstream by
# autonomous-control-channel.sh (issue author + last labeled-event actor both
# checked against safety.issue_intent_gate.trusted_actors, fail closed);
# these consumers act on the DECISION the control channel already produced.
# All gh/git calls are best-effort (fail-open comments) — a GitHub hiccup
# here must never crash the outer conductor loop.
#
# Trigger-state hygiene (memory: one-shot→tier must clear trigger state):
# EVERY terminal path — refused, no-op, success, failure — removes the
# autospec:promote / autospec:discard label after commenting, so the control
# channel does not re-emit the same decision every cycle (duplicate comments
# + severity starvation of lower tiers). The operator re-fires by re-applying
# the label.

# _autospec_conductor_default_branch REPO — repo default branch, `main`
# fallback when GitHub does not expose a defaultBranchRef.
_autospec_conductor_default_branch() {
    local repo="$1" b
    b="$(gh repo view "$repo" --json defaultBranchRef --jq '.defaultBranchRef.name // empty' 2>/dev/null || true)"
    if [ -n "$b" ]; then
        printf '%s\n' "$b"
    else
        printf 'main\n'
    fi
}

# _autospec_conductor_clear_control_label REPO ISSUE LABEL — best-effort
# trigger-state clear; never fails the caller.
_autospec_conductor_clear_control_label() {
    local repo="$1" issue="$2" label="$3"
    if [ -n "$repo" ] && [ -n "$issue" ]; then
        gh issue edit "$issue" --repo "$repo" --remove-label "$label" >/dev/null 2>&1 || true
    fi
}

# _autospec_conductor_clear_self_pause REPO_ROOT — promote/discard are the
# designed operator exits from a rollup-red/caps park (#1767): clear the
# durable pause marker so self-originated tiers resume next cycle.
_autospec_conductor_clear_self_pause() {
    local repo_root="$1"
    if [ -n "$repo_root" ]; then
        rm -f "${repo_root}/.autospec/self-originated-pause.json" 2>/dev/null || true
    fi
}

_autospec_conductor_discard_pending_file() {
    local repo_root="$1"
    [ -n "$repo_root" ] || return 1
    printf '%s/.autospec/discard-pending.json\n' "$repo_root"
}

_autospec_conductor_write_discard_pending() {
    local repo_root="$1" issue="$2" rollup_pr="$3" rolled_issues="$4"
    local pending_file pending_dir tmp updated_at
    pending_file="$(_autospec_conductor_discard_pending_file "$repo_root" 2>/dev/null || true)"
    [ -n "$pending_file" ] || return 1
    pending_dir="$(dirname "$pending_file")"
    mkdir -p "$pending_dir" 2>/dev/null || return 1
    tmp="$(mktemp "${pending_file}.XXXXXX" 2>/dev/null || true)"
    [ -n "$tmp" ] || return 1
    updated_at="$(date -u +'%Y-%m-%dT%H:%M:%SZ' 2>/dev/null || printf unknown)"
    if printf '%s\n' "$rolled_issues" \
        | AUTOSPEC_DISCARD_CONTROL_ISSUE="$issue" \
            AUTOSPEC_DISCARD_ROLLUP_PR="$rollup_pr" \
            AUTOSPEC_DISCARD_UPDATED_AT="$updated_at" \
            jq -R -s \
        '{
            control_issue: (env.AUTOSPEC_DISCARD_CONTROL_ISSUE | tonumber),
            rollup_pr: (env.AUTOSPEC_DISCARD_ROLLUP_PR | tonumber),
            pending_issues: (split("\n") | map(select(length > 0) | tonumber)),
            updated_at: env.AUTOSPEC_DISCARD_UPDATED_AT
        }' > "$tmp" 2>/dev/null; then
        mv "$tmp" "$pending_file"
    else
        rm -f "$tmp" 2>/dev/null || true
        return 1
    fi
}

_autospec_conductor_read_discard_pending() {
    local repo_root="$1" issue="$2" pending_file
    pending_file="$(_autospec_conductor_discard_pending_file "$repo_root" 2>/dev/null || true)"
    [ -n "$pending_file" ] && [ -f "$pending_file" ] || return 1
    AUTOSPEC_DISCARD_CONTROL_ISSUE="$issue" jq -er '
        select((.control_issue // 0) == (env.AUTOSPEC_DISCARD_CONTROL_ISSUE | tonumber))
        | (.rollup_pr | tostring),
          ((.pending_issues // []) | map(tostring) | join("\n"))
    ' "$pending_file" 2>/dev/null
}

_autospec_conductor_clear_discard_pending() {
    local repo_root="$1" issue="$2" pending_file
    pending_file="$(_autospec_conductor_discard_pending_file "$repo_root" 2>/dev/null || true)"
    [ -n "$pending_file" ] && [ -f "$pending_file" ] || return 0
    if AUTOSPEC_DISCARD_CONTROL_ISSUE="$issue" jq -e \
        '(.control_issue // 0) == (env.AUTOSPEC_DISCARD_CONTROL_ISSUE | tonumber)' \
        "$pending_file" >/dev/null 2>&1; then
        rm -f "$pending_file" 2>/dev/null || true
    fi
}

# _autospec_conductor_rollup_ci_green REPO PR — exit 0 only when the roll-up
# PR's status checks are fully settled with no failures. An empty rollup (no
# CI configured) counts as green; a failed probe fails CLOSED (refuse the
# merge — promote must never admin-merge red/unknown CI; the operator's
# manual GitHub merge stays the explicit override). Green is an ALLOWLIST of
# explicitly-good terminal conclusions (SUCCESS/NEUTRAL/SKIPPED) — a
# blocklist would fail open on enum values it doesn't know about
# (STARTUP_FAILURE, STALE, future additions). conclusion==null (still
# running, or a legacy status context reporting `state` instead) counts as
# unsettled and refuses — fail closed.
_autospec_conductor_rollup_ci_green() {
    local repo="$1" pr="$2" rollup not_green
    rollup="$(gh pr view "$pr" --repo "$repo" --json statusCheckRollup --jq '.statusCheckRollup // []' 2>/dev/null || true)"
    if [ -z "$rollup" ]; then
        return 1
    fi
    not_green="$(printf '%s' "$rollup" | jq '[.[] | select((.conclusion=="SUCCESS" or .conclusion=="NEUTRAL" or .conclusion=="SKIPPED") | not)] | length' 2>/dev/null || echo 1)"
    [ "$not_green" = "0" ]
}

# _autospec_conductor_handle_promote REPO INTBRANCH_SH ISSUE REPO_ROOT —
# merge the open, CI-green roll-up PR (integration branch -> parent) and
# reset the integration branch. A missing/absent roll-up is a clean no-op
# (comment only, no merge call). A MERGED roll-up re-attempts `reset` so a
# prior merge-ok/reset-fail promote is recoverable by re-firing promote.
_autospec_conductor_handle_promote() {
    local repo="$1" intbranch_sh="$2" issue="$3" repo_root="${4:-}"
    if [ -z "$issue" ]; then
        return 0
    fi
    if [ -z "$repo" ] || [ -z "$intbranch_sh" ]; then
        return 0
    fi

    local parent status_json rollup_pr rollup_state
    parent="$(_autospec_conductor_default_branch "$repo")"
    status_json="$(bash "$intbranch_sh" status --parent "$parent" --repo "$repo" 2>/dev/null || true)"
    rollup_pr="$(printf '%s' "$status_json" | jq -r '.rollup_pr.number // empty' 2>/dev/null || true)"
    rollup_state="$(printf '%s' "$status_json" | jq -r '.rollup_pr.state // empty' 2>/dev/null || true)"

    case "$rollup_pr" in
        ''|*[!0-9]*)
            printf '[conductor] promote: no roll-up PR found for issue #%s — nothing to promote\n' "$issue" >&2
            gh issue comment "$issue" --repo "$repo" \
                --body "promote: no open roll-up PR found — nothing to promote." >/dev/null 2>&1 || true
            _autospec_conductor_clear_control_label "$repo" "$issue" "autospec:promote"
            return 0
            ;;
    esac
    if [ "$rollup_state" != "OPEN" ]; then
        if [ "$rollup_state" = "MERGED" ]; then
            # Recovery path: a previous promote merged the roll-up but its
            # reset failed. Re-attempt reset so re-firing promote is
            # idempotent recovery, not a dead end.
            printf '[conductor] promote: roll-up PR #%s already merged — re-attempting integration-branch reset (recovery)\n' \
                "$rollup_pr" >&2
            if bash "$intbranch_sh" reset --parent "$parent" --repo "$repo" >&2; then
                _autospec_conductor_clear_self_pause "$repo_root"
                gh issue comment "$issue" --repo "$repo" \
                    --body "promote: roll-up PR #${rollup_pr} was already merged; integration-branch reset completed (recovered)." \
                    >/dev/null 2>&1 || true
                gh issue close "$issue" --repo "$repo" >/dev/null 2>&1 || true
            else
                gh issue comment "$issue" --repo "$repo" \
                    --body "promote: roll-up PR #${rollup_pr} is merged but the integration-branch reset failed again. Re-apply autospec:promote to retry, or reset manually." \
                    >/dev/null 2>&1 || true
            fi
        else
            printf '[conductor] promote: roll-up PR #%s is not open (state=%s) — nothing to promote\n' \
                "$rollup_pr" "$rollup_state" >&2
            gh issue comment "$issue" --repo "$repo" \
                --body "promote: roll-up PR #${rollup_pr} is not open (state=${rollup_state}) — nothing to promote." \
                >/dev/null 2>&1 || true
        fi
        _autospec_conductor_clear_control_label "$repo" "$issue" "autospec:promote"
        return 0
    fi

    # Red/unsettled CI must never be admin-merged; manual GitHub merge is
    # the operator's explicit override.
    if ! _autospec_conductor_rollup_ci_green "$repo" "$rollup_pr"; then
        printf '[conductor] promote: roll-up PR #%s CI is red or unsettled — refusing to merge\n' "$rollup_pr" >&2
        gh issue comment "$issue" --repo "$repo" \
            --body "promote refused: roll-up PR #${rollup_pr} has red or unsettled CI. Fix CI (or merge manually on GitHub as an explicit override), then re-apply autospec:promote." \
            >/dev/null 2>&1 || true
        _autospec_conductor_clear_control_label "$repo" "$issue" "autospec:promote"
        return 0
    fi

    # No --delete-branch: reset recreates the integration branch from the
    # parent tip, and deleting here could also remove a local branch in the
    # conductor's checkout.
    printf '[conductor] promote: merging roll-up PR #%s (issue #%s)\n' "$rollup_pr" "$issue" >&2
    if ! gh pr merge "$rollup_pr" --repo "$repo" --admin --squash >/dev/null 2>&1; then
        gh issue comment "$issue" --repo "$repo" \
            --body "promote: failed to merge roll-up PR #${rollup_pr}. Re-apply autospec:promote to retry." \
            >/dev/null 2>&1 || true
        _autospec_conductor_clear_control_label "$repo" "$issue" "autospec:promote"
        return 1
    fi

    if bash "$intbranch_sh" reset --parent "$parent" --repo "$repo" >&2; then
        _autospec_conductor_clear_self_pause "$repo_root"
        gh issue comment "$issue" --repo "$repo" \
            --body "promote: merged roll-up PR #${rollup_pr} and reset the integration branch." \
            >/dev/null 2>&1 || true
        gh issue close "$issue" --repo "$repo" >/dev/null 2>&1 || true
    else
        gh issue comment "$issue" --repo "$repo" \
            --body "promote: merged roll-up PR #${rollup_pr} but the integration-branch reset failed. Re-apply autospec:promote to retry the reset (recovery is idempotent)." \
            >/dev/null 2>&1 || true
    fi
    _autospec_conductor_clear_control_label "$repo" "$issue" "autospec:promote"
}

# _autospec_conductor_handle_control_refused REPO ISSUE VERB — the control
# channel refused (untrusted author or label applicator); comment on the
# issue and clear the trigger label, never act.
_autospec_conductor_handle_control_refused() {
    local repo="$1" issue="$2" verb="$3"
    if [ -z "$issue" ]; then
        return 0
    fi
    printf '[conductor] DECISION:%s-refused — issue #%s author/label-applicator is not a trusted actor; refusing (no action taken)\n' \
        "$verb" "$issue" >&2
    if [ -n "$repo" ]; then
        gh issue comment "$issue" --repo "$repo" \
            --body "${verb} refused: the issue author or the autospec:${verb} label applicator is not a trusted actor (safety.issue_intent_gate.trusted_actors). No action was taken." \
            >/dev/null 2>&1 || true
        _autospec_conductor_clear_control_label "$repo" "$issue" "autospec:${verb}"
    fi
}

# _autospec_conductor_handle_discard REPO INTBRANCH_SH ISSUE REPO_ROOT —
# close the open roll-up PR (deleting the integration branch), then reopen
# every issue listed in the roll-up PR BODY's manifest (between the
# autospec-rollup-manifest markers — bot/maintainer-written; PR COMMENTS are
# writable by anyone and are never trusted for this) with a
# discarded-from-rollup comment. A missing/merged roll-up is a clean no-op.
_autospec_conductor_handle_discard() {
    local repo="$1" intbranch_sh="$2" issue="$3" repo_root="${4:-}"
    if [ -z "$issue" ]; then
        return 0
    fi
    if [ -z "$repo" ] || [ -z "$intbranch_sh" ]; then
        return 0
    fi

    local parent status_json rollup_pr rollup_state
    local pending_data pending_rollup pending_issues discard_retry=0
    parent="$(_autospec_conductor_default_branch "$repo")"
    status_json="$(bash "$intbranch_sh" status --parent "$parent" --repo "$repo" 2>/dev/null || true)"
    rollup_pr="$(printf '%s' "$status_json" | jq -r '.rollup_pr.number // empty' 2>/dev/null || true)"
    rollup_state="$(printf '%s' "$status_json" | jq -r '.rollup_pr.state // empty' 2>/dev/null || true)"
    pending_data="$(_autospec_conductor_read_discard_pending "$repo_root" "$issue" 2>/dev/null || true)"
    if [ -n "$pending_data" ]; then
        pending_rollup="$(printf '%s\n' "$pending_data" | sed -n '1p')"
        pending_issues="$(printf '%s\n' "$pending_data" | sed '1d')"
    fi

    case "$rollup_pr" in
        ''|*[!0-9]*)
            if [ -n "${pending_rollup:-}" ] && [ "$rollup_state" != "MERGED" ] && [ -n "${pending_issues:-}" ]; then
                rollup_pr="$pending_rollup"
                discard_retry=1
            else
            printf '[conductor] discard: no roll-up PR found for issue #%s — nothing to discard\n' "$issue" >&2
            gh issue comment "$issue" --repo "$repo" \
                --body "discard: no open roll-up PR found — nothing to discard." >/dev/null 2>&1 || true
            _autospec_conductor_clear_control_label "$repo" "$issue" "autospec:discard"
            return 0
            fi
            ;;
    esac
    # `status` falls back to the MERGED roll-up when no open one exists —
    # discarding that would reopen already-landed issues. Only an OPEN
    # roll-up is discardable (mirrors promote's state guard).
    if [ "$discard_retry" != "1" ] && [ "$rollup_state" != "OPEN" ]; then
        if [ -n "${pending_rollup:-}" ] && [ "$pending_rollup" = "$rollup_pr" ] && [ "$rollup_state" != "MERGED" ] && [ -n "${pending_issues:-}" ]; then
            discard_retry=1
        else
        printf '[conductor] discard: roll-up PR #%s is not open (state=%s) — nothing to discard\n' \
            "$rollup_pr" "$rollup_state" >&2
        gh issue comment "$issue" --repo "$repo" \
            --body "discard: roll-up PR #${rollup_pr} is not open (state=${rollup_state}) — nothing to discard." \
            >/dev/null 2>&1 || true
        _autospec_conductor_clear_control_label "$repo" "$issue" "autospec:discard"
        return 0
        fi
    fi

    # Landed-issue list from the PR BODY manifest (between the
    # autospec-rollup-manifest markers): manifest lines are
    # `  - #N — <title> ...`; take the FIRST issue ref per line.
    local rollup_body rolled_issues
    if [ "$discard_retry" = "1" ]; then
        rolled_issues="$pending_issues"
        printf '[conductor] discard: retrying pending discard for roll-up PR #%s (issue #%s)\n' \
            "$rollup_pr" "$issue" >&2
    else
        rollup_body="$(gh pr view "$rollup_pr" --repo "$repo" --json body --jq '.body // ""' 2>/dev/null || true)"
        rolled_issues="$(printf '%s\n' "$rollup_body" \
            | awk '/<!-- autospec-rollup-manifest:begin -->/{f=1;next} /<!-- autospec-rollup-manifest:end -->/{f=0} f' \
            | grep -E '^[[:space:]]*- #[0-9]+' \
            | sed -E 's/^[[:space:]]*- #([0-9]+).*/\1/' \
            | sort -un || true)"

        if [ -n "$rolled_issues" ] && ! _autospec_conductor_write_discard_pending "$repo_root" "$issue" "$rollup_pr" "$rolled_issues"; then
            printf '[conductor] discard: failed to persist retry manifest for roll-up PR #%s — aborting discard\n' "$rollup_pr" >&2
            gh issue comment "$issue" --repo "$repo" \
                --body "discard: failed to persist retry manifest for roll-up PR #${rollup_pr}; nothing was closed or reopened. Re-apply autospec:discard to retry." \
                >/dev/null 2>&1 || true
            _autospec_conductor_clear_control_label "$repo" "$issue" "autospec:discard"
            return 1
        fi

        printf '[conductor] discard: closing roll-up PR #%s and deleting its integration branch (issue #%s)\n' \
            "$rollup_pr" "$issue" >&2
        if ! gh pr close "$rollup_pr" --repo "$repo" --delete-branch >/dev/null 2>&1; then
        # Close failed → nothing else happens (no reopen, no control-issue
        # close): a half-discarded state must not reopen issues whose work
        # is still in an open roll-up.
        printf '[conductor] discard: closing roll-up PR #%s FAILED — aborting discard\n' "$rollup_pr" >&2
        gh issue comment "$issue" --repo "$repo" \
            --body "discard: failed to close roll-up PR #${rollup_pr}; nothing was reopened. Re-apply autospec:discard to retry." \
            >/dev/null 2>&1 || true
        _autospec_conductor_clear_discard_pending "$repo_root" "$issue"
        _autospec_conductor_clear_control_label "$repo" "$issue" "autospec:discard"
        return 1
        fi
    fi

    # Reopen loop tracks per-issue failures — the discard path is
    # destructive, so a reopen/comment failure must NOT be swallowed and
    # reported as success. A plain for-loop (not a piped while) keeps the
    # counter out of a subshell; the manifest-derived tokens are numeric.
    local reopen_failures=0
    if [ -n "$rolled_issues" ]; then
        local n existing_marker
        for n in $rolled_issues; do
            [ -n "$n" ] || continue
            # Idempotency: a re-run (crash/resume, label re-applied) must not
            # duplicate the reopen comment for the same roll-up.
            existing_marker="$(gh issue view "$n" --repo "$repo" --json comments --jq '.comments[].body' 2>/dev/null \
                | grep -cF "discarded-from-rollup: roll-up PR #${rollup_pr}" || true)"
            if [ "${existing_marker:-0}" != "0" ]; then
                continue
            fi
            if ! gh issue reopen "$n" --repo "$repo" >/dev/null 2>&1; then
                printf '[conductor] discard: failed to reopen issue #%s\n' "$n" >&2
                reopen_failures=$((reopen_failures + 1))
                continue
            fi
            if ! gh issue comment "$n" --repo "$repo" \
                --body "discarded-from-rollup: roll-up PR #${rollup_pr} was discarded via control-channel issue #${issue}; reopened for re-drain." \
                >/dev/null 2>&1; then
                printf '[conductor] discard: failed to comment on reopened issue #%s\n' "$n" >&2
                reopen_failures=$((reopen_failures + 1))
            fi
        done
    fi

    if [ "$reopen_failures" -gt 0 ]; then
        # Partial failure: do NOT claim success or close the control issue —
        # re-firing autospec:discard retries; the per-issue idempotency
        # marker above makes the retry skip the issues that did complete.
        printf '[conductor] discard: %s issue reopen/comment failure(s) — leaving control issue #%s open for retry\n' \
            "$reopen_failures" "$issue" >&2
        gh issue comment "$issue" --repo "$repo" \
            --body "discard: closed roll-up PR #${rollup_pr}, but ${reopen_failures} issue reopen/comment call(s) failed. Re-apply autospec:discard to retry the remainder (already-processed issues are skipped)." \
            >/dev/null 2>&1 || true
        _autospec_conductor_clear_control_label "$repo" "$issue" "autospec:discard"
        return 1
    fi

    _autospec_conductor_clear_discard_pending "$repo_root" "$issue"
    _autospec_conductor_clear_self_pause "$repo_root"
    gh issue comment "$issue" --repo "$repo" \
        --body "discard: closed roll-up PR #${rollup_pr}, deleted the integration branch, and reopened its issues." \
        >/dev/null 2>&1 || true
    gh issue close "$issue" --repo "$repo" >/dev/null 2>&1 || true
    _autospec_conductor_clear_control_label "$repo" "$issue" "autospec:discard"
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

_autospec_conductor_main_sha() {
    local repo_root="$1"
    if [ -n "${AUTOSPEC_CONDUCTOR_MAIN_SHA_CMD:-}" ]; then
        bash -c "$AUTOSPEC_CONDUCTOR_MAIN_SHA_CMD" 2>/dev/null | sed -n '1p'
        return 0
    fi
    git -C "$repo_root" rev-parse --verify refs/remotes/origin/main 2>/dev/null \
        || git -C "$repo_root" rev-parse --verify main 2>/dev/null \
        || git -C "$repo_root" rev-parse --verify HEAD 2>/dev/null \
        || true
}

_autospec_conductor_fetch_main() {
    local repo_root="$1"
    if [ -n "${AUTOSPEC_CONDUCTOR_MAIN_SHA_CMD:-}" ]; then
        return 0
    fi
    git -C "$repo_root" fetch origin main >/dev/null 2>&1 || true
}

_autospec_conductor_self_repair_pending_file() {
    local repo_root="$1"
    printf '%s/.autospec/autonomous-self-repair-refresh.json\n' "$repo_root"
}

_autospec_conductor_integration_conflict_cooldown_file() {
    local repo_root="$1"
    printf '%s/.autospec/integration-conflict-cooldown\n' "$repo_root"
}

_autospec_conductor_normalize_issue_list() {
    tr ' ' '\n' \
        | sed '/^[[:space:]]*$/d' \
        | sort -n \
        | paste -sd ' ' -
}

_autospec_conductor_same_issue_list() {
    local left="$1"
    local right="$2"
    local left_norm right_norm
    left_norm="$(printf '%s\n' "$left" | _autospec_conductor_normalize_issue_list)"
    right_norm="$(printf '%s\n' "$right" | _autospec_conductor_normalize_issue_list)"
    [ "$left_norm" = "$right_norm" ]
}

_autospec_conductor_integration_requested_branch() {
    local parent_branch="$1"
    local prefix
    if command -v autospec_runtime_config_get >/dev/null 2>&1; then
        prefix="$(autospec_runtime_config_get "autonomous.self_originated.integration_branch_prefix" "autospec/autonomous-")"
    else
        prefix="autospec/autonomous-"
    fi
    printf '%s%s\n' "$prefix" "${parent_branch#origin/}"
}

_autospec_conductor_mode_file_field() {
    local repo_root="$1"
    local field="$2"
    local mode_file="${repo_root}/.autospec/explore-mode.json"
    [ -f "$mode_file" ] || { printf '<none>\n'; return 0; }
    jq -r --arg field "$field" '.[$field] // "<none>"' "$mode_file" 2>/dev/null \
        || printf '<none>\n'
}

_autospec_conductor_conflict_field() {
    local payload="$1"
    local field="$2"
    printf '%s\n' "$payload" \
        | sed -n "s/.*${field}=\\([^[:space:]]*\\).*/\\1/p" \
        | tail -1
}

_autospec_conductor_integration_conflict_cooldown_active() {
    local repo_root="$1"
    local issues="$2"
    local cycle="$3"
    local requested_branch="$4"
    local file until stored_issues rc stored_requested stored_existing stored_kind current_existing current_kind
    file="$(_autospec_conductor_integration_conflict_cooldown_file "$repo_root")"
    [ -f "$file" ] || return 1

    until="$(sed -n 's/^until_cycle=//p' "$file" 2>/dev/null | head -1)"
    stored_issues="$(sed -n 's/^issues=//p' "$file" 2>/dev/null | head -1)"
    rc="$(sed -n 's/^rc=//p' "$file" 2>/dev/null | head -1)"
    stored_requested="$(sed -n 's/^requested_branch=//p' "$file" 2>/dev/null | head -1)"
    stored_existing="$(sed -n 's/^existing_branch=//p' "$file" 2>/dev/null | head -1)"
    stored_kind="$(sed -n 's/^existing_kind=//p' "$file" 2>/dev/null | head -1)"
    case "$until" in ''|*[!0-9]*) rm -f "$file" 2>/dev/null || true; return 1 ;; esac
    if [ "$cycle" -gt "$until" ] 2>/dev/null; then
        rm -f "$file" 2>/dev/null || true
        return 1
    fi
    _autospec_conductor_same_issue_list "$issues" "$stored_issues" || return 1
    [ "${stored_requested:-}" = "$requested_branch" ] || return 1
    current_existing="$(_autospec_conductor_mode_file_field "$repo_root" branch)"
    current_kind="$(_autospec_conductor_mode_file_field "$repo_root" kind)"
    [ "${stored_existing:-<none>}" = "$current_existing" ] || return 1
    [ "${stored_kind:-<none>}" = "$current_kind" ] || return 1
    printf '[conductor] integration conflict cooldown: excluding self batch (issues: %s, rc=%s, requested=%s, existing=%s, until-cycle=%s)\n' \
        "$issues" "${rc:-unknown}" "$requested_branch" "${stored_existing:-<none>}" "$until" >&2
    return 0
}

_autospec_conductor_arm_integration_conflict_cooldown() {
    local repo_root="$1"
    local issues="$2"
    local cycle="$3"
    local rc="$4"
    local payload="$5"
    local requested_branch="$6"
    local file until existing_branch existing_kind
    file="$(_autospec_conductor_integration_conflict_cooldown_file "$repo_root")"
    existing_branch="$(_autospec_conductor_conflict_field "$payload" existing_branch)"
    existing_kind="$(_autospec_conductor_conflict_field "$payload" existing_kind)"
    [ -n "$requested_branch" ] || requested_branch="$(_autospec_conductor_conflict_field "$payload" requested_branch)"
    [ -n "$existing_branch" ] || existing_branch="$(_autospec_conductor_mode_file_field "$repo_root" branch)"
    [ -n "$existing_kind" ] || existing_kind="$(_autospec_conductor_mode_file_field "$repo_root" kind)"
    until=$((cycle + 4))
    mkdir -p "$(dirname "$file")" 2>/dev/null || true
    {
        printf 'until_cycle=%s\n' "$until"
        printf 'rc=%s\n' "$rc"
        printf 'issues=%s\n' "$issues"
        printf 'requested_branch=%s\n' "$requested_branch"
        printf 'existing_branch=%s\n' "${existing_branch:-<none>}"
        printf 'existing_kind=%s\n' "${existing_kind:-<none>}"
    } > "$file" 2>/dev/null || true
}

_autospec_conductor_filter_integration_conflict_cooldown_queue() {
    local repo_root="$1"
    local queue_json="$2"
    local cycle="$3"
    local requested_branch="$4"
    local file issues
    file="$(_autospec_conductor_integration_conflict_cooldown_file "$repo_root")"
    [ -f "$file" ] || { printf '%s' "$queue_json"; return 0; }
    issues="$(sed -n 's/^issues=//p' "$file" 2>/dev/null | head -1)"
    if [ -z "$issues" ] || ! _autospec_conductor_integration_conflict_cooldown_active \
        "$repo_root" "$issues" "$cycle" "$requested_branch"; then
        printf '%s' "$queue_json"
        return 0
    fi
    SKIP_ISSUES="$issues" jq '
      (.batch | length) as $batch_len |
      (env.SKIP_ISSUES | split(" ") | map(select(length > 0))) as $skip |
      .ready = [(.ready // [])[] | select((.number | tostring) as $n | ($skip | index($n) | not))] |
      .batch = [(.batch // [])[] | select((.number | tostring) as $n | ($skip | index($n) | not))] |
      if ((.batch | length) < $batch_len) then
        .batch = (.ready[0:$batch_len])
      else
        .
      end
    ' 2>/dev/null <<EOF_QUEUE_FILTER
$queue_json
EOF_QUEUE_FILTER
}

_autospec_conductor_pr_touches_refresh_surface() {
    local repo="$1"
    local pr="$2"
    [ -n "$repo" ] || return 1
    case "$pr" in ''|*[!0-9]*) return 1 ;; esac
    command -v gh >/dev/null 2>&1 || return 1

    gh pr view "$pr" --repo "$repo" --json files --jq '.files[].path' 2>/dev/null \
        | while IFS= read -r _path; do
            case "$_path" in
                scripts/*|skills/*)
                    printf 'yes\n'
                    break
                    ;;
            esac
        done | grep -q '^yes$'
}

_autospec_conductor_arm_self_repair_refresh() {
    local repo_root="$1"
    local repo="$2"
    local pr="$3"
    local old_sha="$4"
    local cycle="$5"
    [ -n "$old_sha" ] || return 0
    _autospec_conductor_pr_touches_refresh_surface "$repo" "$pr" || return 0

    local pending_file
    pending_file="$(_autospec_conductor_self_repair_pending_file "$repo_root")"
    mkdir -p "$(dirname "$pending_file")" 2>/dev/null || true
    python3 - "$pending_file" "$repo" "$pr" "$old_sha" "$cycle" <<'PY' 2>/dev/null || true
import json, sys, time
path, repo, pr, old_sha, cycle = sys.argv[1:6]
payload = {
    "repo": repo,
    "pr": int(pr),
    "old_main_sha": old_sha,
    "cycle": int(cycle),
    "reason": "self-repair merge touched scripts/ or skills/",
    "created_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
}
with open(path, "w", encoding="utf-8") as fh:
    json.dump(payload, fh, sort_keys=True)
    fh.write("\n")
PY
    printf '[conductor] self-repair refresh armed for PR #%s after cycle %s (main=%s)\n' \
        "$pr" "$cycle" "$old_sha" >&2
}

_autospec_conductor_maybe_arm_self_repair_refresh() {
    local repo_root="$1"
    local repo="$2"
    local outcome_file="$3"
    local old_sha="$4"
    local cycle="$5"
    [ -f "$outcome_file" ] || return 0
    command -v jq >/dev/null 2>&1 || return 0

    local is_self outcome pr
    is_self="$(jq -r '.self_originated // false' "$outcome_file" 2>/dev/null || printf 'false')"
    outcome="$(jq -r '.outcome // ""' "$outcome_file" 2>/dev/null || printf '')"
    pr="$(jq -r '.pr // empty' "$outcome_file" 2>/dev/null || printf '')"
    [ "$is_self" = "true" ] || return 0
    [ "$outcome" = "merged" ] || return 0
    _autospec_conductor_arm_self_repair_refresh "$repo_root" "$repo" "$pr" "$old_sha" "$cycle"
}

_autospec_conductor_run_refresh_command() {
    local repo_root="$1"
    local old_sha="$2"
    local new_sha="$3"
    local pr="$4"

    export AUTOSPEC_CONDUCTOR_REFRESH_OLD_SHA="$old_sha"
    export AUTOSPEC_CONDUCTOR_REFRESH_NEW_SHA="$new_sha"
    export AUTOSPEC_CONDUCTOR_REFRESH_PR="$pr"
    if [ -n "${AUTOSPEC_CONDUCTOR_REFRESH_CMD:-}" ]; then
        bash -c "$AUTOSPEC_CONDUCTOR_REFRESH_CMD"
        return $?
    fi
    if [ -f "$repo_root/install.sh" ]; then
        AUTOSPEC_AUTO_YES=1 \
        AUTOSPEC_NO_STAR_PROMPT=1 \
        AUTOSPEC_SKIP_ECOSYSTEM_BOOTSTRAP=1 \
        bash "$repo_root/install.sh" --update --skill autospec-autonomous --harness codex
        return $?
    fi
    return 0
}

_autospec_conductor_maybe_refresh_self_repair() {
    local repo_root="$1"
    local repo="$2"
    local dry="$3"
    local pending_file
    pending_file="$(_autospec_conductor_self_repair_pending_file "$repo_root")"
    [ -f "$pending_file" ] || return 0
    command -v jq >/dev/null 2>&1 || return 0

    local old_sha pr new_sha digest_file pid_value
    old_sha="$(jq -r '.old_main_sha // empty' "$pending_file" 2>/dev/null || printf '')"
    pr="$(jq -r '.pr // empty' "$pending_file" 2>/dev/null || printf '')"
    [ -n "$old_sha" ] || return 0

    _autospec_conductor_fetch_main "$repo_root"
    new_sha="$(_autospec_conductor_main_sha "$repo_root" | sed -n '1p')"
    [ -n "$new_sha" ] || return 0
    if [ "$new_sha" = "$old_sha" ]; then
        return 0
    fi

    pid_value="${AUTOSPEC_CONDUCTOR_PID:-$$}"
    printf '[conductor] self-repair refresh: %s -> %s (PR #%s, PID %s)\n' \
        "$old_sha" "$new_sha" "${pr:-unknown}" "$pid_value" >&2
    if [ "$dry" != "1" ]; then
        _autospec_conductor_run_refresh_command "$repo_root" "$old_sha" "$new_sha" "$pr" \
            >/dev/null 2>&1 || true
    fi

    digest_file="${repo_root}/.autospec/autonomous-digest.md"
    mkdir -p "$(dirname "$digest_file")" 2>/dev/null || true
    {
        printf '\n## Self-repair refresh — %s\n\n' \
            "$(date -u +'%Y-%m-%dT%H:%M:%SZ' 2>/dev/null || echo unknown)"
        printf -- '- PR: #%s\n' "${pr:-unknown}"
        printf -- '- Conductor PID: `%s`\n' "$pid_value"
        printf -- '- Main SHA: `%s -> %s`\n' "$old_sha" "$new_sha"
        if [ -n "$repo" ]; then
            printf -- '- Repo: `%s`\n' "$repo"
        fi
    } >> "$digest_file" 2>/dev/null || true
    rm -f "$pending_file" 2>/dev/null || true
}

_autospec_conductor_repo_stop_flag_path() {
    local repo="${1:-}"
    local repo_root="${2:-}"
    local state_root="${AUTOSPEC_AUTONOMOUS_OPERATOR_DIR:-$HOME/.autospec/autonomous-operator}"
    local scope=""
    if [ -n "$repo" ]; then
        scope="$(printf '%s' "$repo" | sed 's#[/:]#_#g; s#[^A-Za-z0-9._-]#_#g')"
    elif [ -n "$repo_root" ]; then
        local real_root
        real_root="$(cd "$repo_root" 2>/dev/null && pwd -P || printf '%s' "$repo_root")"
        scope="dir_$(printf '%s' "$real_root" | sed 's#[^A-Za-z0-9._-]#_#g')"
    fi
    [ -n "$scope" ] || return 1
    printf '%s/%s/stop.flag\n' "${state_root%/}" "$scope"
}

_autospec_conductor_operator_stop_flag_path() {
    local repo="${1:-}"
    local repo_root="${2:-}"
    local repo_flag=""
    repo_flag="$(_autospec_conductor_repo_stop_flag_path "$repo" "$repo_root" 2>/dev/null || true)"
    if [ -n "$repo_flag" ] && [ -f "$repo_flag" ]; then
        printf '%s\n' "$repo_flag"
        return 0
    fi

    local isolated_flag="${AUTOSPEC_STOP_FLAG_FILE:-${HOME}/.autospec/stop.flag}"
    if [ -f "$isolated_flag" ]; then
        printf '%s\n' "$isolated_flag"
        return 0
    fi
    return 1
}

_autospec_conductor_persona_sources_cmd() {
    local sdir="$1"
    if [ -n "${AUTOSPEC_PERSONA_SOURCES_CMD:-}" ] && [ -f "$AUTOSPEC_PERSONA_SOURCES_CMD" ]; then
        printf '%s\n' "$AUTOSPEC_PERSONA_SOURCES_CMD"
    elif [ -f "${sdir}/autonomous-persona-sources.sh" ]; then
        printf '%s\n' "${sdir}/autonomous-persona-sources.sh"
    else
        printf '\n'
    fi
}

_autospec_conductor_inferred_source_summary() {
    local sdir="$1"
    local repo_root="$2"
    local autospec_home="${HOME}/.autospec"
    local sources_cmd
    sources_cmd="$(_autospec_conductor_persona_sources_cmd "$sdir")"

    if [ -z "$sources_cmd" ] || ! command -v jq >/dev/null 2>&1; then
        printf '0 none\n'
        return 0
    fi

    local bundle
    bundle="$(REPO_ROOT="$repo_root" AUTOSPEC_HOME="$autospec_home" \
        bash "$sources_cmd" 2>/dev/null || true)"
    if [ -z "$bundle" ]; then
        printf '0 none\n'
        return 0
    fi

    printf '%s' "$bundle" | jq -r '
        (.meta.source_count // (((.global // []) | length) + ((.overlay // []) | length))) as $count
        | (.meta.confidence // (
            if $count == 0 then "none"
            elif $count == 1 then "low"
            elif ((.global // []) | length) == 0 then "medium"
            else "high"
            end
          )) as $confidence
        | "\($count) \($confidence)"
    ' 2>/dev/null || printf '0 none\n'
}

_autospec_conductor_interactive_bootstrap_enabled() {
    if [ "${AUTOSPEC_BOOTSTRAP_INTERACTIVE:-0}" = "1" ]; then
        return 0
    fi
    if [ "${AUTOSPEC_AUTONOMOUS_HEADLESS:-0}" = "1" ]; then
        return 1
    fi
    [ -t 0 ] && [ -t 1 ]
}

_autospec_conductor_handle_empty_inference_bundle() {
    local repo_root="$1"
    local repo="$2"

    if _autospec_conductor_interactive_bootstrap_enabled; then
        printf '[conductor] bootstrap: empty inference bundle — running bootstrap interview dialog\n' >&2
        if [ -n "${AUTOSPEC_BOOTSTRAP_INTERVIEW_CMD:-}" ]; then
            bash -c "$AUTOSPEC_BOOTSTRAP_INTERVIEW_CMD" >/dev/null 2>&1 || true
            return 0
        fi
        printf '[conductor] bootstrap: no AUTOSPEC_BOOTSTRAP_INTERVIEW_CMD seam; dialog deferred to harness\n' >&2
        return 0
    fi

    printf '[conductor] bootstrap: empty inference bundle — filing bootstrap issue/PR decision\n' >&2
    if [ -n "${AUTOSPEC_BOOTSTRAP_DECISION_CMD:-}" ]; then
        bash -c "$AUTOSPEC_BOOTSTRAP_DECISION_CMD" >/dev/null 2>&1 || true
        return 0
    fi

    local report="${repo_root}/.autospec/reports/bootstrap-decision.md"
    mkdir -p "$(dirname "$report")" 2>/dev/null || true
    {
        printf '# Bootstrap context needed\n\n'
        if [ -n "$repo" ]; then
            printf 'Repo: `%s`\n\n' "$repo"
        fi
        printf 'The autonomous conductor found no inferable operator persona, repo memory, autonomy charter, or overlay sources.\n\n'
        printf 'Next step: file a bootstrap issue/PR to collect project intent before self-originated generation resumes.\n'
    } > "$report" 2>/dev/null || true
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
    # Never-idle: idle-rescan heartbeat interval (default 30m per the 2026-07-06
    # platform design). A fully-dry cascade idles here rather than converge-stops.
    # Keep an empty backlog responsive: discovery is the autonomous worker's
    # source of new work, so a dry Tier-1 cycle should not sleep for 30 minutes.
    local _rescan_interval="${AUTOSPEC_RESCAN_INTERVAL:-300}"

    # Resolve helper script paths.
    local _control_ch="${_sdir}/autonomous-control-channel.sh"
    local _waterfall="${_sdir}/autonomous-waterfall.sh"
    local _spend="${_sdir}/autonomous-spend-ledger.sh"
    local _gate="${_sdir}/autonomous-premerge-gate.sh"
    local _resilience="${_sdir}/autonomous-resilience.sh"
    local _usage_limit="${_sdir}/autospec-usage-limit.sh"
    local _governor="${_sdir}/autonomous-usage-governor.sh"
    local _queue_bin="${AUTOSPEC_QUEUE_BIN:-${AUTOSPEC_BIN:-}}"
    if [ -z "$_queue_bin" ]; then
        _queue_bin="$(command -v autospec 2>/dev/null || true)"
    fi
    _AUTOSPEC_CONDUCTOR_ACCOUNTABILITY_BIN="$_queue_bin"

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

    # ── Provenance + integration-branch wiring (integration-branch design §5) ──
    # Reusing the F3 env-seam resolution idiom above (AUTOSPEC_*_BIN override >
    # sibling of scripts/). Both scripts AND a repo slug must resolve for the
    # dispatch-time provenance split to activate; otherwise the Tier-1 drain
    # keeps today's single-dispatch behavior (back-compat, capability-gated).
    local _provenance_sh=""
    if [ -n "${AUTOSPEC_PROVENANCE_BIN:-}" ] && [ -x "$AUTOSPEC_PROVENANCE_BIN" ]; then
        _provenance_sh="$AUTOSPEC_PROVENANCE_BIN"
    elif [ -f "${_sdir}/autonomous-provenance.sh" ]; then
        _provenance_sh="${_sdir}/autonomous-provenance.sh"
    fi
    local _intbranch_sh=""
    if [ -n "${AUTOSPEC_INTEGRATION_BRANCH_BIN:-}" ] && [ -x "$AUTOSPEC_INTEGRATION_BRANCH_BIN" ]; then
        _intbranch_sh="$AUTOSPEC_INTEGRATION_BRANCH_BIN"
    elif [ -f "${_sdir}/autonomous-integration-branch.sh" ]; then
        _intbranch_sh="${_sdir}/autonomous-integration-branch.sh"
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
    if [ -z "$_queue_bin" ]; then
        if [ -x "${_repo_root}/target/debug/autospec" ]; then
            _queue_bin="${_repo_root}/target/debug/autospec"
        elif command -v autospec >/dev/null 2>&1; then
            _queue_bin="$(command -v autospec)"
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
    # GROWTH fold-in: Tier G1 (run-growth-define) dry-cycle counter. Only ever
    # incremented/reset when growth is enabled (see _growth_enabled below).
    local _tierg_dry_cycles=0
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
        local _operator_stop_flag
        _operator_stop_flag="$(_autospec_conductor_operator_stop_flag_path "$_repo" "$_repo_root" 2>/dev/null || true)"
        if [ -n "$_operator_stop_flag" ]; then
            printf '[conductor] operator stop flag detected: %s\n' \
                "$_operator_stop_flag" >&2
            _stop_reason="operator:stop-flag"
            break
        fi
        _cycle=$((_cycle + 1))
        _AUTOSPEC_CONDUCTOR_CYCLE="$_cycle"
        printf '[conductor] cycle %s starting\n' "$_cycle" >&2
        _autospec_conductor_maybe_refresh_self_repair "$_repo_root" "$_repo" "$_dry" || true
        local _cycle_main_sha
        _cycle_main_sha="$(_autospec_conductor_main_sha "$_repo_root" | sed -n '1p')"

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
        local _inferred_summary _inferred_source_count _inferred_confidence
        _inferred_summary="$(_autospec_conductor_inferred_source_summary "$_sdir" "$_repo_root")"
        _inferred_source_count="$(printf '%s' "$_inferred_summary" | awk '{print $1}')"
        _inferred_confidence="$(printf '%s' "$_inferred_summary" | awk '{print $2}')"
        case "$_inferred_source_count" in
            ''|*[!0-9]*) _inferred_source_count=0 ;;
        esac
        [ -n "$_inferred_confidence" ] || _inferred_confidence="none"

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
                promote)
                    local _promote_issue
                    _promote_issue="$(printf '%s' "$_ctrl_out" \
                        | grep '^PROMOTE_ISSUE:' | head -1 | sed 's/^PROMOTE_ISSUE://' || true)"
                    printf '[conductor] DECISION:promote received (issue #%s) — merging roll-up + resetting integration branch\n' \
                        "${_promote_issue:-unknown}" >&2
                    _autospec_conductor_handle_promote "$_repo" "$_intbranch_sh" "$_promote_issue" "$_repo_root" || true
                    ;;
                promote-refused)
                    local _promote_refused_issue
                    _promote_refused_issue="$(printf '%s' "$_ctrl_out" \
                        | grep '^PROMOTE_ISSUE:' | head -1 | sed 's/^PROMOTE_ISSUE://' || true)"
                    _autospec_conductor_handle_control_refused "$_repo" "$_promote_refused_issue" "promote" || true
                    ;;
                discard)
                    local _discard_issue
                    _discard_issue="$(printf '%s' "$_ctrl_out" \
                        | grep '^DISCARD_ISSUE:' | head -1 | sed 's/^DISCARD_ISSUE://' || true)"
                    printf '[conductor] DECISION:discard received (issue #%s) — closing roll-up, deleting integration branch, reopening issues\n' \
                        "${_discard_issue:-unknown}" >&2
                    _autospec_conductor_handle_discard "$_repo" "$_intbranch_sh" "$_discard_issue" "$_repo_root" || true
                    ;;
                discard-refused)
                    local _discard_refused_issue
                    _discard_refused_issue="$(printf '%s' "$_ctrl_out" \
                        | grep '^DISCARD_ISSUE:' | head -1 | sed 's/^DISCARD_ISSUE://' || true)"
                    _autospec_conductor_handle_control_refused "$_repo" "$_discard_refused_issue" "discard" || true
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
        if [ -n "$_queue_bin" ] && { [ -x "$_queue_bin" ] || command -v "$_queue_bin" >/dev/null 2>&1; } && [ -n "$_repo" ]; then
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
            _queue_json="$("$_queue_bin" queue ready --repo "$_repo" --batch-size "$_queue_batch_request" 2>/dev/null || true)"
            if [ -n "$_queue_json" ]; then
                local _cooldown_parent_branch _cooldown_requested_branch
                _cooldown_parent_branch="$(_autospec_conductor_default_branch "$_repo")"
                _cooldown_requested_branch="$(_autospec_conductor_integration_requested_branch "$_cooldown_parent_branch")"
                _queue_json="$(_autospec_conductor_filter_integration_conflict_cooldown_queue \
                    "$_repo_root" "$_queue_json" "$_cycle" "$_cooldown_requested_branch")"
            fi
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

        # A backlog where every implementation candidate is blocked by the
        # autospec issue safety gate is a platform/readiness failure, not a
        # dry product backlog. Continuing into discovery here only files more
        # work that Tier 1 cannot legally claim. Pause the conductor and surface
        # the blocker instead of growing an undrainable queue.
        if [ "${_all_blocked_count:-0}" -gt 0 ] && [ "$_dry" != "1" ]; then
            local _all_blocked_reason
            _all_blocked_reason="$(_autospec_conductor_all_blocked_single_reason "$_queue_json")"
            if [ "$_all_blocked_reason" = "safety_gate_failed" ]; then
                _autospec_conductor_escalate_all_blocked \
                    "$_repo" "$_queue_json" "$_all_blocked_count" "$_all_blocked_refs" "$_no_digest"
                printf '[conductor] blocked-backlog: all implementation candidates failed the autospec safety gate — pausing discovery until readiness is repaired\n' >&2
                _stop_reason="blocked-backlog:safety_gate_failed"
                break
            fi
        fi

        # ── GROWTH fold-in: capability detection ──────────────────────────────
        # Growth tiers are opted-in per repo via .autospec/growth.yml. Growth
        # stays fully inert (byte-identical waterfall/loop behavior) unless the
        # file exists AND passes validate-growth-config.sh — any missing file,
        # missing yq/jq, or validation failure leaves _growth_enabled=0 and no
        # --growth-* flags are threaded into the waterfall call below.
        local _growth_enabled=0
        local _growth_control_repo=""
        local _growth_backlog_floor=""
        if [ -f "${_repo_root}/.autospec/growth.yml" ] && command -v yq >/dev/null 2>&1; then
            local _gv="${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/validate-growth-config.sh"
            [ -f "$_gv" ] || _gv="${_sdir}/validate-growth-config.sh"
            if [ -f "$_gv" ]; then
                local _gjson
                _gjson="$(yq -o=json '.' "${_repo_root}/.autospec/growth.yml" 2>/dev/null \
                    || yq '.' "${_repo_root}/.autospec/growth.yml" 2>/dev/null \
                    || true)"
                if [ -n "$_gjson" ]; then
                    local _gjson_tmp
                    _gjson_tmp="$(mktemp 2>/dev/null || printf '%s/.autospec-growth-cfg.%s.json' "${TMPDIR:-/tmp}" "$$")"
                    printf '%s' "$_gjson" > "$_gjson_tmp" 2>/dev/null || true
                    if bash "$_gv" "$_gjson_tmp" >/dev/null 2>&1; then
                        _growth_enabled=1
                        # Repo that holds the growth/needs-approval control
                        # issues (Tier G2's approval-servicing signal). Defaults
                        # to the product repo when approval.control_repo is unset.
                        _growth_control_repo="$(printf '%s' "$_gjson" | jq -r '.approval.control_repo // empty' 2>/dev/null || true)"
                        # Configured backlog floor for Tier G1 (grow-define);
                        # honored symmetrically with grow.measure_interval. Empty
                        # -> the waterfall keeps its env/default floor.
                        _growth_backlog_floor="$(printf '%s' "$_gjson" | jq -r '.grow.backlog_floor // empty' 2>/dev/null || true)"
                        case "$_growth_backlog_floor" in *[!0-9]*) _growth_backlog_floor="" ;; esac
                    fi
                    rm -f "$_gjson_tmp" 2>/dev/null || true
                fi
            fi
        elif [ -f "${_repo_root}/.autospec/growth.yml" ] && [ "$_cycle" -eq 1 ]; then
            # growth.yml present but yq missing: growth stays disabled
            # (fail-closed). Say so ONCE (first cycle only — not every cycle of
            # a perpetual loop) so the operator can diagnose why their opted-in
            # repo shows no growth activity.
            printf '[conductor] growth.yml present but yq not installed — growth tiers disabled (install yq to enable)\n' >&2
        fi
        [ -n "$_growth_control_repo" ] || _growth_control_repo="$_repo"

        # ── GROWTH fold-in: cheap per-cycle state (only when enabled) ─────────
        # Each query is individually guarded: a `gh`/helper failure yields
        # 0/not-due rather than aborting the cycle (growth tiers never take the
        # loop down).
        local _growth_flags=""
        if [ "$_growth_enabled" = "1" ]; then
            local _g_backlog _g_outbound _g_approvals _g_outbound_pending _g_measure_due
            _g_backlog="$(gh issue list --repo "$_repo" --state open --label growth:artifact --json number --jq 'length' 2>/dev/null || echo '')"
            case "$_g_backlog" in *[!0-9]*) _g_backlog="" ;; esac
            # Drafts awaiting R2 (product repo).
            _g_outbound="$(gh issue list --repo "$_repo" --state open --label growth/needs-draft --json number --jq 'length' 2>/dev/null || echo 0)"
            case "$_g_outbound" in *[!0-9]*) _g_outbound=0 ;; esac
            # Human-decided approval control issues awaiting R3 (control repo).
            # Spec Tier G2: service-growth-outbound must also fire on any open
            # growth/needs-approval issue carrying a decision label. Without this,
            # R3 only runs by accident of a still-open draft. `growth/published`
            # is included so a human-confirmed post self-triggers R3 to record
            # the terminal published ledger line and close the issue (otherwise
            # that attribution line is only written opportunistically).
            _g_approvals="$(gh issue list --repo "$_growth_control_repo" --state open --label growth/needs-approval --json labels \
                --jq '[.[] | select(.labels | map(.name) | any(. == "growth/approved" or . == "growth/edited" or . == "growth/rejected" or . == "growth/published"))] | length' 2>/dev/null || echo 0)"
            case "$_g_approvals" in *[!0-9]*) _g_approvals=0 ;; esac
            # Outbound tier fires on drafts-to-draft OR approvals-to-service.
            _g_outbound_pending=$(( _g_outbound + _g_approvals ))
            _g_measure_due="$(bash "${_sdir}/growth-measure-due.sh" "$_repo_root" 2>/dev/null || echo 0)"
            _growth_flags="--growth-enabled 1 --growth-outbound-pending ${_g_outbound_pending:-0} --tierg-dry-cycles ${_tierg_dry_cycles:-0}"
            # if/then/fi, not `[ ] && ...`: a failing test short-circuits to a
            # non-zero statement exit under `set -e` and would abort the loop.
            if [ -n "$_g_backlog" ]; then
                _growth_flags="$_growth_flags --growth-backlog $_g_backlog"
            fi
            if [ "$_g_measure_due" = "1" ]; then
                _growth_flags="$_growth_flags --growth-measure-due 1"
            fi
            if [ -n "$_growth_backlog_floor" ]; then
                _growth_flags="$_growth_flags --growth-backlog-floor $_growth_backlog_floor"
            fi
        fi

        # ── Step 3: Waterfall tier selection ──────────────────────────────────
        local _tier_json
        local _waterfall_backlog_args=()
        # Preserve an explicit zero from the dependency-aware queue snapshot.
        # `${_ready_count:+...}` drops zero, causing the waterfall to re-query
        # GitHub and mistake blocked/non-ready issues for runnable backlog,
        # trapping the conductor in Tier 1 forever.
        if [ -n "$_ready_count" ]; then
            _waterfall_backlog_args=(--backlog-count "$_ready_count")
        fi
        _tier_json="$(bash "$_waterfall" \
            --dry-cycles "$_dry_cycles" \
            --tier15-dry-cycles "$_tier15_dry_cycles" \
            --tier2-dry-cycles "$_tier2_dry_cycles" \
            --tier3-dry-cycles "$_tier3_dry_cycles" \
            --tier4-dry-cycles "$_tier4_dry_cycles" \
            ${_repo:+--repo "$_repo"} \
            "${_waterfall_backlog_args[@]}" \
            ${_growth_flags} \
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

        if ! _autospec_conductor_accountability_event selected \
            "Selected Tier ${_tier} action ${_action}" \
            "The waterfall chose the highest-priority runnable workstream" \
            "${_reason:-waterfall selection}" 0; then
            printf '[conductor] HALT: accountability selection event journal failed\n' >&2
            _stop_reason="accountability:journal-failed"
            break
        fi

        if { [ "$_action" = "run-explore-once" ] \
                || [ "$_action" = "run-architecture-improvement" ] \
                || [ "$_action" = "run-explore-once-internet" ]; } \
                && [ "${AUTOSPEC_ALLOW_UNSTEERED_GENERATION:-1}" != "1" ] \
                && [ ! -s "$_priorities_file" ] \
                && [ ! -s "$_eff_persona" ]; then
            if [ "$_inferred_source_count" -gt 0 ]; then
                printf '[conductor] inferred steering bundle present: sources=%s confidence=%s\n' \
                    "$_inferred_source_count" "$_inferred_confidence" >&2
            else
                _action="park"
                _reason="bootstrap-empty-intent-bundle"
                _autospec_conductor_handle_empty_inference_bundle "$_repo_root" "$_repo"
            fi
        fi

        printf '[conductor] tier=%s action=%s\n' "$_tier" "$_action" >&2

        # ── Step 4 + 5: Tier-1 drain gated on premerge check ─────────────────
        local _work_done=0
        local _filed_issues=0
        if [ "$_action" = "park" ]; then
            printf '[conductor] parking: %s\n' "$_reason" >&2
            _stop_reason="waterfall:park:${_reason}"
            break
        elif [ "$_action" = "idle-rescan" ]; then
            # ── Never-idle: value-floor / all-tiers-dry idle (F1) ─────────────
            # A fully-dry cascade is NOT a convergence-stop. Arm resume context
            # (belt-and-suspenders if the process dies mid-idle), notify async/
            # informationally, sleep the re-scan interval, then CONTINUE the loop.
            # Falls through (no break/continue) so resource-park (governor/spend)
            # and the --max-cycles cap below still apply — idle never bypasses them.
            printf '[conductor] idle-rescan: %s\n' "$_reason" >&2
            _conductor_arm_resume \
                "$_sdir" "$_repo" "$_conductor_session" \
                "$_notify_sh" "idle-rescan:${_reason}"
            # A rescan must restart the full waterfall.  Preserve the
            # never-idle loop, but clear exhausted dry counters so the next
            # selection re-enters Tier 1 instead of emitting idle-rescan
            # forever with the same counters.
            _dry_cycles=0
            _tier15_dry_cycles=0
            _tier2_dry_cycles=0
            _tier3_dry_cycles=0
            _tier4_dry_cycles=0
            _tierg_dry_cycles=0
            _work_done=0
            if [ "$_dry" != "1" ] && [ "$_rescan_interval" -gt 0 ] 2>/dev/null; then
                printf '[conductor] idle-rescan: sleeping %ss before re-scan\n' \
                    "$_rescan_interval" >&2
                sleep "$_rescan_interval" || true
            fi
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

            local _promote_dry _promote_filed _promote_promoted
            _promote_dry="$(printf '%s' "$_promote_out" | jq -r 'if has("dry") then .dry else true end' 2>/dev/null || echo 'true')"
            _promote_filed="$(printf '%s' "$_promote_out" | jq -r '
                if (.filed | type) == "number" then .filed
                elif (.filed | type) == "string" then (.filed | tonumber? // 0)
                else 0 end
            ' 2>/dev/null || echo 0)"
            _promote_promoted="$(printf '%s' "$_promote_out" | jq -r '
                if (.promoted | type) == "array" then (.promoted | length)
                elif (.promoted | type) == "number" then .promoted
                elif (.promoted | type) == "string" then (.promoted | tonumber? // 0)
                else 0 end
            ' 2>/dev/null || echo 0)"
            case "$_promote_filed" in ''|*[!0-9]*) _promote_filed=0 ;; esac
            case "$_promote_promoted" in ''|*[!0-9]*) _promote_promoted=0 ;; esac
            printf '[conductor] Tier 1.5 promotion result: dry=%s filed=%s promoted=%s
'                 "$_promote_dry" "$_promote_filed" "$_promote_promoted" >&2

            # ── Grooming telemetry + self-governance tick ─────────────────────
            # The promoter (autonomous-promote-open-issues.sh) already owns the
            # deterministic safety→classify→eligibility→promote/groom/split/hold
            # pipeline and its own policy gate — the loop does NOT duplicate that
            # logic. It only (a) records telemetry: one line per eligible promote
            # (template_groomed:false) and one line per template groom routed by
            # the promoter (template_groomed:true, canary per action), each with
            # outcome:null; (b) runs a reconcile pass (groom-reconcile.sh) that
            # stamps closing_pr + outcome onto unresolved+closed records from
            # GitHub BEFORE observing; and (c) ticks the self-governance ratchet
            # (grooming-govern.sh) when policy resolves to auto. A non-auto
            # policy must NOT tick (guarded below). Skipped entirely during the
            # conductor's own --dry-run (nothing real happened this cycle).
            #
            # grooming template-fill is performed deterministically by the
            # promoter via groom-fill.sh (codex exec) — canary(seed)/auto
            # (graduated); the loop only records telemetry + ticks governance.
            if [ "$_dry" != "1" ]; then
                local _groom_config_sh="${AUTOSPEC_GROOMING_CONFIG_BIN:-}"
                if [ -z "$_groom_config_sh" ]; then
                    if [ -f "${_sdir}/grooming-config.sh" ]; then
                        _groom_config_sh="${_sdir}/grooming-config.sh"
                    elif [ -f "${_sdir}/../skills/autospec-shared/scripts/grooming-config.sh" ]; then
                        _groom_config_sh="${_sdir}/../skills/autospec-shared/scripts/grooming-config.sh"
                    fi
                fi
                local _groom_observe_sh="${AUTOSPEC_GROOMING_OBSERVE_BIN:-}"
                if [ -z "$_groom_observe_sh" ]; then
                    if [ -f "${_sdir}/grooming-observe.sh" ]; then
                        _groom_observe_sh="${_sdir}/grooming-observe.sh"
                    elif [ -f "${_sdir}/../skills/autospec-shared/scripts/grooming-observe.sh" ]; then
                        _groom_observe_sh="${_sdir}/../skills/autospec-shared/scripts/grooming-observe.sh"
                    fi
                fi
                local _groom_govern_sh="${AUTOSPEC_GROOMING_GOVERN_BIN:-}"
                if [ -z "$_groom_govern_sh" ]; then
                    if [ -f "${_sdir}/grooming-govern.sh" ]; then
                        _groom_govern_sh="${_sdir}/grooming-govern.sh"
                    elif [ -f "${_sdir}/../skills/autospec-shared/scripts/grooming-govern.sh" ]; then
                        _groom_govern_sh="${_sdir}/../skills/autospec-shared/scripts/grooming-govern.sh"
                    fi
                fi

                local _groom_policy="auto"
                if [ -n "$_groom_config_sh" ]; then
                    _groom_policy="$(bash "$_groom_config_sh" --key policy 2>/dev/null || printf 'auto')"
                    [ -n "$_groom_policy" ] || _groom_policy="auto"
                fi

                # Append telemetry records (jq-built, never printf, so the JSON is
                # always well-formed). Eligible promotes populate the baseline
                # (template_groomed:false); promoter-routed template grooms
                # (action groom-canary|groom-auto) populate the graduation sample
                # (template_groomed:true, canary per action). outcome + closing_pr
                # start null and are stamped later by the reconcile pass.
                local _groom_telemetry="${AUTOSPEC_GROOMING_TELEMETRY:-${HOME}/.autospec/grooming-telemetry.jsonl}"
                local _groom_promoted_json
                _groom_promoted_json="$(printf '%s' "$_promote_out" | jq -c '.promoted // []' 2>/dev/null || printf '[]')"
                local _groom_promoted_count
                _groom_promoted_count="$(printf '%s' "$_groom_promoted_json" | jq 'length' 2>/dev/null || printf '0')"
                case "$_groom_promoted_count" in ''|*[!0-9]*) _groom_promoted_count=0 ;; esac
                local _groom_routed_json
                _groom_routed_json="$(printf '%s' "$_promote_out" | jq -c '[.routed[]? | select(.action|type=="string" and startswith("groom-"))]' 2>/dev/null || printf '[]')"
                local _groom_routed_count
                _groom_routed_count="$(printf '%s' "$_groom_routed_json" | jq 'length' 2>/dev/null || printf '0')"
                case "$_groom_routed_count" in ''|*[!0-9]*) _groom_routed_count=0 ;; esac
                if [ "$_groom_promoted_count" -gt 0 ] 2>/dev/null || [ "$_groom_routed_count" -gt 0 ] 2>/dev/null; then
                    mkdir -p "$(dirname "$_groom_telemetry")" 2>/dev/null || true
                    local _groom_ts
                    _groom_ts="$(date -u +'%Y-%m-%dT%H:%M:%SZ' 2>/dev/null || echo 'unknown')"
                    # Eligible promotes → baseline population (template_groomed:false).
                    local _gi=0
                    while [ "$_gi" -lt "$_groom_promoted_count" ]; do
                        local _gnum
                        _gnum="$(printf '%s' "$_groom_promoted_json" | jq -r ".[$_gi]" 2>/dev/null || printf '')"
                        if [ -n "$_gnum" ]; then
                            jq -cn \
                                --argjson issue "$_gnum" \
                                --arg ts "$_groom_ts" \
                                '{ts:$ts, issue:$issue, source:"grooming", template_groomed:false,
                                  closing_pr:null, outcome:null}' \
                                >> "$_groom_telemetry" 2>/dev/null || true
                        fi
                        _gi=$((_gi + 1))
                    done
                    # Template grooms → graduation sample (template_groomed:true).
                    local _rj=0
                    while [ "$_rj" -lt "$_groom_routed_count" ]; do
                        local _rnum _ract
                        _rnum="$(printf '%s' "$_groom_routed_json" | jq -r ".[$_rj].issue" 2>/dev/null || printf '')"
                        _ract="$(printf '%s' "$_groom_routed_json" | jq -r ".[$_rj].action" 2>/dev/null || printf '')"
                        if [ -n "$_rnum" ] && [ "$_rnum" != "null" ]; then
                            local _canary=false
                            if [ "$_ract" = "groom-canary" ]; then
                                _canary=true
                            fi
                            jq -cn \
                                --argjson issue "$_rnum" \
                                --arg ts "$_groom_ts" \
                                --argjson canary "$_canary" \
                                '{ts:$ts, issue:$issue, source:"grooming", template_groomed:true,
                                  canary:$canary, closing_pr:null, outcome:null}' \
                                >> "$_groom_telemetry" 2>/dev/null || true
                        fi
                        _rj=$((_rj + 1))
                    done
                    printf '[conductor] Tier 1.5 grooming telemetry: appended %s promote + %s groom record(s) to %s\n' \
                        "$_groom_promoted_count" "$_groom_routed_count" "$_groom_telemetry" >&2
                fi

                # Reconcile outcomes into the telemetry log BEFORE observing —
                # stamps closing_pr + outcome for unresolved+closed records from
                # GitHub (fail-closed: a gh failure leaves the record unresolved).
                # Resolved the same lazy way as observe/govern; skipped when policy
                # is off. (_dry != 1 is already guaranteed by the enclosing guard.)
                if [ "$_groom_policy" != "off" ]; then
                    local _groom_reconcile_sh="${AUTOSPEC_GROOMING_RECONCILE_BIN:-}"
                    if [ -z "$_groom_reconcile_sh" ]; then
                        if [ -f "${_sdir}/groom-reconcile.sh" ]; then
                            _groom_reconcile_sh="${_sdir}/groom-reconcile.sh"
                        elif [ -f "${_sdir}/../scripts/groom-reconcile.sh" ]; then
                            _groom_reconcile_sh="${_sdir}/../scripts/groom-reconcile.sh"
                        fi
                    fi
                    if [ -n "$_groom_reconcile_sh" ] && [ -f "$_groom_telemetry" ]; then
                        bash "$_groom_reconcile_sh" \
                            --telemetry "$_groom_telemetry" \
                            --repo "$_repo" >/dev/null 2>&1 || true
                    fi
                fi

                # Self-governance tick — auto policy ONLY (must not tick on/off).
                if [ "$_groom_policy" = "auto" ] && [ -n "$_groom_observe_sh" ] && [ -n "$_groom_govern_sh" ]; then
                    local _groom_observed
                    _groom_observed="$(bash "$_groom_observe_sh" --telemetry "$_groom_telemetry" 2>/dev/null || printf '')"
                    if [ -n "$_groom_observed" ]; then
                        # NOTE: canary_floor is the SINGLE floor govern applies to
                        # BOTH the groomed-sample count AND the baseline-sample count
                        # (its widen-guard needs baseline_samples >= min-samples). So
                        # canary->auto graduation can't fire until >= canary_floor
                        # eligible-promotes have ALSO closed+reconciled — intentional
                        # fail-closed coupling (no widening without a real baseline).
                        local _groom_min_samples="${AUTOSPEC_GROOMING_MIN_SAMPLES:-}"
                        if [ -z "$_groom_min_samples" ] && [ -n "$_groom_config_sh" ]; then
                            _groom_min_samples="$(bash "$_groom_config_sh" --key budget.canary_floor 2>/dev/null || printf '5')"
                        fi
                        case "$_groom_min_samples" in ''|*[!0-9]*) _groom_min_samples=5 ;; esac
                        local _groom_tick_out
                        _groom_tick_out="$(bash "$_groom_govern_sh" tick \
                            --observed "$_groom_observed" \
                            --min-samples "$_groom_min_samples" 2>/dev/null || printf '')"
                        if [ -n "$_groom_tick_out" ]; then
                            printf '[conductor] Tier 1.5 grooming governance tick: %s\n' \
                                "$_groom_tick_out" >&2
                        fi
                    fi
                fi
            fi

            if [ "$_promote_filed" -gt 0 ] 2>/dev/null || [ "$_promote_promoted" -gt 0 ] 2>/dev/null; then
                _work_done=1
                _filed_issues=$((_filed_issues + _promote_filed))
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
            # autospec queue ready call the waterfall's --backlog-count used) —
            # do not re-query.
            if [ -n "$_queue_bin" ] && { [ -x "$_queue_bin" ] || command -v "$_queue_bin" >/dev/null 2>&1; } && [ -n "$_repo" ]; then
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
                    # Main-health admission is owned by Rust before this legacy
                    # conductor can consider or drain a ready issue. Keep this
                    # shell path non-authoritative so missing branches cannot be
                    # downgraded to a silent wait here.
                    local _main_health="continue"

                    if [ "$_main_health" = "continue" ]; then
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
                                local _tier1_drain_dispatched=0
                                # ── Dispatch-time provenance split (integration-branch
                                # design §Architecture item 5). Provenance is re-resolved
                                # from GitHub labels EVERY cycle — never cached in memory
                                # — so a crash/resume re-derives identical routing. self
                                # issues dispatch with the kind=integration mode file
                                # active (Phase 4 targets the integration branch);
                                # operator issues dispatch with the mode file parked
                                # (Phase 4 targets the parent as today). Lock-step rules
                                # are unchanged: the split only scopes each dispatch via
                                # AUTOSPEC_RUN_ONLY_ISSUES; dep gating stays in the run
                                # path. Split requires the resolver + branch scripts, a
                                # repo slug, and a non-empty batch snapshot; otherwise
                                # single-dispatch exactly as before.
                                local _prov_batch=""
                                if [ -n "$_provenance_sh" ] && [ -n "$_intbranch_sh" ] \
                                    && [ -n "$_repo" ] && [ -n "${_queue_json:-}" ]; then
                                    _prov_batch="$(printf '%s' "$_queue_json" \
                                        | jq -r '.batch[]?.number // empty' 2>/dev/null || true)"
                                fi
                                if [ -n "$_prov_batch" ]; then
                                    local _prov_self="" _prov_operator="" _prov_n _prov_val _prov_parent_branch
                                    _prov_parent_branch="$(_autospec_conductor_default_branch "$_repo")"
                                    while IFS= read -r _prov_n; do
                                        if [ -z "$_prov_n" ]; then
                                            continue
                                        fi
                                        # Fail closed: resolver failure or unexpected
                                        # output routes the issue to the integration
                                        # branch (self), never to the parent.
                                        _prov_val="$(bash "$_provenance_sh" resolve \
                                            --issue "$_prov_n" --repo "$_repo" \
                                            2>/dev/null || printf 'self')"
                                        if [ "$_prov_val" = "operator" ]; then
                                            _prov_operator="${_prov_operator:+${_prov_operator} }${_prov_n}"
                                        else
                                            _prov_self="${_prov_self:+${_prov_self} }${_prov_n}"
                                        fi
                                    done <<EOF_PROV_BATCH
$_prov_batch
EOF_PROV_BATCH
                                    # Operator subset first: park a kind=integration
                                    # mode file so Phase 4 targets the parent. A
                                    # kind=explore mode file (standalone explore
                                    # session) is never touched here.
                                    if [ -n "$_prov_operator" ]; then
                                        local _prov_mode_file="${_repo_root}/.autospec/explore-mode.json"
                                        local _prov_mode_kind=""
                                        if [ -f "$_prov_mode_file" ]; then
                                            _prov_mode_kind="$(jq -r '.kind // empty' \
                                                "$_prov_mode_file" 2>/dev/null || printf '')"
                                        fi
                                        if [ "$_prov_mode_kind" = "integration" ]; then
                                            mv -f "$_prov_mode_file" "${_prov_mode_file}.parked" 2>/dev/null \
                                                || rm -f "$_prov_mode_file" 2>/dev/null || true
                                            printf '[conductor] provenance: parked integration mode file for operator batch\n' >&2
                                        fi
                                        printf '[conductor] provenance: dispatching operator batch (issues: %s) -> parent\n' \
                                            "$_prov_operator" >&2
                                        AUTOSPEC_NO_SELF_UPDATE=1 \
                                            AUTOSPEC_RUN_ONLY_ISSUES="$_prov_operator" \
                                            bash -c "$_run_cmd" 2>&1 || true
                                        _tier1_drain_dispatched=1
                                    fi
                                    # Self subset: ensure + sync the integration branch
                                    # so its kind=integration mode file routes Phase 4
                                    # PRs to it. A sync merge conflict (exit 65) parks
                                    # the self subset this cycle and notifies — never
                                    # bypassed; the operator dispatch above is
                                    # unaffected. Any other ensure/sync failure also
                                    # parks the self subset (fail closed: self work is
                                    # never dispatched at the parent).
                                    #
                                    # ── Self-merge aftermath pre-gate (spec item 7 caps
                                    # + item 5/Error-handling rollup-red pause). A
                                    # durable pause marker (written by the aftermath
                                    # block below on rollup-red / post-merge sync
                                    # conflict) and the live caps probe both park the
                                    # self subset BEFORE dispatch — the operator subset
                                    # above is never affected by either.
                                    local _sm_pause_file="${_repo_root}/.autospec/self-originated-pause.json"
                                    local _sm_park_reason=""
                                    if [ -n "$_prov_self" ] && [ -f "$_sm_pause_file" ]; then
                                        _sm_park_reason="$(jq -r '.reason // "paused"' \
                                            "$_sm_pause_file" 2>/dev/null || printf 'paused')"
                                    fi
                                    if [ -n "$_prov_self" ] && [ -z "$_sm_park_reason" ] && [ -n "$_intbranch_sh" ]; then
                                        local _sm_status_json="" _sm_status_rc=0
                                        _sm_status_json="$(bash "$_intbranch_sh" status --parent "$_prov_parent_branch" \
                                            ${_repo:+--repo "$_repo"} 2>/dev/null)" || _sm_status_rc=$?
                                        if [ "$_sm_status_rc" -eq 0 ] && [ -n "$_sm_status_json" ]; then
                                            local _sm_runtime_config_sh=""
                                            if [ -f "${_repo_root}/scripts/autospec-runtime-config.sh" ]; then
                                                _sm_runtime_config_sh="${_repo_root}/scripts/autospec-runtime-config.sh"
                                            elif [ -f "${_sdir}/autospec-runtime-config.sh" ]; then
                                                _sm_runtime_config_sh="${_sdir}/autospec-runtime-config.sh"
                                            elif [ -f "$HOME/.autospec/scripts/autospec-runtime-config.sh" ]; then
                                                _sm_runtime_config_sh="$HOME/.autospec/scripts/autospec-runtime-config.sh"
                                            fi
                                            if [ -n "$_sm_runtime_config_sh" ]; then
                                                # shellcheck source=/dev/null
                                                . "$_sm_runtime_config_sh"
                                            fi
                                            local _sm_cap_open _sm_cap_age _sm_cap_diff
                                            local _sm_max_open _sm_max_age _sm_max_diff
                                            _sm_cap_open="$(printf '%s' "$_sm_status_json" | jq -r '.accumulated_pr_count // 0' 2>/dev/null || echo 0)"
                                            _sm_cap_age="$(printf '%s' "$_sm_status_json" | jq -r '.age_days // 0' 2>/dev/null || echo 0)"
                                            _sm_cap_diff="$(printf '%s' "$_sm_status_json" | jq -r '.diff_lines // 0' 2>/dev/null || echo 0)"
                                            if command -v autospec_runtime_config_get >/dev/null 2>&1; then
                                                _sm_max_open="$(autospec_runtime_config_get "autonomous.self_originated.max_open_prs" "20")"
                                                _sm_max_age="$(autospec_runtime_config_get "autonomous.self_originated.max_age_days" "14")"
                                                _sm_max_diff="$(autospec_runtime_config_get "autonomous.self_originated.max_diff_lines" "5000")"
                                            else
                                                _sm_max_open=20; _sm_max_age=14; _sm_max_diff=5000
                                            fi
                                            case "$_sm_cap_open" in *[!0-9]*|'') _sm_cap_open=0 ;; esac
                                            case "$_sm_cap_age" in *[!0-9]*|'') _sm_cap_age=0 ;; esac
                                            case "$_sm_cap_diff" in *[!0-9]*|'') _sm_cap_diff=0 ;; esac
                                            case "$_sm_max_open" in *[!0-9]*|'') _sm_max_open=20 ;; esac
                                            case "$_sm_max_age" in *[!0-9]*|'') _sm_max_age=14 ;; esac
                                            case "$_sm_max_diff" in *[!0-9]*|'') _sm_max_diff=5000 ;; esac
                                            if [ "$_sm_cap_open" -gt "$_sm_max_open" ]; then
                                                _sm_park_reason="max_open_prs:${_sm_cap_open}>${_sm_max_open}"
                                            elif [ "$_sm_cap_age" -gt "$_sm_max_age" ]; then
                                                _sm_park_reason="max_age_days:${_sm_cap_age}>${_sm_max_age}"
                                            elif [ "$_sm_cap_diff" -gt "$_sm_max_diff" ]; then
                                                _sm_park_reason="max_diff_lines:${_sm_cap_diff}>${_sm_max_diff}"
                                            fi
                                        fi
                                    fi

                                    if [ -n "$_prov_self" ] && [ -n "$_sm_park_reason" ]; then
                                        printf 'code_health:self_originated_parked\n' >&2
                                        printf '[conductor] self-originated tiers parked (%s) — issues: %s\n' \
                                            "$_sm_park_reason" "$_prov_self" >&2
                                        if [ -n "$_notify_sh" ]; then
                                            bash "$_notify_sh" "autospec-autonomous" \
                                                "self-originated tiers parked: ${_sm_park_reason}" || true
                                        fi
                                    elif [ -n "$_prov_self" ]; then
                                        local _prov_int_rc=0 _prov_requested_branch _prov_int_output _prov_int_tmp
                                        _prov_requested_branch="$(_autospec_conductor_integration_requested_branch "$_prov_parent_branch")"
                                        if _autospec_conductor_integration_conflict_cooldown_active \
                                            "$_repo_root" "$_prov_self" "$_cycle" "$_prov_requested_branch"; then
                                            _prov_int_rc=6
                                        else
                                            _prov_int_tmp="$(mktemp -t autospec-intbranch-XXXXXX.log 2>/dev/null || printf '/tmp/autospec-intbranch-%s.log' "$$")"
                                            : > "$_prov_int_tmp" 2>/dev/null || true
                                            bash "$_intbranch_sh" ensure --parent "$_prov_parent_branch" \
                                                --repo "$_repo" 2>"$_prov_int_tmp" || _prov_int_rc=$?
                                            _prov_int_output="$(cat "$_prov_int_tmp" 2>/dev/null || true)"
                                            [ -z "$_prov_int_output" ] || printf '%s\n' "$_prov_int_output" >&2
                                            if [ "$_prov_int_rc" -eq 0 ]; then
                                                : > "$_prov_int_tmp" 2>/dev/null || true
                                                bash "$_intbranch_sh" sync --parent "$_prov_parent_branch" \
                                                    --repo "$_repo" 2>"$_prov_int_tmp" || _prov_int_rc=$?
                                                _prov_int_output="$(cat "$_prov_int_tmp" 2>/dev/null || true)"
                                                [ -z "$_prov_int_output" ] || printf '%s\n' "$_prov_int_output" >&2
                                            fi
                                            if [ "$_prov_int_rc" -ne 0 ]; then
                                                _autospec_conductor_arm_integration_conflict_cooldown \
                                                    "$_repo_root" "$_prov_self" "$_cycle" "$_prov_int_rc" \
                                                    "$_prov_int_output" "$_prov_requested_branch"
                                            fi
                                            rm -f "$_prov_int_tmp" 2>/dev/null || true
                                        fi
                                        if [ "$_prov_int_rc" -ne 0 ]; then
                                            # Peer-review must-fix: ensure may have
                                            # written a kind=integration mode file
                                            # before sync failed — park it so nothing
                                            # routes Phase 4 work onto a conflicted /
                                            # unsynced integration branch until a later
                                            # cycle's ensure+sync succeeds.
                                            local _prov_fail_mode="${_repo_root}/.autospec/explore-mode.json"
                                            local _prov_fail_kind=""
                                            if [ -f "$_prov_fail_mode" ]; then
                                                _prov_fail_kind="$(jq -r '.kind // empty' \
                                                    "$_prov_fail_mode" 2>/dev/null || printf '')"
                                            fi
                                            if [ "$_prov_fail_kind" = "integration" ]; then
                                                mv -f "$_prov_fail_mode" "${_prov_fail_mode}.parked" 2>/dev/null \
                                                    || rm -f "$_prov_fail_mode" 2>/dev/null || true
                                            fi
                                        fi
                                        if [ "$_prov_int_rc" -eq 65 ]; then
                                            printf 'code_health:integration_sync_conflict\n' >&2
                                            printf '[conductor] provenance: integration sync conflict — parking self batch (issues: %s)\n' \
                                                "$_prov_self" >&2
                                            if [ -n "$_notify_sh" ]; then
                                                bash "$_notify_sh" "autospec-autonomous" \
                                                    "integration branch sync conflict — self-originated batch parked for operator resolution" || true
                                            fi
                                        elif [ "$_prov_int_rc" -ne 0 ]; then
                                            printf '[conductor] provenance: integration branch unavailable (rc=%s) — parking self batch (issues: %s)\n' \
                                                "$_prov_int_rc" "$_prov_self" >&2
                                        else
                                            printf '[conductor] provenance: dispatching self batch (issues: %s) -> integration branch\n' \
                                                "$_prov_self" >&2
                                            AUTOSPEC_NO_SELF_UPDATE=1 \
                                                AUTOSPEC_RUN_ONLY_ISSUES="$_prov_self" \
                                                bash -c "$_run_cmd" 2>&1 || true
                                            _tier1_drain_dispatched=1

                                            # ── Self-merge aftermath (spec item 5 tail):
                                            # after a self-originated PR merges into the
                                            # integration branch, sync the parent in and
                                            # run rollup-update so the roll-up PR gains a
                                            # manifest entry + per-feature comment. Merge
                                            # detection comes from the SAME per-cycle
                                            # outcome file Step 5b already consumes below
                                            # — extended here with self_originated/pr
                                            # fields; a Tier-1 backlog cycle that never
                                            # wrote one is a silent no-op.
                                            local _sm_outcome_file
                                            _sm_outcome_file="${AUTOSPEC_LAST_OUTCOME_FILE:-${_repo_root}/.autospec/last-outcome.json}"
                                            if [ -f "$_sm_outcome_file" ]; then
                                                local _sm_is_self _sm_out_val _sm_issue _sm_pr
                                                _sm_is_self="$(jq -r '.self_originated // false' \
                                                    "$_sm_outcome_file" 2>/dev/null || printf 'false')"
                                                _sm_out_val="$(jq -r '.outcome // ""' \
                                                    "$_sm_outcome_file" 2>/dev/null || printf '')"
                                                if [ "$_sm_is_self" = "true" ] && [ "$_sm_out_val" = "merged" ]; then
                                                    _sm_issue="$(jq -r '.issue // empty' \
                                                        "$_sm_outcome_file" 2>/dev/null || printf '')"
                                                    _sm_pr="$(jq -r '.pr // empty' \
                                                        "$_sm_outcome_file" 2>/dev/null || printf '')"
                                                    case "$_sm_issue" in ''|*[!0-9]*) _sm_issue="" ;; esac
                                                    case "$_sm_pr" in ''|*[!0-9]*) _sm_pr="" ;; esac
                                                    if [ -n "$_sm_issue" ] && [ -n "$_sm_pr" ]; then
                                                        local _sm_sync_rc=0
                                                        bash "$_intbranch_sh" sync --parent "$_prov_parent_branch" \
                                                            ${_repo:+--repo "$_repo"} 1>&2 || _sm_sync_rc=$?
                                                        if [ "$_sm_sync_rc" -eq 65 ]; then
                                                            printf 'code_health:integration_sync_conflict\n' >&2
                                                            printf '[conductor] selfmerge-aftermath: post-merge sync conflict — parking self-originated tiers\n' >&2
                                                            printf '{"reason":"sync_conflict","issue":%s,"pr":%s}\n' \
                                                                "$_sm_issue" "$_sm_pr" > "$_sm_pause_file" 2>/dev/null || true
                                                            if [ -n "$_notify_sh" ]; then
                                                                bash "$_notify_sh" "autospec-autonomous" \
                                                                    "integration branch post-merge sync conflict — self-originated tiers parked" || true
                                                            fi
                                                        elif [ "$_sm_sync_rc" -ne 0 ]; then
                                                            printf '[conductor] selfmerge-aftermath: post-merge sync failed rc=%s — parking self-originated tiers\n' \
                                                                "$_sm_sync_rc" >&2
                                                            printf '{"reason":"sync_failed","issue":%s,"pr":%s}\n' \
                                                                "$_sm_issue" "$_sm_pr" > "$_sm_pause_file" 2>/dev/null || true
                                                        else
                                                            local _sm_rollup_out _sm_rollup_rc=0
                                                            _sm_rollup_out="$(bash "$_intbranch_sh" rollup-update \
                                                                --parent "$_prov_parent_branch" --issue "$_sm_issue" --pr "$_sm_pr" \
                                                                ${_repo:+--repo "$_repo"} 2>&1)" || _sm_rollup_rc=$?
                                                            printf '%s\n' "$_sm_rollup_out" >&2
                                                            if [ "$_sm_rollup_rc" -ne 0 ]; then
                                                                # rollup-update itself already parks/notifies on gh
                                                                # failure (exit 8/9) or multi-open (exit 9), but that
                                                                # is a one-shot notify — the conductor must ALSO
                                                                # persist a pause marker here so later cycles don't
                                                                # resume dispatch until an operator clears it
                                                                # (peer-review must-fix: a nonzero, non-rollup-red
                                                                # exit must never fall through to the pause-clearing
                                                                # branch below).
                                                                printf '[conductor] selfmerge-aftermath: rollup-update failed rc=%s — parking self-originated tiers\n' \
                                                                    "$_sm_rollup_rc" >&2
                                                                printf '{"reason":"rollup_update_failed","issue":%s,"pr":%s}\n' \
                                                                    "$_sm_issue" "$_sm_pr" > "$_sm_pause_file" 2>/dev/null || true
                                                                if [ -n "$_notify_sh" ]; then
                                                                    bash "$_notify_sh" "autospec-autonomous" \
                                                                        "rollup-update failed (rc=${_sm_rollup_rc}) — self-originated tiers parked" || true
                                                                fi
                                                            elif printf '%s\n' "$_sm_rollup_out" | grep -q '^rollup-red$'; then
                                                                printf '[conductor] selfmerge-aftermath: rollup-red — pausing further self-originated merges\n' >&2
                                                                printf '{"reason":"rollup_red","issue":%s,"pr":%s}\n' \
                                                                    "$_sm_issue" "$_sm_pr" > "$_sm_pause_file" 2>/dev/null || true
                                                                if [ -n "$_notify_sh" ]; then
                                                                    bash "$_notify_sh" "autospec-autonomous" \
                                                                        "roll-up CI red — self-originated merges paused pending green or discard" || true
                                                                fi
                                                            else
                                                                rm -f "$_sm_pause_file" 2>/dev/null || true
                                                            fi
                                                        fi
                                                    fi
                                                fi
                                            fi
                                        fi
                                    fi
                                else
                                    AUTOSPEC_NO_SELF_UPDATE=1 \
                                        bash -c "$_run_cmd" 2>&1 || true
                                    _tier1_drain_dispatched=1
                                fi
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
                                _autospec_conductor_maybe_arm_self_repair_refresh \
                                    "$_repo_root" "$_repo" "$_outcome_file" "$_cycle_main_sha" "$_cycle" || true
                                # Always consume the outcome file so a later cycle never
                                # re-processes a stale outcome (LOW review fix).
                                rm -f "$_outcome_file" 2>/dev/null || true
                            fi
                        fi
                        if [ "${_tier1_drain_dispatched:-0}" -eq 1 ]; then
                            _work_done=1
                        fi
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
                elif [ -f "${_sdir}/autonomous-self-improvement.sh" ]; then
                    printf -v _arch_cmd 'bash %q advance --repo-root %q --review-outcomes %q --gaps %q >/dev/null; bash %q apply --repo-root %q --review-outcomes %q --gaps %q' "${_sdir}/autonomous-self-improvement.sh" "$_repo_root" "$_repo_root/.autospec/review-outcomes.jsonl" "$_repo_root/.autospec/gaps.json" "${_sdir}/autonomous-self-improvement.sh" "$_repo_root" "$_repo_root/.autospec/review-outcomes.jsonl" "$_repo_root/.autospec/gaps.json"
                    if [ -n "$_repo" ]; then
                        printf -v _arch_cmd '%s --repo %q' "$_arch_cmd" "$_repo"
                    fi
                    _arch_cmd="${_arch_cmd} --apply"
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
            case "$_arch_filed" in ''|*[!0-9]*) _arch_filed=0 ;; esac
            printf '[conductor] Tier 3 architecture result: dry=%s filed=%s
'                 "$_arch_dry" "$_arch_filed" >&2
            if [ "$_arch_dry" = "false" ] || { [ "$_arch_filed" -gt 0 ] 2>/dev/null; }; then
                _work_done=1
                _filed_issues=$((_filed_issues + _arch_filed))
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
            # Integration-branch design §Architecture item 2 (unification):
            # under conductor-driven discovery the sandbox IS the integration
            # branch — ensure it so the kind=integration mode file routes
            # Phase 4 PRs to it, instead of minting an ephemeral explore
            # sandbox. Fall back to the ephemeral sandbox when integration
            # routing is unavailable or ensure fails (fail-open to F3).
            local _discovery_mode_ready=0
            if [ -n "$_intbranch_sh" ] && [ -n "$_repo" ]; then
                local _discovery_parent_branch
                _discovery_parent_branch="$(_autospec_conductor_default_branch "$_repo")"
                printf '[conductor] Tier %s: ensuring integration branch as discovery base\n' \
                    "$_tier" >&2
                if bash "$_intbranch_sh" ensure --parent "$_discovery_parent_branch" --repo "$_repo" 1>&2; then
                    _discovery_mode_ready=1
                fi
            fi
            if [ "$_discovery_mode_ready" -eq 0 ] && [ -n "$_sandbox_sh" ]; then
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
                case "$_explore_filed" in ''|*[!0-9]*) _explore_filed=0 ;; esac

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
                    _filed_issues=$((_filed_issues + _explore_filed))
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

        elif [ "$_action" = "run-growth-define" ]; then
            # ── Tier G1: growth candidate research + decompose ─────────────
            local _gd_cmd="${AUTOSPEC_GROWTH_DEFINE_CMD:-}"
            [ -n "$_gd_cmd" ] || _gd_cmd="autospec-grow-define"

            local _gd_out
            if [ "$_dry" = "1" ]; then
                printf '[conductor] [dry-run] would generate Tier G1 growth-define work\n' >&2
                _gd_out='{"dry":true,"filed":0,"reason":"dry-run"}'
            elif [ -n "$_gd_cmd" ]; then
                printf '[conductor] Tier G1: running growth-define\n' >&2
                _gd_out="$(bash -c "$_gd_cmd" 2>/dev/null || printf '{"dry":true,"filed":0,"reason":"growth-define-error"}')"
            else
                printf '[conductor] WARN: no growth-define command available — treating Tier G1 as dry\n' >&2
                _gd_out='{"dry":true,"filed":0,"reason":"growth-define-command-missing"}'
            fi

            local _gd_dry _gd_filed
            _gd_dry="$(printf '%s' "$_gd_out" | jq -r 'if has("dry") then .dry else true end' 2>/dev/null || echo 'true')"
            _gd_filed="$(printf '%s' "$_gd_out" | jq -r '.filed // 0' 2>/dev/null || echo 0)"
            case "$_gd_filed" in ''|*[!0-9]*) _gd_filed=0 ;; esac
            printf '[conductor] Tier G1 growth-define result: dry=%s filed=%s\n' \
                "$_gd_dry" "$_gd_filed" >&2
            if [ "$_gd_dry" = "false" ] || { [ "$_gd_filed" -gt 0 ] 2>/dev/null; }; then
                _work_done=1
                _filed_issues=$((_filed_issues + _gd_filed))
            else
                _tierg_dry_cycles=$((_tierg_dry_cycles + 1))
                printf '[conductor] Tier G1 dry (tierg-dry-cycles=%s)\n' \
                    "$_tierg_dry_cycles" >&2
            fi

        elif [ "$_action" = "service-growth-outbound" ]; then
            # ── Tier G2: outbound draft -> ethics/cadence gate -> approval
            # queue, and servicing pending human approvals. Always "work" when
            # it runs cleanly (no dry-cycle counter — this is a service poll,
            # not a discovery cascade).
            local _go_cmd="${AUTOSPEC_GROWTH_OUTBOUND_CMD:-}"
            # Bare fallback runs grow-run's OUTBOUND-ONLY mode (R0+R2+R3), not
            # the full pipeline — the artifact drain (R1) is already Tier 1's
            # job, so a full-pipeline fallback would redundantly re-drain it.
            [ -n "$_go_cmd" ] || _go_cmd="autospec-grow-run outbound"

            local _go_out
            if [ "$_dry" = "1" ]; then
                printf '[conductor] [dry-run] would service Tier G2 growth outbound queue\n' >&2
                _go_out='{"dry":true,"filed":0,"reason":"dry-run"}'
            elif [ -n "$_go_cmd" ]; then
                printf '[conductor] Tier G2: servicing growth outbound queue\n' >&2
                _go_out="$(bash -c "$_go_cmd" 2>/dev/null || printf '{"dry":true,"filed":0,"reason":"growth-outbound-error"}')"
            else
                printf '[conductor] WARN: no growth-outbound command available — treating Tier G2 as dry\n' >&2
                _go_out='{"dry":true,"filed":0,"reason":"growth-outbound-command-missing"}'
            fi

            local _go_dry
            _go_dry="$(printf '%s' "$_go_out" | jq -r 'if has("dry") then .dry else true end' 2>/dev/null || echo 'true')"
            printf '[conductor] Tier G2 growth-outbound result: dry=%s\n' "$_go_dry" >&2
            if [ "$_go_dry" != "true" ]; then
                _work_done=1
            fi

        elif [ "$_action" = "run-growth-measure" ]; then
            # ── Tier G3: measure & attribute (cadence-gated, not per-cycle) ─
            local _gm_cmd="${AUTOSPEC_GROWTH_MEASURE_CMD:-}"
            # Bare fallback runs grow-run's MEASURE-ONLY mode (R0+R4), not the
            # full pipeline — measure/attribute is all Tier G3 owes; a full
            # fallback would redundantly re-drain artifacts and re-queue outbound.
            [ -n "$_gm_cmd" ] || _gm_cmd="autospec-grow-run measure"

            local _gm_out
            if [ "$_dry" = "1" ]; then
                printf '[conductor] [dry-run] would run Tier G3 growth measure/attribute\n' >&2
                _gm_out='{"dry":true,"filed":0,"reason":"dry-run"}'
            elif [ -n "$_gm_cmd" ]; then
                printf '[conductor] Tier G3: running growth measure/attribute\n' >&2
                _gm_out="$(bash -c "$_gm_cmd" 2>/dev/null || printf '{"dry":true,"filed":0,"reason":"growth-measure-error"}')"
            else
                printf '[conductor] WARN: no growth-measure command available — treating Tier G3 as dry\n' >&2
                _gm_out='{"dry":true,"filed":0,"reason":"growth-measure-command-missing"}'
            fi

            local _gm_dry
            _gm_dry="$(printf '%s' "$_gm_out" | jq -r 'if has("dry") then .dry else true end' 2>/dev/null || echo 'true')"
            printf '[conductor] Tier G3 growth-measure result: dry=%s\n' "$_gm_dry" >&2
            if [ "$_gm_dry" != "true" ]; then
                _work_done=1
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
            _tierg_dry_cycles=0
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
                    --filed-issues "$_filed_issues" \
                    --budget-issues "$_work_done" \
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
        _new_day="$(AUTOSPEC_CONDUCTOR_INFER_CONFIDENCE="${_inferred_confidence:-}" \
            AUTOSPEC_CONDUCTOR_INFER_SOURCE_COUNT="${_inferred_source_count:-0}" \
            _conductor_maybe_write_digest \
            "$_no_digest" "$_last_digest_day" "$_sdir" "$_repo" "$_dry" 2>&1 \
            | tail -1 || printf '%s' "$_last_digest_day")"
        # _new_day stdout is the updated day; progress log went to stderr.
        # Re-capture cleanly by calling the helper with stderr suppressed.
        _last_digest_day="$(AUTOSPEC_CONDUCTOR_INFER_CONFIDENCE="${_inferred_confidence:-}" \
            AUTOSPEC_CONDUCTOR_INFER_SOURCE_COUNT="${_inferred_source_count:-0}" \
            _conductor_maybe_write_digest \
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
        if [ -n "${AUTOSPEC_CONDUCTOR_INFER_CONFIDENCE:-}" ] \
            && [ "${AUTOSPEC_CONDUCTOR_INFER_CONFIDENCE:-none}" != "none" ]; then
            printf '\n### Inferred steering\n\n'
            printf -- '- **Confidence:** %s-confidence from %s source(s).\n' \
                "$AUTOSPEC_CONDUCTOR_INFER_CONFIDENCE" \
                "${AUTOSPEC_CONDUCTOR_INFER_SOURCE_COUNT:-0}"
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
