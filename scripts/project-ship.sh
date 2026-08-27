#!/usr/bin/env bash
# scripts/project-ship.sh — the `ship` mode chain for autospec-project:
#
#   resolve board -> filter to project_board.repo_allowlist -> write/update
#   autospec-fleet.yml (allowlisted repos only) -> provision each allowlisted
#   checkout (clone-if-missing / fetch+ff-only-update, never destructive) ->
#   launch a per-repo autospec-autonomous conductor for each eligible
#   checkout.
#
# SECURITY: project_board.repo_allowlist (read via `autospec autonomous
# project-board-config`, the same validated Rust config sync/status already
# use) is the ONLY gate. It is checked first, before the board is even
# resolved: an empty or unset allowlist refuses outright (exit 3) with zero
# git/gh/autospec-autonomous calls of any kind. Once the board is resolved,
# every repo it names is filtered through the SAME prefix-or-equality match
# scripts/autonomous-promote-open-issues.sh's board_stage() `allowed($r)`
# filter uses (never jq test()/regex — a board repo string is untrusted
# data). A repo outside the allowlist is reported as skipped and is never
# written into the fleet config, never provisioned, never launched, and
# never passed as an argument to git, gh, or autospec-autonomous.
#
# FAILURE MODEL: resolving the board is one all-or-nothing call (matches
# bare/sync/status mode) — either the whole board resolves or it doesn't.
# Once resolved, every allowlisted repo is provisioned and launched
# independently; one repo's provisioning or launch failure is reported and
# never aborts the others (deliberate if/then throughout — this script runs
# under `set -euo pipefail`, so a one-sided `&&`/`!` on a fallible call
# would abort the whole run).
#
# HONESTY: every allowlisted repo gets exactly one
# `project-ship: repo=<repo> ... provision=<ok|failed>` line and one
# `project-ship: repo=<repo> ... launch=<status>` line; every non-
# allowlisted repo gets exactly one `action=skipped reason=not-allowlisted`
# line. Nothing is summarized away.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

fail() {
    printf 'project-ship: %s\n' "$*" >&2
    exit 2
}

url=""
repo_dir="."
workspace=""
fleet_config="autospec-fleet.yml"
dry_run=0
once=0

usage() {
    cat <<'EOF'
Usage: project-ship.sh --url URL [--repo-dir DIR] [--workspace PATH]
                        [--fleet-config PATH] [--dry-run] [--once]

Resolves a GitHub Projects v2 board, filters its repos to
project_board.repo_allowlist (read via `autospec autonomous
project-board-config --repo-dir DIR`), writes/updates an autospec-fleet.yml
covering only the allowlisted repos, provisions a checkout for each
(clone-if-missing / fetch+ff-only-update — a dirty or non-fast-forwardable
checkout is skipped, never touched), then launches a per-repo
autospec-autonomous conductor for each eligible checkout.

Refuses outright (exit 3, zero git/gh/autospec-autonomous calls) when the
allowlist bridged from .autospec/autonomous.yml is empty or unset — an
unscoped board can never be shipped.
EOF
}

while [ $# -gt 0 ]; do
    case "$1" in
        --url) shift; [ $# -gt 0 ] || fail "--url requires a value"; url="$1" ;;
        --url=*) url="${1#--url=}" ;;
        --repo-dir) shift; [ $# -gt 0 ] || fail "--repo-dir requires a path"; repo_dir="$1" ;;
        --repo-dir=*) repo_dir="${1#--repo-dir=}" ;;
        --workspace) shift; [ $# -gt 0 ] || fail "--workspace requires a path"; workspace="$1" ;;
        --workspace=*) workspace="${1#--workspace=}" ;;
        --fleet-config) shift; [ $# -gt 0 ] || fail "--fleet-config requires a path"; fleet_config="$1" ;;
        --fleet-config=*) fleet_config="${1#--fleet-config=}" ;;
        --dry-run) dry_run=1 ;;
        --once) once=1 ;;
        -h|--help) usage; exit 0 ;;
        *) fail "unknown argument: $1" ;;
    esac
    shift
done

[ -n "$url" ] || fail "--url is required"
command -v jq >/dev/null 2>&1 || fail "jq is required"

