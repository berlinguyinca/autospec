#!/usr/bin/env bats
# Boundary-truth guardrails for production-realistic proof.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    SCRIPT="$REPO_ROOT/scripts/autospec-boundary-guardrails.sh"
    TMP="$(mktemp -d -t boundary-guardrails.XXXXXX)"
    mkdir -p "$TMP/repo/src" "$TMP/repo/migrations" "$TMP/repo/tests"
}

teardown() {
    rm -rf "$TMP"
}

@test "scan detects code allow-list and persistence check drift" {
    cat > "$TMP/repo/src/chat.py" <<'PY'
VALID_DOMAINS = {"chat", "garden"}
PY
    cat > "$TMP/repo/migrations/001_chat.sql" <<'SQL'
ALTER TABLE ai_chat_sessions ADD CONSTRAINT ai_chat_sessions_domain_check CHECK (domain IN ('chat'));
SQL

    run bash "$SCRIPT" scan --repo-root "$TMP/repo"

    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq '[.findings[] | select(.rule_id=="CONTRACT_DRIFT")] | length')" -eq 1 ]
    [[ "$output" == *"garden"* ]]
}

@test "scan detects silent success on error branches" {
    cat > "$TMP/repo/src/sync.py" <<'PY'
def sync_device(client):
    try:
        return client.list_devices()
    except Exception:
        return None
PY

    run bash "$SCRIPT" scan --repo-root "$TMP/repo"

    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq '[.findings[] | select(.rule_id=="SILENT_FAILURE")] | length')" -eq 1 ]
}

@test "scan detects typed fake tests that miss raw decode boundary" {
    cat > "$TMP/repo/src/external_client.py" <<'PY'
import json

def parse_devices(payload):
    return json.loads(payload)
PY
    cat > "$TMP/repo/tests/test_external_client.py" <<'PY'
class FakeExternalClient:
    def list_devices(self):
        return [{"vacation_mode": False}]

def test_fake_client():
    assert FakeExternalClient().list_devices()[0]["vacation_mode"] is False
PY

    run bash "$SCRIPT" scan --repo-root "$TMP/repo"

    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq '[.findings[] | select(.rule_id=="BOUNDARY_TEST_MISSING")] | length')" -eq 1 ]
}

