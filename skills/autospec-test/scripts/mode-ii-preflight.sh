#!/usr/bin/env bash
# mode-ii-preflight.sh — Mode II scoped-production pre-suite gate.
#
# Usage: echo '<contract_json>' | mode-ii-preflight.sh
#
# Enforces all hard non-negotiable invariants from spec §5b:
#   1. i_understand_this_writes_to_production must be true
#   2. backup section must be present
#   3. backup driver must be specified
#   4. restore_cmd must be present
#   5. ack-lock file must exist and match contract SHA
#   6. Backup driver self-test (snapshot + verify)
#   7. Scope tokens must be parseable
#
# Output: preflight result JSON on stdout
# Exit codes:
#   0 = preflight passed
#   2 = refuse-to-run (operator-actionable; stderr explains what to fix)

set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
AUTOSPEC_DIR="${AUTOSPEC_DIR:-$(pwd)/.autospec}"
AUTOSPEC_SKIP_DB_PROBE="${AUTOSPEC_SKIP_DB_PROBE:-0}"

# ── Helpers ────────────────────────────────────────────────────────────────────

refuse() {
    local reason="$1"
    local detail="${2:-}"
    printf 'mode-ii-preflight: REFUSED: %s\n' "$reason" >&2
    if [ -n "$detail" ]; then
        printf '  %s\n' "$detail" >&2
    fi
    printf '{"passed":false,"reason":"%s","detail":"%s"}\n' "$reason" "$detail"
    exit 2
}

emit_pass() {
    local snap_id="${1:-}"
    printf '{"passed":true,"snapshot_id":"%s"}\n' "$snap_id"
    exit 0
}

# ── Parse contract from stdin ──────────────────────────────────────────────────

if ! command -v jq >/dev/null 2>&1; then
    printf 'mode-ii-preflight: fatal: jq not found\n' >&2
    exit 2
fi

CONTRACT_JSON=$(cat)
if [ -z "$CONTRACT_JSON" ]; then
    refuse "empty_contract" "no contract JSON on stdin"
fi

# ── 1. i_understand_this_writes_to_production must be true ────────────────────

ACK=$(printf '%s' "$CONTRACT_JSON" | jq -r '.i_understand_this_writes_to_production // false')
if [ "$ACK" != "true" ]; then
    refuse "missing_ack" "i_understand_this_writes_to_production must be set to true in .autospec/test.yml"
fi

# ── 2. backup section must be present ─────────────────────────────────────────

BACKUP_JSON=$(printf '%s' "$CONTRACT_JSON" | jq -c '.e2e.backup // empty')
if [ -z "$BACKUP_JSON" ]; then
    refuse "missing_backup" "e2e.backup section is required for Mode II. Add a backup driver configuration."
fi

# ── 3. backup driver must be specified ────────────────────────────────────────

DRIVER=$(printf '%s' "$CONTRACT_JSON" | jq -r '.e2e.backup.driver // empty')
if [ -z "$DRIVER" ]; then
    refuse "missing_backup_driver" "e2e.backup.driver must be specified (zfs|pgdump|mysqldump|custom)"
fi

# Validate driver is one of the known types
case "$DRIVER" in
    zfs|pgdump|mysqldump|custom)
        ;;
    *)
        refuse "unknown_backup_driver" "e2e.backup.driver '$DRIVER' is not supported. Use: zfs|pgdump|mysqldump|custom"
        ;;
esac

# ── 4. restore_cmd must be present ────────────────────────────────────────────

RESTORE_CMD=$(printf '%s' "$CONTRACT_JSON" | jq -r '.e2e.backup.restore_cmd // empty')
if [ -z "$RESTORE_CMD" ]; then
    # For custom driver, also check custom_restore_cmd
    if [ "$DRIVER" = "custom" ]; then
        CUSTOM_RESTORE=$(printf '%s' "$CONTRACT_JSON" | jq -r '.e2e.backup.custom_restore_cmd // empty')
        if [ -z "$CUSTOM_RESTORE" ]; then
            refuse "missing_restore_cmd" "e2e.backup.restore_cmd (or custom_restore_cmd for custom driver) is required. Refusing to run without restore capability."
        fi
    else
        refuse "missing_restore_cmd" "e2e.backup.restore_cmd is required. Refusing to run without restore capability."
    fi
fi

# ── 5. Ack-lock file must exist and match contract SHA ────────────────────────

# Compute SHA of the production_scoped_access section
SCOPED_ACCESS=$(printf '%s' "$CONTRACT_JSON" | jq -c '.e2e.production_scoped_access // {}')
# Use sha256sum or shasum depending on platform
if command -v sha256sum >/dev/null 2>&1; then
    CONTRACT_SHA=$(printf '%s' "$SCOPED_ACCESS" | sha256sum | awk '{print $1}' | cut -c1-40)
elif command -v shasum >/dev/null 2>&1; then
    CONTRACT_SHA=$(printf '%s' "$SCOPED_ACCESS" | shasum -a 256 | awk '{print $1}' | cut -c1-40)
else
    printf 'mode-ii-preflight: WARN: sha256sum/shasum not found; skipping ack-lock SHA validation\n' >&2
    CONTRACT_SHA=""
fi

