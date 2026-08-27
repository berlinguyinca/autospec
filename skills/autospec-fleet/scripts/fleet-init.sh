#!/usr/bin/env bash
# Provision autospec-fleet checkouts for repository URLs: clone repos that
# are missing, fetch+fast-forward repos that already exist, and skip a
# dirty or non-fast-forwardable checkout without touching it. --dry-run
# stays a pure preview (see fleet_provision helpers in fleet-lib.sh).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/fleet-lib.sh"

workspace=".autospec-fleet/repos"
dry_run=0

usage() {
    cat <<'EOF'
Usage: fleet-init.sh [--workspace PATH] [--dry-run] <repo-url>...

Provisions deterministic autospec-fleet checkouts: clones a repo that is
missing, and fetch+fast-forward-updates a repo that already exists (never
touching a dirty checkout or one that would not fast-forward). In dry-run
mode, no directories are created and no repositories are cloned.
EOF
}

while [ $# -gt 0 ]; do
    case "$1" in
        --workspace)
            shift
            [ $# -gt 0 ] || { printf 'fleet: --workspace requires a path\n' >&2; exit 2; }
            workspace="$1"
            ;;
        --workspace=*)
            workspace="${1#--workspace=}"
            ;;
        --dry-run)
            dry_run=1
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        --*)
            printf 'fleet: unknown option: %s\n' "$1" >&2
            usage >&2
            exit 2
            ;;
        *)
            break
            ;;
    esac
    shift
done

[ $# -gt 0 ] || { printf 'fleet: at least one repo URL is required\n' >&2; usage >&2; exit 2; }

if [ "$dry_run" -eq 1 ]; then
    for repo_url in "$@"; do
        normalized="$(normalize_repo_url "$repo_url")"
        checkout_path="$(repo_checkout_path "$workspace" "$normalized")"
        printf 'fleet: plan clone %s -> %s\n' "$normalized" "$checkout_path"
    done
    exit 0
fi

mkdir -p "$workspace"

for repo_url in "$@"; do
    # A single repo's provisioning failure must never abort the loop over
    # the rest of the fleet — deliberate if/then, never a one-sided `&&`,
    # since this script runs under `set -euo pipefail`. Per-repo failures
    # are reported to stderr as code_health: markers (see fleet-lib.sh);
    # the overall run still exits 0, matching fleet-run.sh's convention
    # that a per-repo failure never fails the batch.
    if fleet_provision_repo "$workspace" "$repo_url"; then
        :
    fi
done

exit 0
