#!/usr/bin/env bash
# scripts/discover-model-supply.sh — deterministic model-supply probe.
#
# Emits a capability document describing what this host can ACTUALLY run:
# accelerator state, reachable local runtimes, per-model measured facts, and
# cloud provider availability. Default: ~/.autospec/model-capability.json
#
# Why this is a script and not prose: the auto-init block in
# skills/autospec-run/SKILL.md used to ask the orchestrating LLM to enumerate
# local models. On a live host that produced nine local profiles for four real
# models — including an unrunnable qwen3-coder-480b — rated every one of them
# ctx: 64k when the measured values were 131072 and 262144, and never re-probed
# because auto-init only ran when the file was absent. Enumerating ground truth
# belongs in a tool (tracker #421).
#
# Probe primitives live in lib/model-supply-probe.sh; the rules they enforce
# (fail closed on the accelerator, measure never parse the tag, completion models
# only, replace never merge, CPU-only is not dispatchable) are documented there
# and in docs/specs/2026-08-05-self-discovering-model-routing-design.md.
#
# Usage:
#   discover-model-supply.sh [--out <path>] [--ttl <seconds>] [--force]
#   discover-model-supply.sh --json          # probe (honouring cache) + print JSON
#   discover-model-supply.sh --profiles      # print a model-profiles.yml fragment
#   discover-model-supply.sh --fingerprint   # print the hardware fingerprint only
#   discover-model-supply.sh -h | --help
#
# Environment:
#   AUTOSPEC_MODEL_CAPABILITY  default output path
#   AUTOSPEC_OLLAMA_HOST       ollama host:port (default 127.0.0.1:11434)
#
# Exit codes:
#   0  probe complete (fresh or cache-hit)
#   1  usage error
#   2  jq missing — fail closed, this is a data-integrity tool
#
# bash 3.2+. set -eu; if/then/fi one-sided conditionals; no RETURN traps.

set -eu

OUT_PATH="${AUTOSPEC_MODEL_CAPABILITY:-$HOME/.autospec/model-capability.json}"
TTL=86400
FORCE=0
PRINT_JSON=0
PRINT_PROFILES=0
PRINT_FINGERPRINT=0
OLLAMA_HOST="${AUTOSPEC_OLLAMA_HOST:-127.0.0.1:11434}"
SCHEMA_VERSION="autospec.model-capability.v1"
CURL_TIMEOUT=2

HELP_TEXT='discover-model-supply.sh — deterministic model-supply probe.

Usage:
  discover-model-supply.sh [--out <path>] [--ttl <seconds>] [--force]
  discover-model-supply.sh --json
  discover-model-supply.sh --profiles
  discover-model-supply.sh --fingerprint
  discover-model-supply.sh -h | --help

Writes a capability document (default ~/.autospec/model-capability.json)
describing the usable accelerator, reachable local runtimes, measured
per-model facts, and cloud provider availability.'

_die() { printf 'discover-model-supply: %s\n' "$1" >&2; exit "${2:-1}"; }

while [ $# -gt 0 ]; do
    case "$1" in
        -h|--help) printf '%s\n' "$HELP_TEXT"; exit 0 ;;
        --out)
            if [ $# -lt 2 ]; then _die '--out requires an argument'; fi
            OUT_PATH="$2"; shift 2 ;;
        --ttl)
            if [ $# -lt 2 ]; then _die '--ttl requires an argument'; fi
            TTL="$2"; shift 2 ;;
        --force)       FORCE=1; shift ;;
        --json)        PRINT_JSON=1; shift ;;
        --profiles)    PRINT_PROFILES=1; shift ;;
        --fingerprint) PRINT_FINGERPRINT=1; shift ;;
        *) _die "unknown option: $1" ;;
    esac
done

case "$TTL" in
    ''|*[!0-9]*) _die "--ttl must be a non-negative integer: $TTL" ;;
esac

if ! command -v jq >/dev/null 2>&1; then
    _die 'jq is required (data-integrity tool, fails closed)' 2
fi

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd -P)"
PROBE_LIB="$SCRIPT_DIR/lib/model-supply-probe.sh"
if [ ! -f "$PROBE_LIB" ]; then
    _die "probe library not found: $PROBE_LIB" 2
fi
# shellcheck source=scripts/lib/model-supply-probe.sh
. "$PROBE_LIB"

# ── probe ─────────────────────────────────────────────────────────────────────

probe_accelerator

cpu_only="true"
if [ "$ACC_USABLE" = "true" ]; then cpu_only="false"; fi

