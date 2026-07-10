#!/usr/bin/env bash
# scripts/autonomous-promote-open-issues.sh — Tier 1.5 backlog-grooming orchestrator.
#
# Grooms genuinely-ready open issues into the `auto-implement` queue so the
# autonomous conductor's Tier 1.5 does real work. This is the orchestrator that
# stitches together the deterministic grooming pipeline (Tasks 1-5):
#
#   list-groomable.sh   → candidate set (needs-classify|needs-template|unlabeled)
#   lint-issue-safety.sh→ per-candidate intent safety gate (PASS|AMBIGUOUS|BLOCK)
#   classify-model-fit.sh→ ctx:/reasoning: model-fit labels
#   promote-eligibility.sh→ eligible|needs-template|epic|hold routing decision
#   grooming-govern.sh  → self-governance active-gate set (template-promote?)
#   grooming-config.sh  → policy (auto|on|off) + budget
#
# Per candidate (SAFETY-CRITICAL — fail-closed everywhere; a pipeline error must
# never cause a promotion):
#   1. safety gate: AMBIGUOUS/BLOCK → security:quarantined + audit + STOP issue;
#      only PASS proceeds. Any indeterminate/error → skip (never promote).
#   2. classify (unlabeled/needs-classify): add ctx:/reasoning:, drop needs-classify.
#   3. eligibility:
#        eligible       → promote now (add auto-implement, drop needs-autospec-template)
#        needs-template → promote ONLY if `template-promote` ∈ govern active set
#                         (route:groom for the loop), ELSE hold:needs-human
#        epic           → route:split (do NOT decompose here)
#        hold / error   → hold:needs-human
#   4. every mutation posts an audit comment stating decision + reason.
#
# POLICY GATE (replaces the old AUTOSPEC_PROMOTE_OPEN_ISSUES_APPLY double-env-gate):
# mutations happen ONLY when `--apply` is passed AND grooming policy ∈ auto|on.
# `off` (or no --apply) → dry JSON, ZERO mutations. Policy resolves via
# grooming-config.sh, which honors the AUTOSPEC_GROOMING_POLICY env override for
# CI/tests. When in doubt, SKIP: under-promotion is far safer than feeding
# ill-formed work to an admin-auto-merge loop.
#
# Usage:
#   autonomous-promote-open-issues.sh [--repo OWNER/REPO] [--apply]
#   autonomous-promote-open-issues.sh --help
#
# Output: a single JSON object on stdout (back-compat envelope + breakdowns):
#   {"dry":<bool>,"filed":<int>,"promoted":[<#>...],
#    "skipped":[{"issue":<#>,"reason":"..."}],
#    "held":[{"issue":<#>,"reason":"..."}],
#    "quarantined":[{"issue":<#>,"reason":"..."}],
#    "routed":[{"issue":<#>,"action":"...","reason":"..."}],
#    "reason":"..."}
#
# Exit codes:
#   0  — success (report-only or apply)
#   2  — usage error

set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

# ── Sub-script paths (injectable seams for CI/test stubs) ──────────────────────
SHARED_DIR="$SCRIPT_DIR/../skills/autospec-shared/scripts"
GROOM_LIST="${AUTOSPEC_GROOM_LIST_SCRIPT:-$SCRIPT_DIR/list-groomable.sh}"
GROOM_SAFETY="${AUTOSPEC_GROOM_SAFETY_SCRIPT:-$SCRIPT_DIR/lint-issue-safety.sh}"
GROOM_CLASSIFY="${AUTOSPEC_GROOM_CLASSIFY_SCRIPT:-$SCRIPT_DIR/classify-model-fit.sh}"
GROOM_ELIGIBILITY="${AUTOSPEC_GROOM_ELIGIBILITY_SCRIPT:-$SCRIPT_DIR/promote-eligibility.sh}"
GROOM_GOVERN="${AUTOSPEC_GROOM_GOVERN_SCRIPT:-$SHARED_DIR/grooming-govern.sh}"
GROOM_CONFIG="${AUTOSPEC_GROOM_CONFIG_SCRIPT:-$SHARED_DIR/grooming-config.sh}"

usage() {
    cat <<'EOF'
autonomous-promote-open-issues.sh — Tier 1.5 backlog-grooming orchestrator

Grooms ready open issues into the `auto-implement` queue via the deterministic
safety → classify → eligibility → promote/groom/split/hold pipeline. Mutates
GitHub state only with --apply AND grooming policy in {auto,on}; otherwise
report-only (dry). Policy resolves via grooming-config.sh.

Usage:
  autonomous-promote-open-issues.sh [--repo OWNER/REPO] [--apply]
  autonomous-promote-open-issues.sh --help

Emits one JSON object:
  {"dry":<bool>,"filed":<int>,"promoted":[...],"skipped":[...],
   "held":[...],"quarantined":[...],"routed":[...],"reason":"..."}
EOF
}

