#!/usr/bin/env bash
# scripts/lib/model-supply-probe.sh — probe primitives for discover-model-supply.sh.
#
# Sourced at $SCRIPT_DIR/lib/model-supply-probe.sh by the CLI. Every function here
# is side-effect free apart from setting the ACC_* globals, and each fails closed:
# an unreadable value is reported as absent-with-a-reason, never guessed.
#
# Design rationale and the observed failures each rule prevents:
# docs/specs/2026-08-05-self-discovering-model-routing-design.md (R2, R3).
#
# bash 3.2+ compatible. No associative arrays, no mapfile, no RETURN traps.

# ── generic helpers ───────────────────────────────────────────────────────────

# Deterministic sha256 of stdin across GNU/BSD. A host with neither hasher gets a
# stable sentinel so caching degrades to TTL-only instead of failing the probe.
_sha256() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 | awk '{print $1}'
    else
        printf 'nohash\n'
    fi
}

_now() { date -u +%s; }
_utc() { date -u +%Y-%m-%dT%H:%M:%SZ; }

# Collapse whitespace so a value cannot break a JSON one-liner.
_clean() { printf '%s' "$1" | tr -d '\r' | tr '\n' ' ' | sed 's/  */ /g; s/^ //; s/ $//'; }

# ── accelerator ───────────────────────────────────────────────────────────────
# Sets ACC_KIND ACC_NAME ACC_VRAM_TOTAL ACC_VRAM_FREE ACC_USABLE ACC_REASON.
#
# FAIL CLOSED. A present `nvidia-smi` and a present /dev/nvidia0 do NOT mean a
# usable GPU: the development host has both while nvidia-smi exits 18 with
# "Failed to initialize NVML: Driver/library version mismatch". Only a
# successful, parseable capability query counts. Device nodes are deliberately
# never consulted — treating them as evidence is the bug this avoids.

_probe_nvidia() {
    ACC_KIND="nvidia"
    # nvidia-smi reports NVML init failure on STDOUT (verified: exit 18, stderr
    # empty), so both streams are inspected to name the reason. Getting this
    # wrong degrades a precise, fixable reason into a generic one.
    _errfile="$(mktemp "${TMPDIR:-/tmp}/autospec-nvsmi-XXXXXX")"
    _out=""
    if _out="$(nvidia-smi --query-gpu=name,memory.total,memory.free \
                 --format=csv,noheader,nounits 2>"$_errfile")"; then
        _first="$(printf '%s\n' "$_out" | head -1)"
        _name="$(printf '%s' "$_first" | awk -F',' '{print $1}' | sed 's/^ *//; s/ *$//')"
        _total="$(printf '%s' "$_first" | awk -F',' '{print $2}' | tr -dc '0-9')"
        _free="$(printf '%s' "$_first" | awk -F',' '{print $3}' | tr -dc '0-9')"
        if [ -n "$_name" ] && [ -n "$_total" ] && [ "$_total" -gt 0 ] 2>/dev/null; then
            ACC_NAME="$_name"
            ACC_VRAM_TOTAL="$_total"
            ACC_VRAM_FREE="${_free:-0}"
            ACC_USABLE="true"
            ACC_REASON=""
        else
            ACC_REASON="nvidia_smi_output_unparseable"
        fi
    else
        _err="$(head -2 "$_errfile" 2>/dev/null || true)"
        case "$_out$_err" in
            *"Driver/library version mismatch"*) ACC_REASON="nvml_driver_library_mismatch" ;;
            *NVML*|*nvml*)                       ACC_REASON="nvml_init_failed" ;;
            *)                                   ACC_REASON="nvidia_smi_query_failed" ;;
        esac
    fi
    rm -f "$_errfile"
}

_probe_apple() {
    _memsize="$(sysctl -n hw.memsize 2>/dev/null | tr -dc '0-9')"
    if [ -z "$_memsize" ] || [ "$_memsize" -le 0 ] 2>/dev/null; then
        return 1
    fi
    ACC_KIND="apple"
    ACC_NAME="$(sysctl -n machdep.cpu.brand_string 2>/dev/null || printf 'apple-silicon')"
    ACC_VRAM_TOTAL=$((_memsize / 1024 / 1024))
    # Unified memory: a precise free figure needs vm_stat maths that varies by OS
    # version, and the conservative ctx heuristic does not depend on precision.
    ACC_VRAM_FREE="$ACC_VRAM_TOTAL"
    ACC_USABLE="true"
    ACC_REASON=""
    return 0
}

_probe_rocm() {
    ACC_KIND="rocm"
    if rocm-smi --showmeminfo vram >/dev/null 2>&1; then
        ACC_NAME="rocm"
        ACC_USABLE="true"
        ACC_REASON=""
    else
        ACC_REASON="rocm_smi_query_failed"
    fi
}

