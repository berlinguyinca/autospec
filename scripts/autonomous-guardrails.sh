#!/usr/bin/env bash
# scripts/autonomous-guardrails.sh — deterministic parent guardrail helpers for
# autonomous merge safety. Issue #1543 intentionally keeps this as a small
# foundation; child issues deepen the individual mechanisms.

set -eu

SCRIPT_NAME="autonomous-guardrails"

usage() {
    cat <<'USAGE'
Usage:
  autonomous-guardrails.sh diff-guard --changed-files <file> [--lane implementer|verifier]
  autonomous-guardrails.sh mutation-guard --baseline <json> --current <json>
  autonomous-guardrails.sh blast-radius --changed-files <file> [--fenced-surfaces <yml>] [--json]
  autonomous-guardrails.sh provenance \
      --repo OWNER/REPO --pr N --changed-files <file> --gate-evidence <json> \
      --rollback-handle <ref-or-command> --out <json>

Decisions:
  diff-guard exits 1 with DECISION:block when an implementer changes test/eval-harness files.
  mutation-guard exits 1 with DECISION:block when current mutation score drops below baseline.
  blast-radius exits 1 with DECISION:quarantine when fenced/high-risk paths changed.
  provenance writes autospec.autonomous.merge_provenance.v1 JSON.
USAGE
}

die() { printf '%s: %s\n' "$SCRIPT_NAME" "$1" >&2; exit 2; }

require_file() {
    local path="$1" label="$2"
    [ -n "$path" ] || die "$label is required"
    [ -f "$path" ] || die "$label not found: $path"
}

read_changed_files() {
    local file="$1"
    sed '/^[[:space:]]*$/d; s#^\./##' "$file"
}

