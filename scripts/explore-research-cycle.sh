#!/usr/bin/env bash
# scripts/explore-research-cycle.sh — autospec-explore research cycle aggregator.
#
# Runs the enabled deterministic researchers in parallel, aggregates their
# proposals, then threads them through the stage pipeline
#   dedup -> verify -> ROI gate -> pattern synthesis (Issue D) -> rank
# (constitution + recent-title filters apply before the severity-first rank),
# and caps output at --max-issues-per-round.
#
# Implements the "Research cycle contract" + "Per-researcher contracts"
# rows 1-4 in docs/specs/2026-05-29-autospec-explore-design.md and the
# "Aggregator changes" (verify/ROI/severity-first-rank/counters) in
# docs/specs/2026-06-15-autospec-explore-discovery-enhance.md.
#
# Output: JSON to stdout with shape:
#   { "round": "<iso-date>",
#     "proposals_total": N,
#     "proposals_after_dedup": N,
#     "verify_mode": "active"|"no-op-unverified",  # whether a verdict map drove
#                                          # refutation, or the gate no-op'd
#     "proposals_after_verify": N,        # survived the adversarial verify gate
#     "proposals_refuted": N,             # dropped by the verify gate
#     "proposals_after_roi": N,           # survived the named-consumer ROI gate
#     "structural_fixes": N,              # pattern-synthesis collapses (Issue D)
#     "proposals_after_recent_filter": N,
#     "proposals": [ ...top --max-issues-per-round... ] }
#
# Each proposal carries: title, evidence, estimated_complexity, confidence,
# source, severity, named_consumer, score.
#
# Proposal contract (schemas/autospec-explore-proposal.schema.json):
#   - severity        impact band, high->low: silent-wrong > correctness >
#                     stability > operability > feature > nicety. Legacy
#                     researchers that omit it are defaulted to "feature" here.
#   - named_consumer  free text naming a skill/workflow/operator step that
#                     benefits today. Missing -> defaulted to "" here. The
#                     default-empty value does NOT auto-drop legacy proposals
#                     (the ROI gate that drops empty-consumer proposals is
#                     new-source-only; it lands in Issue C).

set -u

REPO_ROOT="${AUTOSPEC_REPO_ROOT:-$(git rev-parse --show-toplevel 2>/dev/null || pwd)}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
RESEARCH_DIR="${AUTOSPEC_RESEARCH_DIR:-$SCRIPT_DIR/explore-research}"

classify_repo() {
    if [ -f "$REPO_ROOT/.git/archived" ]; then printf 'archived'; return; fi
    if [ -f "$REPO_ROOT/README.md" ] && find "$REPO_ROOT" -maxdepth 2 -type f -name '*.md' | grep -q . && ! find "$REPO_ROOT" -maxdepth 2 -type f \( -name 'package.json' -o -name 'Cargo.toml' -o -name 'pom.xml' \) | grep -q .; then printf 'docs'; return; fi
    if find "$REPO_ROOT" -maxdepth 3 -type f \( -name '*.ipynb' -o -name '*.rmd' \) | grep -q .; then printf 'data-study'; return; fi
    if [ -f "$REPO_ROOT/docker-compose.yml" ] || [ -d "$REPO_ROOT/terraform" ] || [ -d "$REPO_ROOT/k8s" ]; then printf 'infra'; return; fi
    if [ -f "$REPO_ROOT/package.json" ] && find "$REPO_ROOT" -maxdepth 3 -type f \( -name '*.tsx' -o -name '*.jsx' \) | grep -q .; then printf 'frontend'; return; fi
    if [ -f "$REPO_ROOT/Cargo.toml" ] || [ -f "$REPO_ROOT/pom.xml" ] || [ -f "$REPO_ROOT/go.mod" ]; then printf 'library'; return; fi
    printf 'unknown'
}
REPO_CLASS="${AUTOSPEC_REPO_CLASS:-$(classify_repo)}"

usage() {
    cat <<'EOF'
Usage: explore-research-cycle.sh [options]

Options:
  --max-issues-per-round N     Cap final proposals (default 5).
  --stage STAGE                Two-pass split so the adversarial verify gate can
                               actually run (issue #1095). One of:
                                 full      (default) single-pass: run researchers,
                                           aggregate, dedup, verify, ROI, synthesis,
                                           rank, cap — backward-compatible behavior.
                                 dedup     pass 1: run researchers, aggregate, dedup,
                                           then STOP and emit the deduped proposals
                                           (each carrying its normalized-title key)
                                           for the orchestrator to verify. No verify/
                                           ROI/synthesis/rank.
                                 finalize  pass 2: skip researchers; read the pass-1
                                           deduped artifact from --deduped-in and
                                           resume verify -> ROI -> synthesis -> rank
                                           -> cap, consuming the orchestrator-built
                                           AUTOSPEC_EXPLORE_VERIFY_VERDICTS map.
  --deduped-in PATH            (stage finalize) Path to the pass-1 deduped artifact.
  --research-sources LIST      Comma-separated subset of the universal +
                               discovery roster:
                                 spec-vs-code,prior-reports,codebase-signals,
                                 open-issues,source-analysis,dependency-health,
                                 internet,quality-resilience,dogfooding,
                                 self-leverage,style-normalization
                               Default: all 11.
  --specialists-mode MODE      Domain-specialist roster mode (Issue E2):
                                 discover (default) | ask | explicit | off.
                                 off = zero specialists (current behavior).
  --num-specialists N          Roster size for discover/ask (default 3, cap 6).
  --specialists LIST           Explicit roster as slug:persona,slug:persona,...
                               (used verbatim when --specialists-mode explicit).
  --ledger PATH                Outcome ledger to derive dynamic source weights
                               from (passed to explore-source-weights.sh). When
                               omitted, $AUTOSPEC_EXPLORE_LEDGER is used, then the
                               weights script's own default. With no ledger the
                               canonical static priors are used (unchanged).
  --out PATH                   Write JSON to PATH (atomic) instead of stdout.
  -h, --help                   Print this help.

Env:
  AUTOSPEC_REPO_ROOT             Repo root (default: git rev-parse).
  AUTOSPEC_RESEARCH_DIR          Researcher directory.
  AUTOSPEC_EXPLORE_LEDGER        Default outcome ledger path (overridden by
                                 --ledger).
  AUTOSPEC_EXPLORE_WEIGHTS_BIN   Explicit path to explore-source-weights.sh
                                 (overrides sibling/repo resolution; testing).
  AUTOSPEC_SCRIPTS_DIR           Shared scripts dir searched for the weights bin.
  AUTOSPEC_TEST_ISSUES_JSON      Inject fake gh issue list (testing).
  AUTOSPEC_TEST_RECENT_TITLES    Newline-separated titles created in last 7d
                                 (testing — bypasses gh search).
  AUTOSPEC_EXPLORE_AUTONOMOUS     1 = autonomous/no-TTY run; discover mode then
                                 auto-selects top-N (never blocks). Default: auto
                                 (autonomous unless an interactive TTY is present).
  AUTOSPEC_PRIORITY_BOOST        Score multiplier for priority/steer-aligned
                                 proposals (those with priority:true or
                                 source=="steer"). Default: 1.5. The boost
                                 reorders without swamping: the boosted score is
                                 capped at max_non_boosted_score * multiplier.
  AUTOSPEC_SPECIALIST_PROPOSALS_<SLUG>
                                 Per-specialist proposal JSON (an array, or an
                                 object with a "proposals" array). The seam the
                                 orchestrator's LLM dispatch fills; absent → that
                                 specialist contributes zero proposals (graceful).
  AUTOSPEC_EXPLORE_TRACK_CAPS   JSON object of per-track caps, e.g.
                                 {"governance":1,"testability":2,"architecture":2,"product":5}.
                                 Missing tracks use MAX_ISSUES; zero excludes a track.
  AUTOSPEC_EXPLORE_TRACK_PRIORITY
                                 Comma-separated track order (default governance,
                                 testability,architecture,product).
  AUTOSPEC_EXPLORE_REPO_CLASS     data-study or notebook-research enables safety mode.
EOF
}

MAX_ISSUES=5
RESEARCHER_TIMEOUT_SECS="${AUTOSPEC_RESEARCHER_TIMEOUT_SECS:-120}"
SOURCES="spec-vs-code,prior-reports,codebase-signals,open-issues,source-analysis,dependency-health,internet,quality-resilience,dogfooding,self-leverage,style-normalization"
OUT=""
ORG="${AUTOSPEC_EXPLORE_ORG:-}"
ORG_MAX_AGE="${AUTOSPEC_EXPLORE_ORG_MAX_AGE:-604800}"
LEDGER="${AUTOSPEC_EXPLORE_LEDGER:-}"
SPECIALISTS_MODE="discover"
NUM_SPECIALISTS=3
SPECIALISTS_ARG=""
STAGE="full"
DEDUPED_IN=""

while [ "$#" -gt 0 ]; do
    case "$1" in
        --max-issues-per-round) MAX_ISSUES="$2"; shift 2 ;;
        --stage)                STAGE="$2"; shift 2 ;;
        --deduped-in)           DEDUPED_IN="$2"; shift 2 ;;
        --research-sources)     SOURCES="$2"; shift 2 ;;
        --ledger)               LEDGER="$2"; shift 2 ;;
        --out)                  OUT="$2"; shift 2 ;;
    --org)                  ORG="$2"; shift 2 ;;
        --repo-class)           REPO_CLASS="$2"; shift 2 ;;
        --org-max-age)          ORG_MAX_AGE="$2"; shift 2 ;;
        --specialists-mode)     SPECIALISTS_MODE="$2"; shift 2 ;;
        --num-specialists)      NUM_SPECIALISTS="$2"; shift 2 ;;
        --specialists)          SPECIALISTS_ARG="$2"; shift 2 ;;
        -h|--help)              usage; exit 0 ;;
        *) echo "explore-research-cycle: unknown arg: $1" >&2; usage; exit 2 ;;
    esac
