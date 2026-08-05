#!/usr/bin/env bats
# tests/discover-model-supply.bats — CLI surface + accelerator detection for
# scripts/discover-model-supply.sh.
#
# Model enumeration, dispatch gating, reconciliation, caching and the prose
# contract live in tests/discover-model-supply-models.bats. Shared stubs and the
# hermetic PATH construction live in tests/model-supply-test-helpers.bash.
#
# The probe replaces a prose instruction that, on a live host, reported a working
# GPU that was not usable. Every accelerator test here pins that failure mode.

SCRIPT="${BATS_TEST_DIRNAME}/../scripts/discover-model-supply.sh"

setup() {
    load "${BATS_TEST_DIRNAME}/model-supply-test-helpers.bash"
    model_supply_setup
}

teardown() {
    model_supply_teardown
}

# ── CLI surface ───────────────────────────────────────────────────────────────

@test "discover-model-supply.sh is executable" {
    run test -x "$SCRIPT"
    [ "$status" -eq 0 ]
}

@test "the probe library ships alongside the CLI" {
    run test -f "${BATS_TEST_DIRNAME}/../scripts/lib/model-supply-probe.sh"
    [ "$status" -eq 0 ]
}

@test "--help exits 0 and does not write the output file" {
    run env PATH="$PROBE_PATH" HOME="$TMP" bash "$SCRIPT" --help
    [ "$status" -eq 0 ]
    [ ! -f "$OUT" ]
}

@test "rejects an unknown option" {
    run env PATH="$PROBE_PATH" HOME="$TMP" bash "$SCRIPT" --wat
    [ "$status" -eq 1 ]
}

@test "rejects a non-numeric --ttl" {
    run env PATH="$PROBE_PATH" HOME="$TMP" bash "$SCRIPT" --ttl abc --out "$OUT"
    [ "$status" -eq 1 ]
}

@test "fails closed with exit 2 when jq is absent" {
    # Absolute bash path: `env PATH=... bash` would otherwise fail with 127
    # (bash not found) and the test would pass for the wrong reason.
    minbin="$(minimal_path_without jq)"
    run env PATH="$minbin" /usr/bin/env bash "$SCRIPT" --out "$OUT"
    [ "$status" -eq 2 ]
    [[ "$output" == *"jq"* ]]
    [ ! -f "$OUT" ]
}

@test "the document carries a schema version" {
    run env PATH="$PROBE_PATH" HOME="$TMP" bash "$SCRIPT" --out "$OUT"
    [ "$(jq -r '.schema' "$OUT")" = "autospec.model-capability.v1" ]
}

@test "--json prints the document to stdout" {
    run env PATH="$PROBE_PATH" HOME="$TMP" bash "$SCRIPT" --out "$OUT" --json
    [ "$status" -eq 0 ]
    printf '%s' "$output" | jq -e '.schema' >/dev/null
}

# ── accelerator: fail closed (the observed host bug) ──────────────────────────

@test "NVML driver mismatch reports usable=false with a precise reason" {
    stub_nvidia_nvml_mismatch
    run env PATH="$PROBE_PATH" HOME="$TMP" bash "$SCRIPT" --out "$OUT"
    [ "$status" -eq 0 ]
    [ "$(jq -r '.accelerator.usable' "$OUT")" = "false" ]
    [ "$(jq -r '.accelerator.reason' "$OUT")" = "nvml_driver_library_mismatch" ]
}

@test "NVML mismatch does not report VRAM it could not read" {
    stub_nvidia_nvml_mismatch
    run env PATH="$PROBE_PATH" HOME="$TMP" bash "$SCRIPT" --out "$OUT"
    [ "$(jq -r '.accelerator.vram_total_mb' "$OUT")" = "0" ]
    [ "$(jq -r '.accelerator.vram_free_mb' "$OUT")" = "0" ]
}

@test "a broken driver forces cpu_only even though nvidia-smi exists" {
    stub_nvidia_nvml_mismatch
    run env PATH="$PROBE_PATH" HOME="$TMP" bash "$SCRIPT" --out "$OUT"
    [ "$(jq -r '.cpu_only' "$OUT")" = "true" ]
}

@test "a healthy GPU reports usable=true with measured VRAM and no reason" {
    stub_nvidia_healthy
    run env PATH="$PROBE_PATH" HOME="$TMP" bash "$SCRIPT" --out "$OUT"
    [ "$status" -eq 0 ]
    [ "$(jq -r '.accelerator.usable' "$OUT")" = "true" ]
    [ "$(jq -r '.accelerator.vram_total_mb' "$OUT")" = "24564" ]
    [ "$(jq -r '.accelerator.reason' "$OUT")" = "" ]
    [ "$(jq -r '.cpu_only' "$OUT")" = "false" ]
}

@test "no accelerator at all is recorded as a reason, not an error" {
    run env PATH="$PROBE_PATH" HOME="$TMP" bash "$SCRIPT" --out "$OUT"
    [ "$status" -eq 0 ]
    [ "$(jq -r '.accelerator.kind' "$OUT")" = "none" ]
    [ "$(jq -r '.accelerator.usable' "$OUT")" = "false" ]
    [ "$(jq -r '.accelerator.reason' "$OUT")" = "no_accelerator_detected" ]
}

@test "a usable accelerator sets the memory budget from free VRAM" {
    stub_nvidia_healthy
    run env PATH="$PROBE_PATH" HOME="$TMP" bash "$SCRIPT" --out "$OUT"
    [ "$(jq -r '.memory_budget_mb' "$OUT")" = "23000" ]
}

# ── runtimes + cloud ──────────────────────────────────────────────────────────

@test "all four local runtimes are probed in a deterministic order" {
    run env PATH="$PROBE_PATH" HOME="$TMP" bash "$SCRIPT" --out "$OUT"
    [ "$(jq -r '[.runtimes[].name] | join(",")' "$OUT")" = "ollama,lmstudio,vllm,llamacpp" ]
}

@test "unreachable runtimes are recorded as unreachable, not omitted" {
    run env PATH="$PROBE_PATH" HOME="$TMP" bash "$SCRIPT" --out "$OUT"
    [ "$(jq -r '[.runtimes[].reachable] | unique | join(",")' "$OUT")" = "false" ]
}

@test "cloud key presence is reported as a boolean and never echoed" {
    run env PATH="$PROBE_PATH" HOME="$TMP" ANTHROPIC_API_KEY="sk-ant-secret-value" \
        bash "$SCRIPT" --out "$OUT"
    [ "$status" -eq 0 ]
    [ "$(jq -r '.cloud.anthropic_api_key_present' "$OUT")" = "true" ]
    run grep -q "sk-ant-secret-value" "$OUT"
    [ "$status" -ne 0 ]
}

@test "absent cloud keys report false" {
    run env PATH="$PROBE_PATH" HOME="$TMP" bash "$SCRIPT" --out "$OUT"
    [ "$(jq -r '.cloud.anthropic_api_key_present' "$OUT")" = "false" ]
    [ "$(jq -r '.cloud.openai_api_key_present' "$OUT")" = "false" ]
}