is_immutable_verifier_path() {
    case "$1" in
        tests/*|*/tests/*|test/*|*/test/*) return 0 ;;
        scripts/validate.sh|scripts/lint-issue.sh|scripts/lint-implementation.sh|scripts/verify-*|tests/verify-*|tests/fixtures/*) return 0 ;;
        skills/*/tests/*|schemas/eval-*|eval/*|evals/*|benchmarks/*) return 0 ;;
        *) return 1 ;;
    esac
}

default_fenced_surfaces_file() {
    if [ -n "${AUTOSPEC_FENCED_SURFACES_FILE:-}" ]; then
        printf '%s' "$AUTOSPEC_FENCED_SURFACES_FILE"
        return 0
    fi
    if [ -f .autospec/fenced-surfaces.yml ]; then
        printf '%s' .autospec/fenced-surfaces.yml
        return 0
    fi
    if [ -f .autospec/autospec.yml ]; then
        printf '%s' .autospec/autospec.yml
        return 0
    fi
    printf ''
}

# Legacy fallback stays in code so older repos without fenced_surfaces config
# remain fail-safe for known dangerous surfaces. New repos should configure
# fenced_surfaces in .autospec/autospec.yml or pass --fenced-surfaces.
is_high_risk_path() {
    case "$1" in
        scripts/autospec-autonomous.sh|scripts/autonomous-*.sh|scripts/autospec-autonomous-run-drain.sh) return 0 ;;
        scripts/worktree-guard.sh|scripts/claim-guard.sh|scripts/autospec-autonomy-gate.sh) return 0 ;;
        skills/autospec*/SKILL.md|skills/autospec*/codex/prompt.md|skills/autospec*/opencode/agent.md) return 0 ;;
        .github/workflows/*|install.sh|bootstrap.sh|uninstall.sh) return 0 ;;
        schemas/*|packages/*|crates/*|Cargo.toml|Cargo.lock) return 0 ;;
        trading-system/money/*|trading-system/money/**|trading-system/risk/*|trading-system/risk/**|trading-system/execution/*|trading-system/execution/**) return 0 ;;
        *migration*|*secret*|*auth*|*token*) return 0 ;;
        *) return 1 ;;
    esac
}

json_array_from_lines() {
    jq -R -s 'split("\n") | map(select(length > 0))'
}

cmd_diff_guard() {
    local changed_files="" lane="implementer"
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --changed-files) changed_files="${2:-}"; shift 2 ;;
            --lane) lane="${2:-}"; shift 2 ;;
            -h|--help) usage; exit 0 ;;
            *) die "diff-guard: unknown option: $1" ;;
        esac
    done
    require_file "$changed_files" "--changed-files"
    case "$lane" in
        implementer|verifier) ;;
        *) die "diff-guard: --lane must be implementer or verifier" ;;
    esac

    local tmp
    tmp="$(mktemp "${TMPDIR:-/tmp}/autonomous-diff-guard.XXXXXX")"
    while IFS= read -r path; do
        if is_immutable_verifier_path "$path"; then
            printf '%s\n' "$path" >> "$tmp"
        fi
    done <<EOF_CHANGED
$(read_changed_files "$changed_files")
EOF_CHANGED

    if [ -s "$tmp" ] && [ "$lane" != "verifier" ]; then
        printf 'DECISION:block\n'
        printf 'REASON:immutable_verifier_modified\n'
        sed 's/^/PATH:/' "$tmp"
        rm -f "$tmp"
        exit 1
    fi
    if [ -s "$tmp" ]; then
        printf 'DECISION:allow\n'
        printf 'REASON:verifier_lane_bypass\n'
        sed 's/^/PATH:/' "$tmp"
        rm -f "$tmp"
        return 0
    fi
    rm -f "$tmp"
    printf 'DECISION:allow\n'
}

cmd_mutation_guard() {
    local baseline="" current=""
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --baseline) baseline="${2:-}"; shift 2 ;;
            --current) current="${2:-}"; shift 2 ;;
            -h|--help) usage; exit 0 ;;
            *) die "mutation-guard: unknown option: $1" ;;
        esac
    done
    require_file "$baseline" "--baseline"
    require_file "$current" "--current"

    local baseline_score current_score mutants
    baseline_score="$(jq -r 'if has("score") then .score else ((.killed // 0) * 100 / (.total // 1) | floor) end' "$baseline")"
    current_score="$(jq -r 'if has("score") then .score else ((.killed // 0) * 100 / (.total // 1) | floor) end' "$current")"

    case "$baseline_score:$current_score" in
        *null*|*:*null*|*[^0-9.:]*) die "mutation-guard: scores must be numeric or derivable from killed/total" ;;
    esac

    if awk "BEGIN { exit !($current_score < $baseline_score) }"; then
        printf 'DECISION:block\n'
        printf 'REASON:mutation_score_regression\n'
        printf 'BASELINE:%s\n' "$baseline_score"
        printf 'CURRENT:%s\n' "$current_score"
        mutants="$(jq -r '
            (.surviving_mutants // .survivors // .mutants // [])[]
            | select((.status // "survived") != "killed")
            | "MUTANT:" + ((.id // .name // "unknown")|tostring)
              + ":" + ((.file // .path // "unknown")|tostring)
              + ":" + ((.line // 0)|tostring)
              + ":" + ((.description // .mutator // .operator // "surviving mutant")|tostring)
        ' "$current")"
        if [ -n "$mutants" ]; then
            printf '%s\n' "$mutants"
        else
            printf 'MUTANT:unknown:unknown:0:mutation score regressed without survivor details\n'
        fi
        exit 1
    fi

    printf 'DECISION:allow\n'
    printf 'BASELINE:%s\n' "$baseline_score"
    printf 'CURRENT:%s\n' "$current_score"
}

cmd_blast_radius() {
    local changed_files="" fenced_surfaces="" json=0
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --changed-files) changed_files="${2:-}"; shift 2 ;;
            --fenced-surfaces) fenced_surfaces="${2:-}"; shift 2 ;;
            --json) json=1; shift ;;
            -h|--help) usage; exit 0 ;;
            *) die "blast-radius: unknown option: $1" ;;
        esac
    done
    require_file "$changed_files" "--changed-files"
    if [ -z "$fenced_surfaces" ]; then
        fenced_surfaces="$(default_fenced_surfaces_file)"
    fi

    local result status
    result="$(blast_radius_classification_json "$changed_files" "$fenced_surfaces")"
    status="$(printf '%s' "$result" | jq -r '.exit_status')"
    if [ "$json" -eq 1 ]; then
        printf '%s\n' "$result" | jq 'del(.exit_status)'
        exit "$status"
    fi
    if [ "$status" -ne 0 ]; then
        printf 'DECISION:quarantine\n'
        printf 'REASON:%s\n' "$(printf '%s' "$result" | jq -r '.reason')"
        printf '%s\n' "$result" | jq -r '.fenced_matches[]? | "SURFACE:" + .surface + ":" + .path + ":" + .reason'
        printf '%s\n' "$result" | jq -r '.paths[]? | "PATH:" + .'
        exit 1
    fi
    printf 'DECISION:allow\n'
    printf 'LABEL:%s\n' "$(printf '%s' "$result" | jq -r '.label')"
}

blast_radius_classification_json() {
    local changed_files="$1" fenced_surfaces="${2:-}"
    python3 - "$changed_files" "$fenced_surfaces" <<'PY_CLASSIFIER'
import fnmatch
import json
import re
import sys
from pathlib import Path

changed_path = Path(sys.argv[1])
config_path = Path(sys.argv[2]) if len(sys.argv) > 2 and sys.argv[2] else None
paths = []
for line in changed_path.read_text(encoding="utf-8").splitlines():
    item = line.strip()
    if not item:
        continue
    if item.startswith("./"):
        item = item[2:]
    paths.append(item)

legacy = [
    {"id":"autonomous-control-plane","severity":"fenced","reason":"autonomous conductor or guardrail control plane","paths":["scripts/autospec-autonomous.sh","scripts/autonomous-*.sh","scripts/autospec-autonomous-run-drain.sh","scripts/worktree-guard.sh","scripts/claim-guard.sh","scripts/autospec-autonomy-gate.sh"]},
    {"id":"skill-contracts","severity":"high","reason":"autospec skill public contracts","paths":["skills/autospec*/SKILL.md","skills/autospec*/codex/prompt.md","skills/autospec*/opencode/agent.md"]},
    {"id":"release-and-ci","severity":"high","reason":"release, install, or CI surface","paths":[".github/workflows/*","install.sh","bootstrap.sh","uninstall.sh"]},
    {"id":"schema-package-core","severity":"high","reason":"schema/package/crate core surface","paths":["schemas/*","packages/*","crates/*","Cargo.toml","Cargo.lock"]},
    {"id":"trading-money-risk","severity":"fenced","reason":"trading system money/risk/execution paths","paths":["trading-system/money/**","trading-system/risk/**","trading-system/execution/**"]},
    {"id":"sensitive-keywords","severity":"high","reason":"migration/auth/secret/token path keyword","paths":["*migration*","*secret*","*auth*","*token*"]},
]