die() {
    printf 'autonomous-promote-open-issues: %s\n' "$1" >&2
    exit 2
}

repo=""
apply_flag=0

while [ "$#" -gt 0 ]; do
    case "$1" in
        --repo) repo="${2:-}"; shift 2 ;;
        --apply) apply_flag=1; shift ;;
        --help|-h) usage; exit 0 ;;
        *) die "unknown option: $1" ;;
    esac
done

# ── Resolve repo ──────────────────────────────────────────────────────────────
if [ -z "$repo" ]; then
    repo="$(gh repo view --json nameWithOwner --jq .nameWithOwner 2>/dev/null || true)"
fi
[ -n "$repo" ] || die "--repo is required when gh cannot infer it"

# ── Policy gate: apply iff --apply AND policy ∈ {auto,on} ──────────────────────
policy="$(bash "$GROOM_CONFIG" --key policy 2>/dev/null || printf 'auto')"
[ -n "$policy" ] || policy="auto"

apply_enabled=0
if [ "$apply_flag" = "1" ]; then
    case "$policy" in
        auto|on) apply_enabled=1 ;;
    esac
fi

# ── Budget ────────────────────────────────────────────────────────────────────
budget="$(bash "$GROOM_CONFIG" --key budget.max_issues_per_cycle 2>/dev/null || printf '5')"
case "$budget" in
    ''|*[!0-9]*) budget=5 ;;
esac

# ── Accumulators ──────────────────────────────────────────────────────────────
promoted_nums=""
SKIPPED_FILE="$(mktemp -t promote-skipped.XXXXXX)"
HELD_FILE="$(mktemp -t promote-held.XXXXXX)"
QUARANTINED_FILE="$(mktemp -t promote-quar.XXXXXX)"
ROUTED_FILE="$(mktemp -t promote-routed.XXXXXX)"
BODY_FILE="$(mktemp -t promote-body.XXXXXX)"
trap 'rm -f "$SKIPPED_FILE" "$HELD_FILE" "$QUARANTINED_FILE" "$ROUTED_FILE" "$BODY_FILE"' EXIT
printf '[]\n' > "$SKIPPED_FILE"
printf '[]\n' > "$HELD_FILE"
printf '[]\n' > "$QUARANTINED_FILE"
printf '[]\n' > "$ROUTED_FILE"

record_skip() {
    jq -c --argjson issue "$1" --arg reason "$2" '. + [{issue:$issue, reason:$reason}]' \
        "$SKIPPED_FILE" > "$SKIPPED_FILE.tmp" && mv "$SKIPPED_FILE.tmp" "$SKIPPED_FILE"
}
record_held() {
    jq -c --argjson issue "$1" --arg reason "$2" '. + [{issue:$issue, reason:$reason}]' \
        "$HELD_FILE" > "$HELD_FILE.tmp" && mv "$HELD_FILE.tmp" "$HELD_FILE"
}
record_quarantine() {
    jq -c --argjson issue "$1" --arg reason "$2" '. + [{issue:$issue, reason:$reason}]' \
        "$QUARANTINED_FILE" > "$QUARANTINED_FILE.tmp" && mv "$QUARANTINED_FILE.tmp" "$QUARANTINED_FILE"
}
record_routed() {
    jq -c --argjson issue "$1" --arg action "$2" --arg reason "$3" \
        '. + [{issue:$issue, action:$action, reason:$reason}]' \
        "$ROUTED_FILE" > "$ROUTED_FILE.tmp" && mv "$ROUTED_FILE.tmp" "$ROUTED_FILE"
}

emit_json() {
    # $1 = dry (true|false), $2 = reason
    dry="$1"; reason="$2"
    filed=0
    promoted_json="[]"
    if [ -n "$promoted_nums" ]; then
        promoted_json="$(printf '%s\n' $promoted_nums | jq -R 'tonumber' | jq -s .)"
        filed="$(printf '%s\n' $promoted_nums | awk 'NF' | wc -l | tr -d ' ')"
    fi
    jq -n \
        --argjson dry "$dry" \
        --argjson filed "$filed" \
        --argjson promoted "$promoted_json" \
        --slurpfile skipped "$SKIPPED_FILE" \
        --slurpfile held "$HELD_FILE" \
        --slurpfile quarantined "$QUARANTINED_FILE" \
        --slurpfile routed "$ROUTED_FILE" \
        --arg reason "$reason" \
        '{dry:$dry, filed:$filed, promoted:$promoted,
          skipped:$skipped[0], held:$held[0],
          quarantined:$quarantined[0], routed:$routed[0], reason:$reason}'
}