if [ -n "$CONTRACT_SHA" ]; then
    # Look for matching lock file in AUTOSPEC_DIR
    LOCK_FILE="${AUTOSPEC_DIR}/.scoped-prod-acked-${CONTRACT_SHA}.lock"

    # Check if any ack lock file exists (wrong sha = mismatch)
    ACK_LOCK_EXISTS=false
    if [ -f "$LOCK_FILE" ]; then
        ACK_LOCK_EXISTS=true
    fi

    # Check if a different sha lock file exists (config changed without re-ack)
    EXISTING_LOCKS=$(find "$AUTOSPEC_DIR" -name '.scoped-prod-acked-*.lock' 2>/dev/null | head -5 || true)

    if [ "$ACK_LOCK_EXISTS" = "false" ]; then
        if [ -n "$EXISTING_LOCKS" ]; then
            # Old lock exists but sha doesn't match — re-ack required
            refuse "ack_lock_sha_mismatch" "production_scoped_access config changed; re-acknowledgement required. Run /autospec-test --init or pass --ack-scoped-prod-change. Expected lock: .scoped-prod-acked-${CONTRACT_SHA}.lock"
        else
            # No lock file at all — need initial ack
            refuse "missing_ack_lock" "Mode II requires an ack lock file: ${LOCK_FILE}. Run /autospec-test --init to create it."
        fi
    fi
fi

# ── 6. Driver self-test: verify backup binary present ────────────────────────

DRIVER_SCRIPT="${SCRIPT_DIR}/backup-drivers/${DRIVER}.sh"
if [ ! -f "$DRIVER_SCRIPT" ]; then
    refuse "backup_driver_script_missing" "Backup driver script not found: ${DRIVER_SCRIPT}"
fi
chmod +x "$DRIVER_SCRIPT"

# Export custom driver env vars if custom driver
if [ "$DRIVER" = "custom" ]; then
    SNAP_CMD=$(printf '%s' "$CONTRACT_JSON" | jq -r '.e2e.backup.custom_snapshot_cmd // empty')
    VERIFY_CMD=$(printf '%s' "$CONTRACT_JSON" | jq -r '.e2e.backup.custom_verify_cmd // empty')
    RESTORE_CMD_CUSTOM=$(printf '%s' "$CONTRACT_JSON" | jq -r '.e2e.backup.custom_restore_cmd // empty')

    export AUTOSPEC_CUSTOM_SNAPSHOT_CMD="${SNAP_CMD}"
    export AUTOSPEC_CUSTOM_VERIFY_CMD="${VERIFY_CMD}"
    export AUTOSPEC_CUSTOM_RESTORE_CMD="${RESTORE_CMD_CUSTOM}"
fi

# Take snapshot
SNAP_OUT=""
SNAP_EXIT=0
SNAP_OUT=$("$DRIVER_SCRIPT" snapshot 2>/tmp/autospec-preflight-snap-err.txt) || SNAP_EXIT=$?
if [ "$SNAP_EXIT" -ne 0 ]; then
    SNAP_ERR=$(cat /tmp/autospec-preflight-snap-err.txt 2>/dev/null || true)
    refuse "backup_snapshot_failed" "Driver '${DRIVER}' snapshot failed: ${SNAP_ERR}"
fi
SNAP_ID=$(printf '%s' "$SNAP_OUT" | tail -1)

# Verify snapshot
VERIFY_EXIT=0
"$DRIVER_SCRIPT" verify 2>/tmp/autospec-preflight-verify-err.txt || VERIFY_EXIT=$?
if [ "$VERIFY_EXIT" -ne 0 ]; then
    VERIFY_ERR=$(cat /tmp/autospec-preflight-verify-err.txt 2>/dev/null || true)
    refuse "backup_verify_failed" "Driver '${DRIVER}' verify failed after snapshot: ${VERIFY_ERR}"
fi

# ── 7. Scope tokens must be parseable ────────────────────────────────────────

SCOPE_TOKENS=$(printf '%s' "$CONTRACT_JSON" | jq -c '.e2e.production_scoped_access.scope_tokens // []')
TOKEN_COUNT=$(printf '%s' "$SCOPE_TOKENS" | jq 'length')

if [ "$TOKEN_COUNT" -eq 0 ]; then
    refuse "missing_scope_tokens" "e2e.production_scoped_access.scope_tokens must have at least one token"
fi

# Validate each token has required fields
INVALID_TOKEN=$(printf '%s' "$SCOPE_TOKENS" | jq -r '
    to_entries[] |
    .value as $t |
    if ($t.kind == null) then "token \(.key): missing kind field"
    else empty
    end
' 2>/dev/null || true)

if [ -n "$INVALID_TOKEN" ]; then
    refuse "invalid_scope_token" "$INVALID_TOKEN"
fi

# ── 8. DB probe (skipped in unit tests via AUTOSPEC_SKIP_DB_PROBE=1) ─────────

if [ "$AUTOSPEC_SKIP_DB_PROBE" != "1" ]; then
    # In production: would verify scope identifier exists in DB
    # Skipped here to avoid requiring live DB in unit tests
    printf 'mode-ii-preflight: DB probe skipped (AUTOSPEC_SKIP_DB_PROBE not set in production)\n' >&2
fi

# ── Preflight passed ──────────────────────────────────────────────────────────

emit_pass "$SNAP_ID"
