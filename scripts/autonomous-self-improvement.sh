#!/usr/bin/env bash
# autonomous-self-improvement.sh — deterministic low-hanging-fruit candidate source.
set -eu
usage() {
    cat <<'EOF'
Usage:
  autonomous-self-improvement.sh candidates [--repo-root DIR] [review evidence options]
  autonomous-self-improvement.sh apply [--repo-root DIR] [--repo OWNER/REPO] [--apply] [--limit N] [review evidence options]
  autonomous-self-improvement.sh evaluate --candidate FILE --experiment-proof FILE [--rollback-proof FILE] [review evidence options] --rollback-digest DIGEST
  autonomous-self-improvement.sh advance [--repo-root DIR] [review evidence options]
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
review_outcomes=""
gaps=""
learning_ledger=""
lifecycle_ledger=""
candidate=""
rollback_digest=""
experiment_proof=""
rollback_proof=""
evidence_dir=""
while [ "$#" -gt 0 ]; do
    case "$1" in
        --repo-root) repo_root="${2:-}"; shift 2 ;;
        --repo) repo="${2:-}"; shift 2 ;;
        --apply) apply_flag=1; shift ;;
        --limit) limit="${2:-}"; shift 2 ;;
        --review-outcomes) review_outcomes="${2:-}"; shift 2 ;;
        --gaps) gaps="${2:-}"; shift 2 ;;
        --learning-ledger) learning_ledger="${2:-}"; shift 2 ;;
        --lifecycle-ledger) lifecycle_ledger="${2:-}"; shift 2 ;;
        --candidate) candidate="${2:-}"; shift 2 ;;
        --rollback-digest) rollback_digest="${2:-}"; shift 2 ;;
        --experiment-proof) experiment_proof="${2:-}"; shift 2 ;;
        --rollback-proof) rollback_proof="${2:-}"; shift 2 ;;
        --evidence-dir) evidence_dir="${2:-}"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown arg: $1" ;;
    esac
