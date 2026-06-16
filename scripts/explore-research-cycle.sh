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

usage() {
    cat <<'EOF'
Usage: explore-research-cycle.sh [options]

Options:
  --max-issues-per-round N     Cap final proposals (default 5).
  --research-sources LIST      Comma-separated subset of:
                                 spec-vs-code,prior-reports,codebase-signals,open-issues
                               Default: all 4.
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
EOF
}

MAX_ISSUES=5
SOURCES="spec-vs-code,prior-reports,codebase-signals,open-issues"
OUT=""
LEDGER="${AUTOSPEC_EXPLORE_LEDGER:-}"

while [ "$#" -gt 0 ]; do
    case "$1" in
        --max-issues-per-round) MAX_ISSUES="$2"; shift 2 ;;
        --research-sources)     SOURCES="$2"; shift 2 ;;
        --ledger)               LEDGER="$2"; shift 2 ;;
        --out)                  OUT="$2"; shift 2 ;;
        -h|--help)              usage; exit 0 ;;
        *) echo "explore-research-cycle: unknown arg: $1" >&2; usage; exit 2 ;;
    esac
done

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
        bash "$script" > "$work_dir/$src.json" 2>"$work_dir/$src.err" \
            || echo '{"source":"'"$src"'","proposals":[],"error":"researcher_failed"}' \
                 > "$work_dir/$src.json"
    ) &
    pids+=("$!")
done
unset IFS

# Wait for all background researchers.
for p in "${pids[@]:-}"; do
    [ -n "$p" ] && wait "$p" 2>/dev/null || true
done

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
python3 - <<'PY' > "$work_dir/final.json"
import json, os, re, glob, datetime

work     = os.environ["WORK_DIR"]
cap      = int(os.environ["MAX_ISSUES"])
recent_f = os.environ["RECENT_FILE"]

# Static source priors per the spec — the defensive fallback used verbatim when
# no dynamic weights are available (no ledger / weights script absent / broken).
DEFAULT_SRC_WEIGHTS = {
    "spec-vs-code":     1.0,
    "prior-reports":    0.9,
    "codebase-signals": 0.7,
    "dependency-health": 0.65,
    "open-issues":      0.6,
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
# rollout (spec: "only the three new ones are ROI-gated, to avoid silently
# muting the existing 7"). Any source NOT in this set — the three discovery
# researchers (quality-resilience, dogfooding, self-leverage) and
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

all_props = []
total = 0
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
        comp = (p.get("estimated_complexity") or "medium").lower()
        try:
            conf = float(p.get("confidence", 0.5))
        except Exception:
            conf = 0.5
        weight = SRC_WEIGHTS.get(src, 0.5)
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
        all_props.append({
            "title": title,
            "evidence": p.get("evidence",""),
            "estimated_complexity": comp,
            "confidence": conf,
            "source": src,
            "severity": severity,
            "named_consumer": named_consumer,
            "score": round(score, 4),
        })
        total += 1

# Dedup by normalized title — keep highest score.
by_norm = {}
for p in all_props:
    n = normalize_title(p["title"])
    if n not in by_norm or p["score"] > by_norm[n]["score"]:
        by_norm[n] = p
deduped = list(by_norm.values())

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
    if src not in LEGACY_SOURCES and consumer == "":
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
_synth_out = []
_by_band = {}
for _i, p in enumerate(roi_kept):
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

# Constitution gate (deterministic rules D1 evidence + D2 confidence floor).
# Keep this byte-aligned with explore-constitution.sh --filter. Floor default 0.3
# reproduces prior behavior for well-formed proposals (all carry evidence and
# confidence >= 0.4); it drops empty-evidence and low-confidence noise.
try:
    _floor = float(os.environ.get("AUTOSPEC_EXPLORE_MIN_CONFIDENCE", "0.3") or "0.3")
except Exception:
    _floor = 0.3
constitutional = [p for p in roi_kept
                  if str(p.get("evidence", "")).strip() != ""
                  and p.get("confidence", 0) >= _floor]

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

# Severity-first ranking (Issue C): primary key = severity rank (lower rank =
# higher impact = ranked first); secondary key = the existing weighted score
# (confidence * source_weight / complexity), descending. A high-severity item
# behind auto-merge thus out-ranks a low-severity high-score one, while score
# still breaks ties within a band.
def _rank_key(p):
    sev_rank = SEVERITY_RANK.get(p.get("severity", "feature"), DEFAULT_SEVERITY_RANK)
    return (sev_rank, -p["score"])
filtered.sort(key=_rank_key)
final = filtered[:cap]

out = {
    "round": datetime.date.today().isoformat(),
    "proposals_total": total,
    "proposals_after_dedup": len(deduped),
    "proposals_after_verify": len(verified),
    "proposals_refuted": refuted_count,
    "proposals_after_roi": len(roi_kept),
    "structural_fixes": structural_fixes,
    "proposals_after_constitution": len(constitutional),
    "proposals_after_recent_filter": len(filtered),
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
    cat "$work_dir/final.json"
fi
