#!/usr/bin/env bash
# autonomous-self-improvement.sh — deterministic low-hanging-fruit candidate source.
set -eu

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
    python3 - "$repo_root" <<'PY'
import json
import re
import sys
from pathlib import Path

root = Path(sys.argv[1]).resolve()


def emit(row):
    print(json.dumps(row, sort_keys=True))


def rel(path):
    return path.relative_to(root).as_posix()


commands_dir = root / "crates" / "autospec-cli" / "src" / "commands"
if commands_dir.is_dir():
    for path in sorted(commands_dir.glob("*.rs")):
        if path.name == "mod.rs":
            continue
        name = path.stem.replace("_", "-")
        text = path.read_text(encoding="utf-8")
        if "not_implemented(" in text:
            emit({
                "id": f"cli-stub-{name}",
                "workstream": "cli-productization",
                "title": f"Implement autospec {name} beyond the explicit stub",
                "severity": 3,
                "value": 4,
                "confidence": 1,
                "reversibility": 1,
                "effort": 3,
                "blast_radius": 2,
                "files": [rel(path), "docs/cli-reference.md"],
                "evidence": f"{rel(path)} calls not_implemented",
            })

reports = sorted((root / "docs" / "reports").glob("*.md")) if (root / "docs" / "reports").is_dir() else []
for report in reports:
    text = report.read_text(encoding="utf-8")
    in_risks = False
    for line in text.splitlines():
        if re.match(r"^## (Remaining Risks|Recommended handling|Next Human Action)", line):
            in_risks = True
            continue
        if in_risks and line.startswith("## "):
            in_risks = False
        if in_risks and line.startswith("- "):
            slug = re.sub(r"[^a-z0-9]+", "-", line[2:].lower()).strip("-")[:48] or "report-risk"
            emit({
                "id": f"report-risk-{slug}",
                "workstream": "report-risk",
                "title": line[2:].strip().rstrip("."),
                "severity": 2,
                "value": 3,
                "confidence": 0.8,
                "reversibility": 1,
                "effort": 2,
                "blast_radius": 1,
                "files": [rel(report)],
                "evidence": f"{rel(report)} risk bullet",
            })

if not (root / "scripts" / "autospec-run-events.sh").exists():
    emit({
        "id": "missing-run-events",
        "workstream": "operability",
        "title": "Add run event recording, explanation, and replay evidence",
        "severity": 4,
        "value": 5,
        "confidence": 1,
        "reversibility": 1,
        "effort": 2,
        "blast_radius": 1,
        "files": ["scripts/autospec-run-events.sh", "tests/autonomous/test_run_events.bats"],
        "evidence": "run black-box helper absent",
    })
PY
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
            title_goal="$(printf '%s' "$title" | sed 's/[.?!][[:space:]]*$//')"
            evidence="$(printf '%s' "$row" | jq -r '.evidence // ""')"
            files_plain="$(printf '%s' "$row" | jq -r '.files[:3][]')"
            files_inline="$(printf '%s' "$row" | jq -r '.files[:3] | map("`" + . + "`") | join(", ")')"
            first_file="$(printf '%s' "$files_plain" | sed '/^[[:space:]]*$/d' | head -n 1)"
            [ -n "$first_file" ] || first_file="scripts/autonomous-self-improvement.sh"
            {
                printf '## Goal\nResolve `%s`: %s.\n\n' "$first_file" "$title_goal"
                printf '## Files to read first\n'
                printf '%s\n' "$files_plain" | sed '/^[[:space:]]*$/d; s/^/- /'
                printf '\n## Implementation outline\n'
                printf '1. Inspect the evidence line and confirm the issue still reproduces.\n'
                printf '2. Make the smallest change in the files listed under `## Files touched`.\n'
                printf '3. Run the primary smoke test and record the result.\n\n'
                printf '## Tests required\n- bash scripts/validate.sh --fast --changed=origin/main\n\n'
                printf '## Dependencies\nnone\n\n'
                printf '## Files touched\n'
                printf '%s\n' "$files_plain" | sed '/^[[:space:]]*$/d; s/^/- /'
                printf '\n## Context\nAutoSpec discovered this deterministic self-improvement candidate while the autonomous queue was dry.\n\n'
                printf '## Evidence\n- %s\n- Files: %s\n\n' "$evidence" "$files_inline"
                printf '## Acceptance criteria\n'
                printf '%s\n' '- [ ] `bash scripts/validate.sh --fast --changed=origin/main` exits 0 after the change.'
                printf '%s\n\n' '- [ ] The final PR closes or supersedes this `needs-classify` issue.'
                printf '## Verification\n\n'
                printf '### Primary smoke test (inner loop)\n\n'
                printf '```bash\n'
                printf 'bash scripts/validate.sh --fast --changed=origin/main\n'
                printf '```\n\n'
                printf '### Operator/full verification\n\n'
                printf '```bash\n'
                printf 'bash scripts/validate.sh\n'
                printf '```\n'
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