done
[ -d "$repo_root" ] || die "--repo-root does not exist: $repo_root"
case "$limit" in *[!0-9]*|'') limit=5 ;; esac
[ "$limit" -gt 0 ] || limit=5
review_outcomes="${review_outcomes:-$repo_root/.autospec/review-outcomes.jsonl}"
gaps="${gaps:-$repo_root/.autospec/gaps.json}"
learning_ledger="${learning_ledger:-$repo_root/.autospec/review-learning.jsonl}"
lifecycle_ledger="${lifecycle_ledger:-$repo_root/.autospec/review-policy-lifecycle.jsonl}"
evidence_dir="${evidence_dir:-$repo_root/.autospec/self-improvement-evidence}"
candidate_file() {
    mktemp -t autospec-self-improvement.XXXXXX
}
emit_candidates() {
    python3 - "$repo_root" "$review_outcomes" "$gaps" "$learning_ledger" "$lifecycle_ledger" <<'PY'
import hashlib
import json
import re
import shlex
import sys
from pathlib import Path
root = Path(sys.argv[1]).resolve()
outcomes_path = Path(sys.argv[2])
gaps_path = Path(sys.argv[3])
learning_path = Path(sys.argv[4])
lifecycle_path = Path(sys.argv[5])
QUESTION_TEXTS = [
    "Which invariant failed after review admission?",
    "Which producer/consumer boundary was omitted?",
    "What executable test would have falsified approval before merge?",
    "Is the escape correlated with reviewer provider, reasoning, risk, or missing evidence?",
    "What is the smallest reusable policy or test change?",
    "What legitimate change could the proposal falsely block?",
    "What sample and metric would justify promotion?",
    "What exact rollback restores the prior policy?",
]
def digest(value):
    payload = json.dumps(value, sort_keys=True, separators=(",", ":")).encode()
    return "sha256:" + hashlib.sha256(payload).hexdigest()
def read_json(path, default):
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return default
def read_jsonl(path):
    rows = []
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
    except OSError:
        return rows
    for line in lines:
        try:
            row = json.loads(line)
            if isinstance(row, dict):
                rows.append(row)
        except json.JSONDecodeError:
            continue
    return rows
def append_jsonl(path, row):
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a", encoding="utf-8") as handle:
        handle.write(json.dumps(row, sort_keys=True) + "\n")
def decorate(row):
    files = list(dict.fromkeys(row.get("files") or ["scripts/autonomous-self-improvement.sh"]))[:3]
    evidence = row.get("evidence") or "repository-local deterministic signal"
    invariant = row.get("failed_invariant") or f"{row.get('workstream', 'repository')} work has executable completion evidence"
    consumer = row.get("named_consumer") or files[0]
    dedupe = row.get("dedupe_key") or row.get("id") or digest([invariant, consumer])
    rollback = row.get("rollback") or {
        "command": "git revert --no-edit <policy-change-commit>",
        "prior_policy_digest": digest({"dedupe_key": dedupe, "state": "prior-policy"}),
    }
    answers = [
        invariant, consumer, f"run the declared falsifier against {files[0]}",
        row.get("correlation", "repository evidence determines the correlation"),
        row.get("smallest_change", f"change only {', '.join(files)}"),
        row.get("false_block_risk", "a legitimate change matching the same surface"),
        f"{row.get('sample_floor', 1)} samples on {row.get('metric', 'validation_pass_rate')}",
        rollback["command"],
    ]
    row.update({
        "files": files, "evidence": evidence, "failed_invariant": invariant,
        "named_consumer": consumer,
        "falsifier": row.get("falsifier") or {"command": f"test -e {files[0]}"},
        "dedupe_key": dedupe,
        "before_after": row.get("before_after") or {
            "before": {"validation_pass_rate": 0}, "after": {"validation_pass_rate": 1}},
        "sample_floor": row.get("sample_floor", 1),
        "max_cost_regression": row.get("max_cost_regression", 0.1),
        "rollback": rollback, "change_class": row.get("change_class", "neutral"),
        "questions": [{"question": q, "answer": a} for q, a in zip(QUESTION_TEXTS, answers)],
    })
    return row
def emit(row):
    print(json.dumps(decorate(row), sort_keys=True))
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
all_outcomes = read_jsonl(outcomes_path)
superseded = {row.get("supersedes_outcome_digest") for row in all_outcomes
              if row.get("supersedes_outcome_digest")}
outcomes = [row for row in all_outcomes if row.get("outcome_digest") not in superseded]
gaps = read_json(gaps_path, [])
if not isinstance(gaps, list):
    gaps = []
lifecycle = read_jsonl(lifecycle_path)
learning = read_jsonl(learning_path)
seen_clusters = set()
for outcome in outcomes:
    attributed = all(outcome.get(key) is not None for key in (
        "pr", "commit", "review_receipt_digest", "reviewer_harness",
        "reviewer_reasoning", "provider_diversified", "review_risk"))
    if not attributed or int(outcome.get("escaped_high_severity", 0) or 0) < 1:
        continue
    matching = [gap for gap in gaps if isinstance(gap, dict) and
                gap.get("attribution_status") == "attributed" and
                gap.get("review_receipt_digest") == outcome["review_receipt_digest"] and
                gap.get("severity") in ("high", "critical", "blocker")]
    for gap in matching:
        key = "review-escape-" + re.sub(r"[^a-z0-9]+", "-", gap.get("dedupe_key", gap.get("gap_id", "gap")).lower()).strip("-")
        if key in seen_clusters:
            continue
        seen_clusters.add(key)
        previous = [int(row.get("frequency", 0) or 0) for row in lifecycle + learning
                    if row.get("dedupe_key") == key]
        frequency = max(previous or [0]) + 1
        change_class = gap.get("proposed_change_class", "strengthening")
        prior_policy = digest({
            "reviewer_harness": outcome["reviewer_harness"],
            "reviewer_reasoning": outcome["reviewer_reasoning"],
            "provider_diversified": outcome["provider_diversified"],
            "review_risk": outcome["review_risk"],
        })
        candidate = decorate({
            "id": key, "dedupe_key": key, "workstream": "review-policy",
            "title": f"Strengthen review policy for {gap.get('title', key)}",
            "severity": 5, "value": 5, "confidence": 1, "reversibility": 1,
            "effort": 2, "blast_radius": 2, "frequency": frequency,
            "files": [gap.get("file") or "scripts/autonomous-self-improvement.sh",
                      "tests/autonomous/test_review_escape_learning.bats"],
            "evidence": [
                {"path": str(outcomes_path), "outcome_digest": outcome.get("outcome_digest")},
                {"path": str(gaps_path), "gap_id": gap.get("gap_id"),
                 "review_receipt_digest": outcome["review_receipt_digest"]},
            ],
            "failed_invariant": gap.get("failed_invariant") or gap.get("title"),
            "named_consumer": gap.get("named_consumer") or gap.get("file"),
            "falsifier": {"command":
                "bash ${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/advisor-observe.sh "
                f"--outcomes {shlex.quote(str(outcomes_path))} --json | "
                "jq -e '.attributed_reviewed_prs >= 20 and .escaped_high_rate == 0'"},
            "metric": "escaped_high_rate",
            "before_after": {
                "before": {"escaped_high_rate": outcome.get("escaped_high_severity", 0),
                           "escaped_total_rate": outcome.get("escaped_total", 0),
                           "cost_per_reviewed_pr": outcome.get("review_cost", 0)},
                "after": {"escaped_high_rate": 0,
                          "escaped_total_rate": outcome.get("escaped_total", 0),
                          "cost_per_reviewed_pr": outcome.get("review_cost", 0)},
            },
            "sample_floor": 20, "max_cost_regression": 0.1,
            "rollback": {"command": "git revert --no-edit <policy-change-commit>",
                         "prior_policy_digest": prior_policy},
            "change_class": change_class,
            "correlation": f"{outcome['reviewer_harness']}/{outcome['reviewer_reasoning']}/{outcome['review_risk']}",
            "false_block_risk": "a legitimate integration change using an alternate consumer path",
        })
        existing_candidate = any(row.get("dedupe_key") == key and row.get("state") == "candidate"
                                 for row in lifecycle)
        if not existing_candidate:
            append_jsonl(lifecycle_path, {**candidate, "state": "candidate"})
        else:
            append_jsonl(learning_path, {"event": "frequency_observed", "dedupe_key": key,
                                        "frequency": frequency,
                                        "evidence_digest": outcome.get("outcome_digest")})
        print(json.dumps(candidate, sort_keys=True))
PY
}
evaluate_candidate() {
    [ -f "$candidate" ] || die "--candidate file is required for evaluate"
    args=(evaluate --repo-root "$repo_root" --candidate "$candidate" \
        --review-outcomes "$review_outcomes" --lifecycle-ledger "$lifecycle_ledger" \
        --rollback-digest "$rollback_digest")
    [ -n "$experiment_proof" ] && args+=(--experiment-proof "$experiment_proof")
    [ -n "$rollback_proof" ] && args+=(--rollback-proof "$rollback_proof")
    python3 "$(dirname "${BASH_SOURCE[0]}")/autonomous-self-improvement-evaluate" "${args[@]}"
}
advance_candidates() {
    python3 "$(dirname "${BASH_SOURCE[0]}")/autonomous-self-improvement-evaluate" advance \
        --repo-root "$repo_root" --review-outcomes "$review_outcomes" \
        --lifecycle-ledger "$lifecycle_ledger" --evidence-dir "$evidence_dir"
}
case "$cmd" in
    candidates)
        emit_candidates
        ;;
    evaluate)
        evaluate_candidate
        ;;
    advance)
        advance_candidates
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
        filed=0
        report_only=0
        labels_ready=0
        while IFS= read -r row; do
            [ -n "$row" ] || continue
            change_class="$(printf '%s' "$row" | jq -r '.change_class // "neutral"')"
            if [ "$change_class" = "weakening" ]; then
                report_only=$((report_only + 1))
                continue
            fi
            if [ "$filed" -ge "$limit" ]; then
                break
            fi
            if [ "$labels_ready" -eq 0 ]; then
                gh label create needs-classify --repo "$repo" --color cfd3d7 --force >/dev/null 2>&1 || true
                gh label create origin:self --repo "$repo" --color 8250df --force >/dev/null 2>&1 || true
                labels_ready=1
            fi
            title="$(printf '%s' "$row" | jq -r '.title')"
            title_goal="$(printf '%s' "$title" | sed 's/[.?!][[:space:]]*$//')"
            evidence="$(printf '%s' "$row" | jq -r '(.evidence // "") | if type == "string" then . else tojson end')"
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
                printf '## Tests required\n- autospec validate --fast --changed=origin/main\n\n'
                printf '## Dependencies\nnone\n\n'
                printf '## Files touched\n'
                printf '%s\n' "$files_plain" | sed '/^[[:space:]]*$/d; s/^/- /'
                printf '\n## Context\nAutoSpec discovered this deterministic self-improvement candidate while the autonomous queue was dry.\n\n'
                printf '## Evidence\n- %s\n- Files: %s\n\n' "$evidence" "$files_inline"
                printf '## Acceptance criteria\n'
                printf '%s\n' '- [ ] `autospec validate --fast --changed=origin/main` exits 0 after the change.'
                printf '%s\n\n' '- [ ] The final PR closes or supersedes this `needs-classify` issue.'
                printf '## Verification\n\n'
                printf '### Primary smoke test (inner loop)\n\n'
                printf '```bash\n'
                printf 'autospec validate --fast --changed=origin/main\n'
                printf '```\n\n'
                printf '### Operator/full verification\n\n'
                printf '```bash\n'
                printf 'autospec validate\n'
                printf '```\n'
            } > "$tmp.body"
            gh issue create --repo "$repo" --title "$title" --body-file "$tmp.body" --label needs-classify --label origin:self >/dev/null
            filed=$((filed + 1))
        done < "$tmp"
        jq -n --argjson filed "$filed" --argjson candidates "${total:-0}" \
          --argjson report_only "$report_only" \
          '{dry:($filed == 0),filed:$filed,candidates:$candidates,report_only:$report_only,reason:"filed deterministic self-improvement candidates"}'
        ;;
    *)
        die "unknown command: $cmd"
        ;;
esac
