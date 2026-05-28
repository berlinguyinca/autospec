#!/usr/bin/env bash
# Plan autospec-fleet checkout paths for repository URLs.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/fleet-lib.sh"

workspace=".autospec-fleet/repos"
dry_run=0

usage() {
    cat <<'EOF'
Usage: fleet-init.sh [--workspace PATH] [--dry-run] <repo-url>...

Plans deterministic autospec-fleet checkout paths. In dry-run mode, no
directories are created and no repositories are cloned.
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

if [ "$dry_run" -eq 0 ]; then
    mkdir -p "$workspace"
fi

for repo_url in "$@"; do
    normalized="$(normalize_repo_url "$repo_url")"
    checkout_path="$(repo_checkout_path "$workspace" "$normalized")"
    if [ "$dry_run" -eq 1 ]; then
        printf 'fleet: plan clone %s -> %s\n' "$normalized" "$checkout_path"
    else
        printf 'fleet: planned %s at %s\n' "$normalized" "$checkout_path"
    fi
done
