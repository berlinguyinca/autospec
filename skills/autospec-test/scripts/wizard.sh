#!/usr/bin/env bash
# wizard.sh — /autospec-test --init operator wizard.
#
# Usage:
#   wizard.sh init [--config <yaml-fragment>] [--ack-i-understand] [--dry-run]
#
# Interactive mode: prompts for mode selection, scope tokens, backup driver.
#   Requires operator to type exactly "I UNDERSTAND" before writing files.
#
# Headless mode: reads preset answers from --config <yaml-fragment>.
#   Requires --ack-i-understand flag (substitutes for the "I UNDERSTAND" prompt).
#
# Steps (spec §5d):
#   1. Mode selection (strict vs scoped; strict is default)
#   2. If scoped: scope-token kinds, identifiers, prod URL
#   3. Backup driver detection (probe for zfs/pg_dump/mysqldump/custom_cmd)
#      → refuse Mode II if none and no custom_cmd provided
#   4. Dry-run preview — print resolved constraints
#   5. Require "I UNDERSTAND" (interactive) or --ack-i-understand (headless)
#   6. Write .autospec/test.yml + initial ack lock if Mode II
#
# Output path: ./.autospec/test.yml (fixed; relative to CWD)
#
# Exit codes:
#   0 = success (or dry-run preview shown)
#   1 = refused (operator declined, driver missing, ack missing)

set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

# ── Temporary files with cleanup ───────────────────────────────────────────────

YQ_ERR_FILE=$(mktemp -t wizard-yq-err.XXXXXX)
PROBE_ERR_FILE=$(mktemp -t wizard-probe-err.XXXXXX)
trap 'rm -f "$YQ_ERR_FILE" "$PROBE_ERR_FILE"' EXIT

# ── Argument parsing ───────────────────────────────────────────────────────────

SUBCOMMAND="${1:-}"
if [ "$SUBCOMMAND" != "init" ]; then
    printf 'wizard: usage: wizard.sh init [--config <yml>] [--ack-i-understand] [--dry-run]\n' >&2
    exit 1
fi
shift

CONFIG_FILE=""
ACK_FLAG=false
DRY_RUN=false

while [ $# -gt 0 ]; do
    case "$1" in
        --config)
            CONFIG_FILE="${2:-}"
            shift 2
            ;;
        --ack-i-understand)
            ACK_FLAG=true
            shift
            ;;
        --dry-run)
            DRY_RUN=true
            shift
            ;;
        *)
            printf 'wizard: unknown flag: %s\n' "$1" >&2
            exit 1
            ;;
    esac
done

AUTOSPEC_OUT_DIR="${PWD}/.autospec"

# ── Determine headless vs interactive ─────────────────────────────────────────

HEADLESS=false
if [ -n "$CONFIG_FILE" ]; then
    HEADLESS=true
fi

# ── Dependency check ──────────────────────────────────────────────────────────

if ! command -v jq >/dev/null 2>&1; then
    printf 'wizard: jq not found\n' >&2
    exit 1
fi

if ! command -v yq >/dev/null 2>&1; then
    printf 'wizard: yq not found. Install: brew install yq\n' >&2
    exit 1
fi

# ── Step 1: Load config or prompt for mode ────────────────────────────────────

if [ "$HEADLESS" = "true" ]; then
    if [ ! -f "$CONFIG_FILE" ]; then
        printf 'wizard: config file not found: %s\n' "$CONFIG_FILE" >&2
        exit 1
    fi
    # Parse YAML config
    CONFIG_JSON=""
    if ! CONFIG_JSON=$(yq -o=json '.' "$CONFIG_FILE" 2>"$YQ_ERR_FILE"); then
        printf 'wizard: failed to parse config YAML:\n' >&2
        cat "$YQ_ERR_FILE" >&2
        exit 1
    fi
    if [ -z "$CONFIG_JSON" ] || [ "$CONFIG_JSON" = "null" ]; then
        printf 'wizard: config file is empty or invalid\n' >&2
        exit 1
    fi
    MODE=$(printf '%s' "$CONFIG_JSON" | jq -r '.mode // "strict_isolation"')
else
    # Interactive mode — prompt for mode
    printf 'autospec-test wizard\n'
    printf '====================\n'
    printf 'Select mode:\n'
    printf '  1) strict_isolation (default — no production access)\n'
    printf '  2) scoped_production (opt-in — requires backup driver + ack)\n'
    printf 'Enter choice [1]: '
    read -r mode_choice </dev/tty
    case "${mode_choice:-1}" in
        2) MODE="scoped_production" ;;
        *) MODE="strict_isolation" ;;
    esac
    CONFIG_JSON="{\"mode\":\"${MODE}\"}"
fi

# ── Step 2: Backup driver probe (Mode II only) ────────────────────────────────

