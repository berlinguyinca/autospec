#!/usr/bin/env bash
# scripts/autospec-control-plane.sh — local control-plane bootstrap helpers.

set -eu

usage() {
    cat <<'USAGE'
Usage:
  scripts/autospec-control-plane.sh --help
  scripts/autospec-control-plane.sh bootstrap --dry-run [--owner OWNER] [--governance-repo NAME] [--observatory-repo NAME]

Commands:
  bootstrap --dry-run    Print governance and observatory scaffolds without GitHub writes.

Defaults:
  --owner OWNER             berlinguyinca
  --governance-repo NAME    autospec-governance
  --observatory-repo NAME   autospec-observatory

The dry-run renderer is intentionally offline-only: it prints policy files,
rules, schemas, fixtures, tests, docs, and observatory service scaffold files
planned for companion repositories and never creates repositories, commits,
pushes, or invokes gh.
USAGE
}

fail() {
    printf 'autospec-control-plane: %s\n' "$*" >&2
    exit 2
}

CONTROL_PLANE_RENDER_LIB="${CONTROL_PLANE_RENDER_LIB:-$(cd "$(dirname "$0")" && pwd)/lib/autospec-control-plane-render.sh}"
# shellcheck source=scripts/lib/autospec-control-plane-render.sh
. "$CONTROL_PLANE_RENDER_LIB"
CONTROL_PLANE_OBSERVATORY_RENDER_LIB="${CONTROL_PLANE_OBSERVATORY_RENDER_LIB:-$(cd "$(dirname "$0")" && pwd)/lib/autospec-control-plane-observatory-render.sh}"
# shellcheck source=scripts/lib/autospec-control-plane-observatory-render.sh
. "$CONTROL_PLANE_OBSERVATORY_RENDER_LIB"

bootstrap() {
    dry_run=0
    owner="berlinguyinca"
    governance_repo="autospec-governance"
    observatory_repo="autospec-observatory"

    while [ "$#" -gt 0 ]; do
        case "$1" in
            --dry-run)
                dry_run=1
                shift
                ;;
            --owner)
                [ "$#" -ge 2 ] || fail "--owner requires a value"
                owner="$2"
                shift 2
                ;;
            --governance-repo)
                [ "$#" -ge 2 ] || fail "--governance-repo requires a value"
                governance_repo="$2"
                shift 2
                ;;
            --observatory-repo)
                [ "$#" -ge 2 ] || fail "--observatory-repo requires a value"
                observatory_repo="$2"
                shift 2
                ;;
            --help|-h)
                usage
                exit 0
                ;;
            *)
                fail "unknown bootstrap argument: $1"
                ;;
        esac
    done

    [ "$dry_run" -eq 1 ] || fail "bootstrap currently supports --dry-run only"
    render_control_plane_dry_run "$owner" "$governance_repo" "$observatory_repo"
}

main() {
    if [ "$#" -eq 0 ]; then
        usage
        exit 0
    fi

    case "$1" in
        --help|-h)
            usage
            ;;
        bootstrap)
            shift
            bootstrap "$@"
            ;;
        *)
            fail "unknown command: $1"
            ;;
    esac
}

main "$@"