# ── Sub-script paths (injectable seams for tests — same idiom as
# scripts/autonomous-promote-open-issues.sh's BOARD_RESOLVE/BOARD_CONFIG_BIN
# variables). An installed layout flattens every autospec helper script into
# one directory (${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}); a git
# checkout keeps fleet-lib.sh/fleet-run.sh under
# skills/autospec-fleet/scripts/ instead, so that path is tried as a
# fallback when the flattened one doesn't exist. ───────────────────────────
RESOLVE_BIN="${AUTOSPEC_PROJECT_BOARD_RESOLVE_BIN:-${AUTOSPEC_SCRIPTS_DIR:-$SCRIPT_DIR}/project-board-resolve.sh}"
BOARD_CONFIG_BIN="${AUTOSPEC_PROJECT_BOARD_CONFIG_BIN:-${AUTOSPEC_BIN:-autospec}}"
FLEET_LIB="${AUTOSPEC_FLEET_LIB_SCRIPT:-${AUTOSPEC_SCRIPTS_DIR:-$SCRIPT_DIR}/fleet-lib.sh}"
FLEET_RUN_BIN="${AUTOSPEC_FLEET_RUN_SCRIPT:-${AUTOSPEC_SCRIPTS_DIR:-$SCRIPT_DIR}/fleet-run.sh}"

if [ ! -f "$FLEET_LIB" ]; then
    _dev_fleet_lib="$SCRIPT_DIR/../skills/autospec-fleet/scripts/fleet-lib.sh"
    [ -f "$_dev_fleet_lib" ] && FLEET_LIB="$_dev_fleet_lib"
fi
if [ ! -f "$FLEET_RUN_BIN" ]; then
    _dev_fleet_run="$SCRIPT_DIR/../skills/autospec-fleet/scripts/fleet-run.sh"
    [ -f "$_dev_fleet_run" ] && FLEET_RUN_BIN="$_dev_fleet_run"
fi

[ -f "$RESOLVE_BIN" ] || fail "project-board-resolve.sh not found at $RESOLVE_BIN"
[ -f "$FLEET_LIB" ] || fail "fleet-lib.sh not found (checked \$AUTOSPEC_FLEET_LIB_SCRIPT and $FLEET_LIB)"
[ -f "$FLEET_RUN_BIN" ] || fail "fleet-run.sh not found (checked \$AUTOSPEC_FLEET_RUN_SCRIPT and $FLEET_RUN_BIN)"
# shellcheck source=/dev/null
source "$FLEET_LIB"

# ── Step 1: the allowlist gate ──────────────────────────────────────────────
# Enforced BEFORE the board is resolved and before any git/gh call: an
# unscoped board must never even be read for the purpose of shipping it.
pb_json="$("$BOARD_CONFIG_BIN" autonomous project-board-config --repo-dir "$repo_dir" 2>/dev/null || true)"
allowlist_json="[]"
if [ -n "$pb_json" ]; then
    allowlist_json="$(printf '%s' "$pb_json" | jq -c '.allowlist // []' 2>/dev/null || printf '[]')"
fi
allowlist_count="$(printf '%s' "$allowlist_json" | jq 'length' 2>/dev/null || printf '0')"
case "$allowlist_count" in
    ''|*[!0-9]*) allowlist_count=0 ;;
esac
if [ "$allowlist_count" -eq 0 ]; then
    printf 'project-ship: project_board.repo_allowlist is empty or unset (checked %s/.autospec/autonomous.yml via the project-board-config bridge); refusing to ship an unscoped board\n' "$repo_dir" >&2
    exit 3
fi

# ── Step 2: resolve the board (read-only; a single all-or-nothing call,
# same as bare/sync/status mode) ────────────────────────────────────────────
repos_json=""
resolve_rc=0
repos_json="$("$RESOLVE_BIN" --url "$url" --emit repos)" || resolve_rc=$?
if [ "$resolve_rc" -ne 0 ]; then
    printf 'project-ship: board resolve failed (exit %s)\n' "$resolve_rc" >&2
    exit "$resolve_rc"
fi

# ── Step 3: filter to the allowlist ─────────────────────────────────────────
filtered_json="$(jq -c -n --argjson repos "$repos_json" --argjson allow "$allowlist_json" '
  def allowed($r):
    $allow | map(
      (. | rtrimstr("*")) as $p
      | if endswith("*") then ($r | startswith($p)) else ($r == .) end
    ) | any;
  { allowed: [ $repos[] | select(allowed(.)) ],
    skipped: [ $repos[] | select(allowed(.) | not) ] }')"

allowed_repos_json="$(printf '%s' "$filtered_json" | jq -c '.allowed')"
skipped_repos_json="$(printf '%s' "$filtered_json" | jq -c '.skipped')"

while IFS= read -r repo; do
    [ -n "$repo" ] || continue
    printf 'project-ship: repo=%s allowlisted=no action=skipped reason=not-allowlisted\n' "$repo"
done < <(printf '%s' "$skipped_repos_json" | jq -r '.[]')

allowed_count="$(printf '%s' "$allowed_repos_json" | jq 'length')"
if [ "$allowed_count" -eq 0 ]; then
    printf 'project-ship: no allowlisted repos found on this board; nothing to ship\n'
    exit 0
