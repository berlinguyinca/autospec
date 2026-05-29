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
# Termination — round loop exits when ANY of:
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
#   4  — empty prompt

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

Environment:
  AUTOSPEC_REFINE_MAX_ROUNDS    default 10
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
check_path_allowed() {
    local p="$1"
    case "$p" in
        *.env|*.env.*|*/.env|.env) return 1 ;;
        *credential*|*Credential*|*CREDENTIAL*) return 1 ;;
        *secret*|*Secret*|*SECRET*) return 1 ;;
        *.pem|*.key) return 1 ;;
        */.git/*|.git/*) return 1 ;;
        */node_modules/*|node_modules/*) return 1 ;;
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

# ── slug helper (early — needed by --continue) ────────────────────
slug_from_prompt_early() {
    printf '%s' "$1" | tr '[:upper:]' '[:lower:]' \
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
#   evidence_based_stop | operator_stop | budget_cap_reached.
#
# Per-iteration record + summary table per spec
# (docs/specs/2026-05-28-autospec-refine-design.md §Continuous-iteration mode).

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
    # 4. Nothing found — convergence.
    printf ''
}

run_continue_loop() {
    local loop_slug
    loop_slug="$(slug_from_prompt_early "$PROMPT")"
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
        # WITHOUT --continue so the single-pass path executes.
        local iter_artifact_subdir="$ARTIFACT_DIR/iter-${iter}"
        mkdir -p "$iter_artifact_subdir"
        local refine_log="$iter_artifact_subdir/refine.log"
        local refine_status=0
        bash "$0" "$cur_prompt" --rounds "$ROUNDS" --dry-run \
            --artifact-dir "$iter_artifact_subdir" \
            --repo-root "$REPO_ROOT" \
            --memory-root "$MEMORY_ROOT" \
            > "$refine_log" 2>&1 || refine_status=$?

        local refinement_artifact
        refinement_artifact="$(ls "$iter_artifact_subdir"/*.json 2>/dev/null | head -1)"
        [ -n "$refinement_artifact" ] || refinement_artifact=""

        # Determine where the iteration report lives.
        local report_path=""
        if [ -n "$SIM_ITER_DIR" ]; then
            report_path="$SIM_ITER_DIR/iter-${iter}-report.md"
        else
            report_path="$REPO_ROOT/.autospec/run-summary.md"
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
        # Keyword match: any whitespace token from prompt ≥4 chars present in file.
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
    body+=$'- [ ] `bash scripts/validate.sh` passes locally.\n'
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

# ── helpers ───────────────────────────────────────────────────────
word_count() { printf '%s' "$1" | wc -w | tr -d ' '; }

slug_from_prompt() {
    printf '%s' "$1" | tr '[:upper:]' '[:lower:]' \
        | sed -E 's/[^a-z0-9]+/-/g; s/^-+//; s/-+$//' \
        | cut -c1-40
}

iso_ts() { date -u +'%Y-%m-%dT%H-%M-%SZ'; }

json_escape() {
    # Stream-safe JSON string escape for a bash string. Reads stdin.
    python3 -c 'import json,sys; sys.stdout.write(json.dumps(sys.stdin.read()))'
}

# ── main loop ─────────────────────────────────────────────────────
SLUG="$(slug_from_prompt "$PROMPT")"
[ -n "$SLUG" ] || SLUG="prompt"
TS="$(iso_ts)"
mkdir -p "$ARTIFACT_DIR"
ARTIFACT="$ARTIFACT_DIR/${SLUG}-${TS}.json"

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

    REFINED="$(printf '%s' "$PREV_PROMPT" | apply_lens "$LENS")"

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
{"round_number":$i,"lens":"$LENS","sources_used":$SOURCES_JSON,"refined_prompt":$REFINED_JSON,"diff_summary":"lens=$LENS applied","word_count_delta":$DELTA,"reasoning":"deterministic lens v1"}
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

# ── write artifact ────────────────────────────────────────────────
ORIG_JSON="$(printf '%s' "$PROMPT" | json_escape)"
FINAL_JSON="$(printf '%s' "$FINAL_PROMPT" | json_escape)"
HEAD_SHA="$( ( cd "$REPO_ROOT" && git rev-parse HEAD 2>/dev/null ) || echo unknown )"

# Degraded rounds array.
DEGRADED_JSON="$(printf '%s\n' "${DEGRADED_ROUNDS[@]:-}" | python3 -c 'import json,sys; arr=[l for l in sys.stdin.read().splitlines() if l]; sys.stdout.write(json.dumps(arr))')"

HANDOFF_EXECUTED=false
if [ "$DRY_RUN" = 1 ]; then
    HANDOFF_TARGET="dry-run"
elif [ "$HANDOFF_MODE" = "interactive" ]; then
    HANDOFF_TARGET="/autospec"
    HANDOFF_EXECUTED=true
else
    HANDOFF_TARGET="/autospec --autonomous"
    HANDOFF_EXECUTED=true
fi

CONTEXT_SPARSE_JSON="$CONTEXT_SPARSE"

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
    "handoff_executed": $HANDOFF_EXECUTED
  }
}
EOF

if [ -n "$OUTPUT" ]; then
    if ! check_path_allowed "$OUTPUT"; then
        echo "code_health:refine_path_violation path=$OUTPUT" >&2
        exit 3
    fi
    printf '%s' "$FINAL_PROMPT" > "$OUTPUT"
fi

echo "refine-prompt: status=$STATUS rounds_executed=$ROUNDS_EXECUTED artifact=$ARTIFACT"

# ── handoff dispatch ──────────────────────────────────────────────
# --dry-run skips handoff. Otherwise invoke the autospec entry point via
# `claude` (slash-command dispatcher) if available, falling back to the
# `autospec` binary on PATH. Test harnesses stub both.
if [ "$DRY_RUN" != 1 ]; then
    if command -v claude >/dev/null 2>&1; then
        if [ "$HANDOFF_MODE" = "interactive" ]; then
            claude "/autospec" "$FINAL_PROMPT" || true
        else
            claude "/autospec" "--autonomous" "$FINAL_PROMPT" || true
        fi
    elif command -v autospec >/dev/null 2>&1; then
        if [ "$HANDOFF_MODE" = "interactive" ]; then
            autospec "$FINAL_PROMPT" || true
        else
            autospec --autonomous "$FINAL_PROMPT" || true
        fi
    else
        echo "refine-prompt: WARN — no handoff dispatcher (claude/autospec) on PATH; artifact retained" >&2
    fi
fi

exit 0
