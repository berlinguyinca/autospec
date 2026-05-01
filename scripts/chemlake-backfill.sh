#!/usr/bin/env bash
# scripts/chemlake-backfill.sh — one-shot backfill driver for
# metabolomics-us/chemlake. Deletes the deprecated `agent:local-safe` label,
# then prints the manual `/autospec-classify --apply-boards` invocation that
# the operator runs to retro-apply ctx:* / reasoning:* labels and assign
# GitHub Project boards.
#
# Usage:
#   scripts/chemlake-backfill.sh --dry-run     # safe preview, no side effects
#   scripts/chemlake-backfill.sh                # actually delete the label
#
# Expected scope (per spec): ~107 issues = 71 originally optimized + 36
# split children (#509-#545 minus #518); type:tracker issues are excluded
# automatically by autospec-classify.
#
# Authoritative source: docs/superpowers/specs/2026-04-30-autospec-model-fit-and-suite-design.md
# (in the chem-evidence repo) — section "Chemlake backfill plan (one-shot,
# after autospec-classify ships)" and "Deprecated label: agent:local-safe".

set -eu

REPO="metabolomics-us/chemlake"
DEPRECATED_LABEL="agent:local-safe"
DRY_RUN=0

err()  { printf 'error: %s\n' "$*" >&2; }
info() { printf '%s\n' "$*"; }

usage() {
    cat <<EOF
Usage: $0 [--dry-run] [--repo <owner/name>]

Deletes the deprecated '${DEPRECATED_LABEL}' label from every issue in
${REPO} and then deletes the label itself, then prints the manual
'/autospec-classify --apply-boards' invocation the operator runs from
their Claude/OpenCode/Codex harness.

Flags:
  --dry-run             list intended actions without making any changes.
  --repo <owner/name>   override the target repo (default: ${REPO}).
  -h, --help            show this help and exit.
EOF
}

while [ $# -gt 0 ]; do
    case "$1" in
        --dry-run) DRY_RUN=1 ;;
        --repo)
            shift
            REPO="${1:-}"
            ;;
        --repo=*)
            REPO="${1#--repo=}"
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            err "unknown arg: $1"
            usage >&2
            exit 2
            ;;
    esac
    shift
done

if ! command -v gh >/dev/null 2>&1; then
    err "gh CLI not found on PATH (https://cli.github.com)"
    exit 1
fi

info "chemlake backfill driver"
info "  repo:    $REPO"
info "  mode:    $([ "$DRY_RUN" -eq 1 ] && printf 'dry-run' || printf 'apply')"
info "  scope:   ~107 issues (71 originally optimized + 36 split children #509-#545 minus #518)"
info ""

info "Step 1 — strip '$DEPRECATED_LABEL' from issues that currently carry it."
if [ "$DRY_RUN" -eq 1 ]; then
    info "  [dry-run] would list issues:"
    info "    gh issue list --repo $REPO --label '$DEPRECATED_LABEL' --state all --limit 500 --json number,title"
    info "  [dry-run] then for each issue N: gh issue edit N --remove-label '$DEPRECATED_LABEL' --repo $REPO"
else
    # gh requires authentication — surface the error if missing.
    if ! gh auth status >/dev/null 2>&1; then
        err "gh is not authenticated. Run: gh auth login"
        exit 1
    fi
    issues_json="$(gh issue list --repo "$REPO" --label "$DEPRECATED_LABEL" --state all --limit 500 --json number 2>/dev/null || printf '[]')"
    count="$(printf '%s' "$issues_json" | grep -o '"number"' | wc -l | tr -d ' ')"
    info "  $count issue(s) currently carry the label."
    if [ "$count" -gt 0 ]; then
        printf '%s' "$issues_json" \
            | grep -oE '"number":[0-9]+' \
            | grep -oE '[0-9]+' \
            | while read -r n; do
                info "  removing $DEPRECATED_LABEL from #$n"
                gh issue edit "$n" --remove-label "$DEPRECATED_LABEL" --repo "$REPO" >/dev/null
            done
    fi
fi
info ""

info "Step 2 — delete the '$DEPRECATED_LABEL' label itself."
if [ "$DRY_RUN" -eq 1 ]; then
    info "  [dry-run] would: gh label delete '$DEPRECATED_LABEL' --repo $REPO --yes"
else
    if gh label list --repo "$REPO" --json name -q '.[].name' 2>/dev/null | grep -qx "$DEPRECATED_LABEL"; then
        gh label delete "$DEPRECATED_LABEL" --repo "$REPO" --yes
        info "  deleted label."
    else
        info "  label already absent — no-op."
    fi
fi
info ""

info "Step 3 — manual invocation (run this from your Claude/OpenCode/Codex harness):"
info ""
info "    /autospec-classify --apply-boards"
info ""
info "On first run, if ~/.autospec/project-map.yml is missing the skill will"
info "auto-init it with null project numbers and exit. Edit those numbers, then"
info "re-run /autospec-classify --apply-boards."
info ""
info "See docs/runbooks/chemlake-backfill.md for the full runbook."

exit 0