# ── Fetch candidate set from the deterministic selector ───────────────────────
list_json="$(bash "$GROOM_LIST" --repo "$repo" --budget "$budget" 2>/dev/null || printf '')"
if [ -z "$list_json" ]; then
    list_json='{"candidates":[],"skipped":[]}'
fi
if ! printf '%s' "$list_json" | jq -e 'type == "object"' >/dev/null 2>&1; then
    list_json='{"candidates":[],"skipped":[]}'
fi

cand_count="$(printf '%s' "$list_json" | jq '.candidates | length' 2>/dev/null || printf '0')"
case "$cand_count" in
    ''|*[!0-9]*) cand_count=0 ;;
esac

# ── Audit-comment helper ──────────────────────────────────────────────────────
audit_comment() {
    # $1 = issue number, $2 = body text
    gh issue comment "$1" --repo "$repo" --body "$2" >/dev/null 2>&1 || true
}

ensure_label() {
    # $1 = label name. Created WITHOUT --force so we never recolor a pre-existing
    # repo label (cosmetic-mutation guard).
    gh label create "$1" --repo "$repo" >/dev/null 2>&1 || true
}

# ── Report-only path: mutate nothing, list candidates as report-only ──────────
if [ "$apply_enabled" != "1" ]; then
    i=0
    while [ "$i" -lt "$cand_count" ]; do
        num="$(printf '%s' "$list_json" | jq -r ".candidates[$i].number" 2>/dev/null || printf '')"
        if [ -n "$num" ]; then
            record_skip "$num" "report-only"
        fi
        i=$((i + 1))
    done
    if [ "$policy" = "off" ]; then
        emit_json "true" "policy off — report-only, zero mutations"
    elif [ "$cand_count" -gt 0 ]; then
        emit_json "true" "report-only (pass --apply with policy auto|on to groom)"
    else
        emit_json "true" "no groomable issues"
    fi
    exit 0
fi

