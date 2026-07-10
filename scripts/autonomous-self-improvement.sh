#!/usr/bin/env bash
# autonomous-self-improvement.sh — deterministic low-hanging-fruit candidate source.
set -eu

script_dir="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)"

usage() {
    cat <<'EOF'
Usage:
  autonomous-self-improvement.sh candidates [--repo-root DIR]
  autonomous-self-improvement.sh apply [--repo-root DIR] [--repo OWNER/REPO] [--apply] [--limit N]

GitHub writes require both --apply and AUTOSPEC_SELF_IMPROVEMENT_APPLY=1.
EOF
}

die() {
    printf 'autonomous-self-improvement: %s\n' "$*" >&2
    exit 2
}

cmd="${1:-}"
[ -n "$cmd" ] || { usage; exit 2; }
shift

repo_root="."
repo=""
apply_flag=0
limit=5
while [ "$#" -gt 0 ]; do
    case "$1" in
        --repo-root) repo_root="${2:-}"; shift 2 ;;
        --repo) repo="${2:-}"; shift 2 ;;
        --apply) apply_flag=1; shift ;;
        --limit) limit="${2:-}"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown arg: $1" ;;
    esac
done

[ -d "$repo_root" ] || die "--repo-root does not exist: $repo_root"
case "$limit" in *[!0-9]*|'') limit=5 ;; esac
[ "$limit" -gt 0 ] || limit=5

candidate_file() {
    mktemp -t autospec-self-improvement.XXXXXX
}

emit_candidates() {
    python3 "$script_dir/autonomous-self-improvement-candidates.py" "$repo_root"
}

case "$cmd" in
    candidates)
        emit_candidates
        ;;
    apply)
        tmp="$(candidate_file)"
        trap 'rm -f "$tmp" "$tmp.body" 2>/dev/null || true' EXIT
        emit_candidates > "$tmp"
        total="$(awk 'NF' "$tmp" | wc -l | tr -d ' ')"
        apply_enabled=0
        if [ "$apply_flag" = "1" ] && [ "${AUTOSPEC_SELF_IMPROVEMENT_APPLY:-}" = "1" ]; then
            apply_enabled=1
        fi
        if [ "$apply_enabled" != "1" ]; then
            jq -n --argjson candidates "${total:-0}" '{dry:true,filed:0,candidates:$candidates,reason:"report-only (set --apply and AUTOSPEC_SELF_IMPROVEMENT_APPLY=1 to file issues)"}'
            exit 0
        fi
        if [ -z "$repo" ]; then
            repo="$(gh repo view --json nameWithOwner -q .nameWithOwner 2>/dev/null || true)"
        fi
        [ -n "$repo" ] || die "--repo is required when gh cannot infer it"
        gh label create needs-classify --repo "$repo" --color cfd3d7 --force >/dev/null 2>&1 || true
        filed=0
        while IFS= read -r row; do
            [ -n "$row" ] || continue
            if [ "$filed" -ge "$limit" ]; then
                break
            fi
            title="$(printf '%s' "$row" | jq -r '.title')"
            evidence="$(printf '%s' "$row" | jq -r '.evidence // ""')"
            files="$(printf '%s' "$row" | jq -r '.files | map("`" + . + "`") | join(", ")')"
            {
                printf '## Goal\n%s.\n\n' "$title"
                printf '## Context\nAutoSpec discovered this deterministic self-improvement candidate while the autonomous queue was dry.\n\n'
                printf '## Evidence\n- %s\n- Files: %s\n\n' "$evidence" "$files"
                printf '## Suggested acceptance criteria\n- [ ] A focused implementation issue exists or this issue is classified into `auto-implement`.\n'
            } > "$tmp.body"
            gh issue create --repo "$repo" --title "$title" --body-file "$tmp.body" --label needs-classify >/dev/null
            filed=$((filed + 1))
        done < "$tmp"
        jq -n --argjson filed "$filed" --argjson candidates "${total:-0}" '{dry:($filed == 0),filed:$filed,candidates:$candidates,reason:"filed deterministic self-improvement candidates"}'
        ;;
    *)
        die "unknown command: $cmd"
        ;;
esac