BACKUP_DRIVER=""
if [ "$MODE" = "scoped_production" ]; then
    # Check if config provides a driver
    if [ "$HEADLESS" = "true" ]; then
        BACKUP_DRIVER=$(printf '%s' "$CONFIG_JSON" | jq -r '.e2e.backup.driver // empty')
    fi

    if [ -z "$BACKUP_DRIVER" ]; then
        # Probe PATH for known drivers
        PROBE_OUT=""
        PROBE_EXIT=0
        PROBE_OUT=$("$SCRIPT_DIR/wizard-probe-backup.sh" 2>"$PROBE_ERR_FILE") || PROBE_EXIT=$?
        if [ "$PROBE_EXIT" -ne 0 ]; then
            printf 'wizard: REFUSED: Mode II requires a backup driver, but none was found on PATH.\n' >&2
            cat "$PROBE_ERR_FILE" >&2
            printf 'Install a backup tool or set driver: custom in your config with custom_snapshot_cmd/custom_restore_cmd.\n' >&2
            exit 1
        fi
        BACKUP_DRIVER="$PROBE_OUT"
    fi

    # If custom driver, verify restore_cmd is available in config
    if [ "$BACKUP_DRIVER" = "custom" ] && [ "$HEADLESS" = "true" ]; then
        RESTORE_CMD=$(printf '%s' "$CONFIG_JSON" | jq -r '.e2e.backup.custom_restore_cmd // .e2e.backup.restore_cmd // empty')
        if [ -z "$RESTORE_CMD" ]; then
            printf 'wizard: REFUSED: custom backup driver requires custom_restore_cmd in config\n' >&2
            exit 1
        fi
    fi
fi

# ── Step 3: Compose final config JSON ─────────────────────────────────────────

FINAL_JSON="$CONFIG_JSON"

# Ensure mode is set
FINAL_JSON=$(printf '%s' "$FINAL_JSON" | jq --arg mode "$MODE" '.mode = $mode')

# For Mode II, inject i_understand flag
if [ "$MODE" = "scoped_production" ]; then
    FINAL_JSON=$(printf '%s' "$FINAL_JSON" | jq '.i_understand_this_writes_to_production = true')
fi

# ── Step 4: Dry-run preview ───────────────────────────────────────────────────

printf '\n=== Contract preview ===\n'
printf '%s\n' "$FINAL_JSON" | yq -P '.' -
printf '\n=== Constraints that will apply ===\n'
printf 'mode: %s\n' "$MODE"
if [ "$MODE" = "scoped_production" ]; then
    printf 'backup driver: %s\n' "$BACKUP_DRIVER"
    TOKEN_COUNT=$(printf '%s' "$FINAL_JSON" | jq '.e2e.production_scoped_access.scope_tokens | length // 0' 2>/dev/null || echo 0)
    printf 'scope tokens: %s\n' "$TOKEN_COUNT"
fi
FORBIDDEN_COUNT=$(printf '%s' "$FINAL_JSON" | jq '.e2e.forbidden_url_patterns | length // 0' 2>/dev/null || echo 0)
printf 'forbidden URL patterns: %s\n' "$FORBIDDEN_COUNT"
printf '========================\n\n'

if [ "$DRY_RUN" = "true" ]; then
    printf 'Dry-run mode — no files written.\n'
    exit 0
fi

# ── Step 5: Ack gate ──────────────────────────────────────────────────────────

if [ "$HEADLESS" = "true" ]; then
    if [ "$ACK_FLAG" = "false" ]; then
        printf 'wizard: REFUSED: headless mode requires --ack-i-understand flag\n' >&2
        printf 'Usage: wizard.sh init --config <yml> --ack-i-understand\n' >&2
        exit 1
    fi
else
    # Interactive: require literal "I UNDERSTAND"
    printf 'Type exactly "I UNDERSTAND" to write the configuration: '
    read -r user_ack </dev/tty
    if [ "$user_ack" != "I UNDERSTAND" ]; then
        printf 'wizard: REFUSED: you did not type "I UNDERSTAND". No files written.\n' >&2
        exit 1
    fi
fi

# ── Step 6: Write .autospec/test.yml ─────────────────────────────────────────

mkdir -p "$AUTOSPEC_OUT_DIR"

TEST_YML_PATH="$AUTOSPEC_OUT_DIR/test.yml"
printf '%s\n' "$FINAL_JSON" | yq -P '.' - > "$TEST_YML_PATH"

printf 'wizard: wrote %s\n' "$TEST_YML_PATH"

# ── Step 7: Write ack-lock file (Mode II only) ────────────────────────────────

if [ "$MODE" = "scoped_production" ]; then
    # Compute SHA of the production_scoped_access section
    SCOPED_ACCESS=$(printf '%s' "$FINAL_JSON" | jq -c '.e2e.production_scoped_access // {}')

    if command -v sha256sum >/dev/null 2>&1; then
        CONTRACT_SHA=$(printf '%s' "$SCOPED_ACCESS" | sha256sum | awk '{print $1}' | cut -c1-40)
    elif command -v shasum >/dev/null 2>&1; then
        CONTRACT_SHA=$(printf '%s' "$SCOPED_ACCESS" | shasum -a 256 | awk '{print $1}' | cut -c1-40)
    else
        # Fallback: use date-based id
        CONTRACT_SHA="$(date -u +%Y%m%d%H%M%S)fallback"
    fi

    LOCK_FILE="$AUTOSPEC_OUT_DIR/.scoped-prod-acked-${CONTRACT_SHA}.lock"
    printf 'acked:%s\n' "$CONTRACT_SHA" > "$LOCK_FILE"
    printf 'wizard: wrote ack-lock: %s\n' "$LOCK_FILE"
fi

printf 'wizard: done. Review %s before committing.\n' "$TEST_YML_PATH"