budget_mb="$ACC_VRAM_FREE"
if [ "$cpu_only" = "true" ]; then budget_mb="$(system_ram_mb)"; fi
if [ -z "$budget_mb" ]; then budget_mb=0; fi

models_json="$(probe_ollama_models "$budget_mb" "$cpu_only")"
fp="$(fingerprint "$models_json")"

if [ "$PRINT_FINGERPRINT" -eq 1 ]; then
    printf '%s\n' "$fp"
    exit 0
fi

# ── cache ─────────────────────────────────────────────────────────────────────
# Same fingerprint AND within TTL means the document is left byte-identical.

reuse=0
if [ "$FORCE" -eq 0 ] && [ -f "$OUT_PATH" ]; then
    prev_fp="$(jq -r '.fingerprint // ""' "$OUT_PATH" 2>/dev/null || printf '')"
    prev_ts="$(jq -r '.probed_at_epoch // 0' "$OUT_PATH" 2>/dev/null || printf '0')"
    case "$prev_ts" in ''|*[!0-9]*) prev_ts=0 ;; esac
    if [ "$prev_fp" = "$fp" ] && [ $(( $(_now) - prev_ts )) -lt "$TTL" ]; then
        reuse=1
    fi
fi

if [ "$reuse" -eq 0 ]; then
    acc_json="$(jq -nc \
        --arg kind "$ACC_KIND" --arg name "$(_clean "$ACC_NAME")" \
        --arg reason "$ACC_REASON" \
        --argjson total "${ACC_VRAM_TOTAL:-0}" --argjson free "${ACC_VRAM_FREE:-0}" \
        --argjson usable "$ACC_USABLE" \
        '{kind:$kind, name:$name, vram_total_mb:$total, vram_free_mb:$free,
          usable:$usable, reason:$reason}')"

    # The model set is REPLACED, never merged, so ghost entries cannot persist.
    doc="$(jq -n \
        --arg schema "$SCHEMA_VERSION" --arg fp "$fp" --arg ts "$(_utc)" \
        --argjson epoch "$(_now)" --argjson budget "$budget_mb" \
        --argjson cpu_only "$cpu_only" --argjson acc "$acc_json" \
        --argjson runtimes "$(probe_runtimes)" \
        --argjson models "$models_json" \
        --argjson cloud "$(probe_cloud)" \
        '{schema:$schema, fingerprint:$fp, probed_at:$ts, probed_at_epoch:$epoch,
          accelerator:$acc, cpu_only:$cpu_only, memory_budget_mb:$budget,
          runtimes:$runtimes, local_models:$models, cloud:$cloud}')"

    out_dir="$(dirname "$OUT_PATH")"
    if [ ! -d "$out_dir" ]; then mkdir -p "$out_dir"; fi
    tmp="${OUT_PATH}.tmp.$$"
    printf '%s\n' "$doc" > "$tmp"
    mv -f "$tmp" "$OUT_PATH"
fi

# ── output ────────────────────────────────────────────────────────────────────

if [ "$PRINT_JSON" -eq 1 ]; then
    cat "$OUT_PATH"
fi

if [ "$PRINT_PROFILES" -eq 1 ]; then
    # Only dispatch-recommended models are emitted uncommented; the rest appear
    # commented so an operator sees what was found AND why it is unused.
    printf '# Generated by discover-model-supply.sh — do not hand-edit.\n'
    printf '# fingerprint: %s\n' "$fp"
    if [ "$cpu_only" = "true" ]; then
        printf '# NOTE: no usable accelerator (%s). Local models are listed but\n' \
            "$(jq -r '.accelerator.reason // "unknown"' "$OUT_PATH")"
        printf '#       NOT dispatch-recommended: CPU inference blows the wall-clock ceiling.\n'
    fi
    # A bare `profiles:` followed only by comments parses as null, and a null
    # profiles map breaks the --profile ordinal filter. Emit an explicit empty
    # mapping so the fragment is safe to paste verbatim.
    if [ "$(jq '[.local_models[] | select(.dispatch_recommended)] | length' "$OUT_PATH")" -eq 0 ]; then
        printf 'profiles: {}\n'
    else
        printf 'profiles:\n'
    fi
    jq -r '.local_models[] |
        if .dispatch_recommended then
          "  \(.profile):\n    model: \(.model)\n    ctx: \(.ctx)\n    reasoning: \(.reasoning)"
        else
          "  # \(.profile): model=\(.model) ctx=\(.ctx) — not dispatch-recommended"
        end' "$OUT_PATH"
fi

exit 0
