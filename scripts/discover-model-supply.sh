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
#   discover-model-supply.sh --profiles --only <profile>   # just that one entry
#   discover-model-supply.sh --fingerprint   # print the hardware fingerprint only
#   discover-model-supply.sh -h | --help
#
# Each discovered model carries the §8 evidence level behind every capability
# field: `advertised` (the model's own untrusted claim, §51), `discovered` (a
# probe returned it), `calibrated` (calibrate-profile.sh confirmed it for a
# role), `observed` (real task outcomes). Routing precedence is
# observed > calibrated > discovered > advertised — see lib/model-capability-evidence.sh.
# Calibration verdicts are folded in on a FRESH probe; --force re-reads them
# after a calibration run on unchanged hardware.
#
# --only narrows the emitted fragment to a single profile. The probe still
# DISCOVERS everything — hiding models from discovery is what caused the
# blindness this tool was written to fix — so --only is a paste-time filter, not
# a probe restriction. Use it when a host has one resident model on purpose: a
# capacity-1 GPU thrashes on weight swaps, so an operator who has picked one
# model wants a fragment with only that model in it.
#
# Environment:
#   AUTOSPEC_MODEL_CAPABILITY  default output path
#   AUTOSPEC_OLLAMA_HOST       ollama host:port (default 127.0.0.1:11434)
#   AUTOSPEC_CALIBRATION_DIR   calibrate-profile.sh verdicts (default ~/.autospec/calibration)
#
# Exit codes:
#   0  probe complete (fresh or cache-hit)
#   1  usage error, including --only naming a profile the probe did not find
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
ONLY_PROFILE=
CALIBRATION_DIR="${AUTOSPEC_CALIBRATION_DIR:-$HOME/.autospec/calibration}"
OLLAMA_HOST="${AUTOSPEC_OLLAMA_HOST:-127.0.0.1:11434}"
SCHEMA_VERSION="autospec.model-capability.v1"
CURL_TIMEOUT=2

HELP_TEXT='discover-model-supply.sh — deterministic model-supply probe.

Usage:
  discover-model-supply.sh [--out <path>] [--ttl <seconds>] [--force]
  discover-model-supply.sh --json
  discover-model-supply.sh --profiles [--only <profile>]
  discover-model-supply.sh --fingerprint
  discover-model-supply.sh -h | --help

--only narrows the emitted fragment to one profile; discovery is unaffected.

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
        --only)
            if [ $# -lt 2 ]; then _die '--only requires an argument'; fi
            ONLY_PROFILE="$2"; shift 2 ;;
        --fingerprint) PRINT_FINGERPRINT=1; shift ;;
        *) _die "unknown option: $1" ;;
    esac
done

case "$TTL" in
    ''|*[!0-9]*) _die "--ttl must be a non-negative integer: $TTL" ;;
esac

if [ -n "$ONLY_PROFILE" ] && [ "$PRINT_PROFILES" -eq 0 ]; then
    _die '--only is only meaningful with --profiles'
fi

if ! command -v jq >/dev/null 2>&1; then
    _die 'jq is required (data-integrity tool, fails closed)' 2
fi

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd -P)"

# Sibling resolution: repo layout first, then the installed runtime tree. A bare
# relative path would resolve against the caller's cwd, not the script's.
_resolve_lib() {
    for _cand in "$SCRIPT_DIR/lib/$1" \
                 "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/lib/$1"; do
        if [ -f "$_cand" ]; then printf '%s' "$_cand"; return 0; fi
    done
    return 1
}

# Evidence helpers are sourced FIRST: _model_entry() calls capability_block().
EVIDENCE_LIB="$(_resolve_lib model-capability-evidence.sh || printf '')"
if [ -z "$EVIDENCE_LIB" ]; then
    _die "evidence library not found: lib/model-capability-evidence.sh" 2
fi
# shellcheck source=scripts/lib/model-capability-evidence.sh
. "$EVIDENCE_LIB"

PROBE_LIB="$(_resolve_lib model-supply-probe.sh || printf '')"
if [ -z "$PROBE_LIB" ]; then
    _die "probe library not found: lib/model-supply-probe.sh" 2
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

    # §51: the model's own advertisement is untrusted. Only a calibration
    # verdict measured on THIS fingerprint may lift a role to `calibrated`.
    models_doc="$(apply_calibration_evidence "$models_json" "$fp" "$CALIBRATION_DIR")"

    # The model set is REPLACED, never merged, so ghost entries cannot persist.
    doc="$(jq -n \
        --arg schema "$SCHEMA_VERSION" --arg fp "$fp" --arg ts "$(_utc)" \
        --argjson epoch "$(_now)" --argjson budget "$budget_mb" \
        --argjson cpu_only "$cpu_only" --argjson acc "$acc_json" \
        --argjson runtimes "$(probe_runtimes)" \
        --argjson models "$models_doc" \
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
    # An unknown --only is a usage error, not an empty fragment: silently
    # emitting `profiles: {}` would read as "the probe found nothing" when the
    # truth is "you asked for a profile that does not exist here".
    if [ -n "$ONLY_PROFILE" ]; then
        if [ "$(jq --arg p "$ONLY_PROFILE" \
                '[.local_models[] | select(.profile == $p)] | length' "$OUT_PATH")" -eq 0 ]; then
            _die "--only: no discovered profile named '$ONLY_PROFILE'"
        fi
    fi
    printf '# Generated by discover-model-supply.sh — do not hand-edit.\n'
    printf '# fingerprint: %s\n' "$fp"
    if [ -n "$ONLY_PROFILE" ]; then
        printf '# filtered to one profile via --only %s\n' "$ONLY_PROFILE"
    fi
    if [ "$cpu_only" = "true" ]; then
        printf '# NOTE: no usable accelerator (%s). Local models are listed but\n' \
            "$(jq -r '.accelerator.reason // "unknown"' "$OUT_PATH")"
        printf '#       NOT dispatch-recommended: CPU inference blows the wall-clock ceiling.\n'
    fi
    # A bare `profiles:` followed only by comments parses as null, and a null
    # profiles map breaks the --profile ordinal filter. Emit an explicit empty
    # mapping so the fragment is safe to paste verbatim.
    # $only is passed with --arg, never interpolated: a profile name reaches jq
    # as data and is compared with ==, so it can never be read as an expression.
    if [ "$(jq --arg only "$ONLY_PROFILE" \
            '[.local_models[]
              | select($only == "" or .profile == $only)
              | select(.dispatch_recommended)] | length' "$OUT_PATH")" -eq 0 ]; then
        printf 'profiles: {}\n'
    else
        printf 'profiles:\n'
    fi
    jq -r --arg only "$ONLY_PROFILE" '.local_models[] |
        select($only == "" or .profile == $only) |
        if .dispatch_recommended then
          "  \(.profile):\n    model: \(.model)\n    ctx: \(.ctx)\n    reasoning: \(.reasoning)"
        else
          "  # \(.profile): model=\(.model) ctx=\(.ctx) — not dispatch-recommended"
        end' "$OUT_PATH"
    # A local profile carries no price, so routing-cost.sh refuses it with "no
    # cost keys in profile catalog" until an operator supplies one. Say so in the
    # fragment rather than letting a pasted profile look routable when it is inert.
    printf '%s\n' '# Add cost_minute: <USD-equivalent per GPU-minute> (and optionally' \
        '# max_wall_clock_ms:) to each profile above — routing-cost.sh will not' \
        '# choose a profile that has no cost keys.'
fi

exit 0