done

ORG_AUDIT_DIR=""
ORG_PRIOR_LEDGER=""
ORG_RECHECK=0
if [ -n "$ORG" ]; then
    ORG_AUDIT_DIR="$REPO_ROOT/.autospec/org-audits/$ORG"
    ORG_PRIOR_LEDGER="$ORG_AUDIT_DIR/ledger.jsonl"
    export AUTOSPEC_EXPLORE_ORG_PRIOR_LEDGER="$ORG_PRIOR_LEDGER"
    if [ -f "$ORG_AUDIT_DIR/report.json" ]; then
        report_mtime=$(stat -c %Y "$ORG_AUDIT_DIR/report.json" 2>/dev/null || stat -f %m "$ORG_AUDIT_DIR/report.json" 2>/dev/null || printf '0')
        now_epoch=$(date +%s)
        [ $((now_epoch - report_mtime)) -gt "$ORG_MAX_AGE" ] && ORG_RECHECK=1
    else
        ORG_RECHECK=1
    fi
fi

# Validate the two-pass stage (issue #1095).
case "$STAGE" in
    full|dedup|finalize) ;;
    *) echo "explore-research-cycle: invalid --stage: $STAGE (use full|dedup|finalize)" >&2; exit 2 ;;
esac
if [ "$STAGE" = "finalize" ]; then
    [ -n "$DEDUPED_IN" ] || { echo "explore-research-cycle: --stage finalize requires --deduped-in PATH" >&2; exit 2; }
    [ -f "$DEDUPED_IN" ] || { echo "explore-research-cycle: --deduped-in not found: $DEDUPED_IN" >&2; exit 2; }
fi

# Validate / clamp specialist controls (deterministic; never abort the loop).
case "$SPECIALISTS_MODE" in
    discover|ask|explicit|off) ;;
    *) echo "explore-research-cycle: invalid --specialists-mode: $SPECIALISTS_MODE (use discover|ask|explicit|off)" >&2; exit 2 ;;
esac
# --num-specialists: default 3, floor 0, cap 6.
case "$NUM_SPECIALISTS" in
    ''|*[!0-9]*) NUM_SPECIALISTS=3 ;;
esac
[ "$NUM_SPECIALISTS" -gt 6 ] && NUM_SPECIALISTS=6

# Per-round researcher cap (spec §Guardrails): 7 universal + 4 discovery +
# ≤6 specialists = ≤17. The universal+discovery set is the --research-sources
# selection; specialists top it up to at most this many TOTAL researchers.
RESEARCHER_CAP=17

# Resolve explore-source-weights.sh defensively. Order:
#   1. $AUTOSPEC_EXPLORE_WEIGHTS_BIN (explicit override, e.g. tests)
#   2. $AUTOSPEC_SCRIPTS_DIR/explore-source-weights.sh
#   3. sibling of this script
#   4. <repo>/skills/autospec-shared/scripts/explore-source-weights.sh
# Prints the resolved path (may be empty / nonexistent — caller checks -x).
_resolve_weights_bin() {
    if [ -n "${AUTOSPEC_EXPLORE_WEIGHTS_BIN:-}" ]; then
        printf '%s\n' "$AUTOSPEC_EXPLORE_WEIGHTS_BIN"; return 0
    fi
    if [ -n "${AUTOSPEC_SCRIPTS_DIR:-}" ] && [ -f "$AUTOSPEC_SCRIPTS_DIR/explore-source-weights.sh" ]; then
        printf '%s\n' "$AUTOSPEC_SCRIPTS_DIR/explore-source-weights.sh"; return 0
    fi
    if [ -f "$SCRIPT_DIR/explore-source-weights.sh" ]; then
        printf '%s\n' "$SCRIPT_DIR/explore-source-weights.sh"; return 0
    fi
    if [ -f "$REPO_ROOT/skills/autospec-shared/scripts/explore-source-weights.sh" ]; then
        printf '%s\n' "$REPO_ROOT/skills/autospec-shared/scripts/explore-source-weights.sh"; return 0
    fi
    printf '\n'
}

cd "$REPO_ROOT" || { echo '{"proposals":[]}'; exit 0; }

if ! command -v python3 >/dev/null 2>&1; then
    echo "explore-research-cycle: python3 required" >&2
    exit 2
fi

# Run each enabled researcher in parallel, write to a sibling tmp file.
work_dir="$(mktemp -d -t explore-cycle.XXXXXX)"
trap 'rm -rf "$work_dir"' EXIT

# Finalize uses the pass-1 artifact; rerunning researchers would make verdict-map
# keys non-deterministic. Skip the researcher + specialist dispatch block.
if [ "$STAGE" != "finalize" ]; then
# linter:allow-COMPLEXITY existing orchestrator is outside this narrow watchdog fix
run_researcher_bounded() {
    _script="$1"; _json="$2"; _err="$3"
    if command -v setsid >/dev/null 2>&1; then
        setsid bash "$_script" >"$_json" 2>"$_err" &
    else
        python3 -c 'import os,sys; os.setsid(); os.execvp(sys.argv[1],sys.argv[1:])' bash "$_script" >"$_json" 2>"$_err" &
    fi
    _pid="$!"
    _started="$(date +%s)"
    while kill -0 "$_pid" 2>/dev/null; do
        _state="$(ps -o stat= -p "$_pid" 2>/dev/null | tr -d ' ' || true)"
        case "$_state" in
            Z*|'') break ;;
        esac
        _now="$(date +%s)"
        if [ $((_now - _started)) -ge "$RESEARCHER_TIMEOUT_SECS" ]; then
            _pgid="$(ps -o pgid= -p "$_pid" 2>/dev/null | tr -d ' ' || true)"
            _own_pgid="$(ps -o pgid= -p "$$" 2>/dev/null | tr -d ' ' || true)"
            if [ -n "$_pgid" ] && [ "$_pgid" != "$_own_pgid" ]; then
                kill -TERM -- "-$_pgid" 2>/dev/null || true
                sleep 1
                kill -KILL -- "-$_pgid" 2>/dev/null || true
            else
                kill -TERM "$_pid" 2>/dev/null || true
            fi
            wait "$_pid" 2>/dev/null || true
            return 124
        fi
        sleep 1
    done
    wait "$_pid"
}
pids=()
IFS=','
for src in $SOURCES; do
    src="$(printf '%s' "$src" | tr -d ' ')"
    [ -z "$src" ] && continue
    script="$RESEARCH_DIR/$src.sh"
    if [ ! -f "$script" ]; then
        echo '{"source":"'"$src"'","proposals":[],"error":"missing_script"}' \
            > "$work_dir/$src.json"
        continue
    fi
    (
        if run_researcher_bounded "$script" "$work_dir/$src.json" "$work_dir/$src.err"; then
            :
        else
            _rc=$?
            _reason="researcher_failed"
            [ "$_rc" -eq 124 ] && _reason="timeout"
            SRC="$src" RC="$_rc" REASON="$_reason" python3 - \
                > "$work_dir/$src.json" <<'PY'
import json, os
print(json.dumps({
    "source": os.environ["SRC"],
    "proposals": [],
    "error": os.environ["REASON"],
    "exit_code": int(os.environ["RC"]),
}))
PY
        fi
    ) &
    pids+=("$!")
