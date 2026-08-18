#!/usr/bin/env bash
# tests/executor-dispatch-test-helpers.bash — shared fixtures for the
# scripts/executor-dispatch.sh suites. Sourced (not run) by:
#   tests/executor-dispatch.bats          — the §16 request/result contract
#   tests/executor-dispatch-metrics.bats  — the "unknown, never 0" rule + schema
#
# Not named *.bats so the bats runner never collects it as a suite.
#
# The harness stubs here are argument-aware: they branch on the flags they are
# handed and write the artifacts those flags name. An argument-blind stub would
# absorb a refusal instead of surfacing it — which is exactly how a missing
# `--skip-git-repo-check` stays hidden until it fails on a real host.

# Every key the §16 result envelope carries, plus the schema self-id.
# shellcheck disable=SC2034  # consumed by the sourcing .bats suites
ENVELOPE_KEYS='["cached_tokens","decode_tok_s","failure_class","input_tokens","output","output_tokens","patch","prompt_tok_s","schema","status","tool_calls","ttft_ms","wall_clock_ms"]'

executor_dispatch_setup() {
    TMP="$(mktemp -d "${BATS_TMPDIR:-/tmp}/executor-dispatch-XXXXXX")"
    STUBS="$TMP/bin"
    WS="$TMP/ws"
    mkdir -p "$STUBS" "$WS"
    ORIG_PATH="$PATH"

    # Honours --output-format: `json` emits a Claude-shaped envelope carrying
    # real usage counts; anything else emits plain text with no counts at all.
    cat > "$STUBS/claude" <<EOF
#!/usr/bin/env bash
printf '%s\n' "\$@" > "$TMP/claude-args"
fmt=text
prev=""
for a in "\$@"; do
    if [ "\$prev" = "--output-format" ]; then fmt="\$a"; fi
    prev="\$a"
done
if [ "\$fmt" = "json" ]; then
    printf '{"result":"claude did the work","usage":{"input_tokens":11,"output_tokens":22,"cache_read_input_tokens":33}}\n'
else
    printf 'claude did the work\n'
fi
exit 0
EOF

    # Refuses unless --skip-git-repo-check is present, exactly as the real
    # `codex exec` refuses a workspace that is not a git repository.
    cat > "$STUBS/codex" <<EOF
#!/usr/bin/env bash
printf '%s\n' "\$@" > "$TMP/codex-args"
case " \$* " in
    *" --skip-git-repo-check "*) ;;
    *) printf 'Not inside a trusted directory\n' >&2; exit 1 ;;
esac
artifact=""
prev=""
for a in "\$@"; do
    if [ "\$prev" = "--output-last-message" ]; then artifact="\$a"; fi
    prev="\$a"
done
if [ -n "\$artifact" ]; then printf 'codex did the work\n' > "\$artifact"; fi
exit 0
EOF

    cat > "$STUBS/opencode" <<EOF
#!/usr/bin/env bash
printf '%s\n' "\$@" > "$TMP/opencode-args"
case " \$* " in
    *" run "*) ;;
    *) printf 'opencode: expected the run subcommand\n' >&2; exit 1 ;;
esac
printf 'opencode did the work\n'
exit 0
EOF

    chmod +x "$STUBS/claude" "$STUBS/codex" "$STUBS/opencode"
    export PATH="$STUBS:$ORIG_PATH"

    # A minimal, valid §16 request. Tests override single fields with jq.
    REQ="$TMP/request.json"
    jq -n --arg ws "$WS" '{
        work_item: "issue-3172",
        role: "implementer",
        dispatch_kind: "implement",
        model: "test-model",
        provider: "claude",
        context_budget: 64000,
        tools: ["Read", "Edit"],
        workspace: $ws,
        acceptance_criteria: "the envelope is stable",
        timeout: 30
    }' > "$REQ"
}

executor_dispatch_teardown() {
    export PATH="${ORIG_PATH:-$PATH}"
    rm -rf "$TMP"
}

# Rewrite $REQ with one field replaced. Not a `run` wrapper — it only edits a file.
req_with() {
    jq --arg k "$1" --argjson v "$2" '.[$k] = $v' "$REQ" > "$TMP/req2.json"
    mv "$TMP/req2.json" "$REQ"
}

# A PATH with the listed tools removed (a stub dir alone cannot hide a real one).
path_without() {
    excluded=" $* "
    minbin="$TMP/minbin"
    rm -rf "$minbin"; mkdir -p "$minbin"
    for tool in bash jq git perl tr sleep kill sed awk cat rm mkdir mktemp \
                dirname date claude codex opencode; do
        case "$excluded" in *" $tool "*) continue ;; esac
        resolved="$(PATH="$ORIG_PATH:$STUBS" command -v "$tool" 2>/dev/null || true)"
        if [ -n "$resolved" ]; then ln -sf "$resolved" "$minbin/$tool"; fi
    done
    printf '%s' "$minbin"
}
