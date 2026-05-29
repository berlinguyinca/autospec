#!/usr/bin/env bash
# scripts/refine-render-overview.sh — dual-artifact renderer for autospec-refine
# (issue #671). Reads the orchestrator JSON output written by
# scripts/refine-prompt.sh (issue #670) and emits:
#   <output-dir>/<slug>-<timestamp>.md   human-readable overview
#   <output-dir>/<slug>-<timestamp>.json  copy of the input JSON (canonical)
#
# Both files are written atomically (.tmp + mv). The input JSON is validated
# against schemas/autospec-refinement.schema.json before rendering.
#
# Usage:
#   refine-render-overview.sh --json <input.json> --slug <slug> \
#       [--output-dir <dir>]
#
# Exit codes:
#   0  on success
#   2  on usage / bad args
#   3  on schema validation failure
#   4  on missing input

set -u

usage() {
    cat <<'EOF'
Usage: refine-render-overview.sh --json <input> --slug <slug> [--output-dir <dir>]

Reads the autospec-refine orchestrator JSON output and writes a dual
(markdown + JSON) artifact pair to <output-dir>, defaulting to
.autospec/refinements/. Files are written atomically via .tmp + mv.
EOF
}

INPUT_JSON=""
SLUG=""
OUTPUT_DIR=".autospec/refinements"
SCHEMA=""

while [ $# -gt 0 ]; do
    case "$1" in
        --json) INPUT_JSON="$2"; shift ;;
        --slug) SLUG="$2"; shift ;;
        --output-dir) OUTPUT_DIR="$2"; shift ;;
        --schema) SCHEMA="$2"; shift ;;
        --help|-h) usage; exit 0 ;;
        *) echo "refine-render-overview: unknown arg: $1" >&2; usage >&2; exit 2 ;;
    esac
    shift
done

if [ -z "$INPUT_JSON" ] || [ -z "$SLUG" ]; then
    echo "refine-render-overview: --json and --slug are required" >&2
    usage >&2
    exit 2
fi

