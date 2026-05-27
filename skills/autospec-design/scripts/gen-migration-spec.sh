#!/usr/bin/env bash
set -u

vendor="${1:-}"
repo_root="${2:-}"
if [ -z "$vendor" ] || [ -z "$repo_root" ]; then
    printf 'usage: gen-migration-spec.sh <vendor> <repo-root>\n' >&2
    exit 1
fi
if [ ! -d "$repo_root" ]; then
    printf 'gen-migration-spec: repo root not found: %s\n' "$repo_root" >&2
    exit 1
fi

script_dir="$(cd "$(dirname "$0")" && pwd)"
fetch_script="$script_dir/fetch-design-md.sh"
scan_script="$script_dir/scan-ui-sources.sh"

design_body="$(bash "$fetch_script" "$vendor")" || exit $?
inventory="$(bash "$scan_script" "$repo_root")" || exit $?
framework="$(printf '%s' "$inventory" | sed -n 's/^.*"framework":"\([^"]*\)".*$/\1/p')"
files_csv="$(printf '%s' "$inventory" | sed -n 's/^.*"files":\[\(.*\)\].*$/\1/p')"

files_tmp="$(mktemp)"
trap 'rm -f "$files_tmp"' EXIT
printf '%s\n' "$files_csv" \
    | tr ',' '\n' \
    | sed 's/^"//; s/"$//; /^$/d' > "$files_tmp"

if [ ! -s "$files_tmp" ]; then
    printf 'gen-migration-spec: no UI files found in %s; nothing to migrate.\n' "$repo_root" >&2
    exit 2
fi

today="$(date -u +%Y-%m-%d)"
spec_dir="$repo_root/docs/specs"
spec_path="$spec_dir/${today}-design-migration-${vendor}.md"
mkdir -p "$spec_dir"

{
    printf '# %s Design Migration To %s\n\n' "$today" "$vendor"
    printf '## Source\n\n'
    printf -- '- Vendor: `%s`\n' "$vendor"
    printf -- '- Catalog: `berlinguyinca/awesome-design-md/design-md/%s/DESIGN.md`\n' "$vendor"
    printf -- '- Framework detected: `%s`\n' "${framework:-unknown}"
    printf -- '- DESIGN.md bytes: `%s`\n\n' "$(printf '%s' "$design_body" | wc -c | tr -d ' ')"

    printf '## Target\n\n'
    printf 'Migrate the scanned UI surface in `%s` toward `%s` while preserving existing behavior.\n\n' \
        "$repo_root" "$vendor"
    printf 'Scanned files:\n'
    while IFS= read -r file; do
        printf -- '- `%s`\n' "$file"
    done < "$files_tmp"
    printf '\n'

    printf '## Team personality\n\n'
    printf 'Frontend/product migration team: frontend developer, UX designer, accessibility reviewer, API/backend developer, QA engineer. Emphasize incremental, reviewable UI changes with behavior-preserving tests.\n\n'

    printf '## Counter-team\n\n'
    printf 'Accessibility + visual regression reviewers: challenge contrast, keyboard flow, layout regressions, and over-broad rewrites. Review must stay scoped to the scanned UI files.\n\n'

    printf '## Per-component outline\n\n'
    while IFS= read -r file; do
        printf '### `%s`\n\n' "$file"
        printf -- '- Compare current structure against `%s` DESIGN.md guidance.\n' "$vendor"
        printf -- '- Apply the smallest visual-system change that preserves behavior.\n'
        printf -- '- Add or update focused UI regression coverage for this file.\n\n'
    done < "$files_tmp"

    printf '## Suggested decomposition\n\n'
    printf -- '- [ ] Create one child issue per 3-5 related UI files.\n'
    printf -- '- [ ] Keep accessibility and visual regression checks in every child issue.\n'
    printf -- '- [ ] Run `/autospec-define %s` to decompose this migration spec.\n' "$spec_path"
} > "$spec_path"

printf '%s\n' "$spec_path"
