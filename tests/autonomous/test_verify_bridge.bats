#!/usr/bin/env bats
# tests/autonomous/test_verify_bridge.bats — explore adversarial-verify LLM bridge.
#
# explore's verify stage (scripts/autospec-explore.sh) is fail-closed: an
# autonomous --once run with NO skeptic verdicts files ZERO proposals. In the
# detached conductor no verifier is wired, so explore generates proposals every
# idle cycle but files nothing. scripts/autospec-autonomous-verify-drain.sh is
# that verifier — the AUTOSPEC_EXPLORE_VERIFY_CMD bridge. It reads the deduped
# proposals from $AUTOSPEC_EXPLORE_DEDUPED_IN, runs an omx-backed adversarial
# skeptic, and writes the {norm_title -> {verdict, reason}} map to
# $AUTOSPEC_EXPLORE_VERDICTS_OUT.
#
# Consumer success gate: exit 0 AND a non-empty verdicts file. This bridge
# INVERTS the sibling drains: any failure (omx absent/non-zero, unparseable
# output, stall) is FAIL-CLOSED — exit non-zero, never an all-survived map.
#
# Mocking: PATH-shim omx; no network.

BRIDGE="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)/scripts/autospec-autonomous-verify-drain.sh"
LAUNCHER="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)/scripts/autospec-autonomous.sh"