probe_accelerator() {
    ACC_KIND="none"; ACC_NAME=""; ACC_VRAM_TOTAL=0; ACC_VRAM_FREE=0
    ACC_USABLE="false"; ACC_REASON="no_accelerator_detected"

    if command -v nvidia-smi >/dev/null 2>&1; then
        _probe_nvidia
        return 0
    fi
    if [ "$(uname -s 2>/dev/null || printf '')" = "Darwin" ]; then
        if _probe_apple; then return 0; fi
    fi
    if command -v rocm-smi >/dev/null 2>&1; then
        _probe_rocm
        return 0
    fi
    return 0
}

# CPU-inference budget when no accelerator is usable.
system_ram_mb() {
    if [ -r /proc/meminfo ]; then
        awk '/^MemAvailable:/ {printf "%d", $2/1024; f=1} END{if(!f) print 0}' /proc/meminfo
        return 0
    fi
    if [ "$(uname -s 2>/dev/null || printf '')" = "Darwin" ]; then
        _m="$(sysctl -n hw.memsize 2>/dev/null | tr -dc '0-9')"
        if [ -n "$_m" ]; then printf '%d' $((_m / 1024 / 1024)); return 0; fi
    fi
    printf '0'
}

# ── local runtime endpoints ───────────────────────────────────────────────────
# Fixed order for deterministic output. Every probe is time-bounded so a hung
# server cannot wedge bootstrap.

probe_runtimes() {
    if ! command -v curl >/dev/null 2>&1; then
        printf '[]'
        return 0
    fi
    _out=""
    for _spec in \
        "ollama|http://${OLLAMA_HOST}/api/tags" \
        "lmstudio|http://127.0.0.1:1234/v1/models" \
        "vllm|http://127.0.0.1:8000/v1/models" \
        "llamacpp|http://127.0.0.1:8080/v1/models"
    do
        _rt="${_spec%%|*}"
        _url="${_spec#*|}"
        _ok="false"
        if curl -sf --max-time "$CURL_TIMEOUT" "$_url" >/dev/null 2>&1; then _ok="true"; fi
        _e="$(jq -nc --arg n "$_rt" --arg u "$_url" --argjson r "$_ok" \
                '{name:$n, endpoint:$u, reachable:$r}')"
        if [ -z "$_out" ]; then _out="$_e"; else _out="$_out,$_e"; fi
    done
    printf '[%s]' "$_out"
}

# ── context ordinal ───────────────────────────────────────────────────────────
# Snap a MEASURED context length down to autospec's 32k|64k|120k ordinals, then
# cap by plausible memory headroom.
#
# Deliberately coarse: per-token KV cost varies by architecture, quantization and
# attention scheme, so a precise formula would be false precision. The rule is
# only "never advertise more context than the host can plausibly hold."

ctx_ordinal() {
    _adv="$1"       # measured tokens
    _headroom="$2"  # budget_mb - weights_mb

    _a=32
    if [ "$_adv" -ge 120000 ] 2>/dev/null; then _a=120
    elif [ "$_adv" -ge 64000 ] 2>/dev/null; then _a=64
    fi

    _m=32
    if [ "$_headroom" -ge 8192 ] 2>/dev/null; then _m=120
    elif [ "$_headroom" -ge 2048 ] 2>/dev/null; then _m=64
    fi

    if [ "$_a" -le "$_m" ]; then printf '%dk' "$_a"; else printf '%dk' "$_m"; fi
}

# ── ollama models ─────────────────────────────────────────────────────────────
# MEASURE, NEVER PARSE THE TAG. `qwen3:32b` self-reports 8.0B parameters, and
# `qwen3:32b` + `gemma4:latest` can share one blob (same weights, two tags).
# Only completion-capable models are kept, which drops embedding models without
# a name blocklist.

# _has_capability <show-output> <name> — exit 0 when the Capabilities block
# lists <name>. The block is the runtime's own mechanical report, so a hit is
# §8 `discovered`; a miss is a discovered ABSENCE, not an unprobed field.
_has_capability() {
    printf '%s\n' "$1" | awk -v want="$2" '
        /^[[:space:]]*Capabilities[[:space:]]*$/ {incap=1; next}
        incap && /^[[:alpha:]]/ {incap=0}
        incap && index($0, want) {found=1}
        END {exit !found}
    '
}

# _has_completion <show-output> — exit 0 when the Capabilities block lists completion.
_has_completion() { _has_capability "$1" "completion"; }

# _weights_mb <list-output> <tag> — on-disk footprint from the SIZE + unit column.
_weights_mb() {
    printf '%s\n' "$1" | awk -v t="$2" '
        $1==t {
            v=$3; u=$4
            if (u ~ /^[Gg]/)      printf "%d", v*1024
            else if (u ~ /^[Mm]/) printf "%d", v
            else                  printf "0"
            exit
        }'
}

