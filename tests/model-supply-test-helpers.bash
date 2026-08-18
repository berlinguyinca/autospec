#!/usr/bin/env bash
# tests/model-supply-test-helpers.bash — shared fixtures for the
# discover-model-supply.sh suites. Sourced (not run) by:
#   tests/discover-model-supply.bats         — accelerator + CLI surface
#   tests/discover-model-supply-models.bats  — enumeration, gating, cache, prose
#
# Not named *.bats so the bats runner never collects it as a suite.
#
# Every external command the probe consults (nvidia-smi, ollama, curl, sysctl,
# rocm-smi) is stubbed, so assertions are about the probe's logic rather than
# about the machine running the suite.

# Build a PATH containing only the tools the probe needs, minus the ones named.
# Prepending a stub dir is NOT enough to hide a tool: the real binary is still
# found further down PATH. Tests that assert "tool absent" behaviour must
# construct the PATH explicitly or they silently exercise the host's own tools.
minimal_path_without() {
    excluded=" $* "
    minbin="$TMP/minbin"
    rm -rf "$minbin"
    mkdir -p "$minbin"
    for tool in bash jq date awk sed tr head cat mv rm mkdir dirname mktemp uname sha256sum shasum; do
        case "$excluded" in *" $tool "*) continue ;; esac
        resolved="$(PATH="$ORIG_PATH" command -v "$tool" 2>/dev/null || true)"
        if [ -n "$resolved" ]; then
            ln -sf "$resolved" "$minbin/$tool"
        fi
    done
    printf '%s' "$minbin"
}

model_supply_setup() {
    TMP="$(mktemp -d "${BATS_TMPDIR:-/tmp}/discover-model-supply-XXXXXX")"
    STUBS="$TMP/bin"
    OUT="$TMP/model-capability.json"
    mkdir -p "$STUBS"

    # curl always fails => no runtime is reachable unless a test says otherwise.
    cat > "$STUBS/curl" <<'EOF'
#!/usr/bin/env bash
exit 7
EOF

    ORIG_PATH="$PATH"
    chmod +x "$STUBS"/*
    export PATH="$STUBS:$ORIG_PATH"
    # `unset` on an already-unset name succeeds, so no `|| true` guard is needed
    # (and a trailing `|| true` trips the VACUOUS_OR_TRUE lint).
    unset ANTHROPIC_API_KEY
    unset OPENAI_API_KEY

    # The PROBE sees a hermetic PATH: stubs first, then only the tools it needs,
    # with NO accelerator and NO model runtime. The test shell keeps a full PATH
    # so test bodies can still use cp/diff/grep.
    PROBE_PATH="$STUBS:$(minimal_path_without ollama nvidia-smi rocm-smi sysctl curl)"
}

model_supply_teardown() {
    export PATH="${ORIG_PATH:-$PATH}"
    rm -rf "$TMP"
}

# ── stub factories ────────────────────────────────────────────────────────────

# nvidia-smi reproducing the observed broken-driver host: the NVML message goes
# to STDOUT (not stderr) and the exit code is 18.
stub_nvidia_nvml_mismatch() {
    cat > "$STUBS/nvidia-smi" <<'EOF'
#!/usr/bin/env bash
printf 'Failed to initialize NVML: Driver/library version mismatch\n'
printf 'NVML library version: 580.173\n'
exit 18
EOF
    chmod +x "$STUBS/nvidia-smi"
    # Device nodes exist on such a host; the probe must NOT treat them as proof.
    mkdir -p "$TMP/dev" && : > "$TMP/dev/nvidia0"
}

stub_nvidia_healthy() {
    cat > "$STUBS/nvidia-smi" <<'EOF'
#!/usr/bin/env bash
printf 'NVIDIA GeForce RTX 4090, 24564, 23000\n'
exit 0
EOF
    chmod +x "$STUBS/nvidia-smi"
}

# ollama with two completion models and one embedding model. The context lengths
# and the 8.0B self-report on a "32b" tag are the real values observed on the
# development host.
stub_ollama_mixed() {
    cat > "$STUBS/ollama" <<'EOF'
#!/usr/bin/env bash
if [ "${1:-}" = "list" ]; then
    printf 'NAME                       ID              SIZE      MODIFIED\n'
    printf 'qwen3:32b                  c6eb396dbd59    9.6 GB    5 weeks ago\n'
    printf 'nomic-embed-text:latest    0a109f422b47    274 MB    3 weeks ago\n'
    printf 'qwen3.5:latest             6488c96fa5fa    6.6 GB    5 weeks ago\n'
    exit 0
fi
if [ "${1:-}" = "show" ]; then
    case "${2:-}" in
        qwen3:32b)
            printf '  Model\n    parameters          8.0B\n    context length      131072\n    quantization        Q4_K_M\n\n  Capabilities\n    completion\n    vision\n'
            ;;
        qwen3.5:latest)
            printf '  Model\n    parameters          9.7B\n    context length      262144\n    quantization        Q4_K_M\n\n  Capabilities\n    completion\n'
            ;;
        nomic-embed-text:latest)
            printf '  Model\n    parameters          137M\n    context length      2048\n    quantization        F16\n\n  Capabilities\n    embedding\n'
            ;;
        *) exit 1 ;;
    esac
    exit 0
fi
exit 1
EOF
    chmod +x "$STUBS/ollama"
}

stub_ollama_daemon_down() {
    cat > "$STUBS/ollama" <<'EOF'
#!/usr/bin/env bash
printf 'Error: could not connect to ollama app\n' >&2
exit 1
EOF
    chmod +x "$STUBS/ollama"
}

# curl that succeeds for exactly one endpoint substring and fails for the rest.
# Argument-aware on purpose: an argument-blind stub would report every runtime
# reachable and the "unreachable endpoints are omitted" assertion would pass for
# the wrong reason.
stub_curl_reachable_only() {
    cat > "$STUBS/curl" <<EOF
#!/usr/bin/env bash
want='$1'
for a in "\$@"; do
    case "\$a" in
        *"\$want"*) exit 0 ;;
    esac
done
exit 7
EOF
    chmod +x "$STUBS/curl"
}