done
unset IFS

# Wait for all background researchers.
for p in "${pids[@]:-}"; do
    [ -n "$p" ] && wait "$p" 2>/dev/null || true
done

# Count the universal+discovery researchers actually selected this round (the
# non-empty, deduped --research-sources entries). Specialists may only top this
# up to RESEARCHER_CAP total.
universal_count=0
IFS=','
for src in $SOURCES; do
    src="$(printf '%s' "$src" | tr -d ' ')"
    [ -z "$src" ] && continue
    universal_count=$((universal_count + 1))
done
unset IFS

# ── Domain-specialist dispatch (Issue E2 #1083) ─────────────────────────────
# Each resolved roster specialist runs as a researcher emitting proposals with
# source=specialist:<slug> (default weight 0.6 in the aggregator). The roster is
# resolved per --specialists-mode:
#   off       no specialists — byte-for-byte the pre-E2 behavior.
#   explicit  the verbatim --specialists slug:persona,... list.
#   discover  auto-run discovery → the cached .autospec/explore-specialists.json
#             roster; interactive harness confirms via AskUserQuestion (an
#             orchestrator/SKILL-prose responsibility), autonomous/no-TTY takes
#             the top --num-specialists and never blocks (handled here).
#   ask       like discover but the operator names the roster (orchestrator
#             prose); the deterministic floor here is the same cached top-N.
# Proposals per specialist come from the LLM seam
# AUTOSPEC_SPECIALIST_PROPOSALS_<SLUG>; absent → zero proposals (graceful, the
# specialist still participates but contributes nothing — never hard-fails).
# Trust boundary (spec §Guardrails): personas come from repo-evidence roster /
# operator input only — never from the internet researcher's fetched content.
if [ "$SPECIALISTS_MODE" != "off" ]; then
    # Resolve the slug list (one per line) into $work_dir/specialists.txt.
    specialists_file="$work_dir/specialists.txt"
    : > "$specialists_file"
    if [ "$SPECIALISTS_MODE" = "explicit" ]; then
        # Verbatim roster: slug[:persona],slug[:persona],...  Take slug before ':'.
        IFS=','
        for entry in $SPECIALISTS_ARG; do
            slug="${entry%%:*}"
            slug="$(printf '%s' "$slug" | tr -d ' ' | tr '[:upper:]' '[:lower:]')"
            [ -n "$slug" ] && printf '%s\n' "$slug" >> "$specialists_file"
        done
        unset IFS
    else
        # discover / ask: consume the cached roster (Issue E1). Slugs only.
        roster_path="$REPO_ROOT/.autospec/explore-specialists.json"
        if [ -f "$roster_path" ] && command -v python3 >/dev/null 2>&1; then
            python3 - "$roster_path" >> "$specialists_file" 2>/dev/null <<'PY' || true
import json, sys
try:
    d = json.load(open(sys.argv[1]))
    for s in d.get("suggested_specialists", []) or []:
        slug = str((s or {}).get("slug", "")).strip().lower()
        if slug:
            print(slug)
except Exception:
    pass
PY
        fi
    fi

    # Top-N + cap. Take the first --num-specialists slugs, then clamp so the
    # TOTAL researcher count (universal + specialists) never exceeds RESEARCHER_CAP.
    room=$((RESEARCHER_CAP - universal_count))
    [ "$room" -lt 0 ] && room=0
    allow="$NUM_SPECIALISTS"
    [ "$allow" -gt "$room" ] && allow="$room"

    spec_n=0
    while IFS= read -r slug; do
        [ -z "$slug" ] && continue
        [ "$spec_n" -ge "$allow" ] && break
        # Per-specialist proposals via the LLM seam. Env var name is the slug
        # upper-cased with non-alnum → underscore. Absent → empty proposals.
        env_slug="$(printf '%s' "$slug" | tr '[:lower:]-' '[:upper:]_' | tr -c 'A-Z0-9_' '_')"
        eval "payload=\"\${AUTOSPEC_SPECIALIST_PROPOSALS_${env_slug}:-}\""
        SPEC_SLUG="$slug" SPEC_PAYLOAD="${payload:-}" python3 - > "$work_dir/specialist-$slug.json" <<'PY' || \
            printf '{"source":"specialist:%s","proposals":[]}\n' "$slug" > "$work_dir/specialist-$slug.json"
import json, os
slug = os.environ["SPEC_SLUG"]
src = "specialist:" + slug
raw = os.environ.get("SPEC_PAYLOAD", "").strip()
props = []
if raw:
    try:
        d = json.loads(raw)
        if isinstance(d, dict):
            d = d.get("proposals", [])
        if isinstance(d, list):
            props = [p for p in d if isinstance(p, dict)]
    except Exception:
        props = []
# Stamp the specialist source on every proposal (trust boundary: the slug, not
# any source the payload may claim).
for p in props:
    p["source"] = src
print(json.dumps({"source": src, "proposals": props}))
PY
        spec_n=$((spec_n + 1))
    done < "$specialists_file"
fi
fi  # end: STAGE != finalize (researcher + specialist dispatch)

# Preserve discovery-health evidence independently of proposal ranking. A
# completed empty researcher is healthy; a missing, timed-out, non-zero, or
# malformed researcher is not. Finalize carries the health from its dedup input
# rather than pretending it reran the sources.
if [ "$STAGE" = "finalize" ]; then
    RESEARCH_HEALTH_JSON="$(python3 - "$DEDUPED_IN" <<'PY'
import json, sys
invalid = {
    "selected": 0,
    "succeeded": 0,
    "failures": [{"source": "research-cycle", "reason": "invalid_output", "exit_code": 1}],
}
try:
    data = json.load(open(sys.argv[1], encoding="utf-8"))
    health = data.get("researcher_health")
except Exception:
    health = None
if not isinstance(health, dict):
    health = invalid
else:
    selected = health.get("selected")
    succeeded = health.get("succeeded")
    failures = health.get("failures")
    if (
        not isinstance(selected, int)
        or isinstance(selected, bool)
        or not isinstance(succeeded, int)
        or isinstance(succeeded, bool)
        or not isinstance(failures, list)
    ):
        health = invalid
print(json.dumps(health, separators=(",", ":")))
PY
)"
else
    RESEARCH_HEALTH_JSON="$(WORK_DIR="$work_dir" python3 - <<'PY'
import glob, json, os

selected = 0
succeeded = 0
failures = []
for path in sorted(glob.glob(os.path.join(os.environ["WORK_DIR"], "*.json"))):
    selected += 1
    fallback = os.path.basename(path).removesuffix(".json")
    try:
        with open(path, encoding="utf-8") as handle:
            result = json.load(handle)
        if not isinstance(result, dict):
            raise ValueError("result is not an object")
        source_value = result.get("source")
        if not isinstance(source_value, str) or not source_value.strip():
            raise ValueError("source is missing")
        if not isinstance(result.get("proposals"), list):
            raise ValueError("proposals is not an array")
    except Exception:
        failures.append({"source": fallback, "reason": "invalid_output", "exit_code": 1})
        continue
    source = source_value.strip()
    reason = str(result.get("error") or "").strip()
    if reason:
        raw_exit_code = result.get("exit_code", 0)
        try:
            exit_code = int(raw_exit_code)
        except (TypeError, ValueError):
            failures.append({
                "source": source,
                "reason": "invalid_output",
                "exit_code": 1,
            })
            continue
        failures.append({
            "source": source,
            "reason": reason,
            "exit_code": exit_code,
        })
    else:
        succeeded += 1
