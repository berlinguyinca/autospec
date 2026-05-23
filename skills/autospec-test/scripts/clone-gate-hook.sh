#!/usr/bin/env bash
# skills/autospec-test/scripts/clone-gate-hook.sh — Mode II clone gate hook.
#
# Called by gate-stage-e2e.sh when e2e.clone_url_env is set in the autospec-test
# contract AND .autospec/clone.yml exists in the target repo.
#
# Responsibilities:
#   1. Invoke autospec-e2e-clone provision.sh to bring up the clone environment.
#   2. Export the resulting URL into the env var named by e2e.clone_url_env.
#   3. Register teardown.sh to run on EXIT (via the caller's accumulating trap).
#
# Usage (called from gate-stage-e2e.sh, not directly):
#   source clone-gate-hook.sh  (sources into caller's shell so exports take effect)
#
# OR invoked with arguments for standalone use:
#   clone-gate-hook.sh <target_repo> <clone_url_env_var> [--add-exit-trap-cmd <cmd>]
#
# Environment variables set on success:
#   <clone_url_env_var>  — the clone URL (e.g. BASE_URL=http://localhost:8080)
#
# Exit codes (standalone):
#   0  success — clone provisioned, URL exported
#   1  fatal (provision failed)
#   2  refuse-to-run (no clone.yml, or clone_url_env not set)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# ---------------------------------------------------------------------------
# Resolve autospec-e2e-clone scripts
# ---------------------------------------------------------------------------

# SCRIPT_DIR = skills/autospec-test/scripts/
# Repo root  = SCRIPT_DIR/../../..  (3 levels up)
# Clone skill = repo_root/skills/autospec-e2e-clone/scripts/
AUTOSPEC_REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"
CLONE_SKILL_SCRIPTS="$AUTOSPEC_REPO_ROOT/skills/autospec-e2e-clone/scripts"

PROVISION_SH="$CLONE_SKILL_SCRIPTS/provision.sh"
TEARDOWN_SH="$CLONE_SKILL_SCRIPTS/teardown.sh"

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

die()    { printf 'clone-gate-hook: fatal: %s\n' "$*" >&2; exit 1; }
refuse() { printf 'clone-gate-hook: refuse-to-run: %s\n' "$*" >&2; exit 2; }
info()   { printf 'clone-gate-hook: %s\n' "$*"; }

# ---------------------------------------------------------------------------
# Arg parsing (standalone mode)
# ---------------------------------------------------------------------------

TARGET_REPO="${1:-}"
CLONE_URL_ENV_VAR="${2:-}"
ADD_EXIT_TRAP_CMD=""

shift 2 2>/dev/null || true
while [ $# -gt 0 ]; do
  case "$1" in
    --add-exit-trap-cmd) ADD_EXIT_TRAP_CMD="$2"; shift 2 ;;
    *) die "unknown option: $1" ;;
  esac
done

[ -n "$TARGET_REPO" ]       || die "target_repo argument required"
[ -n "$CLONE_URL_ENV_VAR" ] || die "clone_url_env_var argument required"

TARGET_REPO="$(cd "$TARGET_REPO" && pwd)"

# ---------------------------------------------------------------------------
# Pre-conditions
# ---------------------------------------------------------------------------

CLONE_YML="$TARGET_REPO/.autospec/clone.yml"
if [ ! -f "$CLONE_YML" ]; then
  refuse ".autospec/clone.yml not found in $TARGET_REPO — clone gate skipped"
fi

[ -f "$PROVISION_SH" ] || die "provision.sh not found at $PROVISION_SH"
[ -f "$TEARDOWN_SH" ]  || die "teardown.sh not found at $TEARDOWN_SH"

# ---------------------------------------------------------------------------
# Provision clone
# ---------------------------------------------------------------------------

info "provisioning clone for $TARGET_REPO (expose URL → \$$CLONE_URL_ENV_VAR)"
CLONE_URL=$(bash "$PROVISION_SH" "$TARGET_REPO") || {
  rc=$?
  die "provision.sh failed (exit $rc)"
}

if [ -z "$CLONE_URL" ]; then
  # Fallback: read from file
  CLONE_URL_FILE="$TARGET_REPO/.autospec/clone-url.txt"
  if [ -f "$CLONE_URL_FILE" ]; then
    CLONE_URL="$(cat "$CLONE_URL_FILE")"
  fi
fi

[ -n "$CLONE_URL" ] || die "provision.sh succeeded but no clone URL was produced"

# ---------------------------------------------------------------------------
# Export URL into named env var
# ---------------------------------------------------------------------------

export "${CLONE_URL_ENV_VAR}=${CLONE_URL}"
info "exported ${CLONE_URL_ENV_VAR}=${CLONE_URL}"

# ---------------------------------------------------------------------------
# Register teardown on EXIT
# ---------------------------------------------------------------------------

TEARDOWN_CMD="bash '${TEARDOWN_SH}' '${TARGET_REPO}' 2>/dev/null || true"

if [ -n "$ADD_EXIT_TRAP_CMD" ]; then
  # Caller provided an add_exit_trap function name — invoke it
  "$ADD_EXIT_TRAP_CMD" "$TEARDOWN_CMD"
  info "registered teardown via $ADD_EXIT_TRAP_CMD"
else
  # Standalone: register our own EXIT trap
  trap 'bash "${TEARDOWN_SH}" "${TARGET_REPO}" 2>/dev/null || true' EXIT
  info "registered teardown via EXIT trap"
fi

info "clone gate hook complete — URL: $CLONE_URL"
