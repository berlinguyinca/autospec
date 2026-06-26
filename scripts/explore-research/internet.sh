#!/usr/bin/env bash
# scripts/explore-research/internet.sh — LLM-heavy researcher #6 of 6.
#
# Multi-stage pipeline:
#   1. Collect candidate URLs: .autospec/competitors.yml (Tier-3) when present;
#      fall back to README/"purpose"-derived URLs when absent.
#   2. For each candidate URL: enforce domain allowlist (PR #685 pattern via
#      scripts/lib/explore-internet-safety.sh::explore_internet_domain_allowed).
#   3. Rate-limit check (5/round, 30/session).
#   4. Fetch (via curl or AUTOSPEC_FETCH_STUB).
#   5. Prompt-injection guard on fetched content.
#   6. LLM extracts feature proposals (behavior-level clean-room directive);
#      each proposal includes the citation URL in its evidence string.
#
# Output: JSON {"source":"internet","proposals":[…]} on stdout.
#
# Exit codes:
#   0 — normal (proposals or empty array).
#   3 — safety-fault occurred mid-pipeline. The cycle aggregator treats this
#       as a researcher failure (per spec §"Failure modes").
#
# Test seams:
#   AUTOSPEC_LLM_STUB_OUTPUT     — bypass LLM dispatch.
#   AUTOSPEC_FETCH_STUB_<host>   — content returned for fetch of <host>.
#   AUTOSPEC_FETCH_URLS_OVERRIDE — newline-separated URLs (skips README scan).
#   AUTOSPEC_EXPLORE_INTERNET_ALLOWLIST — CSV overriding the default allowlist.
#   AUTOSPEC_COMPETITORS_YML     — override path for competitors.yml (tests).

set -u

REPO_ROOT="${AUTOSPEC_REPO_ROOT:-$(git rev-parse --show-toplevel 2>/dev/null || pwd)}"

_SAFETY_LIB="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/lib/explore-internet-safety.sh"
if [ ! -f "$_SAFETY_LIB" ]; then
    echo "internet: missing safety library: $_SAFETY_LIB" >&2
    echo '{"source":"internet","proposals":[]}'
    exit 0
fi
# shellcheck source=../lib/explore-internet-safety.sh
. "$_SAFETY_LIB"

ALLOWLIST="${AUTOSPEC_EXPLORE_INTERNET_ALLOWLIST:-$EXPLORE_DEFAULT_ALLOWLIST}"

cd "$REPO_ROOT" 2>/dev/null || true

_emit_empty() {
    echo '{"source":"internet","proposals":[]}'
    exit 0
}

# ── competitors.yml parser (F7, issue #1399) ───────────────────────
# Reads URL entries from .autospec/competitors.yml.
# Schema:
#   version: 1
#   competitors:
#     - name: "tool-a"
#       url: "https://github.com/tool-a/tool-a"
# Falls back to a simple line-scan when PyYAML is unavailable.
_parse_competitors_yml() {
    local yml="$1"
    [ -f "$yml" ] || return 0
    python3 - "$yml" <<'PY'
import sys, re
path = sys.argv[1]

def line_scan():
    # Fallback: extract url: lines without a PyYAML dependency.
    with open(path) as fh:
        for line in fh:
            m = re.match(r'\s+url:\s*["\']?(https?://[^\s"\'#]+)', line)
            if m:
                print(m.group(1))

try:
    import yaml  # PyYAML
except ImportError:
    line_scan()
    sys.exit(0)

try:
    with open(path) as fh:
        d = yaml.safe_load(fh)
    for entry in (d or {}).get("competitors", []):
        u = str(entry.get("url", "")).strip()
        if u.startswith("http"):
            print(u)
except Exception as exc:
    # Malformed YAML (or unexpected shape): degrade to the line-scan so a
    # well-formed `url:` line is still honored instead of silently dropped.
    sys.stderr.write(f"competitors.yml parse error: {exc}; line-scan fallback\n")
    line_scan()
PY
}