# ── Apply path: orchestrate the pipeline per candidate ────────────────────────
i=0
while [ "$i" -lt "$cand_count" ]; do
    cand="$(printf '%s' "$list_json" | jq -c ".candidates[$i]" 2>/dev/null || printf '{}')"
    i=$((i + 1))

    num="$(printf '%s' "$cand" | jq -r '.number // empty' 2>/dev/null || printf '')"
    if [ -z "$num" ]; then
        continue
    fi
    class="$(printf '%s' "$cand" | jq -r '.class // "unlabeled"' 2>/dev/null || printf 'unlabeled')"

    # Fetch full issue detail (body + labels) for the gates.
    detail="$(gh issue view "$num" --repo "$repo" --json number,title,body,labels 2>/dev/null || printf '')"
    if [ -z "$detail" ]; then
        record_skip "$num" "detail-fetch-error"
        continue
    fi
    if ! printf '%s' "$detail" | jq -e 'type == "object"' >/dev/null 2>&1; then
        record_skip "$num" "detail-fetch-error"
        continue
    fi

    title="$(printf '%s' "$detail" | jq -r '.title // ""' 2>/dev/null || printf '')"
    body="$(printf '%s' "$detail" | jq -r '.body // ""' 2>/dev/null || printf '')"
    labels_csv="$(printf '%s' "$detail" | jq -r '[.labels[]?.name] | join(",")' 2>/dev/null || printf '')"
    printf '%s' "$body" > "$BODY_FILE"

    # ── 1. Safety gate (fail-closed) ─────────────────────────────────────────
    safety_out="$(bash "$GROOM_SAFETY" --title "$title" "$BODY_FILE" 2>/dev/null || true)"
    safety_decision="$(printf '%s\n' "$safety_out" | grep -Eo 'SAFETY_(PASS|AMBIGUOUS|BLOCK)' | head -1 || true)"

    if [ -z "$safety_decision" ]; then
        # Indeterminate safety output → never promote; skip (no quarantine on a
        # transient error to avoid mass-quarantine from a pipeline bug).
        record_skip "$num" "safety-indeterminate"
        continue
    fi
    if [ "$safety_decision" != "SAFETY_PASS" ]; then
        ensure_label "security:quarantined"
        gh issue edit "$num" --repo "$repo" \
            --add-label "security:quarantined" \
            --remove-label "auto-implement,needs-classify,needs-autospec-template" \
            >/dev/null 2>&1 || true
        audit_comment "$num" "grooming: quarantined — safety gate returned ${safety_decision}. Removed queue labels; requires human review."
        record_quarantine "$num" "safety:${safety_decision}"
        continue
    fi

    # ── 2. Classify (unlabeled / needs-classify) ─────────────────────────────
    ctx=""
    reasoning=""
    case "$class" in
        unlabeled|needs-classify)
            classify_json="$(bash "$GROOM_CLASSIFY" "$BODY_FILE" --json 2>/dev/null || printf '{}')"
            ctx="$(printf '%s' "$classify_json" | jq -r '.ctx // "64k"' 2>/dev/null || printf '64k')"
            reasoning="$(printf '%s' "$classify_json" | jq -r '.reasoning // "medium"' 2>/dev/null || printf 'medium')"
            [ -n "$ctx" ] || ctx="64k"
            [ -n "$reasoning" ] || reasoning="medium"
            ensure_label "ctx:${ctx}"
            ensure_label "reasoning:${reasoning}"
            gh issue edit "$num" --repo "$repo" \
                --add-label "ctx:${ctx},reasoning:${reasoning}" \
                --remove-label "needs-classify" >/dev/null 2>&1 || true
            ;;
    esac

    # ── 3. Eligibility routing (fail-closed to hold) ─────────────────────────
    elig_json="$(bash "$GROOM_ELIGIBILITY" "$BODY_FILE" --labels "$labels_csv" 2>/dev/null || printf '{"decision":"hold","reason":"eligibility-error"}')"
    decision="$(printf '%s' "$elig_json" | jq -r '.decision // "hold"' 2>/dev/null || printf 'hold')"
    ereason="$(printf '%s' "$elig_json" | jq -r '.reason // ""' 2>/dev/null || printf '')"
    [ -n "$decision" ] || decision="hold"

    case "$decision" in
        eligible)
            ensure_label "auto-implement"
            gh issue edit "$num" --repo "$repo" \
                --add-label "auto-implement" \
                --remove-label "needs-autospec-template" >/dev/null 2>&1 || true
            audit_comment "$num" "grooming: promoted to auto-implement — eligible (${ereason})."
            promoted_nums="${promoted_nums}${promoted_nums:+ }${num}"
            ;;
        needs-template)
            active="$(bash "$GROOM_GOVERN" show 2>/dev/null | jq -r '.active // [] | join(" ")' 2>/dev/null || printf '')"
            has_tp=0
            case " $active " in
                *" template-promote "*) has_tp=1 ;;
            esac
            if [ "$has_tp" = "1" ]; then
                # LLM-template promotion is self-governance-enabled → hand to the
                # groom loop (do NOT go straight to auto-implement).
                ensure_label "route:groom"
                gh issue edit "$num" --repo "$repo" \
                    --add-label "route:groom" >/dev/null 2>&1 || true
                audit_comment "$num" "grooming: routed to template-groom loop — needs-template with template-promote gate active (${ereason})."
                record_routed "$num" "groom" "needs-template:${ereason}"
            else
                ensure_label "hold:needs-human"
                gh issue edit "$num" --repo "$repo" \
                    --add-label "hold:needs-human" >/dev/null 2>&1 || true
                audit_comment "$num" "grooming: held for human — needs-template but template-promote gate not active (${ereason})."
                record_held "$num" "needs-template:${ereason}"
            fi
            ;;
        epic)
            ensure_label "route:split"
            gh issue edit "$num" --repo "$repo" \
                --add-label "route:split" >/dev/null 2>&1 || true
            audit_comment "$num" "grooming: routed to /autospec-split — epic scope (${ereason})."
            record_routed "$num" "split" "epic:${ereason}"
            ;;
        *)
            ensure_label "hold:needs-human"
            gh issue edit "$num" --repo "$repo" \
                --add-label "hold:needs-human" >/dev/null 2>&1 || true
            audit_comment "$num" "grooming: held for human — ${decision} (${ereason})."
            record_held "$num" "${decision}:${ereason}"
            ;;
    esac
done

# ── Emit result ───────────────────────────────────────────────────────────────
if [ -n "$promoted_nums" ]; then
    emit_json "false" "groomed backlog: promoted eligible candidate(s)"
else
    emit_json "false" "groomed backlog: no candidate met promotion bar"
fi
