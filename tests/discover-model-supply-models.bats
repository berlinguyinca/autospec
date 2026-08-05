#!/usr/bin/env bats
# tests/discover-model-supply-models.bats — model enumeration, dispatch gating,
# reconciliation, caching, and the auto-init prose contract for
# scripts/discover-model-supply.sh.
#
# CLI surface and accelerator detection live in tests/discover-model-supply.bats.
# Shared stubs live in tests/model-supply-test-helpers.bash.
#
# The probe replaces a prose instruction that, on a live host, fabricated 5 of 9
# local profiles and rated every one of them ctx: 64k. Each test pins one of
# those failure modes.

SCRIPT="${BATS_TEST_DIRNAME}/../scripts/discover-model-supply.sh"

setup() {
    load "${BATS_TEST_DIRNAME}/model-supply-test-helpers.bash"
    model_supply_setup
}

teardown() {
    model_supply_teardown
}

# ── enumeration: measure, filter, never fabricate ─────────────────────────────

@test "only completion-capable models are recorded (embedding model dropped)" {
    stub_nvidia_healthy
    stub_ollama_mixed
    run env PATH="$PROBE_PATH" HOME="$TMP" bash "$SCRIPT" --out "$OUT"
    [ "$status" -eq 0 ]
    [ "$(jq '.local_models | length' "$OUT")" -eq 2 ]
    [ "$(jq -r '[.local_models[].model] | index("nomic-embed-text:latest") // "absent"' "$OUT")" = "absent" ]
}

@test "context length comes from the runtime, not from the model name" {
    stub_nvidia_healthy
    stub_ollama_mixed
    run env PATH="$PROBE_PATH" HOME="$TMP" bash "$SCRIPT" --out "$OUT"
    # qwen3:32b advertises 131072 despite the "32b" tag and an 8.0B self-report.
    [ "$(jq -r '.local_models[] | select(.model=="qwen3:32b") | .context_length_measured' "$OUT")" = "131072" ]
    [ "$(jq -r '.local_models[] | select(.model=="qwen3.5:latest") | .context_length_measured' "$OUT")" = "262144" ]
}

@test "measured context is never flattened to the old 64k guess" {
    stub_nvidia_healthy
    stub_ollama_mixed
    run env PATH="$PROBE_PATH" HOME="$TMP" bash "$SCRIPT" --out "$OUT"
    # The bug being pinned: every local profile used to be rated ctx: 64k.
    ctx_values="$(jq -r '[.local_models[].ctx] | unique | join(",")' "$OUT")"
    [ "$ctx_values" != "64k" ]
}

@test "every model records the original runtime tag as model:" {
    stub_nvidia_healthy
    stub_ollama_mixed
    run env PATH="$PROBE_PATH" HOME="$TMP" bash "$SCRIPT" --out "$OUT"
    # Without model:, select-model-profile.sh --print-model exits 3 and the
    # dispatch override silently no-ops.
    [ "$(jq -r '[.local_models[] | select(.model == null or .model == "")] | length' "$OUT")" -eq 0 ]
    [ "$(jq -r '.local_models[] | select(.profile=="qwen3-32b-laptop") | .model' "$OUT")" = "qwen3:32b" ]
}

@test "profile keys are normalized from the tag" {
    stub_nvidia_healthy
    stub_ollama_mixed
    run env PATH="$PROBE_PATH" HOME="$TMP" bash "$SCRIPT" --out "$OUT"
    [ "$(jq -r '[.local_models[].profile] | sort | join(" ")' "$OUT")" = "qwen3-32b-laptop qwen3-5-latest-laptop" ]
}

@test "a new local model starts at the lowest reasoning tier" {
    stub_nvidia_healthy
    stub_ollama_mixed
    run env PATH="$PROBE_PATH" HOME="$TMP" bash "$SCRIPT" --out "$OUT"
    [ "$(jq -r '[.local_models[].reasoning] | unique | join(",")' "$OUT")" = "shallow" ]
}

@test "ollama daemon down yields zero models rather than a false positive" {
    stub_nvidia_healthy
    stub_ollama_daemon_down
    run env PATH="$PROBE_PATH" HOME="$TMP" bash "$SCRIPT" --out "$OUT"
    [ "$status" -eq 0 ]
    [ "$(jq '.local_models | length' "$OUT")" -eq 0 ]
}