# ── Stage 1: collect candidate URLs ───────────────────────────────
collect_urls() {
    if [ -n "${AUTOSPEC_FETCH_URLS_OVERRIDE:-}" ]; then
        printf '%s\n' "$AUTOSPEC_FETCH_URLS_OVERRIDE"
        return
    fi

    # F7: prefer .autospec/competitors.yml (Tier-3 targets) when present.
    _COMPETITORS_YML="${AUTOSPEC_COMPETITORS_YML:-$REPO_ROOT/.autospec/competitors.yml}"
    if [ -f "$_COMPETITORS_YML" ]; then
        _parse_competitors_yml "$_COMPETITORS_YML" | head -n 20
        return
    fi

    # Fallback: README-derived URLs from Alternatives/Competitors sections.
    [ -f README.md ] || return 0
    # Pull URLs from "Alternatives" / "Prior art" / "Competitors" sections.
    awk '
        /^##+/ {
            section = tolower($0)
            in_section = 0
            if (section ~ /alternative|prior art|competitor|inspired by|related/) in_section = 1
        }
        in_section { print }
    ' README.md 2>/dev/null | grep -oE 'https?://[A-Za-z0-9./?&_%=:#~+-]+' 2>/dev/null | head -n 20
}

URLS="$(collect_urls)"
[ -n "$URLS" ] || _emit_empty

# ── Stage 2-5: per-URL safety + fetch ─────────────────────────────
FETCHED_DIR="$(mktemp -d -t explore-internet.XXXXXX)"
trap 'rm -rf "$FETCHED_DIR"' EXIT

EXIT_CODE=0
COUNT=0

while IFS= read -r url; do
    [ -n "$url" ] || continue

    # Allowlist gate.
    if ! explore_internet_domain_allowed "$url" "$ALLOWLIST"; then
        EXIT_CODE=3
        break
    fi

    # Rate-limit gate.
    if ! explore_internet_rate_check "$url"; then
        EXIT_CODE=3
        break
    fi

    # Fetch — stub seam first.
    host="$(printf '%s' "$url" | sed -E -e 's#^https?://##' -e 's#/.*##' -e 's#:.*##' | tr '[:upper:]' '[:lower:]')"
    stub_var="AUTOSPEC_FETCH_STUB_$(printf '%s' "$host" | tr '.' '_' | tr '-' '_')"
    content=""
    if [ -n "${!stub_var:-}" ]; then
        content="${!stub_var}"
    elif command -v curl >/dev/null 2>&1; then
        content="$(curl -fsSL --max-time 10 "$url" 2>/dev/null || true)"
    fi
    [ -n "$content" ] || continue

    # Injection guard.
    if ! explore_internet_injection_guard "$content"; then
        EXIT_CODE=3
        break
    fi

    COUNT=$((COUNT + 1))
    # Record citation alongside fetched content.
    printf '%s\n---END-URL---\n%s\n---END-CONTENT---\n' "$url" "$content" \
        >> "$FETCHED_DIR/corpus.txt"
done <<< "$URLS"

if [ "$EXIT_CODE" -ne 0 ]; then
    echo '{"source":"internet","proposals":[]}'
    exit "$EXIT_CODE"
fi

[ "$COUNT" -gt 0 ] || _emit_empty

# ── Stage 6: LLM extracts features ────────────────────────────────
LLM_OUT=""
if [ -n "${AUTOSPEC_LLM_STUB_OUTPUT:-}" ]; then
    LLM_OUT="$AUTOSPEC_LLM_STUB_OUTPUT"
else
    _HARNESS_LIB="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/lib/autospec-harness-detect.sh"
    if [ -f "$_HARNESS_LIB" ]; then
        # shellcheck source=../lib/autospec-harness-detect.sh
        . "$_HARNESS_LIB"
    fi
    KIND=""
    if declare -F autospec_harness_detect >/dev/null 2>&1; then
        KIND="$(autospec_harness_detect 2>/dev/null || echo claude)"
    fi
    DISPATCHER=""
    case "$KIND" in
        codex) command -v codex >/dev/null 2>&1 && DISPATCHER="$(command -v codex)" ;;
        *)     command -v claude >/dev/null 2>&1 && DISPATCHER="$(command -v claude)" ;;
    esac
    if [ -z "$DISPATCHER" ]; then
        echo "internet: no LLM harness on PATH; graceful degrade" >&2
        _emit_empty
    fi
    if declare -F autospec_harness_dispatcher_safe >/dev/null 2>&1; then
        if ! autospec_harness_dispatcher_safe "$DISPATCHER"; then
            echo "internet: unsafe dispatcher; graceful degrade" >&2
            _emit_empty
        fi
    fi
    INSTRUCTION='You are the autospec-explore "internet" researcher. Given the
