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
#        needs-template → deterministic codex template-fill (groom-fill.sh), then:
#                           fill fails        → hold:needs-human (fail-closed)
#                           template-promote ∈ govern active set (graduated)
#                                             → auto: apply filled body,
#                                               add auto-implement, drop needs-autospec-template
#                           else (seed)       → canary: apply filled body,
#                                               add groom:proposed (human approves via
#                                               auto-implement / rejects via groom:rejected),
#                                               drop needs-autospec-template
#                         Candidates already carrying groom:proposed/groom:rejected are
#                         skipped (already-groomed) before any decision (no re-fill).
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
GROOM_FILL="${AUTOSPEC_GROOM_FILL_SCRIPT:-$SCRIPT_DIR/groom-fill.sh}"
GROOM_APPLY_SAFETY="${AUTOSPEC_GROOM_APPLY_SAFETY_SCRIPT:-$SCRIPT_DIR/apply-safety-review.sh}"

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
bf=""
trap 'rm -f "$SKIPPED_FILE" "$HELD_FILE" "$QUARANTINED_FILE" "$ROUTED_FILE" "$BODY_FILE" "${bf:-}"' EXIT
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

list_skip_count="$(printf '%s' "$list_json" | jq '.skipped | length' 2>/dev/null || printf '0')"
case "$list_skip_count" in
    ''|*[!0-9]*) list_skip_count=0 ;;
esac
i=0
while [ "$i" -lt "$list_skip_count" ]; do
    skip_issue="$(printf '%s' "$list_json" | jq -r ".skipped[$i].issue // .skipped[$i].number // empty" 2>/dev/null || printf '')"
    skip_reason="$(printf '%s' "$list_json" | jq -r ".skipped[$i].reason // \"selector-skip\"" 2>/dev/null || printf 'selector-skip')"
    if [ -n "$skip_issue" ]; then
        record_skip "$skip_issue" "$skip_reason"
    fi
    i=$((i + 1))
done

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