# _model_entry <tag> <show-out> <list-out> <budget_mb> <cpu_only> — one JSON object.
_model_entry() {
    _tag="$1"; _show="$2"; _list="$3"; _budget="$4"; _cpu="$5"

    _ctx_tokens="$(printf '%s\n' "$_show" \
        | awk '/context length/ {gsub(/[^0-9]/,"",$NF); print $NF; exit}')"
    if [ -z "$_ctx_tokens" ]; then _ctx_tokens=0; fi
    _params="$(printf '%s\n' "$_show" | awk '/^[[:space:]]*parameters[[:space:]]/ {print $2; exit}')"
    _quant="$(printf '%s\n' "$_show" | awk '/^[[:space:]]*quantization[[:space:]]/ {print $2; exit}')"

    _size="$(_weights_mb "$_list" "$_tag")"
    if [ -z "$_size" ]; then _size=0; fi
    _head=$((_budget - _size))
    if [ "$_head" -lt 0 ]; then _head=0; fi

    # Fail closed: without a usable accelerator, or when weights exceed the
    # budget, the model is found but not worth dispatching to.
    _ok="true"
    if [ "$_cpu" = "true" ]; then _ok="false"; fi
    if [ "$_size" -gt "$_budget" ] 2>/dev/null; then _ok="false"; fi

    # Normalized profile key; `model:` keeps the original dispatchable tag.
    _key="$(printf '%s' "$_tag" | tr '[:upper:]' '[:lower:]' | sed 's/[:\/. 	]/-/g')-laptop"

    # §8: the Capabilities block is a mechanical report, so vision and tool
    # calling are `discovered` either way. Everything the runtime does not report
    # stays "unknown"/`advertised` inside capability_block.
    _vision="false"
    if _has_capability "$_show" "vision"; then _vision="true"; fi
    _tools="false"
    if _has_capability "$_show" "tools"; then _tools="true"; fi

    _params_c="$(_clean "${_params:-unknown}")"
    _quant_c="$(_clean "${_quant:-unknown}")"

    jq -nc \
        --arg tag "$_tag" --arg profile "$_key" \
        --arg params "$_params_c" \
        --arg quant "$_quant_c" \
        --arg ctx "$(ctx_ordinal "$_ctx_tokens" "$_head")" \
        --argjson ctx_tokens "${_ctx_tokens:-0}" \
        --argjson size_mb "${_size:-0}" \
        --argjson dispatchable "$_ok" \
        --argjson cap "$(capability_block "ollama" "${_ctx_tokens:-0}" \
            "$_params_c" "$_quant_c" "${_size:-0}" "$_vision" "$_tools")" \
        '{profile:$profile, model:$tag, runtime:"ollama",
          context_length_measured:$ctx_tokens, ctx:$ctx,
          reasoning:"shallow",
          parameters_reported:$params, quantization:$quant,
          weights_mb:$size_mb, dispatch_recommended:$dispatchable} + $cap'
}

probe_ollama_models() {
    _budget="$1"; _cpu="$2"

    if ! command -v ollama >/dev/null 2>&1; then printf '[]'; return 0; fi
    _list=""
    if ! _list="$(ollama list 2>/dev/null)"; then printf '[]'; return 0; fi

    _acc=""
    for _tag in $(printf '%s\n' "$_list" | awk 'NR>1 && NF>0 {print $1}'); do
        _show=""
        if ! _show="$(ollama show "$_tag" 2>/dev/null)"; then continue; fi
        if ! _has_completion "$_show"; then continue; fi
        _e="$(_model_entry "$_tag" "$_show" "$_list" "$_budget" "$_cpu")"
        if [ -z "$_acc" ]; then _acc="$_e"; else _acc="$_acc,$_e"; fi
    done
    printf '[%s]' "$_acc"
}

# ── cloud + fingerprint ───────────────────────────────────────────────────────

# Presence only — key material is never read into the document.
probe_cloud() {
    _a="false"; _o="false"
    if [ -n "${ANTHROPIC_API_KEY:-}" ]; then _a="true"; fi
    if [ -n "${OPENAI_API_KEY:-}" ]; then _o="true"; fi
    jq -nc --argjson a "$_a" --argjson o "$_o" \
        '{anthropic_api_key_present:$a, openai_api_key_present:$o}'
}

# A change in accelerator state or in the model set invalidates the cache.
fingerprint() {
    _sig="$(printf '%s' "$1" | jq -r '[.[].model] | sort | join(",")')"
    printf '%s|%s|%s|%s|%s' \
        "$ACC_KIND" "$ACC_NAME" "$ACC_VRAM_TOTAL" "$ACC_USABLE" "$_sig" | _sha256
}