def parse_scalar(value):
    value = value.strip()
    if (value.startswith('"') and value.endswith('"')) or (value.startswith("'") and value.endswith("'")):
        return value[1:-1]
    return value

def load_registry(path):
    if not path or not path.exists():
        return []
    rows = []
    active = False
    base_indent = 0
    cur = None
    in_paths = False
    for raw in path.read_text(encoding="utf-8").splitlines():
        if not raw.strip() or raw.lstrip().startswith('#'):
            continue
        indent = len(raw) - len(raw.lstrip(' '))
        stripped = raw.strip()
        if not active:
            if re.match(r'^fenced_surfaces\s*:', stripped):
                active = True
                base_indent = indent
            continue
        if indent <= base_indent and not re.match(r'^fenced_surfaces\s*:', stripped):
            break
        if re.match(r'^-\s+id\s*:', stripped):
            if cur:
                rows.append(cur)
            cur = {"id": parse_scalar(stripped.split(':',1)[1]), "severity":"high", "reason":"configured fenced surface", "paths":[]}
            in_paths = False
            continue
        if cur is None:
            continue
        if re.match(r'^paths\s*:', stripped):
            in_paths = True
            continue
        if in_paths and stripped.startswith('- '):
            cur["paths"].append(parse_scalar(stripped[2:]))
            continue
        if ':' in stripped:
            key, val = stripped.split(':', 1)
            key = key.strip()
            val = parse_scalar(val)
            if key in {"id", "severity", "reason"}:
                cur[key] = val
            in_paths = False
    if cur:
        rows.append(cur)
    return [r for r in rows if r.get("id") and r.get("paths")]