fi

# ── Step 4: write/update autospec-fleet.yml — allowlisted repos ONLY ───────
[ -n "$workspace" ] || workspace=".autospec-fleet/repos"
parallel="${AUTOSPEC_PROJECT_BOARD_PARALLEL:-2}"
case "$parallel" in
    [1-9]|[1-9][0-9]*) ;;
    *) parallel=2 ;;
esac
{
    printf 'version: 1\n'
    printf 'workspace: %s\n' "$workspace"
    printf 'parallel_repos: %s\n' "$parallel"
    printf 'repos:\n'
    printf '%s' "$allowed_repos_json" \
        | jq -r '.[] | "  - url: " + (("https://github.com/" + . + ".git") | tojson) + "\n    enabled: true"'
} > "$fleet_config"

# ── Step 5: provision each allowlisted repo. Per-repo failure never aborts
# the loop — deliberate if/then, never a one-sided `&&`/`!`, since this
# script runs under `set -euo pipefail`. fleet_provision_repo (fleet-lib.sh,
# the same function fleet-init.sh uses) already refuses to touch a dirty or
# non-fast-forwardable checkout; nothing here may override that. ───────────
if [ "$dry_run" -eq 0 ]; then
    mkdir -p -- "$workspace"
fi

while IFS= read -r repo; do
    [ -n "$repo" ] || continue
    repo_url="https://github.com/$repo.git"
    if [ "$dry_run" -eq 1 ]; then
        normalized="$(normalize_repo_url "$repo_url" 2>/dev/null || printf '%s' "$repo")"
        checkout_path="$(repo_checkout_path "$workspace" "$normalized" 2>/dev/null || printf 'unknown')"
        printf 'project-ship: repo=%s allowlisted=yes action=plan-provision checkout=%s\n' "$repo" "$checkout_path"
        continue
    fi
    # Captured combined so the dirty/non-fast-forward cases (which
    # fleet_provision_repo reports via a `return 0` plus a distinguishing
    # message + code_health: marker, never a nonzero return — see
    # fleet-lib.sh) can be told apart from a genuine success, instead of
    # being misreported as "provision=ok" when nothing was actually
    # touched.
    provision_output=""
    provision_rc=0
    provision_output="$(fleet_provision_repo "$workspace" "$repo_url" 2>&1)" || provision_rc=$?
    case "$provision_output" in
        *"local changes present"*) provision_status="skipped:dirty" ;;
        *"would not fast-forward"*) provision_status="skipped:not-fast-forward" ;;
        *)
            if [ "$provision_rc" -eq 0 ]; then
                provision_status="ok"
            else
                provision_status="failed"
            fi
            ;;
    esac
    printf 'project-ship: repo=%s allowlisted=yes provision=%s\n' "$repo" "$provision_status"
    if [ "$provision_status" = "failed" ]; then
        printf '%s\n' "$provision_output" >&2
    fi
done < <(printf '%s' "$allowed_repos_json" | jq -r '.[]')

# ── Step 6: launch — a per-repo autospec-autonomous conductor for each
# eligible checkout, via the existing, tested fleet-run.sh. A repo that
# failed to provision above simply has no usable checkout, so fleet-run.sh
# reports it as "checkout not found" rather than launching a broken worker —
# the same per-repo failure isolation fleet-run.sh already guarantees. ─────
launch_output=""
if [ "$dry_run" -eq 1 ]; then
    launch_output="$(bash "$FLEET_RUN_BIN" --config "$fleet_config" --dry-run 2>&1)" || true
elif [ "$once" -eq 1 ]; then
    launch_output="$(bash "$FLEET_RUN_BIN" --config "$fleet_config" --once 2>&1)" || true
else
    launch_output="$(bash "$FLEET_RUN_BIN" --config "$fleet_config" 2>&1)" || true
fi

while IFS= read -r repo; do
    [ -n "$repo" ] || continue
    line="$(printf '%s\n' "$launch_output" | grep -F -- "$repo" | head -n 1)"
    if [ "$dry_run" -eq 1 ]; then
        status="preview"
    elif [ -z "$line" ]; then
        status="skipped:no-ready-work-or-capacity"
    else
        case "$line" in
            *"launch $repo:"*) status="launched" ;;
            *"checkout not found"*) status="skipped:checkout-not-found" ;;
            *"worker already live"*) status="skipped:already-live" ;;
            *"fleet_worker_spawn_failed"*) status="failed" ;;
            *) status="skipped:unclassified" ;;
        esac
    fi
    printf 'project-ship: repo=%s allowlisted=yes launch=%s\n' "$repo" "$status"
done < <(printf '%s' "$allowed_repos_json" | jq -r '.[]')

printf '%s\n' "$launch_output"

exit 0