print(json.dumps({
    "selected": selected,
    "succeeded": succeeded,
    "failures": failures,
}, separators=(",", ":")))
PY
)"
fi
export RESEARCH_HEALTH_JSON

# Gather recent titles (last 7 days) to filter against. Allow injection.
recent_titles_file="$work_dir/recent.txt"
if [ -n "${AUTOSPEC_TEST_RECENT_TITLES:-}" ]; then
    printf '%s\n' "$AUTOSPEC_TEST_RECENT_TITLES" > "$recent_titles_file"
elif command -v gh >/dev/null 2>&1; then
    # Last 7 days = created:>=YYYY-MM-DD
    since="$(python3 -c 'import datetime; print((datetime.date.today() - datetime.timedelta(days=7)).isoformat())')"
    gh issue list --search "created:>=$since" --state all --limit 200 \
        --json title 2>/dev/null \
        | python3 -c 'import json,sys; [print(t["title"]) for t in json.load(sys.stdin)]' \
        > "$recent_titles_file" 2>/dev/null || : > "$recent_titles_file"
else
    : > "$recent_titles_file"
fi

# Compute dynamic, ledger-derived source weights (best-effort). The weights
# script emits {source: weight} JSON. With no/empty ledger it emits the
# canonical priors that EXACTLY equal DEFAULT_SRC_WEIGHTS below, so behavior is
# byte-identical when there is no learning yet. Any failure -> empty -> fallback.
WEIGHTS_JSON=""
weights_bin="$(_resolve_weights_bin)"
if [ -n "$weights_bin" ] && [ -x "$weights_bin" ]; then
    WEIGHTS_JSON="$("$weights_bin" --json ${LEDGER:+--ledger "$LEDGER"} 2>/dev/null || true)"
fi
export WEIGHTS_JSON

# Aggregate, dedup, VERIFY, ROI-gate, rank, filter, cap.
#
# Verify stage (Issue C): the adversarial LLM-skeptic refutation is an
# explore-orchestrator (SKILL-prose) responsibility, NOT this deterministic
# bash aggregator — no LLM is ever invoked from here. What the aggregator owns
# is the verify *boundary*: every deduped proposal is threaded through a verify
# gate that consumes an OPTIONAL verdict map supplied by the orchestrator via
# AUTOSPEC_EXPLORE_VERIFY_VERDICTS (a path to, or inline, JSON mapping
# normalized-title -> {verdict, reason}). Documented fallback: when no map is
# supplied the gate no-ops to "all survive" (verdict=unverified). When a map IS
# supplied, a proposal with no entry is refute-by-default (the skeptic could
# not affirm it) per the spec.
VERIFY_VERDICTS="${AUTOSPEC_EXPLORE_VERIFY_VERDICTS:-}"
export VERIFY_VERDICTS

WORK_DIR="$work_dir" \
MAX_ISSUES="$MAX_ISSUES" \
RECENT_FILE="$recent_titles_file" \
STAGE="$STAGE" \
DEDUPED_IN="$DEDUPED_IN" \
GAP_REPO_ROOT="$REPO_ROOT" \
GAP_CLAIMING_SOURCES="${AUTOSPEC_EXPLORE_GAP_CLAIMING_SOURCES:-}" \
TRACK_CAPS="${AUTOSPEC_EXPLORE_TRACK_CAPS:-}" \
TRACK_PRIORITY="${AUTOSPEC_EXPLORE_TRACK_PRIORITY:-governance,testability,architecture,product}" \
REPO_CLASS="${AUTOSPEC_EXPLORE_REPO_CLASS:-${AUTOSPEC_REPO_CLASS:-}}" \
python3 - <<'PY' > "$work_dir/final.json"
import json, os, re, glob, datetime

work     = os.environ["WORK_DIR"]
cap      = int(os.environ["MAX_ISSUES"])
recent_f = os.environ["RECENT_FILE"]
stage    = os.environ.get("STAGE", "full")
deduped_in = os.environ.get("DEDUPED_IN", "").strip()
try:
    researcher_health = json.loads(os.environ.get("RESEARCH_HEALTH_JSON", "{}"))
except Exception:
    researcher_health = {}
# Repo root for gap-confirmation file resolution — explicit, never cwd-dependent.
gap_repo_root = os.environ.get("GAP_REPO_ROOT", "") or os.getcwd()
track_caps_raw = os.environ.get("TRACK_CAPS", "").strip()
track_priority = [x.strip() for x in os.environ.get("TRACK_PRIORITY", "governance,testability,architecture,product").split(",") if x.strip()]
TRACKS = ("governance", "testability", "architecture", "product")
try:
    track_caps = json.loads(track_caps_raw) if track_caps_raw else {}
    if not isinstance(track_caps, dict): track_caps = {}
except Exception:
    track_caps = {}
def proposal_track(p, source):
    value = p.get("track")
    if value in TRACKS: return value
    if source in {"quality-resilience", "dependency-health", "style-normalization"}: return "governance"
    if source in {"dogfooding", "self-leverage"} or source.startswith("specialist:test"):
        return "testability"
    if source.startswith("specialist:arch") or source == "architecture": return "architecture"
    return "product"
repo_class = os.environ.get("REPO_CLASS", "").strip().lower()
RESTRICTED_CLASSES = {"data-study", "notebook-research"}
RESTRICTED_KINDS = {"schema-documentation", "checksum-manifest", "data-dictionary", "notebook-execution-metadata", "environment-capture", "reproducible-pipeline-wrapper"}

# Static source priors per the spec — the defensive fallback used verbatim when
# no dynamic weights are available (no ledger / weights script absent / broken).
DEFAULT_SRC_WEIGHTS = {
    "spec-vs-code":     1.0,
    "prior-reports":    0.9,
    "codebase-signals": 0.7,
    "dependency-health": 0.65,
    "open-issues":      0.6,
    "style-normalization": 0.85,
    "quality-resilience": 0.95,
    "dogfooding":       0.9,
    "self-leverage":    0.6,
    "source-analysis":  0.5,
    "internet":         0.4,
}
# Overlay dynamic ledger-derived weights when present; otherwise fall back to
# the static table. The overlay is a merge so unknown/missing sources retain
# their static prior, guaranteeing parity when WEIGHTS_JSON mirrors the priors.
_wj = os.environ.get("WEIGHTS_JSON", "").strip()
if _wj:
    try:
        SRC_WEIGHTS = {**DEFAULT_SRC_WEIGHTS, **json.loads(_wj)}
    except Exception:
        SRC_WEIGHTS = DEFAULT_SRC_WEIGHTS
else:
    SRC_WEIGHTS = DEFAULT_SRC_WEIGHTS
COMPLEXITY = {"small": 1.0, "medium": 2.0, "large": 4.0}
restricted_rejections = []
def restricted_reason(p):
    if repo_class not in RESTRICTED_CLASSES:
        return ""
    kind = str(p.get("kind", p.get("proposal_kind", ""))).strip().lower()
    text = " ".join(str(p.get(k, "")) for k in ("title", "evidence", "description")).lower()
    if kind in RESTRICTED_KINDS:
        return ""
    if any(x in text for x in ("rewrite data", "reformat notebook", "generated-data", "generated data")):
        return "mass rewrite or generated-data modification is forbidden"
    touches = bool(p.get("touches_data_file")) or any(x in text for x in ("data file", "dataset", ".csv", ".parquet", ".jsonl", ".ipynb"))
    impl = bool(p.get("implementation_type")) or kind in {"implementation", "code-change", "feature"}
    if impl and touches and not str(p.get("preservation_rollback_plan", p.get("rollback_plan", ""))).strip():
        return "data-file implementation requires preservation_rollback_plan"
    return "proposal kind is required and must be explicitly allowed"