fetched competitor pages below (delimited by ---END-URL--- and ---END-CONTENT---),
propose 1-5 feature ideas inspired by what those competitors do that this
repo does NOT. For each proposal include the source URL in the "evidence"
field (REQUIRED). Output ONLY a JSON array of {title, evidence,
estimated_complexity ∈ small|medium|large, confidence ∈ 0.0-1.0}.

CLEAN-ROOM DIRECTIVE (MANDATORY): Describe only what each competitor feature
DOES at the behavioral/UX level — what a user can observe or trigger. Never
reproduce competitor source code, copy algorithm implementations, or include
proprietary implementation details. Your output must be behavior-level
observations only. The autospec-secaudit IP/license check is the backstop;
your role is behavioral observation, not reverse-engineering.'
    CORPUS="$(cat "$FETCHED_DIR/corpus.txt")"
    INPUT="$INSTRUCTION"$'\n\n## Corpus\n'"$CORPUS"
    RC=0
    case "$KIND" in
        codex) LLM_OUT="$("$DISPATCHER" exec --reasoning-effort medium "$INPUT" 2>/dev/null)" || RC=$? ;;
        *)     LLM_OUT="$("$DISPATCHER" -p "$INPUT" 2>/dev/null)" || RC=$? ;;
    esac
    if [ "$RC" -ne 0 ] || [ -z "$LLM_OUT" ]; then
        echo "internet: LLM error rc=$RC; graceful degrade" >&2
        _emit_empty
    fi
fi

if ! command -v python3 >/dev/null 2>&1; then
    _emit_empty
fi

# Collect the citation URLs so the parser can enforce that every proposal
# carries one in its evidence string (per spec §Internet researcher safety).
CITATIONS="$(awk -v RS='---END-URL---' 'NR>1{ print prev } { prev=$0 } END { print prev }' "$FETCHED_DIR/corpus.txt" 2>/dev/null | head -n 1)"
URL_LIST="$(echo "$URLS")"

export _LLM_RAW="$LLM_OUT"
export _CITATION_URLS="$URL_LIST"

python3 - <<'PY'
import json, sys, re, os
raw = os.environ.get("_LLM_RAW","")
urls = [u for u in os.environ.get("_CITATION_URLS","").splitlines() if u.strip()]
out = {"source": "internet", "proposals": []}
m = re.search(r'\[.*\]', raw, re.DOTALL)
arr = None
if m:
    try:
        arr = json.loads(m.group(0))
    except Exception:
        arr = None
if isinstance(arr, list):
    for item in arr:
        if not isinstance(item, dict):
            continue
        title = str(item.get("title","")).strip()
        evidence = str(item.get("evidence","")).strip()
        complexity = item.get("estimated_complexity","medium")
        if complexity not in ("small","medium","large"):
            complexity = "medium"
        try:
            conf = float(item.get("confidence", 0.4))
        except Exception:
            conf = 0.4
        conf = max(0.0, min(1.0, conf))
        if not title or not evidence:
            continue
        # Enforce citation: if no URL in evidence, attach the first fetched one.
        if "http://" not in evidence and "https://" not in evidence and urls:
            evidence = f"{evidence} (source: {urls[0]})"
        # If still missing a URL, drop the proposal.
        if "http://" not in evidence and "https://" not in evidence:
            continue
        out["proposals"].append({
            "title": title,
            "evidence": evidence,
            "estimated_complexity": complexity,
            "confidence": conf,
        })
print(json.dumps(out))
PY
