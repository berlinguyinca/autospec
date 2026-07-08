#!/usr/bin/env bash
# scripts/autospec-control-plane.sh — local control-plane bootstrap helpers.

set -eu

usage() {
    cat <<'USAGE'
Usage:
  scripts/autospec-control-plane.sh --help
  scripts/autospec-control-plane.sh bootstrap --dry-run [--owner OWNER] [--governance-repo NAME]

Commands:
  bootstrap --dry-run    Print the autospec-governance scaffold without GitHub writes.

Defaults:
  --owner OWNER          berlinguyinca
  --governance-repo NAME autospec-governance

The dry-run renderer is intentionally offline-only: it prints policy files,
rules, schemas, fixtures, tests, and docs planned for autospec-governance and
never creates repositories, commits, pushes, or invokes gh.
USAGE
}

fail() {
    printf 'autospec-control-plane: %s\n' "$*" >&2
    exit 2
}

render_governance_dry_run() {
    owner="$1"
    governance_repo="$2"

    cat <<EOF_RENDER
# autospec-control-plane bootstrap --dry-run

owner: ${owner}
governance_repo: ${governance_repo}
mode: dry-run
github_writes: false

${governance_repo}/
EOF_RENDER

    print_group "policies" \
        "open-source-maintainer-default.yml" \
        "private-personal-default.yml" \
        "private-company-default.yml" \
        "client-project-default.yml" \
        "research-default.yml" \
        "sandbox-default.yml"
    print_group "rules" \
        "qa.yml" "testing.yml" "documentation.yml" "security.yml" \
        "accessibility.yml" "performance.yml" "skill-generation.yml" \
        "release-readiness.yml"
    print_group "schemas" \
        "policy.schema.json" "rule.schema.json" \
        "project-class.schema.json" "priority.schema.json"
    print_group "fixtures" \
        "projects/open-source-cli.yml" "projects/private-saas.yml" \
        "projects/client-webapp.yml" "projects/ai-product.yml"
    print_group "tests" \
        "policy-schema.bats" "priority-resolution.bats" "privacy-tier.bats" \
        "merge-rules.bats" "project-classification.bats" "cost-limits.bats" \
        "evidence-requirements.bats"
    print_group "docs" \
        "policy-authoring.md" "project-classes.md" "priority-waterfall.md"
}

print_group() {
    group="$1"
    shift
    printf '  %s/\n' "$group"
    for item in "$@"; do
        printf '    %s/%s\n' "$group" "$item"
    done
}

bootstrap() {
    dry_run=0
    owner="berlinguyinca"
    governance_repo="autospec-governance"

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
                # Accepted for CLI compatibility with the full bootstrap shape,
                # but intentionally unused by this governance-only dry run.
                [ "$#" -ge 2 ] || fail "--observatory-repo requires a value"
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
    render_governance_dry_run "$owner" "$governance_repo"
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