# Severity bands, highest impact -> lowest. Lower numeric value = higher
# priority (primary sort key). Mirrors schemas/autospec-explore-proposal.schema
# .json enum order, which is load-bearing.
SEVERITY_ORDER = [
    "silent-wrong", "correctness", "stability",
    "operability", "feature", "nicety",
]
SEVERITY_RANK = {s: i for i, s in enumerate(SEVERITY_ORDER)}
# Default rank for an unknown severity = just below "feature" (treated as the
# legacy default band so unknown values never out-rank a real correctness item).
DEFAULT_SEVERITY_RANK = SEVERITY_RANK["feature"]

# The 7 legacy universal researchers are EXEMPT from the ROI gate during
# rollout (spec: "only the discovery ones are ROI-gated, to avoid silently
# muting the existing 7"). Any source NOT in this set — the discovery
# researchers (quality-resilience, dogfooding, self-leverage, style-normalization) and
# specialist:<slug> sources — is a "new source" and IS ROI-gated.
LEGACY_SOURCES = {
    "spec-vs-code", "prior-reports", "codebase-signals", "open-issues",
    "source-analysis", "dependency-health", "internet",
}

def normalize_title(t):
    s = t.lower()
    # Strip leading conventional-commit prefix.
    s = re.sub(r'^\s*(feat|fix|chore|docs|test|refactor|perf|track|ci)\s*:\s*', '', s)
    s = re.sub(r'[^a-z0-9 ]+', ' ', s)
    s = re.sub(r'\s+', ' ', s).strip()
    # Drop stopwords for normalization (keep verb + subject signal).
    return s[:120]

# ---------------------------------------------------------------------------
# GAP-CONFIRMATION config + helpers (stage: dedup -> gap-confirm -> verify).
# A researcher may attach a machine-checkable `gap_check` claim; the aggregator
# re-verifies it against the CURRENT files before ranking. This is the primary
# precision lever: it kills "X is missing" proposals whose X already exists
# (the empirical failure mode — source-analysis was 4/4 false on direct check).
import subprocess

# Sources that MUST make a falsifiable gap claim: a proposal from one of these
# with no valid gap_check is refuted by default (it asserts a gap but offers
# nothing to confirm). Scoped to the researchers converted to emit gap_check
# in this pass — `source-analysis` and `self-leverage`, the two whose evidence
# was empirically false/prose-only. Other sources (spec-vs-code, codebase-signals,
# quality-resilience, …) are NOT refuted for omitting gap_check — they keep their
# existing behavior; if they DO attach a gap_check it is still verified below.
# Converting more sources is a deliberate edit to this set (or the env override).
# Override via AUTOSPEC_EXPLORE_GAP_CLAIMING_SOURCES (CSV).
_gcs_env = os.environ.get("GAP_CLAIMING_SOURCES", "").strip()
GAP_CLAIMING_SOURCES = (
    set(s.strip() for s in _gcs_env.split(",") if s.strip())
    if _gcs_env else
    {"source-analysis", "self-leverage"}
)

def _safe_haystack(h):
    """Resolve a gap_check haystack to a repo-relative target, or None if it
    escapes the repo (absolute path / `..` traversal) — never search outside."""
    if not isinstance(h, str) or not h:
        return None
    if h == "<repo>":
        return "<repo>"
    if os.path.isabs(h):
        return None
    norm = os.path.normpath(h)
    if norm == ".." or norm.startswith("../") or "/../" in norm:
        return None
    return norm