finalize_ready() {
    # $1 = issue number, $2 = FINAL body file (exact text that will live on the issue),
    # $3 = existing labels csv.
    fr_num="$1"; fr_body="$2"; fr_labels="$3"

    # 1. Model-fit classify (skip if BOTH ctx:* and reasoning:* already present).
    case ",${fr_labels}," in
        *",ctx:"*",reasoning:"*|*",reasoning:"*",ctx:"*) : ;;  # already classified
        *)
            fr_cls="$(bash "$GROOM_CLASSIFY" "$fr_body" --json 2>/dev/null || printf '{}')"
            fr_ctx="$(printf '%s' "$fr_cls" | jq -r '.ctx // "64k"' 2>/dev/null || printf '64k')"
            fr_rsn="$(printf '%s' "$fr_cls" | jq -r '.reasoning // "medium"' 2>/dev/null || printf 'medium')"
            [ -n "$fr_ctx" ] || fr_ctx="64k"
            [ -n "$fr_rsn" ] || fr_rsn="medium"
            ensure_label "ctx:${fr_ctx}"
            ensure_label "reasoning:${fr_rsn}"
            gh issue edit "$fr_num" --repo "$repo" \
                --add-label "ctx:${fr_ctx},reasoning:${fr_rsn}" \
                --remove-label "needs-classify" >/dev/null 2>&1 || true
            ;;
    esac

    # 2. Safety-stamp on the FINAL body (fail-closed: non-PASS → quarantine, return 1).
    #    apply-safety-review itself writes safety:reviewed + block (PASS) or
    #    security:quarantined + strips queue labels (non-PASS).
    fr_safe_rc=0
    bash "$GROOM_APPLY_SAFETY" --issue "$fr_num" --repo "$repo" \
        --body-file "$fr_body" --title "$title" --actor "$author" --apply >/dev/null 2>&1 || fr_safe_rc=$?
    if [ "$fr_safe_rc" -ne 0 ]; then
        return 1
    fi
    return 0
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
    detail="$(gh issue view "$num" --repo "$repo" --json number,title,body,labels,author 2>/dev/null || printf '')"
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
    author="$(printf '%s' "$detail" | jq -r '.author.login // ""' 2>/dev/null || printf '')"
    printf '%s' "$body" > "$BODY_FILE"

    # ── 0. Already-groomed skip (before any decision → no re-fill) ────────────
    # A candidate that already carries the canary proposal (`groom:proposed`) or
    # a human rejection (`groom:rejected`) is a completed grooming outcome; never
    # re-fill or re-route it.
    case ",${labels_csv}," in
        *",groom:proposed,"*|*",groom:rejected,"*)
            record_skip "$num" "already-groomed"
            continue
            ;;
    esac

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

    # ── 2. Eligibility routing (fail-closed to hold) ─────────────────────────
    elig_json="$(bash "$GROOM_ELIGIBILITY" "$BODY_FILE" --labels "$labels_csv" --repo "$repo" --title "$title" 2>/dev/null || printf '{"decision":"hold","reason":"eligibility-error"}')"
    decision="$(printf '%s' "$elig_json" | jq -r '.decision // "hold"' 2>/dev/null || printf 'hold')"
    ereason="$(printf '%s' "$elig_json" | jq -r '.reason // ""' 2>/dev/null || printf '')"
    [ -n "$decision" ] || decision="hold"

    case "$decision" in
        eligible)
            if ! finalize_ready "$num" "$BODY_FILE" "$labels_csv"; then
                audit_comment "$num" "grooming: final-body safety gate did not pass (quarantined or stamp error) — not promoted."
                record_quarantine "$num" "safety:final-body"
                continue
            fi
            ensure_label "auto-implement"
            gh issue edit "$num" --repo "$repo" \
                --add-label "auto-implement" \
                --remove-label "needs-autospec-template" >/dev/null 2>&1 || true
            audit_comment "$num" "grooming: promoted to auto-implement — eligible (${ereason})."
            promoted_nums="${promoted_nums}${promoted_nums:+ }${num}"
            ;;
        needs-template)
            # Deterministic LLM template-fill (codex exec via groom-fill.sh),
            # then route by the govern ratchet: canary (seed) proposes for a human;
            # auto (graduated: template-promote active) queues straight into
            # auto-implement. Fail-closed: any fill failure holds for a human.
            active="$(bash "$GROOM_GOVERN" show 2>/dev/null | jq -r '.active // [] | join(" ")' 2>/dev/null || printf '')"
            fill_out="$(bash "$GROOM_FILL" --issue "$num" --repo "$repo" 2>/dev/null || printf '')"
            fill_ok="$(printf '%s' "$fill_out" | jq -r '.ok // false' 2>/dev/null || printf 'false')"
            if [ "$fill_ok" != "true" ]; then
                freason="$(printf '%s' "$fill_out" | jq -r '.reason // "fill-error"' 2>/dev/null || printf 'fill-error')"
                ensure_label "hold:needs-human"
                gh issue edit "$num" --repo "$repo" \
                    --add-label "hold:needs-human" >/dev/null 2>&1 || true
                audit_comment "$num" "grooming: codex template-fill unavailable/failed (${freason}) — held for human (${ereason})."
                record_held "$num" "needs-template:fill-${freason}"
            else
                fbody="$(printf '%s' "$fill_out" | jq -r '.body' 2>/dev/null || printf '')"
                if [ -z "$fbody" ] || [ "$fbody" = "null" ]; then
                    # Defense-in-depth: a contract-violating fill (ok:true with
                    # no/null body) must never reach auto-implement or overwrite
                    # the issue body — hold for a human instead.
                    ensure_label "hold:needs-human"
                    gh issue edit "$num" --repo "$repo" \
                        --add-label "hold:needs-human" >/dev/null 2>&1 || true
                    audit_comment "$num" "grooming: codex template-fill returned an empty body — held for human (${ereason})."
                    record_held "$num" "needs-template:fill-empty-body"
                else
                    bf="$(mktemp "${TMPDIR:-/tmp}/groom-body.XXXXXX")"
                    printf '%s' "$fbody" > "$bf"
                    if ! finalize_ready "$num" "$bf" "$labels_csv"; then
                        audit_comment "$num" "grooming: final-body safety gate did not pass (quarantined or stamp error) — not promoted."
                        record_quarantine "$num" "safety:final-body"
                        rm -f "$bf"
                        bf=""
                        continue
                    fi
                    case " $active " in
                        *" template-promote "*)
                            # Graduated → auto: filled body queued straight into auto-implement.
                            ensure_label "auto-implement"
                            gh issue edit "$num" --repo "$repo" \
                                --add-label "auto-implement" \
                                --remove-label "needs-autospec-template" >/dev/null 2>&1 || true
                            audit_comment "$num" "grooming: auto-template-groomed — template-promote gate active (${ereason})."
                            record_routed "$num" "groom-auto" "needs-template:${ereason}"
                            ;;
                        *)
                            # Seed → canary: filled body proposed for human approval.
                            ensure_label "groom:proposed"
                            gh issue edit "$num" --repo "$repo" \
                                --add-label "groom:proposed" \
                                --remove-label "needs-autospec-template" >/dev/null 2>&1 || true
                            audit_comment "$num" "grooming: template drafted for human approval (canary) — approve by adding auto-implement, reject with groom:rejected (${ereason})."
                            record_routed "$num" "groom-canary" "needs-template:${ereason}"
                            ;;
                    esac
                    rm -f "$bf"
                    bf=""
                fi
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
    emit_json "true" "groomed backlog: no candidate met promotion bar"
fi