setup() {
    TMP="$(mktemp -d -t verify-bridge.XXXXXX)"
    mkdir -p "$TMP/bin"
    export PATH="$TMP/bin:$PATH"

    DEDUPED_IN="$TMP/dedup.json"
    VERDICTS_OUT="$TMP/verdicts.json"
    export AUTOSPEC_EXPLORE_DEDUPED_IN="$DEDUPED_IN"
    export AUTOSPEC_EXPLORE_VERDICTS_OUT="$VERDICTS_OUT"

    # Two deduped proposals: one real, one bogus.
    cat > "$DEDUPED_IN" <<'JSON'
{"deduped":[
  {"norm_title":"add retry logic","title":"feat: add retry logic","evidence":"reproduced gap","estimated_complexity":"small","confidence":0.9},
  {"norm_title":"bogus claim","title":"feat: bogus claim","evidence":"none","estimated_complexity":"small","confidence":0.4}
]}
JSON

    # Default omx mock: chatty, emits a valid verdict object in a fenced block.
    OMX_LOG="$TMP/omx-args.log"
    cat > "$TMP/bin/omx" <<EOF
#!/usr/bin/env bash
printf '%s\n' "\$*" >> "$OMX_LOG"
cat <<'OUT'
Analyzing the two proposals as an adversarial skeptic.

\`\`\`json
{"add retry logic":{"verdict":"survived","reason":"evidence reproduces the gap"},
 "bogus claim":{"verdict":"refuted","reason":"no supporting evidence"}}
\`\`\`

Done.
OUT
exit 0
EOF
    chmod +x "$TMP/bin/omx"

    # Deterministic, fast: no stall watchdog (plain wait).
    export AUTOSPEC_AUTONOMOUS_VERIFY_STALL_SECS=0
    export AUTOSPEC_REPO_DIR="$TMP"
}

teardown() {
    rm -rf "$TMP"
}

@test "verify bridge avoids ambiguous any token in audit-sensitive shell text" {
    ! grep -Eq '\bany\b' "$BRIDGE"
}

# ── happy path ────────────────────────────────────────────────────────────────

@test "bridge writes a valid {norm_title:{verdict,reason}} map on a clean skeptic run" {
    run bash "$BRIDGE"
    [ "$status" -eq 0 ] || { echo "$output"; false; }
    [ -s "$VERDICTS_OUT" ]
    python3 -c "
import json
m = json.load(open('$VERDICTS_OUT'))
assert m['add retry logic']['verdict'] == 'survived', m
assert m['bogus claim']['verdict'] == 'refuted', m
assert m['add retry logic']['reason'], m
"
}

@test "bridge forwards the prompt to omx exec with the bypass flag" {
    run bash "$BRIDGE"
    [ "$status" -eq 0 ]
    grep -q 'exec' "$OMX_LOG"
    grep -q 'dangerously-bypass-approvals-and-sandbox' "$OMX_LOG"
}

# ── empty deduped ─────────────────────────────────────────────────────────────

@test "bridge writes {} and exits 0 when there is nothing to verify" {
    printf '{"deduped":[]}\n' > "$DEDUPED_IN"
    run bash "$BRIDGE"
    [ "$status" -eq 0 ] || { echo "$output"; false; }
    [ -s "$VERDICTS_OUT" ]
    python3 -c "import json; assert json.load(open('$VERDICTS_OUT')) == {}, 'not empty map'"
}

# ── normalization ─────────────────────────────────────────────────────────────

@test "bridge drops unknown keys and coerces non-exact verdicts to refuted" {
    cat > "$TMP/bin/omx" <<'EOF'
#!/usr/bin/env bash
cat <<'OUT'
```json
{"add retry logic":{"verdict":"SURVIVED","reason":"wrong case"},
 "hallucinated title":{"verdict":"survived","reason":"not a real proposal"}}
```
OUT
exit 0
EOF
    chmod +x "$TMP/bin/omx"

    run bash "$BRIDGE"
    [ "$status" -eq 0 ] || { echo "$output"; false; }
    python3 -c "
import json
m = json.load(open('$VERDICTS_OUT'))
# Unknown key dropped; wrong-case verdict coerced to refuted (safe direction).
assert 'hallucinated title' not in m, m
assert m['add retry logic']['verdict'] == 'refuted', m
"
}

# ── robust extraction: real codex/omx output shape ────────────────────────────
# The REAL failure (#1707): against live omx the skeptic emits perfect verdicts,
# but they arrive as RAW JSON buried in brace-heavy chatter — NOT a clean fenced
# block. Shell-var braces (${VAR:-default}), hook telemetry lines, the final
# message printed TWICE, a `tokens used` block, and a trailing JSON-ish telemetry
# footer that closes LAST. The old extractor took the FIRST parseable dict (the
# footer, keys ∉ known) → normalized to {} → wrote {} and exited 0 → explore
# refuted-by-default and filed nothing while looking successful.

@test "REAL SHAPE: recovers verdicts from raw JSON buried in brace-heavy codex chatter (no fenced block)" {
    cat > "$TMP/bin/omx" <<'EOF'
#!/usr/bin/env bash
cat <<'OUT'
reasoning effort: high
--------
user
Adversarial skeptic. DEFAULT REFUTED. Judge the proposals.
hook: SessionStart
exec
/bin/bash -lc 'cat /home/u/.autospec/skills/explore/SKILL.md >/dev/null; echo "using ${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts} and {other braces} in here"'
hook: PreToolUse Completed
codex
{"add retry logic":{"verdict":"survived","reason":"evidence reproduces the gap"},"bogus claim":{"verdict":"refuted","reason":"no supporting evidence"}}
hook: Stop
hook: Stop Completed
tokens used
30982
{"add retry logic":{"verdict":"survived","reason":"evidence reproduces the gap"},"bogus claim":{"verdict":"refuted","reason":"no supporting evidence"}}
{"event":"turn_complete","tokens":30982,"model":"codex-high"}
OUT
exit 0
EOF
    chmod +x "$TMP/bin/omx"

    run bash "$BRIDGE"
    [ "$status" -eq 0 ] || { echo "$output"; false; }
    [ -s "$VERDICTS_OUT" ]
    python3 -c "
import json
m = json.load(open('$VERDICTS_OUT'))
# Both known proposals must be recovered with the correct verdicts — not {}.
assert m['add retry logic']['verdict'] == 'survived', m
assert m['bogus claim']['verdict'] == 'refuted', m
# The telemetry footer key must never leak in.
assert 'event' not in m, m
"
}

@test "SENTINELS: recovers verdicts wrapped in BEGIN/END markers amid noise" {
    cat > "$TMP/bin/omx" <<'EOF'
#!/usr/bin/env bash
cat <<'OUT'
reasoning effort: high
hook: SessionStart
exec /bin/bash -lc 'echo "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts} {noise}"'
hook: Stop Completed
Here is my determination:
===AUTOSPEC_VERDICTS_BEGIN===
{"add retry logic":{"verdict":"survived","reason":"clear reproduced evidence"},"bogus claim":{"verdict":"refuted","reason":"restates the title"}}
===AUTOSPEC_VERDICTS_END===
tokens used
41231
{"event":"turn_complete","tokens":41231}
OUT
exit 0
EOF
    chmod +x "$TMP/bin/omx"

    run bash "$BRIDGE"
    [ "$status" -eq 0 ] || { echo "$output"; false; }
    python3 -c "
import json
m = json.load(open('$VERDICTS_OUT'))
assert m['add retry logic']['verdict'] == 'survived', m
assert m['bogus claim']['verdict'] == 'refuted', m
"
}

@test "FAIL-CLOSED (extraction): deduped had proposals but no recoverable verdict overlapping known -> non-zero, no populated map" {
    # Chatter with only UNRELATED JSON objects (no key intersects known). This is
    # an extraction FAILURE, not 'all refuted' — must fail LOUD, not write {}+exit0.
    cat > "$TMP/bin/omx" <<'EOF'
#!/usr/bin/env bash
cat <<'OUT'
reasoning effort: high
hook: SessionStart
exec /bin/bash -lc 'echo "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts} {stuff}"'
hook: Stop Completed
I could not format a verdict.
{"event":"turn_complete","tokens":1200,"model":"codex-high"}
{"unrelated":{"foo":"bar"}}
OUT
exit 0
EOF
    chmod +x "$TMP/bin/omx"

    run bash "$BRIDGE"
    [ "$status" -ne 0 ]
    # Must NOT have written a populated/all-survived map.
    python3 -c "
import json, os
p = '$VERDICTS_OUT'
if os.path.exists(p) and os.path.getsize(p) > 0:
    m = json.load(open(p))
    assert m == {} or all(v.get('verdict') != 'survived' for v in m.values()), ('leaked verdicts on extraction failure', m)
    assert len(m) == 0, ('populated map written on extraction failure', m)
"
}

# ── fail-closed: safety-critical ──────────────────────────────────────────────

@test "FAIL-CLOSED: omx exits non-zero (even with valid-looking output) -> non-zero, no all-survived" {
    # Valid all-survived JSON, but omx failed. Must NOT be trusted.
    cat > "$TMP/bin/omx" <<'EOF'
#!/usr/bin/env bash
cat <<'OUT'
```json
{"add retry logic":{"verdict":"survived","reason":"x"},
 "bogus claim":{"verdict":"survived","reason":"x"}}
```
OUT
exit 3
EOF
    chmod +x "$TMP/bin/omx"

    run bash "$BRIDGE"
    [ "$status" -ne 0 ]
    # Never an all-survived map for every proposal.
    python3 -c "
import json, os
p = '$VERDICTS_OUT'
if os.path.exists(p) and os.path.getsize(p) > 0:
    m = json.load(open(p))
    survived = [k for k,v in m.items() if isinstance(v,dict) and v.get('verdict')=='survived']
    assert len(survived) < 2, ('all-survived leaked on failure', m)
"
}

@test "FAIL-CLOSED: omx exits 0 but emits unparseable garbage -> non-zero, no all-survived" {
    cat > "$TMP/bin/omx" <<'EOF'
#!/usr/bin/env bash
echo "I was unable to reach a determination about these proposals."
exit 0
EOF
    chmod +x "$TMP/bin/omx"

    run bash "$BRIDGE"
    [ "$status" -ne 0 ]
    python3 -c "
import json, os
p = '$VERDICTS_OUT'
if os.path.exists(p) and os.path.getsize(p) > 0:
    m = json.load(open(p))
    survived = [k for k,v in m.items() if isinstance(v,dict) and v.get('verdict')=='survived']
    assert len(survived) < 2, ('all-survived leaked on garbage', m)
"
}

@test "FAIL-CLOSED: omx absent from PATH -> non-zero, no all-survived" {
    # Build an isolated bin with every real tool EXCEPT omx symlinked in, so the
    # bridge genuinely cannot resolve omx (the author's machine ships a real
    # /usr/bin/omx that a naive PATH would still reach — and invoke for tokens).
    NOMX="$TMP/nomx"
    mkdir -p "$NOMX"
    for d in /usr/bin /bin; do
        [ -d "$d" ] || continue
        for f in "$d"/*; do
            [ -f "$f" ] || continue
            ln -sf "$f" "$NOMX/$(basename "$f")"
        done
    done
    rm -f "$NOMX/omx"
    run env PATH="$NOMX" bash "$BRIDGE"
    [ "$status" -ne 0 ]
    python3 -c "
import json, os
p = '$VERDICTS_OUT'
if os.path.exists(p) and os.path.getsize(p) > 0:
    m = json.load(open(p))
    survived = [k for k,v in m.items() if isinstance(v,dict) and v.get('verdict')=='survived']
    assert len(survived) < 2, ('all-survived leaked when omx absent', m)
"
}

@test "bridge runs safely under set -eu (no errexit abort on a failing skeptic)" {
    cat > "$TMP/bin/omx" <<'EOF'
#!/usr/bin/env bash
exit 5
EOF
    chmod +x "$TMP/bin/omx"

    run bash -c "set -eu; bash '$BRIDGE'"
    [ "$status" -ne 0 ]
}

@test "bridge uses a bounded 120 second skeptic stall default" {
    run bash -n "$BRIDGE"
    [ "$status" -eq 0 ]
    grep -q 'AUTOSPEC_AUTONOMOUS_VERIFY_STALL_SECS 120' "$BRIDGE"
    grep -q 'AUTOSPEC_AUTONOMOUS_VERIFY_STALL_SECS:-120' "$BRIDGE"
}

# ── launcher wiring ───────────────────────────────────────────────────────────

@test "launcher exports AUTOSPEC_EXPLORE_VERIFY_CMD to the bridge path by default" {
    grep -q 'AUTOSPEC_EXPLORE_VERIFY_CMD="\${AUTOSPEC_EXPLORE_VERIFY_CMD:-bash \$SCRIPT_DIR/autospec-autonomous-verify-drain.sh}"' \
        "$LAUNCHER"
}

@test "launcher AUTOSPEC_EXPLORE_VERIFY_CMD default is overridable (guarded with :-)" {
    grep -q 'AUTOSPEC_EXPLORE_VERIFY_CMD:-' "$LAUNCHER"
}