# ── slug sanitization (issue #680) ────────────────────────────────
# Reject /, .., whitespace, control chars, or any char outside [a-z0-9-]
# after lowercasing. Matches the orchestrator's slug_from_prompt() shape.
_slug_sanitize_check() {
    local s="$1"
    [ -n "$s" ] || { echo "refine-render-overview: --slug is empty" >&2; return 2; }
    case "$s" in
        */*) echo "refine-render-overview: --slug contains '/': $s" >&2; return 2 ;;
        *..*) echo "refine-render-overview: --slug contains '..': $s" >&2; return 2 ;;
    esac
    # Lowercase + check character class.
    local lower
    lower="$(printf '%s' "$s" | tr '[:upper:]' '[:lower:]')"
    if ! printf '%s' "$lower" | LC_ALL=C grep -qE '^[a-z0-9-]+$'; then
        echo "refine-render-overview: --slug has invalid chars (allowed: [a-z0-9-]): $s" >&2
        return 2
    fi
    return 0
}
if ! _slug_sanitize_check "$SLUG"; then
    exit 2
fi

# ── path allowlist (issue #680) ───────────────────────────────────
# Same forbidden patterns as refine-prompt.sh::check_path_allowed.
_render_match_forbidden() {
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
_render_canonicalize() {
    local p="$1"
    local r=""
    if command -v realpath >/dev/null 2>&1; then
        r="$(realpath -m "$p" 2>/dev/null)" || r=""
        if [ -n "$r" ]; then printf '%s' "$r"; return; fi
        r="$(realpath "$p" 2>/dev/null)" || r=""
        if [ -n "$r" ]; then printf '%s' "$r"; return; fi
    fi
    if command -v readlink >/dev/null 2>&1; then
        r="$(readlink -f "$p" 2>/dev/null)" || r=""
        if [ -n "$r" ]; then printf '%s' "$r"; return; fi
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
    if command -v python3 >/dev/null 2>&1; then
        r="$(python3 -c 'import os,sys; print(os.path.realpath(sys.argv[1]))' "$p" 2>/dev/null)" || r=""
        if [ -n "$r" ]; then printf '%s' "$r"; return; fi
    fi
    printf '%s' "$p"
}
_render_check_path_allowed() {
    local p="$1"
    _render_match_forbidden "$p" && return 1
    local resolved
    resolved="$(_render_canonicalize "$p")"
    if [ -n "$resolved" ] && [ "$resolved" != "$p" ]; then
        _render_match_forbidden "$resolved" && return 1
    fi
    case "$resolved" in
        *..*) return 1 ;;
    esac
    return 0
}
for _p in "$INPUT_JSON" "$OUTPUT_DIR"; do
    if ! _render_check_path_allowed "$_p"; then
        echo "code_health:refine_path_violation path=$_p" >&2
        exit 3
    fi
done

if [ ! -f "$INPUT_JSON" ]; then
    echo "refine-render-overview: input not found: $INPUT_JSON" >&2
    exit 4
fi

# Locate the schema. Honor --schema, then sibling schemas/ dir.
if [ -z "$SCHEMA" ]; then
    SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
    CANDIDATE="$SCRIPT_DIR/../schemas/autospec-refinement.schema.json"
    if [ -f "$CANDIDATE" ]; then
        SCHEMA="$CANDIDATE"
    fi
fi

# ── schema validation ─────────────────────────────────────────────
# Prefer ajv-cli, fall back to python3 jsonschema, fall back to a shape check.
validate_with_ajv() {
    command -v ajv >/dev/null 2>&1 || return 2
    # Try draft2020 spec first (matches our schema), fall back to default.
    if ajv validate --spec=draft2020 -s "$SCHEMA" -d "$INPUT_JSON" >/dev/null 2>&1; then
        return 0
    fi
    # If ajv can't even load the schema (e.g. unsupported draft URI on this
    # ajv build), treat as "tool unable to validate" (rc=2) so we fall through
    # to python jsonschema, NOT as a validation failure.
    local err
    err="$(ajv validate -s "$SCHEMA" -d "$INPUT_JSON" 2>&1)"
    if printf '%s' "$err" | grep -q "schema .* is invalid\|no schema with key or ref"; then
        return 2
    fi
    return 1
}

validate_with_python() {
    command -v python3 >/dev/null 2>&1 || return 2
    python3 - "$SCHEMA" "$INPUT_JSON" <<'PY' 2>/dev/null
import json, sys
try:
    import jsonschema
except Exception:
    sys.exit(2)
schema = json.load(open(sys.argv[1]))
data = json.load(open(sys.argv[2]))
try:
    jsonschema.validate(data, schema)
except Exception as e:
    print(str(e), file=sys.stderr)
    sys.exit(1)
PY
}

validate_shape_check() {
    # Minimum awk/python3-driven shape check: must have the 5 top-level keys.
    command -v python3 >/dev/null 2>&1 || return 0
    python3 - "$INPUT_JSON" <<'PY'
import json, sys
data = json.load(open(sys.argv[1]))
required = ["original_prompt", "rounds", "final_prompt", "status", "metadata"]
missing = [k for k in required if k not in data]
if missing:
    print("missing keys: " + ",".join(missing), file=sys.stderr)
    sys.exit(1)
if not isinstance(data["rounds"], list):
    print("rounds must be array", file=sys.stderr); sys.exit(1)
md = data["metadata"]
for k in ["head_sha","timestamp","rounds_requested","rounds_executed",
          "converged_early","degraded_rounds","handoff_target","handoff_executed"]:
    if k not in md:
        print("metadata missing: " + k, file=sys.stderr); sys.exit(1)
allowed_status = {"approved","completed","converged","degraded","round_cap_reached","aborted"}
if data["status"] not in allowed_status:
    print("bad status: " + data["status"], file=sys.stderr); sys.exit(1)
PY
}

VALIDATED=0
if [ -n "$SCHEMA" ] && [ -f "$SCHEMA" ]; then
    if validate_with_ajv; then
        VALIDATED=1
    else
        rc=$?
        if [ "$rc" -eq 1 ]; then
            echo "refine-render-overview: schema validation failed (ajv) for $INPUT_JSON" >&2
            exit 3
        fi
        if validate_with_python; then
            VALIDATED=1
        else
            rc=$?
            if [ "$rc" -eq 1 ]; then
                echo "refine-render-overview: schema validation failed (jsonschema) for $INPUT_JSON" >&2
                exit 3
            fi
        fi
    fi
fi

if [ "$VALIDATED" = 0 ]; then
    # Fallback shape check — exits 1 on shape failure.
    if ! validate_shape_check; then
        echo "refine-render-overview: shape check failed for $INPUT_JSON" >&2
        exit 3
    fi
fi

# ── render markdown ───────────────────────────────────────────────
mkdir -p "$OUTPUT_DIR"
# Derive the timestamp from the input filename when it matches the
# orchestrator's naming convention (<slug>-<ISO-ts>.json), so the .md
# sibling lands beside its source .json instead of forking a new
# timestamp. Falls back to "now" only when the input is renamed/synthetic.
INPUT_BASE="$(basename "$INPUT_JSON" .json)"
INPUT_TS="${INPUT_BASE#${SLUG}-}"
if [ "$INPUT_TS" != "$INPUT_BASE" ] && \
   printf '%s' "$INPUT_TS" | grep -qE '^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}-[0-9]{2}-[0-9]{2}Z$'; then
    TS="$INPUT_TS"
else
    TS="$(date -u +'%Y-%m-%dT%H-%M-%SZ')"
fi
BASE="$OUTPUT_DIR/${SLUG}-${TS}"
MD_PATH="$BASE.md"
JSON_PATH="$BASE.json"
MD_TMP="$MD_PATH.tmp"
JSON_TMP="$JSON_PATH.tmp"

python3 - "$INPUT_JSON" "$MD_TMP" "$SLUG" <<'PY'
import json, sys, difflib

src, md_path, slug = sys.argv[1], sys.argv[2], sys.argv[3]
data = json.load(open(src))

orig = data["original_prompt"]
rounds = data.get("rounds", [])
final = data["final_prompt"]
status = data["status"]
md = data["metadata"]

lines = []
lines.append(f"# autospec-refine overview: {slug}")
lines.append("")
lines.append(f"- Status: `{status}`")
lines.append(f"- Head SHA: `{md.get('head_sha','unknown')}`")
lines.append(f"- Timestamp: `{md.get('timestamp','')}`")
lines.append(f"- Rounds requested / executed: "
             f"{md.get('rounds_requested','?')} / {md.get('rounds_executed','?')}")
lines.append(f"- Converged early: `{md.get('converged_early', False)}`")
deg = md.get("degraded_rounds", []) or []
lines.append(f"- Degraded rounds: {', '.join(deg) if deg else 'none'}")
lines.append(f"- Handoff target: `{md.get('handoff_target','')}`")
lines.append(f"- Handoff executed: `{md.get('handoff_executed', False)}`")
if "context_sparse" in md:
    lines.append(f"- Context sparse: `{md['context_sparse']}`")
lines.append("")

lines.append("## Original prompt")
lines.append("")
lines.append("```")
lines.append(orig)
lines.append("```")
lines.append("")

prev = orig
for r in rounds:
    n = r.get("round_number", "?")
    lens = r.get("lens", "?")
    lines.append(f"## Round {n} — lens: `{lens}`")
    lines.append("")
    srcs = r.get("sources_used", []) or []
    if srcs:
        lines.append("Sources used:")
        for s in srcs:
            lines.append(f"- `{s}`")
    else:
        lines.append("Sources used: _none_")
    lines.append("")
    ds = r.get("diff_summary", "")
    if ds:
        lines.append(f"Diff summary: {ds}")
    wcd = r.get("word_count_delta", 0)
    lines.append(f"Word count delta: {wcd:+d}" if isinstance(wcd, int) else
                 f"Word count delta: {wcd}")
    lines.append("")
    refined = r.get("refined_prompt", "")
    diff = difflib.unified_diff(
        prev.splitlines(),
        refined.splitlines(),
        fromfile=f"round-{int(n)-1 if isinstance(n,int) else 'prev'}",
        tofile=f"round-{n}",
        lineterm="",
    )
    diff_text = "\n".join(diff)
    lines.append("Unified diff vs previous round:")
    lines.append("")
    lines.append("```diff")
    if diff_text.strip():
        lines.append(diff_text)
    else:
        lines.append("(no textual change)")
    lines.append("```")
    lines.append("")
    reasoning = r.get("reasoning", "")
    if reasoning:
        lines.append(f"Reasoning: {reasoning}")
        lines.append("")
    prev = refined

lines.append("## Final prompt")
lines.append("")
lines.append("```")
lines.append(final)
lines.append("```")
lines.append("")

with open(md_path, "w") as f:
    f.write("\n".join(lines))
PY

# Copy the input JSON atomically as well.
cp "$INPUT_JSON" "$JSON_TMP"

mv "$MD_TMP" "$MD_PATH"
mv "$JSON_TMP" "$JSON_PATH"

echo "refine-render-overview: wrote $MD_PATH and $JSON_PATH"
exit 0
