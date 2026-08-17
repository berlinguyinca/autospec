#!/usr/bin/env bats
# tests/executor-dispatch-metrics.bats — the telemetry half of the §16 contract.
#
# The failure this file exists to prevent is `decode_tok_s: 0` reported where the
# provider measured nothing. Once such a row reaches the routing ledger it is
# indistinguishable from a measured zero, and it makes the provider look
# infinitely slow — so the rule is "unknown" over a fabricated value, always.
#
# The assertions run in both directions on purpose. Proving that unobserved
# metrics are "unknown" is only half of it: a script that emitted "unknown"
# unconditionally would pass that half while reporting nothing at all, so a
# harness that *does* report counts must still yield real integers.
#
# The request/result contract itself lives in tests/executor-dispatch.bats.

SCRIPT="${BATS_TEST_DIRNAME}/../scripts/executor-dispatch.sh"
SCHEMA="${BATS_TEST_DIRNAME}/../schemas/autospec-dispatch-result.schema.json"

setup() {
    load "${BATS_TEST_DIRNAME}/executor-dispatch-test-helpers.bash"
    executor_dispatch_setup
}

teardown() {
    executor_dispatch_teardown
}

# ── the fabricated-zero attack ────────────────────────────────────────────────

@test "metrics the harness never reported serialize as \"unknown\", never 0" {
    req_with provider '"opencode"'
    # The envelope goes to a real file: a `run jq` inside the loop would
    # overwrite $output, so the second iteration would parse the first's result.
    bash "$SCRIPT" --request "$REQ" > "$TMP/envelope.json"
    for field in input_tokens output_tokens cached_tokens prompt_tok_s \
                 decode_tok_s ttft_ms tool_calls; do
        run jq -r --arg f "$field" '.[$f]' "$TMP/envelope.json"
        [ "$output" = "unknown" ]
    done
}

@test "no unreported metric is ever emitted as a number" {
    req_with provider '"opencode"'
    run bash "$SCRIPT" --request "$REQ"
    [ "$status" -eq 0 ]
    # Any metric that is a number here would be fabricated: the opencode adapter
    # observes nothing but wall clock.
    run jq -c '[.input_tokens, .output_tokens, .cached_tokens, .prompt_tok_s,
                .decode_tok_s, .ttft_ms, .tool_calls]
               | map(select(type == "number"))' <<< "$output"
    [ "$output" = "[]" ]
}

@test "a failure envelope reports unknown metrics, not zeroes" {
    req_with provider '"skynet"'
    run bash "$SCRIPT" --request "$REQ"
    [ "$status" -eq 12 ]
    run jq -c '[.input_tokens, .output_tokens, .cached_tokens, .prompt_tok_s,
                .decode_tok_s, .ttft_ms, .tool_calls]
               | map(select(. == 0))' <<< "$output"
    [ "$output" = "[]" ]
}

@test "a timed-out dispatch reports unknown metrics, not zeroes" {
    cat > "$STUBS/opencode" <<'EOF'
#!/usr/bin/env bash
sleep 30
EOF
    chmod +x "$STUBS/opencode"
    req_with provider '"opencode"'
    req_with timeout '1'
    run bash "$SCRIPT" --request "$REQ"
    [ "$status" -eq 4 ]
    run jq -c '[.input_tokens, .output_tokens, .decode_tok_s, .prompt_tok_s]
               | map(select(type == "number"))' <<< "$output"
    [ "$output" = "[]" ]
}

@test "a harness that does report token counts yields real integers" {
    # The other half of the contract: "unknown" must mean *unobserved*, not
    # "this script never reports anything".
    run bash "$SCRIPT" --request "$REQ"
    [ "$status" -eq 0 ]
    run jq -c '[.input_tokens, .output_tokens, .cached_tokens]' <<< "$output"
    [ "$output" = "[11,22,33]" ]
}

@test "a harness whose JSON is unparseable degrades to unknown, not 0" {
    cat > "$STUBS/claude" <<'EOF'
#!/usr/bin/env bash
printf 'this is not json\n'
exit 0
EOF
    chmod +x "$STUBS/claude"
    run bash "$SCRIPT" --request "$REQ"
    [ "$status" -eq 0 ]
    run jq -r '.input_tokens' <<< "$output"
    [ "$output" = "unknown" ]
}