registry = load_registry(config_path) or legacy
matches = []
for path in paths:
    for surface in registry:
        for pattern in surface.get("paths", []):
            pat = str(pattern).lstrip('./')
            if fnmatch.fnmatch(path, pat) or (pat.endswith('/**') and path.startswith(pat[:-3].rstrip('/') + '/')):
                matches.append({"path": path, "surface": surface.get("id", "unknown"), "severity": surface.get("severity", "high"), "reason": surface.get("reason", "configured fenced surface"), "pattern": pat})
                break

fenced = bool(matches)
if fenced:
    label = "blast:fenced" if any(m.get("severity") == "fenced" for m in matches) else "blast:high"
    decision = "quarantine"
    reason = "fenced_surface"
    exit_status = 1
else:
    if len(set(p.split('/')[0] for p in paths)) > 3 or len(paths) > 10:
        label = "blast:medium"
    else:
        label = "blast:low"
    decision = "allow"
    reason = None
    exit_status = 0
reversibility = "reversible" if not any(re.search(r'(migration|schema|auth|secret|token)', p, re.I) for p in paths) else "requires-review"
print(json.dumps({
    "decision": decision,
    "reason": reason,
    "label": label,
    "fenced": fenced,
    "reversibility": reversibility,
    "paths": paths,
    "fenced_matches": matches,
    "registry": str(config_path) if config_path else "legacy-defaults",
    "exit_status": exit_status,
}, sort_keys=True))
PY_CLASSIFIER
}
blast_radius_json() {
    local changed_files="$1" fenced_surfaces="${2:-}"
    if [ -z "$fenced_surfaces" ]; then
        fenced_surfaces="$(default_fenced_surfaces_file)"
    fi
    blast_radius_classification_json "$changed_files" "$fenced_surfaces" | jq 'del(.exit_status)'
}
cmd_provenance() {
    local repo="" pr="" changed_files="" gate_evidence="" rollback_handle="" out=""
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --repo) repo="${2:-}"; shift 2 ;;
            --pr) pr="${2:-}"; shift 2 ;;
            --changed-files) changed_files="${2:-}"; shift 2 ;;
            --gate-evidence) gate_evidence="${2:-}"; shift 2 ;;
            --rollback-handle) rollback_handle="${2:-}"; shift 2 ;;
            --out) out="${2:-}"; shift 2 ;;
            -h|--help) usage; exit 0 ;;
            *) die "provenance: unknown option: $1" ;;
        esac
    done
    [ -n "$repo" ] || die "provenance: --repo is required"
    [ -n "$pr" ] || die "provenance: --pr is required"
    [ -n "$rollback_handle" ] || die "provenance: --rollback-handle is required"
    [ -n "$out" ] || die "provenance: --out is required"
    require_file "$changed_files" "--changed-files"
    require_file "$gate_evidence" "--gate-evidence"

    local changed_json evidence_json blast_json generated_at
    changed_json="$(read_changed_files "$changed_files" | json_array_from_lines)"
    evidence_json="$(cat "$gate_evidence")"
    blast_json="$(blast_radius_json "$changed_files")"
    generated_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

    mkdir -p "$(dirname "$out")"
    jq -n \
        --arg schema 'autospec.autonomous.merge_provenance.v1' \
        --arg repo "$repo" \
        --argjson pr "$pr" \
        --arg generated_at "$generated_at" \
        --arg rollback_handle "$rollback_handle" \
        --argjson changed_files "$changed_json" \
        --argjson gate_evidence "$evidence_json" \
        --argjson blast_radius "$blast_json" \
        '{schema:$schema, repo:$repo, pr:$pr, generated_at:$generated_at, rollback_handle:$rollback_handle, changed_files:$changed_files, gate_evidence:$gate_evidence, blast_radius:$blast_radius}' \
        > "$out"
    printf 'DECISION:provenance-written\n'
    printf 'PROVENANCE:%s\n' "$out"
}

main() {
    [ "$#" -gt 0 ] || { usage >&2; exit 2; }
    local sub="$1"; shift
    case "$sub" in
        diff-guard) cmd_diff_guard "$@" ;;
        mutation-guard) cmd_mutation_guard "$@" ;;
        blast-radius) cmd_blast_radius "$@" ;;
        provenance) cmd_provenance "$@" ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown subcommand: $sub" ;;
    esac
}

main "$@"