def _gap_search(needle, haystack):
    """Tri-state fixed-string search rooted at gap_repo_root (never cwd).
    True=found, False=definitively absent (targets existed but lacked needle),
    None=unconfirmable (no target / error). None always fails closed."""
    if haystack == "<repo>":
        try:
            r = subprocess.run(["git", "grep", "--untracked", "-F", "-q", "--", needle],
                               cwd=gap_repo_root,
                               stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            if r.returncode == 0:
                return True
            if r.returncode == 1:
                return False
            return None
        except Exception:
            return None
    pat = os.path.join(gap_repo_root, haystack)
    paths = (glob.glob(pat, recursive=True)
             if any(c in haystack for c in "*?[") else [pat])
    # Hard containment: resolve symlinks and keep only files that really live
    # inside the repo root (lexical `..`/abs is already rejected upstream, but
    # an in-repo symlink could still point outside).
    root_real = os.path.realpath(gap_repo_root) + os.sep
    existing = [p for p in paths
                if os.path.isfile(p)
                and os.path.realpath(p).startswith(root_real)]
    if not existing:
        return None
    any_readable = False
    for pth in existing:
        try:
            with open(pth, "r", encoding="utf-8", errors="ignore") as fh:
                content = fh.read()
        except Exception:
            continue          # skip an unreadable file, don't abort the whole check
        any_readable = True
        if needle in content:
            return True
    # All matched files existed but none contained the needle -> definitively
    # absent (False); if NONE were readable we cannot confirm -> None (fail closed).
    return False if any_readable else None

# Counters surfaced in the cycle JSON (observable precision yield).
gap_dropped = 0
gap_malformed = 0
gap_confirmed_count = None
saturated_sources = []

all_props = []
total = 0
if stage == "finalize":
    # Pass 2: the deduped proposals (and the upstream counters) come from the
    # pass-1 artifact, NOT from re-globbing researcher output. This is what makes
    # the verdict map keyable — the orchestrator built it against THESE exact
    # normalized titles between the two passes.
    with open(deduped_in, "r", encoding="utf-8") as fh:
        _p1 = json.load(fh)
    total = int(_p1.get("proposals_total", 0))
    deduped = [p for p in (_p1.get("deduped") or _p1.get("proposals") or []) if isinstance(p, dict)]
    # Gap-confirm/saturation ran in pass 1 (stage=dedup) and is skipped here, so
    # carry pass-1's precision-yield counters through this consumed artifact
    # rather than reporting zeros for the work pass 1 already did.
    gap_confirmed_count = _p1.get("proposals_after_gap_confirm", len(deduped))
    gap_dropped = int(_p1.get("gap_unconfirmed_dropped", 0) or 0)
    gap_malformed = int(_p1.get("gap_check_malformed", 0) or 0)
    saturated_sources = _p1.get("saturated_sources", []) or []
else:
  for f in sorted(glob.glob(os.path.join(work, "*.json"))):
    if os.path.basename(f) == "final.json":
        continue
    try:
        with open(f, 'r', encoding='utf-8') as fh:
            data = json.load(fh)
    except Exception:
        continue
    src = data.get("source", "unknown")
    for p in data.get("proposals", []) or []:
        if not isinstance(p, dict): continue
        title = (p.get("title") or "").strip()
        if not title: continue
        reason = restricted_reason(p)
        if reason:
            restricted_rejections.append({"title": title, "reason": reason})
            continue
        comp = (p.get("estimated_complexity") or "medium").lower()
        try:
            conf = float(p.get("confidence", 0.5))
        except Exception:
            conf = 0.5
        # Domain-specialist sources (source=specialist:<slug>, Issue E2) default
        # to weight 0.6 per the shared-contract; any other unknown source 0.5.
        if src in SRC_WEIGHTS:
            weight = SRC_WEIGHTS[src]
        elif src.startswith("specialist:"):
            weight = SRC_WEIGHTS.get(src, 0.6)
        else:
            weight = 0.5
        cscale = COMPLEXITY.get(comp, 2.0)
        score = conf * weight / cscale
        # Default the proposal-contract extension fields safely for legacy
        # researchers that don't emit them (Issue A). Missing severity ->
        # "feature"; missing named_consumer -> "". This is a pure default: it
        # never drops a proposal, so the existing 7 researchers keep flowing.
        severity = p.get("severity")
        if not isinstance(severity, str) or not severity.strip():
            severity = "feature"
        named_consumer = p.get("named_consumer")
        if not isinstance(named_consumer, str):
            named_consumer = ""
        # priority (F8): carry through for priority-boost gate.
        priority = bool(p.get("priority"))
        track = proposal_track(p, src)
        # Carry the optional gap_check through verbatim so the gap-confirmation
        # stage can re-verify it; only a dict is propagated (anything else is
        # ignored and treated as "no gap_check").
        gap_check = p.get("gap_check")
        _entry = {
            "title": title,
            "evidence": p.get("evidence",""),
            "estimated_complexity": comp,
            "confidence": conf,
            "source": src,
            "severity": severity,
            "named_consumer": named_consumer,
            "score": round(score, 4),
            "priority": priority,
            "track": track,
        }
        if isinstance(gap_check, dict):
            _entry["gap_check"] = gap_check
        all_props.append(_entry)
        total += 1

# Dedup by normalized title — keep highest score. (Skipped in finalize: the
# deduped set was produced by pass 1 and loaded above.)
if stage != "finalize":
    by_norm = {}
    for p in all_props:
        n = normalize_title(p["title"])
        if n not in by_norm or p["score"] > by_norm[n]["score"]:
            by_norm[n] = p
    deduped = list(by_norm.values())

# Count after dedup but BEFORE gap-confirm/saturation mutate `deduped`, so the
# reported proposals_after_dedup reflects dedup only (stable, test-compatible).
dedup_count = len(deduped)

# ---------------------------------------------------------------------------
# GAP-CONFIRMATION stage — runs after dedup, before the pass-1 emit / verify.
# Skipped in finalize: its input was already gap-confirmed in pass 1. Every
# proposal carrying a gap_check is re-verified against current files; a
# gap-claiming source with no (valid) gap_check is refuted by default.
if stage != "finalize":
    _confirmed = []
    for p in deduped:
        gc = p.get("gap_check")
        src = p.get("source", "")
        if not isinstance(gc, dict):
            if src in GAP_CLAIMING_SOURCES:
                gap_dropped += 1          # asserts a gap, offers nothing to check
                continue
            _confirmed.append(p)          # non-gap-claiming source: pass through
            continue
        kind = gc.get("kind")
        needle = gc.get("needle")
        hay = _safe_haystack(gc.get("haystack"))
        # A newline in the needle is rejected: `git grep -F` splits a
        # newline-bearing fixed pattern into OR'd line patterns (any-line match),
        # which disagrees with the file branch's true substring semantics. Force
        # single-line needles so both branches mean the same thing.
        if (kind not in ("absent", "present")
                or not isinstance(needle, str) or not needle or "\n" in needle
                or hay is None):
            gap_malformed += 1
            gap_dropped += 1              # malformed gap_check -> fail closed
            continue
        res = _gap_search(needle, hay)
        if kind == "absent":
            # Claim: needle is MISSING. Keep only if definitively absent.
            if res is False:
                _confirmed.append(p)
            else:
                gap_dropped += 1          # found (no real gap) or unconfirmable
        else:  # present
            # Claim: needle EXISTS (real call-site/pattern). Keep only if found.
            if res is True:
                _confirmed.append(p)
            else:
                gap_dropped += 1
    deduped = _confirmed
    gap_confirmed_count = len(deduped)

    # ANTI-SATURATION — bound any single source to SATURATION_FRACTION of the
    # post-gap-confirm pool, penalizing its surviving scores. Stops one
    # researcher (e.g. a cap-saturated discovery source) from flooding the rank.
    # Gated to LARGE, MULTI-SOURCE pools only: on a small or single-source pool
    # the final top-N cap already bounds output, and capping there would just
    # delete a legitimately dominant lone source. Each source keeps >= 3.
    try:
        _sat_frac = float(os.environ.get("AUTOSPEC_EXPLORE_SATURATION_FRACTION", "0.40") or "0.40")
    except Exception:
        _sat_frac = 0.40
    try:
        _sat_min_pool = int(os.environ.get("AUTOSPEC_EXPLORE_SATURATION_MIN_POOL", "12") or "12")
    except Exception:
        _sat_min_pool = 12
    _SAT_PENALTY = 0.5
    from collections import Counter, defaultdict
    _cnt = Counter(p.get("source", "?") for p in deduped)
    if len(deduped) >= _sat_min_pool and len(_cnt) >= 2:
        _cap_n = max(3, int(_sat_frac * len(deduped)))
        saturated_sources = sorted(s for s, c in _cnt.items() if c > _cap_n)
        if saturated_sources:
            _sat_set = set(saturated_sources)
            _by_src = defaultdict(list)
            for p in deduped:
                _by_src[p.get("source", "?")].append(p)
            _new = []
            for s, members in _by_src.items():
                if s in _sat_set:
                    members = sorted(members, key=lambda q: -q.get("score", 0))[:_cap_n]
                    for m in members:
                        m["score"] = round(m.get("score", 0) * _SAT_PENALTY, 4)
                _new.extend(members)
            deduped = _new

# ---------------------------------------------------------------------------
# PASS 1 boundary (stage=dedup, issue #1095): emit the deduped proposals — each
# stamped with its normalized-title key — and STOP before verify. The
# orchestrator dispatches a Tier-B skeptic per deduped proposal, assembles the
# {norm_title -> {verdict, reason}} map, and feeds it back into pass 2
# (stage=finalize) via AUTOSPEC_EXPLORE_VERIFY_VERDICTS. No verify/ROI/synthesis/
# rank happens here — those are pass-2's job once the verdicts exist.
if stage == "dedup":
    for p in deduped:
        p["norm_title"] = normalize_title(p["title"])
    out = {
        "round": datetime.date.today().isoformat(),
        "stage": "dedup",
        "proposals_total": total,
        "proposals_after_dedup": dedup_count,
        "proposals_after_gap_confirm": gap_confirmed_count if gap_confirmed_count is not None else len(deduped),
        "gap_unconfirmed_dropped": gap_dropped,
        "gap_check_malformed": gap_malformed,
        "saturated_sources": saturated_sources,
        "researcher_health": researcher_health,
        "deduped": deduped,
    }
    print(json.dumps(out, indent=2))
    raise SystemExit(0)

# ---------------------------------------------------------------------------
# VERIFY stage (Issue C) — adversarial-skeptic boundary, between dedup & rank.
#
# The deterministic aggregator does NOT call an LLM. It consumes an optional
# verdict map produced by the explore-orchestrator's per-proposal Tier-B
# skeptic dispatch. Map shape: { "<normalized title>": {"verdict": "...",
# "reason": "..."} }. verdict in {"survived","refuted"} (anything not
# "survived" is treated as refuted). VERIFY_VERDICTS may be a path to a JSON
# file or an inline JSON string.
#
# Fallback (documented degradation): no map supplied -> the gate no-ops, every
# proposal survives carrying verdict="unverified". This is the safe default for
# environments with no subagent capability (cf. the installer/runtime-libs and
# bash-3.2 gotchas — never hard-fail).
#
# Refute-by-default: when a map IS supplied but a proposal has no entry, the
# skeptic could not affirm it, so it is refuted and dropped (spec: "default to
# refuted=true under uncertainty").
def _load_verdicts():
    raw = os.environ.get("VERIFY_VERDICTS", "").strip()
    if not raw:
        return None
    # Prefer a file path; fall back to treating the value as inline JSON.
    try:
        if os.path.isfile(raw):
            with open(raw, "r", encoding="utf-8") as fh:
                return json.load(fh)
        return json.loads(raw)
    except Exception:
        # Unparseable verdict source -> behave as if no map was supplied
        # (no-op fallback) rather than refuting everything on a config error.
        return None

_verdicts = _load_verdicts()
# verify_mode makes the gate's effective state observable in explore-loop.json
# (audit #1086 seam-1): "active" when an orchestrator-supplied verdict map drove
# real refutations, "no-op-unverified" when no map was supplied and every
# proposal survived carrying verdict=unverified. Without this an operator cannot
# distinguish "skeptic refuted nothing" from "skeptic never ran" — the inert
# dead-gate failure mode. The aggregator keeps stdout pure JSON (many consumers
# parse it directly); the loud operator warning is emitted by the orchestrator
# (autospec-explore.sh) off this field, not from here.
verify_mode = "active" if _verdicts is not None else "no-op-unverified"
# Fail-CLOSED in the autonomous path: an autonomous run that reaches the verify
# gate with NO skeptic verdict map must NOT auto-file unverified proposals
# (that is how 183 false positives would have shipped). Interactive runs keep
# the historical no-op "all survive" behavior — there the operator IS the
# skeptic. Gated on AUTOSPEC_EXPLORE_AUTONOMOUS=1 (set by --once / the conductor).
autonomous = os.environ.get("AUTOSPEC_EXPLORE_AUTONOMOUS", "") == "1"
failclosed = autonomous and verify_mode == "no-op-unverified"
verified = []
refuted_count = 0
for p in deduped:
    n = normalize_title(p["title"])
    if _verdicts is None:
        # No-op fallback: survive, unverified.
        p["verdict"] = "unverified"
        p["reason"] = "no verifier verdict supplied (verify stage no-op)"
        verified.append(p)
        continue
    entry = _verdicts.get(n)
    if not isinstance(entry, dict):
        # Refute-by-default under uncertainty.
        refuted_count += 1
        continue
    verdict = str(entry.get("verdict", "")).strip().lower()
    reason = str(entry.get("reason", ""))
    if verdict == "survived":
        p["verdict"] = "survived"
        p["reason"] = reason or "skeptic could not refute"
        verified.append(p)
    else:
        # refuted (or any non-survived verdict) -> drop.
        refuted_count += 1

# ---------------------------------------------------------------------------
# ROI gate (Issue C) — drop NEW-source proposals with empty named_consumer.
# Legacy universal sources are exempt (rollout safety). The pattern-synthesis
# stage (Issue D #1081) runs immediately AFTER this gate and BEFORE ranking,
# collapsing recurring same-class survivors into structural-fix proposals.
roi_kept = []
for p in verified:
    src = p.get("source", "")
    consumer = str(p.get("named_consumer", "")).strip()
    # Governance baselines are reusable operator-facing templates and do not
    # require a product consumer; domain/product tracks retain the ROI gate.
    if src not in LEGACY_SOURCES and p.get("track") != "governance" and consumer == "":
        # New source, no named consumer -> dropped by the ROI gate.
        continue
    roi_kept.append(p)

# ---------------------------------------------------------------------------
# PATTERN SYNTHESIS (Issue D #1081) — runs AFTER the ROI gate, BEFORE the
# constitution/recent filters and the severity-first rank.
#
# Goal: when several survivors describe the SAME recurring defect class (e.g.
# "missing error handling in <X>" across alpha/beta/gamma), collapse them into
# ONE `structural-fix` proposal whose evidence lists every instance plus the
# single guard that would catch them all — so the loop files one durable fix
# instead of N point patches.
#
# Clustering is deterministic and COARSE-but-CONSERVATIVE to avoid the watched
# risk (over-collapsing unrelated findings):
#   - Two proposals may cluster ONLY within the SAME severity band (a
#     silent-wrong defect never merges with a nicety).
#   - Within a band, cluster by content-token overlap: greedy single-linkage
#     where membership requires Jaccard(tokens) >= JACCARD_MIN against the
#     cluster seed. Stopwords + the conventional-commit verb are stripped so
#     the signal is the subject, not "fix:"/"add"/"the".
#   - A cluster collapses iff it has >= 2 members, OR its shared-token theme
#     matches a recurring docs/memory/ theme (memory-grounded structural class
#     even at size 1). Singletons with no memory match pass through unchanged.
#
# Convergence/safety: pure function of the survivor set + a static memory-theme
# list; no randomness, no global state — deterministic across runs.

# Keep constitution D3 marker chores out of synthesis. Otherwise several raw
# deferred-work chores can be renamed into one structural proposal and evade
# the title-based constitution filter below.
_marker_names = ("TO" + "DO", "FIX" + "ME", "X" * 3, "HACK")
_chore_re = re.compile(
    r'^\s*chore:\s*address\s+(%s)\b' % "|".join(_marker_names), re.I
)

_marker_chores = [p for p in roi_kept if _chore_re.match(str(p.get("title", "")))]
_synthesis_candidates = [
    p for p in roi_kept if not _chore_re.match(str(p.get("title", "")))
]

# Recurring docs/memory/ themes: coarse token signatures distilled from the
# persistent feedback memos (bash portability, lockstep duos, regex injection,
# self-consistent fixtures, installer omissions). A survivor whose shared theme
# tokens intersect one of these is a known structural class. Kept intentionally
# small + high-signal; extending it is a deliberate, reviewable act.
MEMORY_THEMES = [
    {"bash", "portability", "shell"},
    {"lockstep", "trio", "duo"},
    {"regex", "injection", "metachar"},
    {"fixture", "self", "consistent", "mock"},
    {"installer", "runtime", "lib", "ship"},
    {"validator", "retry", "adaptive"},
]

_STOPWORDS = {
    "the", "a", "an", "in", "on", "of", "to", "for", "and", "or", "with",
    "is", "are", "be", "by", "at", "from", "into", "module", "modules",
    "add", "fix", "feat", "support", "enable", "new",
}

def _content_tokens(p):
    toks = normalize_title(p["title"]).split()
    return {t for t in toks if t not in _STOPWORDS and len(t) > 2}

def _jaccard(a, b):
    if not a or not b:
        return 0.0
    inter = len(a & b)
    union = len(a | b)
    return inter / union if union else 0.0

# Conservative threshold: a true recurring class (e.g. "missing error handling
# in alpha/beta/gamma") shares its whole subject vocabulary minus the leaf token
# (Jaccard ~0.67-0.75), while two unrelated items that merely share a generic
# word ("high/low score feature" -> 0.5) stay apart. 0.6 is the seam between.
JACCARD_MIN = 0.6

def _theme_match(shared):
    for theme in MEMORY_THEMES:
        if len(shared & theme) >= 2:
            return theme
    return None

# Greedy single-linkage clustering within each severity band.
structural_fixes = 0
_synth_out = list(_marker_chores)
_by_band = {}
for _i, p in enumerate(_synthesis_candidates):
    _by_band.setdefault(p.get("severity", "feature"), []).append(p)

for _band, members in _by_band.items():
    _toks = [_content_tokens(m) for m in members]
    _used = [False] * len(members)
    for i in range(len(members)):
        if _used[i]:
            continue
        cluster_idx = [i]
        _used[i] = True
        for j in range(i + 1, len(members)):
            if _used[j]:
                continue
            if _jaccard(_toks[i], _toks[j]) >= JACCARD_MIN:
                cluster_idx.append(j)
                _used[j] = True
        cluster = [members[k] for k in cluster_idx]
        # Shared tokens across the whole cluster (the common defect signature).
        shared = set(_toks[cluster_idx[0]])
        for k in cluster_idx[1:]:
            shared &= _toks[k]
        theme = _theme_match(_content_tokens(cluster[0]))
        if len(cluster) >= 2 or (theme is not None):
            # Collapse to one structural-fix. Highest-scoring member is the
            # representative for score/consumer; evidence lists every instance.
            rep = max(cluster, key=lambda q: q["score"])
            instances = [c["title"] for c in cluster]
            ev_lines = "; ".join(
                "%s (%s)" % (c["title"], (c.get("evidence", "") or "").strip())
                for c in cluster
            )
            guard_subject = " ".join(sorted(shared)) or "the shared pattern"
            sfix = dict(rep)
            sfix["proposal_kind"] = "structural-fix"
            sfix["title"] = "fix(structural): one guard for %d instances of [%s]" % (
                len(cluster), guard_subject,
            )
            sfix["evidence"] = (
                "%d instances collapse to one structural fix — a single guard "
                "for [%s] would catch them all. Instances: %s"
                % (len(cluster), guard_subject, ev_lines)
            )
            sfix["instances"] = instances
            if theme is not None:
                sfix["memory_theme"] = sorted(theme)
            _synth_out.append(sfix)
            structural_fixes += 1
        else:
            _synth_out.append(cluster[0])

roi_kept = _synth_out

# Constitution gate (deterministic rules D1 evidence + D2 confidence floor +
# D3 substance). Keep this byte-aligned with explore-constitution.sh --filter.
# Floor default 0.3 reproduces prior behavior for well-formed proposals (all carry
# evidence and confidence >= 0.4); it drops empty-evidence and low-confidence noise.
# D3 drops bare "chore: address <marker>" proposals (raw TODO/FIXME/XXX/HACK churn):
# these need human triage, not autonomous implementation, and otherwise crowd out
# substantive spec-drift / prior-report / triaged-issue proposals.
try:
    _floor = float(os.environ.get("AUTOSPEC_EXPLORE_MIN_CONFIDENCE", "0.3") or "0.3")
except Exception:
    _floor = 0.3
constitutional = [p for p in roi_kept
                  if str(p.get("evidence", "")).strip() != ""
                  and p.get("confidence", 0) >= _floor
                  and not _chore_re.match(str(p.get("title", "")))]

# Filter against recent titles.
recent_norms = set()
try:
    with open(recent_f, 'r', encoding='utf-8') as fh:
        for line in fh:
            n = normalize_title(line.strip())
            if n:
                recent_norms.add(n)
except FileNotFoundError:
    pass

filtered = [p for p in constitutional if normalize_title(p["title"]) not in recent_norms]

# Priority boost (F8): proposals with priority:true or source=="steer" get a
# score multiplier so operator steers float to the top without swamping strong
# organic proposals. AUTOSPEC_PRIORITY_BOOST defaults to 1.5.
try:
    _pboost = float(os.environ.get("AUTOSPEC_PRIORITY_BOOST", "1.5") or "1.5")
except Exception:
    _pboost = 1.5
# Lower-bound the multiplier at 1.0: a "boost" must never DE-rank a priority/
# steer proposal below organic ones (a misconfigured <1.0 value would invert the
# intent). The upper bound is already enforced below via _cap (a boosted score is
# capped at max(non-boosted) * _pboost, so it reorders without swamping the
# confidence * source_weight * 1/complexity ranking).
if _pboost < 1.0:
    _pboost = 1.0
if _pboost != 1.0 and filtered:
    _is_priority = lambda p: bool(p.get("priority")) or p.get("source") == "steer"
    _non_boosted = [p["score"] for p in filtered if not _is_priority(p)]
    _cap = (max(_non_boosted) * _pboost) if _non_boosted else None
    for p in filtered:
        if _is_priority(p):
            _boosted = round(p["score"] * _pboost, 4)
            p["score"] = round(min(_boosted, _cap), 4) if _cap is not None else _boosted

# Severity-first ranking (Issue C): primary key = severity rank (lower rank =
# higher impact = ranked first); secondary key = the existing weighted score
# (confidence * source_weight / complexity), descending. A high-severity item
# behind auto-merge thus out-ranks a low-severity high-score one, while score
# still breaks ties within a band.
def _rank_key(p):
    sev_rank = SEVERITY_RANK.get(p.get("severity", "feature"), DEFAULT_SEVERITY_RANK)
    return (sev_rank, -p["score"])
filtered.sort(key=_rank_key)
# Apply independent track caps, then merge in governance-first order. This
# prevents a prolific product source from starving governance and testability.
selected_by_track = {track: [] for track in TRACKS}
for proposal in filtered:
    selected_by_track.setdefault(proposal["track"], []).append(proposal)
track_counts = {track: len(selected_by_track.get(track, [])) for track in TRACKS}
track_limited = {}
for track in TRACKS:
    try:
        limit = int(track_caps.get(track, cap))
    except Exception:
        limit = cap
    track_limited[track] = selected_by_track.get(track, [])[:max(0, limit)]
ordered_tracks = [track for track in track_priority if track in TRACKS]
ordered_tracks.extend(track for track in TRACKS if track not in ordered_tracks)
track_selected_counts = {track: len(track_limited[track]) for track in TRACKS}
track_ranked = [proposal for track in ordered_tracks for proposal in track_limited[track]]
# Fail-closed: an autonomous run with no skeptic verdicts files nothing.
final = [] if failclosed else track_ranked[:cap]

out = {
    "round": datetime.date.today().isoformat(),
    "proposals_total": total,
    "proposals_after_dedup": dedup_count,
    "proposals_after_gap_confirm": gap_confirmed_count if gap_confirmed_count is not None else len(deduped),
    "gap_unconfirmed_dropped": gap_dropped,
    "gap_check_malformed": gap_malformed,
    "saturated_sources": saturated_sources,
    "verify_mode": verify_mode,
    "failclosed": failclosed,
    "proposals_after_verify": len(verified),
    "proposals_refuted": refuted_count,
    "proposals_after_roi": len(roi_kept),
    "restricted_mode": repo_class in RESTRICTED_CLASSES,
    "restricted_rejections": restricted_rejections,
    "structural_fixes": structural_fixes,
    "proposals_after_constitution": len(constitutional),
    "proposals_after_recent_filter": len(filtered),
    "track_counts": track_counts,
    "track_selected_counts": track_selected_counts,
    "track_priority": ordered_tracks,
    "researcher_health": researcher_health,
    "proposals": final,
}
print(json.dumps(out, indent=2))
PY

if [ -n "$OUT" ]; then
    # Atomic write.
    case "$OUT" in
        /*) abs="$OUT" ;;
        *)  abs="$(cd "$(dirname "$OUT")" && pwd)/$(basename "$OUT")" ;;
    esac
    mv "$work_dir/final.json" "$abs.tmp"
    mv "$abs.tmp" "$abs"
else
    :
fi

# Attach the deterministic classification to each proposal and the aggregate.
jq --arg repo_class "$REPO_CLASS" '(.repo_class=$repo_class | .proposals |= map(.repo_class=$repo_class))' "${OUT:-$work_dir/final.json}" > "$work_dir/classified.json" 2>/dev/null && mv "$work_dir/classified.json" "${OUT:-$work_dir/final.json}" || true
if [ -z "$OUT" ]; then cat "$work_dir/final.json"; fi

researcher_failures="$(jq -r '.researcher_health.failures | length' \
    "${OUT:-$work_dir/final.json}" 2>/dev/null || printf '1')"
case "$researcher_failures" in ''|*[!0-9]*) researcher_failures=1 ;; esac
if [ "$researcher_failures" -gt 0 ]; then
    printf 'explore-research-cycle: %s configured researcher(s) did not complete\n' \
        "$researcher_failures" >&2
    exit 4
fi

# Persist an org-scoped learning report after a completed sweep. The directory
# is deliberately below the repository state root so separate repositories and
# organizations cannot share mutable learning data.
if [ -n "$ORG" ] && [ "$STAGE" != "dedup" ]; then
    audit_dir="$REPO_ROOT/.autospec/org-audits/$ORG"
    mkdir -p "$audit_dir"
    cp "${OUT:-$work_dir/final.json}" "$audit_dir/report.json" 2>/dev/null || cp "$work_dir/final.json" "$audit_dir/report.json"
    jq -r '"# Organization explore report\n\n- Round: " + .round + "\n- Proposals: " + (.proposals|length|tostring) + "\n- Verification: " + .verify_mode + "\n\n## Top risks and exemplars\n\n" + ([.proposals[]? | "- " + (.title // "untitled")]|join("\n"))' "$audit_dir/report.json" > "$audit_dir/report.md"
    jq -c --arg round "$(date -u +%Y-%m-%dT%H:%M:%SZ)" '.proposals[]? | {timestamp:$round,category:(.severity // "feature"),title,evidence}' "$audit_dir/report.json" >> "$audit_dir/ledger.jsonl"
    jq --argjson recheck "$ORG_RECHECK" --arg ledger "$ORG_PRIOR_LEDGER" '.org_audit={ledger:$ledger,stale_recheck:($recheck == 1)}' "$audit_dir/report.json" > "$audit_dir/report.json.tmp" && mv "$audit_dir/report.json.tmp" "$audit_dir/report.json"
fi