@test "no ollama binary yields zero models and still exits 0" {
    # PATH must genuinely exclude ollama — a stub dir alone still finds the
    # host's real ollama further down PATH.
    minbin="$(minimal_path_without ollama nvidia-smi curl)"
    run env PATH="$minbin" HOME="$TMP" bash "$SCRIPT" --out "$OUT"
    [ "$status" -eq 0 ]
    [ "$(jq '.local_models | length' "$OUT")" -eq 0 ]
    [ "$(jq -r '.accelerator.kind' "$OUT")" = "none" ]
}

# ── dispatch gating (fail closed) ─────────────────────────────────────────────

@test "cpu-only host marks discovered models as not dispatch-recommended" {
    stub_nvidia_nvml_mismatch
    stub_ollama_mixed
    run env PATH="$PROBE_PATH" HOME="$TMP" bash "$SCRIPT" --out "$OUT"
    [ "$(jq -r '[.local_models[].dispatch_recommended] | unique | join(",")' "$OUT")" = "false" ]
}

@test "usable GPU with headroom marks models dispatch-recommended" {
    stub_nvidia_healthy
    stub_ollama_mixed
    run env PATH="$PROBE_PATH" HOME="$TMP" bash "$SCRIPT" --out "$OUT"
    [ "$(jq -r '[.local_models[].dispatch_recommended] | unique | join(",")' "$OUT")" = "true" ]
}

@test "--profiles comments out models that are not dispatch-recommended" {
    stub_nvidia_nvml_mismatch
    stub_ollama_mixed
    run env PATH="$PROBE_PATH" HOME="$TMP" bash "$SCRIPT" --out "$OUT" --profiles
    [ "$status" -eq 0 ]
    [[ "$output" == *"not dispatch-recommended"* ]]
    # No uncommented profile entry may appear for a non-dispatchable model.
    run grep -qE '^  [a-z0-9.-]+-laptop:' <<< "$output"
    [ "$status" -ne 0 ]
}

@test "--profiles emits dispatchable models with model/ctx/reasoning keys" {
    stub_nvidia_healthy
    stub_ollama_mixed
    run env PATH="$PROBE_PATH" HOME="$TMP" bash "$SCRIPT" --out "$OUT" --profiles
    [ "$status" -eq 0 ]
    [[ "$output" == *"qwen3-32b-laptop:"* ]]
    [[ "$output" == *"model: qwen3:32b"* ]]
    [[ "$output" == *"reasoning: shallow"* ]]
}

@test "--profiles emits an empty mapping when nothing is dispatchable" {
    stub_nvidia_nvml_mismatch
    stub_ollama_mixed
    run env PATH="$PROBE_PATH" HOME="$TMP" bash "$SCRIPT" --out "$OUT" --profiles
    [ "$status" -eq 0 ]
    # `profiles:` with only comments under it parses as null, which breaks the
    # --profile ordinal filter. It must be an explicit empty mapping.
    [[ "$output" == *"profiles: {}"* ]]
}

@test "--profiles output is valid YAML in both the empty and populated cases" {
    if ! command -v python3 >/dev/null 2>&1; then skip "python3 unavailable"; fi

    stub_nvidia_nvml_mismatch
    stub_ollama_mixed
    env PATH="$PROBE_PATH" HOME="$TMP" bash "$SCRIPT" --out "$OUT" --profiles > "$TMP/empty.yml"
    run python3 -c "
import yaml
d = yaml.safe_load(open('$TMP/empty.yml'))
assert isinstance(d.get('profiles'), dict), 'profiles must be a mapping, got %r' % (d.get('profiles'),)
"
    [ "$status" -eq 0 ]

    stub_nvidia_healthy
    env PATH="$PROBE_PATH" HOME="$TMP" bash "$SCRIPT" --out "$OUT" --force --profiles > "$TMP/full.yml"
    run python3 -c "
import yaml
p = yaml.safe_load(open('$TMP/full.yml'))['profiles']
assert isinstance(p, dict) and p, 'expected populated mapping'
e = p['qwen3-32b-laptop']
assert e['model'] == 'qwen3:32b', e
assert e['reasoning'] == 'shallow', e
"
    [ "$status" -eq 0 ]
}

# ── reconciliation: ghosts must not survive a re-probe ────────────────────────

