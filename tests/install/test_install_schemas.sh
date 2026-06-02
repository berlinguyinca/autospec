#!/usr/bin/env bash
# Regression guard: install.sh must copy schemas/*.json into
# $AUTOSPEC_SCHEMAS_DIR (default ~/.autospec/schemas/) on install and --update.
# Also verifies that validate-qa-artifacts.sh honors AUTOSPEC_SCHEMAS_DIR and
# fails with a non-zero exit + actionable message when a schema is missing.
#
# Mutation-verify: if install.sh does NOT copy schemas/, the first assertion
# fails because the schema file will be absent from the temp install prefix.
set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"

# --- helper: create a temp install prefix --------------------------------
tmpdir="$(mktemp -d /tmp/autospec-test-schemas-XXXXXX)"
trap 'rm -rf "$tmpdir"' EXIT

export AUTOSPEC_SCRIPTS_DIR="$tmpdir/scripts"
export AUTOSPEC_SCHEMAS_DIR="$tmpdir/schemas"
# Ensure no prompt / interactive steps are triggered.
export AUTOSPEC_AUTO_YES=1
export AUTOSPEC_NO_STAR_PROMPT=1
export AUTOSPEC_SKIP_SYSTEM_TOOLS=1
export AUTOSPEC_SKIP_ECOSYSTEM_BOOTSTRAP=1
export AUTOSPEC_SKIP_SUPERPOWERS=1
export CI=true

# --------------------------------------------------------------------------
# Part 1: fresh install places schemas in AUTOSPEC_SCHEMAS_DIR
# --------------------------------------------------------------------------

# Run install in dry-run first to ensure the dry-run path also emits the right
# message (and does NOT write any files).
dry_run_output=$(bash "$SCRIPT_DIR/install.sh" \
    --dry-run --skill autospec --harness claude 2>&1 || true)

case "$dry_run_output" in
    *"copy_schemas"*)
        ;;
    *)
        echo "FAIL: dry-run did not mention copy_schemas step"
        echo "--- output ---"
        echo "$dry_run_output"
        exit 1
        ;;
esac

# Dry-run must NOT have created the schemas dir.
if [ -d "$AUTOSPEC_SCHEMAS_DIR" ] && [ -n "$(ls -A "$AUTOSPEC_SCHEMAS_DIR" 2>/dev/null)" ]; then
    echo "FAIL: dry-run created files in AUTOSPEC_SCHEMAS_DIR"
    exit 1
fi

# Now do a real (non-dry-run) install.
bash "$SCRIPT_DIR/install.sh" \
    --skill autospec --harness claude 2>&1 | grep -v "^$" || true

# Every schema in the repo's schemas/ dir must now exist in the install prefix.
missing=""
for schema_file in "$SCRIPT_DIR/schemas/"*.json; do
    [ -e "$schema_file" ] || continue
    schema_name="$(basename "$schema_file")"
    if [ ! -f "$AUTOSPEC_SCHEMAS_DIR/$schema_name" ]; then
        missing="$missing $schema_name"
    fi
done

if [ -n "$missing" ]; then
    echo "FAIL: after install, the following schemas are missing from $AUTOSPEC_SCHEMAS_DIR:"
    echo "  $missing"
    echo "Root cause: install.sh does not copy schemas/ (issue #856)"
    exit 1
fi
echo "PASS (part 1): all $(ls "$AUTOSPEC_SCHEMAS_DIR" | wc -l | tr -d ' ') schemas present in $AUTOSPEC_SCHEMAS_DIR after install"

# --------------------------------------------------------------------------
# Part 2: --update also re-copies schemas (idempotent)
# --------------------------------------------------------------------------

# Remove one schema to simulate an out-of-date install.
first_schema="$(ls "$AUTOSPEC_SCHEMAS_DIR"/*.json | head -1)"
rm "$first_schema"

bash "$SCRIPT_DIR/install.sh" \
    --update --skill autospec --harness claude 2>&1 | grep -v "^$" || true

if [ ! -f "$first_schema" ]; then
    echo "FAIL: --update did not restore the missing schema $first_schema"
    exit 1
fi
echo "PASS (part 2): --update re-installs a deleted schema"

# --------------------------------------------------------------------------
# Part 3: validate-qa-artifacts.sh honors AUTOSPEC_SCHEMAS_DIR and fails with
# an actionable message when a schema is missing.
# --------------------------------------------------------------------------

# Create a schemas dir that is empty (no schemas inside).
bad_schema_dir="$(mktemp -d /tmp/autospec-test-bad-schemas-XXXXXX)"
trap 'rm -rf "$bad_schema_dir"' EXIT

repo_dir="$(mktemp -d /tmp/autospec-test-repo-XXXXXX)"
trap 'rm -rf "$repo_dir"' EXIT

# Run the validator pointing at the empty schema dir; expect non-zero exit and
# an actionable error message.
validator_out=$(
    AUTOSPEC_SCHEMAS_DIR="$bad_schema_dir" \
    bash "$SCRIPT_DIR/scripts/validate-qa-artifacts.sh" \
        --repo "$repo_dir" 2>&1 \
    ; true
)
validator_rc=$(
    AUTOSPEC_SCHEMAS_DIR="$bad_schema_dir" \
    bash "$SCRIPT_DIR/scripts/validate-qa-artifacts.sh" \
        --repo "$repo_dir" 2>&1 >/dev/null; echo $?
) || true

# Capture actual exit code
set +e
AUTOSPEC_SCHEMAS_DIR="$bad_schema_dir" \
    bash "$SCRIPT_DIR/scripts/validate-qa-artifacts.sh" \
        --repo "$repo_dir" >/dev/null 2>&1
actual_rc=$?
set -e

if [ "$actual_rc" -eq 0 ]; then
    echo "FAIL: validate-qa-artifacts.sh exited 0 when schemas are missing (should be non-zero)"
    exit 1
fi

case "$validator_out" in
    *install.sh*|*install*|*AUTOSPEC_SCHEMAS_DIR*|*schemas*missing*|*missing*schema*)
        ;;
    *)
        echo "FAIL: validate-qa-artifacts.sh missing-schema message is not actionable"
        echo "--- output ---"
        echo "$validator_out"
        exit 1
        ;;
esac
echo "PASS (part 3): validate-qa-artifacts.sh exits non-zero with actionable message when schemas are missing"

echo "PASS"
