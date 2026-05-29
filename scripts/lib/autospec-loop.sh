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