@test "a re-probe replaces the model set instead of merging it" {
    stub_nvidia_healthy
    stub_ollama_mixed
    run env PATH="$PROBE_PATH" HOME="$TMP" bash "$SCRIPT" --out "$OUT"
    [ "$(jq '.local_models | length' "$OUT")" -eq 2 ]

    # Inject a ghost of exactly the kind found on the live host, then re-probe.
    jq '.local_models += [{"profile":"qwen3-coder-480b-laptop","model":"qwen3-coder:480b","runtime":"ollama","context_length_measured":0,"ctx":"64k","reasoning":"medium","dispatch_recommended":true}]' \
        "$OUT" > "$OUT.tmp" && mv "$OUT.tmp" "$OUT"
    [ "$(jq '.local_models | length' "$OUT")" -eq 3 ]

    run env PATH="$PROBE_PATH" HOME="$TMP" bash "$SCRIPT" --out "$OUT" --force
    [ "$status" -eq 0 ]
    [ "$(jq '.local_models | length' "$OUT")" -eq 2 ]
    [ "$(jq -r '[.local_models[].model] | index("qwen3-coder:480b") // "gone"' "$OUT")" = "gone" ]
}

# ── caching ───────────────────────────────────────────────────────────────────

@test "an unchanged fingerprint within TTL does not rewrite the document" {
    stub_nvidia_healthy
    stub_ollama_mixed
    run env PATH="$PROBE_PATH" HOME="$TMP" bash "$SCRIPT" --out "$OUT"
    sentinel="$TMP/sentinel"
    cp "$OUT" "$sentinel"
    run env PATH="$PROBE_PATH" HOME="$TMP" bash "$SCRIPT" --out "$OUT"
    [ "$status" -eq 0 ]
    run diff -q "$sentinel" "$OUT"
    [ "$status" -eq 0 ]
}

@test "--force re-probes even on a fingerprint hit" {
    stub_nvidia_healthy
    stub_ollama_mixed
    run env PATH="$PROBE_PATH" HOME="$TMP" bash "$SCRIPT" --out "$OUT"
    [ "$status" -eq 0 ]
    run env PATH="$PROBE_PATH" HOME="$TMP" bash "$SCRIPT" --out "$OUT" --force
    [ "$status" -eq 0 ]
    [ -f "$OUT" ]
}

@test "a TTL of 0 always re-probes" {
    stub_nvidia_healthy
    stub_ollama_mixed
    run env PATH="$PROBE_PATH" HOME="$TMP" bash "$SCRIPT" --out "$OUT"
    run env PATH="$PROBE_PATH" HOME="$TMP" bash "$SCRIPT" --out "$OUT" --ttl 0
    [ "$status" -eq 0 ]
}

@test "changing the model set changes the fingerprint" {
    stub_nvidia_healthy
    stub_ollama_mixed
    fp_two="$(env PATH="$PROBE_PATH" HOME="$TMP" bash "$SCRIPT" --out "$OUT" --fingerprint)"
    stub_ollama_daemon_down
    fp_zero="$(env PATH="$PROBE_PATH" HOME="$TMP" bash "$SCRIPT" --out "$OUT" --fingerprint)"
    [ -n "$fp_two" ]
    [ "$fp_two" != "$fp_zero" ]
    # A fingerprint must be a bare digest — no stray probe output mixed in.
    run grep -qE '^[0-9a-f]{64}$|^nohash$' <<< "$fp_two"
    [ "$status" -eq 0 ]
}

@test "--fingerprint is a read-only query that writes nothing" {
    stub_nvidia_healthy
    stub_ollama_mixed
    a="$(env PATH="$PROBE_PATH" HOME="$TMP" bash "$SCRIPT" --out "$OUT" --fingerprint)"
    b="$(env PATH="$PROBE_PATH" HOME="$TMP" bash "$SCRIPT" --out "$OUT" --fingerprint)"
    [ "$a" = "$b" ]
    run env PATH="$PROBE_PATH" HOME="$TMP" bash "$SCRIPT" --out "$OUT" --fingerprint
    [ "$status" -eq 0 ]
    [ ! -f "$OUT" ]
}

# ── prose contract: auto-init must call the probe, not enumerate models ───────

@test "all three harness surfaces call discover-model-supply.sh for auto-init" {
    for surface in SKILL.md codex/prompt.md opencode/agent.md; do
        f="${BATS_TEST_DIRNAME}/../skills/autospec-run/$surface"
        run test -f "$f"
        [ "$status" -eq 0 ]
        run grep -q "discover-model-supply.sh" "$f"
        [ "$status" -eq 0 ]
    done
}