@test "a harness reporting a usage block with no counts yields unknown" {
    cat > "$STUBS/claude" <<'EOF'
#!/usr/bin/env bash
printf '{"result":"done","usage":{}}\n'
exit 0
EOF
    chmod +x "$STUBS/claude"
    run bash "$SCRIPT" --request "$REQ"
    [ "$status" -eq 0 ]
    run jq -r '.input_tokens, .output_tokens, .cached_tokens' <<< "$output"
    [ "$output" = "unknown
unknown
unknown" ]
}

@test "a harness reporting a non-numeric count yields unknown, not 0" {
    cat > "$STUBS/claude" <<'EOF'
#!/usr/bin/env bash
printf '{"result":"done","usage":{"input_tokens":"lots"}}\n'
exit 0
EOF
    chmod +x "$STUBS/claude"
    run bash "$SCRIPT" --request "$REQ"
    [ "$status" -eq 0 ]
    run jq -r '.input_tokens' <<< "$output"
    [ "$output" = "unknown" ]
}

@test "wall_clock_ms is always measured and never \"unknown\"" {
    req_with provider '"opencode"'
    run bash "$SCRIPT" --request "$REQ"
    [ "$status" -eq 0 ]
    run jq -r '.wall_clock_ms | type' <<< "$output"
    [ "$output" = "number" ]
}

@test "wall_clock_ms is numeric even on the paths that never reach a harness" {
    req_with provider '"skynet"'
    run bash "$SCRIPT" --request "$REQ"
    [ "$status" -eq 12 ]
    run jq -r '.wall_clock_ms | type' <<< "$output"
    [ "$output" = "number" ]
}

# ── schema conformance ────────────────────────────────────────────────────────

@test "the result schema is valid JSON and self-identifies" {
    run jq -r '.["$id"], .properties.schema.const' "$SCHEMA"
    [ "$status" -eq 0 ]
    [[ "$output" == *"https://github.com/berlinguyinca/autospec/schemas/autospec-dispatch-result.schema.json"* ]]
    [[ "$output" == *"autospec.dispatch-result.v1"* ]]
}

@test "the schema allows \"unknown\" for every metric" {
    for field in input_tokens output_tokens cached_tokens prompt_tok_s \
                 decode_tok_s ttft_ms tool_calls; do
        run jq -r --arg f "$field" \
            '[.properties[$f].anyOf[].const] | index("unknown") != null' "$SCHEMA"
        [ "$output" = "true" ]
    done
}

@test "the schema pins wall_clock_ms as a plain integer" {
    # It is the one metric the dispatcher measures itself, so "unknown" would
    # mean the dispatcher failed to read its own clock.
    run jq -r '.properties.wall_clock_ms.type' "$SCHEMA"
    [ "$output" = "integer" ]
}

@test "every harness envelope validates against the result schema" {
    if ! command -v ajv >/dev/null 2>&1; then skip "ajv not installed"; fi
    for provider in claude codex opencode; do
        jq --arg p "$provider" '.provider = $p' "$REQ" > "$TMP/req-p.json"
        bash "$SCRIPT" --request "$TMP/req-p.json" > "$TMP/result-$provider.json"
        run ajv validate --spec=draft2020 -s "$SCHEMA" -d "$TMP/result-$provider.json"
        [ "$status" -eq 0 ]
    done
}

@test "a timeout envelope validates against the result schema" {
    if ! command -v ajv >/dev/null 2>&1; then skip "ajv not installed"; fi
    cat > "$STUBS/opencode" <<'EOF'
#!/usr/bin/env bash
sleep 30
EOF
    chmod +x "$STUBS/opencode"
    req_with provider '"opencode"'
    req_with timeout '1'
    bash "$SCRIPT" --request "$REQ" > "$TMP/timeout.json" || true
    run ajv validate --spec=draft2020 -s "$SCHEMA" -d "$TMP/timeout.json"
    [ "$status" -eq 0 ]
}

@test "an unsupported-provider envelope validates against the result schema" {
    if ! command -v ajv >/dev/null 2>&1; then skip "ajv not installed"; fi
    req_with provider '"skynet"'
    bash "$SCRIPT" --request "$REQ" > "$TMP/unsupported.json" || true
    run ajv validate --spec=draft2020 -s "$SCHEMA" -d "$TMP/unsupported.json"
    [ "$status" -eq 0 ]
}

@test "the schema rejects a fabricated null metric" {
    if ! command -v ajv >/dev/null 2>&1; then skip "ajv not installed"; fi
    bash "$SCRIPT" --request "$REQ" > "$TMP/ok.json"
    jq '.decode_tok_s = null' "$TMP/ok.json" > "$TMP/bad.json"
    run ajv validate --spec=draft2020 -s "$SCHEMA" -d "$TMP/bad.json"
    [ "$status" -ne 0 ]
}

@test "the schema rejects an envelope with a key the contract does not define" {
    if ! command -v ajv >/dev/null 2>&1; then skip "ajv not installed"; fi
    bash "$SCRIPT" --request "$REQ" > "$TMP/ok.json"
    jq '.vibes = "good"' "$TMP/ok.json" > "$TMP/extra.json"
    run ajv validate --spec=draft2020 -s "$SCHEMA" -d "$TMP/extra.json"
    [ "$status" -ne 0 ]
}