@test "auto-init no longer instructs the orchestrator to parse ollama list itself" {
    f="${BATS_TEST_DIRNAME}/../skills/autospec-run/SKILL.md"
    # This is the instruction that fabricated 5 of 9 profiles on a live host.
    run grep -n "once and parse returned model rows" "$f"
    [ "$status" -ne 0 ]
}

@test "auto-init no longer hardcodes ctx: 64k as the local default" {
    f="${BATS_TEST_DIRNAME}/../skills/autospec-run/SKILL.md"
    run grep -n "conservative defaults:" "$f"
    [ "$status" -ne 0 ]
}

# ── --only: a paste-time filter, never a probe restriction ────────────────────

@test "--only narrows the fragment to one profile and drops the others" {
    stub_nvidia_healthy
    stub_ollama_mixed
    run env PATH="$PROBE_PATH" HOME="$TMP" bash "$SCRIPT" \
        --out "$OUT" --profiles --only qwen3-32b-laptop
    [ "$status" -eq 0 ]
    [[ "$output" == *"qwen3-32b-laptop:"* ]]
    [[ "$output" != *"qwen3-5-latest-laptop"* ]]
    [[ "$output" == *"--only qwen3-32b-laptop"* ]]
}

@test "--only does not narrow discovery — the document still holds every model" {
    stub_nvidia_healthy
    stub_ollama_mixed
    run env PATH="$PROBE_PATH" HOME="$TMP" bash "$SCRIPT" \
        --out "$OUT" --profiles --only qwen3-32b-laptop
    [ "$status" -eq 0 ]
    # Hiding models from the probe is the blindness bug this tool exists to fix.
    [ "$(jq '.local_models | length' "$OUT")" -eq 2 ]
}

@test "--only naming an undiscovered profile is a usage error, not an empty map" {
    stub_nvidia_healthy
    stub_ollama_mixed
    run env PATH="$PROBE_PATH" HOME="$TMP" bash "$SCRIPT" \
        --out "$OUT" --profiles --only no-such-profile
    [ "$status" -eq 1 ]
    # `profiles: {}` here would read as "the probe found nothing".
    [[ "$output" == *"no discovered profile named 'no-such-profile'"* ]]
    [[ "$output" != *"profiles: {}"* ]]
}

@test "--only a discovered but non-dispatchable profile yields an empty mapping" {
    stub_nvidia_nvml_mismatch
    stub_ollama_mixed
    run env PATH="$PROBE_PATH" HOME="$TMP" bash "$SCRIPT" \
        --out "$OUT" --profiles --only qwen3-32b-laptop
    [ "$status" -eq 0 ]
    # The profile exists, so this is not an error — but a CPU-only host must not
    # emit it as routable.
    [[ "$output" == *"profiles: {}"* ]]
    [[ "$output" == *"not dispatch-recommended"* ]]
}

@test "--only without --profiles is rejected rather than silently ignored" {
    stub_nvidia_healthy
    stub_ollama_mixed
    run env PATH="$PROBE_PATH" HOME="$TMP" bash "$SCRIPT" \
        --out "$OUT" --only qwen3-32b-laptop
    [ "$status" -eq 1 ]
    [[ "$output" == *"--only is only meaningful with --profiles"* ]]
}

@test "--only rejects a profile name carrying regex metacharacters" {
    stub_nvidia_healthy
    stub_ollama_mixed
    # A name reaching jq as an expression rather than as data would match
    # everything here and emit the whole catalog (feedback_jq_test_regex_metachar).
    run env PATH="$PROBE_PATH" HOME="$TMP" bash "$SCRIPT" \
        --out "$OUT" --profiles --only '.*'
    [ "$status" -eq 1 ]
    [[ "$output" != *"qwen3-32b-laptop:"* ]]
}

@test "the emitted fragment says a local profile needs cost keys to route" {
    stub_nvidia_healthy
    stub_ollama_mixed
    run env PATH="$PROBE_PATH" HOME="$TMP" bash "$SCRIPT" --out "$OUT" --profiles
    [ "$status" -eq 0 ]
    # routing-cost.sh refuses a profile with no cost keys, so a pasted fragment
    # that looks routable but is inert would be a silent dead end.
    [[ "$output" == *"cost_minute"* ]]
}
