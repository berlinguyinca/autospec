#!/usr/bin/env bash
# scripts/autonomous-promote-open-issues.sh — Tier 1.5 backlog-grooming orchestrator.
#
# Grooms genuinely-ready open issues into the `auto-implement` queue so the
# autonomous conductor's Tier 1.5 does real work. This is the orchestrator that
# stitches together the deterministic grooming pipeline (Tasks 1-5):
#
#   list-groomable.sh   → candidate set (needs-classify|needs-template|unlabeled)
#   classify-model-fit.sh→ ctx:/reasoning: model-fit labels
#   promote-eligibility.sh→ eligible|needs-template|epic|hold routing decision
#   grooming-config.sh  → policy (auto|on|off) + budget
#
# Per candidate (SAFETY-CRITICAL — fail-closed everywhere; a pipeline error must
# never cause a promotion):
#   1. classify (unlabeled/needs-classify): add ctx:/reasoning:, drop needs-classify.
#   2. eligibility:
#        eligible       → ask Rust to stamp and atomically transition owned labels
#        needs-template → deterministic codex template-fill (groom-fill.sh), then:
#                           fill fails → hold:needs-human (fail-closed)
#                           fill passes → comment-only groom:proposed for a human
#                                         to apply; never replace the issue body
#                         Candidates already carrying groom:proposed/groom:rejected are
#                         skipped (already-groomed) before routing (no re-fill).
#        epic           → route:split (do NOT decompose here)
#        hold / error   → hold:needs-human
#   3. Rust owns safety stamping and the `auto-implement` transition. It
#      re-reads canonical GitHub state before and after each mutation and rolls
#      the queue label back if the final payload drifts.
#   4. every non-safety routing mutation posts an audit comment stating decision + reason.
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
# GROOM_SAFETY_BIN is test-only injection; production resolves the Rust autospec
# binary through AUTOSPEC_BIN or PATH. Its typed JSON gate prevents test stubs
# from becoming shell safety authorities.
SHARED_DIR="$SCRIPT_DIR/../skills/autospec-shared/scripts"
GROOM_LIST="${AUTOSPEC_GROOM_LIST_SCRIPT:-$SCRIPT_DIR/list-groomable.sh}"
GROOM_SAFETY_BIN="${AUTOSPEC_GROOM_SAFETY_BIN:-${AUTOSPEC_BIN:-autospec}}"
GROOM_CLASSIFY="${AUTOSPEC_GROOM_CLASSIFY_SCRIPT:-$SCRIPT_DIR/classify-model-fit.sh}"
GROOM_ELIGIBILITY="${AUTOSPEC_GROOM_ELIGIBILITY_SCRIPT:-$SCRIPT_DIR/promote-eligibility.sh}"
GROOM_CONFIG="${AUTOSPEC_GROOM_CONFIG_SCRIPT:-$SHARED_DIR/grooming-config.sh}"
GROOM_FILL="${AUTOSPEC_GROOM_FILL_SCRIPT:-$SCRIPT_DIR/groom-fill.sh}"

# Tier 1.5 board source (Tasks 8/10): env vars are the shell-side contract —
# the conductor exports them from the validated Rust ProjectBoardConfig, so
# this script never parses the board YAML itself.
BOARD_RESOLVE="${AUTOSPEC_BOARD_RESOLVE_SCRIPT:-$SCRIPT_DIR/project-board-resolve.sh}"
BOARD_NORMALIZE="${AUTOSPEC_BOARD_NORMALIZE_SCRIPT:-$SCRIPT_DIR/project-board-normalize.sh}"
BOARD_DEPS="${AUTOSPEC_BOARD_DEPS_SCRIPT:-$SCRIPT_DIR/project-board-deps.sh}"
BOARD_WRITEBACK="${AUTOSPEC_BOARD_WRITEBACK_SCRIPT:-$SCRIPT_DIR/project-board-writeback.sh}"
BOARD_TTL="${AUTOSPEC_PROJECT_BOARD_TTL:-300}"
case "$BOARD_TTL" in
    ''|*[!0-9]*) BOARD_TTL=300 ;;
esac

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
policy="$(bash "$GROOM_CONFIG" --key policy 2>/dev/null || printf 'off')"
[ -n "$policy" ] || policy="off"

apply_enabled=0
if [ "$apply_flag" = "1" ]; then
    case "$policy" in
        auto|on) apply_enabled=1 ;;
    esac
fi

# ── Budget ────────────────────────────────────────────────────────────────────
budget="$(bash "$GROOM_CONFIG" --key budget.max_issues_per_cycle 2>/dev/null || printf '10')" # linter:allow-DOC_OUT_OF_SYNC internal default only; no CLI surface changed
case "$budget" in
    ''|*[!0-9]*) budget=10 ;;
esac

# ── Board source (Tasks 8/10) ──────────────────────────────────────────────────
# BOARD_CACHE is set as a side effect of board_plan() to the path of the cached
# plan JSON so downstream readers (board_stage, write-back) share one file.
BOARD_CACHE=""

# Resolve the board once per TTL. N repo workers across a fleet each invoke
# this orchestrator independently, so caching the resolved+normalized+
# dependency-resolved plan on disk keyed by URL means the fleet costs one
# board read per TTL, not one per worker.
board_plan() {
    _url="${AUTOSPEC_PROJECT_BOARD_URL:-}"
    if [ -z "$_url" ]; then
        BOARD_CACHE="$(mktemp -t board-empty.XXXXXX)"
        printf '{"items":[]}' > "$BOARD_CACHE"
        return 0
    fi

    _cache_dir="${AUTOSPEC_STATE_DIR:-$HOME/.autospec}/board-cache"
    mkdir -p "$_cache_dir"
    _cache="$_cache_dir/$(printf '%s' "$_url" | shasum | cut -c1-16).json"
    if [ -f "$_cache" ]; then
        _mtime="$(stat -f %m "$_cache" 2>/dev/null || stat -c %Y "$_cache" 2>/dev/null || printf '0')"
        _age=$(( $(date +%s) - _mtime ))
        if [ "$_age" -lt "$BOARD_TTL" ]; then
            BOARD_CACHE="$_cache"
            return 0
        fi
    fi

    # Fail-closed: any resolver failure (bad URL, auth, truncated read) yields
    # an empty board, never a partial/garbage promotion.
    if _plan="$(bash "$BOARD_RESOLVE" --url "$_url" --emit plan 2>/dev/null)"; then
        if printf '%s' "$_plan" \
             | bash "$BOARD_NORMALIZE" ${AUTOSPEC_PROJECT_BOARD_LABEL_MAP:+--label-map "$AUTOSPEC_PROJECT_BOARD_LABEL_MAP"} \
             | bash "$BOARD_DEPS" --resolve > "$_cache.tmp" 2>/dev/null; then
            mv "$_cache.tmp" "$_cache"
        else
            printf '{"items":[]}' > "$_cache"
        fi
    else
        printf '{"items":[]}' > "$_cache"
    fi
    BOARD_CACHE="$_cache"
}

# Reduce the cached plan to the Tier 1.5 board envelope. Allowlist matching
# uses --arg + a literal prefix/equality compare, never jq test() — a repo
# name is board-controlled data and may contain regex metacharacters.
board_stage() {
    jq --arg repo "$repo" --arg allow "${AUTOSPEC_PROJECT_BOARD_ALLOWLIST:-}" '
      def allowed($r):
        ($allow | split(",") | map(select(length > 0))) as $pats
        | if ($pats | length) == 0 then false
          else $pats | map(
              (. | rtrimstr("*")) as $p
              | if endswith("*") then ($r | startswith($p)) else ($r == .) end) | any
          end;
      # Priority rank for stable promotion ordering. A missing/unrecognized
      # priority label ranks LAST (4) — never highest — so an unprioritized
      # item never jumps ahead of a genuinely prioritized one. Measured: 30
      # of 80 items on the p1 board carry no priority label at all.
      def prio_rank:
        {"critical":0,"high":1,"normal":2,"low":3}[(.normalized.priority // "")] // 4;
      ([.items[]? | select(.repo == $repo and allowed(.repo))]
        | sort_by([(if .ready == true then 0 else 1 end), prio_rank, .number])) as $own
      | ([$own[] | select(.ready == true)]) as $ready
      | {
          ready:        ($ready | length),
          promotable:   [$ready[] | .number],
          out_of_scope: [.items[]? | select(allowed(.repo) | not) | {repo: .repo, number: .number}],
          demoted:      [],
          items:        [$own[] | {
                           item_id: .item_id,
                           number:  .number,
                           ready:   (.ready == true),
                           labels:  ([.labels[]? | select(type == "string")] | join(","))
                         }]
        }' "$BOARD_CACHE" 2>/dev/null || printf '{"ready":0,"promotable":[],"out_of_scope":[],"demoted":[],"items":[]}'
}

board_plan
board_json="$(board_stage)"
if ! printf '%s' "$board_json" | jq -e 'type == "object"' >/dev/null 2>&1; then
    board_json='{"ready":0,"promotable":[],"out_of_scope":[],"demoted":[],"items":[]}'
fi

# Admission control (shared with GROOM_LIST — see the board apply loop below):
# how many ready board items were withheld this cycle purely because the
# shared per-cycle budget ran out. Always defined (0 outside --apply) so the
# envelope can report it honestly rather than reading as "nothing was ready".
board_truncated=0

# Write-back is advisory: a board mutation failure must never fail a
# promotion that already succeeded. Fires only under --apply AND policy
# auto|on — never in report-only mode.
board_writeback() {
    _wb_item="$1"; _wb_state="$2"
    [ "$apply_enabled" -eq 1 ] || return 0
    [ -f "$BOARD_WRITEBACK" ] || return 0
    bash "$BOARD_WRITEBACK" --plan "$BOARD_CACHE" --item "$_wb_item" --state "$_wb_state" >/dev/null 2>&1 || true
}

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
        --argjson board "$(printf '%s' "$board_json" | jq --argjson truncated "$board_truncated" 'del(.items) + {truncated: $truncated}' 2>/dev/null || printf '%s' "$board_json")" \
        --arg reason "$reason" \
        '{dry:$dry, filed:$filed, promoted:$promoted,
          skipped:$skipped[0], held:$held[0],
          quarantined:$quarantined[0], routed:$routed[0], reason:$reason,
          board:$board}'
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
    # $1 = issue number, $2 = authoritative body file used for classification,
    # $3 = existing labels csv. Body replacement is deliberately not supported:
    # GitHub issue updates have no server-enforced compare-and-swap primitive.
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

}

review_admitted_issue() {
    # $1 = issue number. Rust performs the authoritative safety stamp, canonical
    # re-reads, queue-label transition, and rollback as one fail-closed command.
    rai_num="$1"
    rai_out="$("$GROOM_SAFETY_BIN" issue promote --repo "$repo" --number "$rai_num" --remove-label needs-autospec-template --json 2>/dev/null || printf '')"
    rai_pass="$(printf '%s' "$rai_out" | jq -r '(."auto-implement" == true) and (.eligible == true)' 2>/dev/null || printf 'false')"
    if [ "$rai_pass" = "true" ]; then
        rust_safety_result="pass"
        return 0
    fi
    rai_decision="$(printf '%s' "$rai_out" | jq -r '.safety.decision // "hold"' 2>/dev/null || printf 'hold')"
    if [ "$rai_decision" = "blocked" ]; then
        rust_safety_result="block"
        return 1
    fi
    rust_safety_result="hold"
    return 1
}

record_rust_safety_result() {
    # The Rust command owns GitHub safety mutations. The groomer only preserves
    # its structured outcome in this local reporting envelope.
    rrs_num="$1"
    if [ "${rust_safety_result:-hold}" = "block" ]; then
        record_quarantine "$rrs_num" "rust-safety-block"
    else
        record_held "$rrs_num" "rust-safety-review"
    fi
}

admit_with_rust_safety() {
    # $1 = issue number, $2 = authoritative body file, $3 = existing labels csv.
    aws_num="$1"; aws_body="$2"; aws_labels="$3"
    rust_safety_result="hold"
    finalize_ready "$aws_num" "$aws_body" "$aws_labels" || return 1
    ensure_label "auto-implement"
    review_admitted_issue "$aws_num"
}

route_template_candidate() {
    # $1 = issue number, $2 = existing labels, $3 = eligibility reason.
    rtc_num="$1"; rtc_reason="$3"
    rtc_fill="$(bash "$GROOM_FILL" --issue "$rtc_num" --repo "$repo" 2>/dev/null || printf '')"
    rtc_ok="$(printf '%s' "$rtc_fill" | jq -r '.ok // false' 2>/dev/null || printf 'false')"
    if [ "$rtc_ok" != "true" ]; then
        rtc_failure="$(printf '%s' "$rtc_fill" | jq -r '.reason // "fill-error"' 2>/dev/null || printf 'fill-error')"
        ensure_label "hold:needs-human"
        gh issue edit "$rtc_num" --repo "$repo" --add-label "hold:needs-human" >/dev/null 2>&1 || true
        audit_comment "$rtc_num" "grooming: codex template-fill unavailable/failed (${rtc_failure}) — held for human (${rtc_reason})."
        record_held "$rtc_num" "needs-template:fill-${rtc_failure}"
        return 0
    fi

    rtc_body="$(printf '%s' "$rtc_fill" | jq -r '.body' 2>/dev/null || printf '')"
    if [ -z "$rtc_body" ] || [ "$rtc_body" = "null" ]; then
        ensure_label "hold:needs-human"
        gh issue edit "$rtc_num" --repo "$repo" --add-label "hold:needs-human" >/dev/null 2>&1 || true
        audit_comment "$rtc_num" "grooming: codex template-fill returned an empty body — held for human (${rtc_reason})."
        record_held "$rtc_num" "needs-template:fill-empty-body"
        return 0
    fi

    rtc_file="$(mktemp "${TMPDIR:-/tmp}/groom-body.XXXXXX")"
    printf '%s' "$rtc_body" > "$rtc_file"
    if gh issue comment "$rtc_num" --repo "$repo" --body-file "$rtc_file" >/dev/null 2>&1; then
        ensure_label "groom:proposed"
        gh issue edit "$rtc_num" --repo "$repo" --add-label "groom:proposed" >/dev/null 2>&1 || true
        audit_comment "$rtc_num" "grooming: generated template left as a human proposal; apply it before admission (${rtc_reason})."
        record_routed "$rtc_num" "groom-canary" "needs-template:${rtc_reason}"
    else
        record_held "$rtc_num" "template-proposal-comment"
    fi
    rm -f "$rtc_file"
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
    printf '%s' "$body" > "$BODY_FILE"

    # ── 0. Already-groomed skip (before routing → no re-fill) ─────────────────
    # A candidate that already carries the canary proposal (`groom:proposed`) or
    # a human rejection (`groom:rejected`) is a completed grooming outcome; never
    # re-fill or re-route it.
    case ",${labels_csv}," in
        *",groom:proposed,"*|*",groom:rejected,"*)
            record_skip "$num" "already-groomed"
            continue
            ;;
    esac

    # ── 1. Eligibility routing (fail-closed to hold) ─────────────────────────
    elig_json="$(bash "$GROOM_ELIGIBILITY" "$BODY_FILE" --labels "$labels_csv" --repo "$repo" --title "$title" 2>/dev/null || printf '{"decision":"hold","reason":"eligibility-error"}')"
    decision="$(printf '%s' "$elig_json" | jq -r '.decision // "hold"' 2>/dev/null || printf 'hold')"
    ereason="$(printf '%s' "$elig_json" | jq -r '.reason // ""' 2>/dev/null || printf '')"
    [ -n "$decision" ] || decision="hold"

    case "$decision" in
        eligible)
            if ! admit_with_rust_safety "$num" "$BODY_FILE" "$labels_csv"; then
                record_rust_safety_result "$num"
                continue
            fi
            audit_comment "$num" "grooming: promoted to auto-implement — eligible (${ereason})."
            promoted_nums="${promoted_nums}${promoted_nums:+ }${num}"
            ;;
        needs-template)
            # Deterministic LLM template-fill (codex exec via groom-fill.sh)
            # produces a comment-only human proposal. Without a server-side CAS,
            # this path never replaces the issue body or admits the proposal.
            route_template_candidate "$num" "$labels_csv" "$ereason"
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

# ── Board apply loop (Tasks 8/10) ──────────────────────────────────────────────
# Board items are a separate candidate source from GROOM_LIST; a genuinely
# ready item is routed through the SAME Rust safety-stamp + auto-implement
# transition used above for `eligible` candidates — no second mutation path.
# State map (Task 10), applied here at promotion time:
#   ready == false                -> Blocked
#   promoted to auto-implement    -> Ready
#   carries in-progress-by-bot    -> Implementation
#   carries autospec:needs-human  -> Blocked
#
# Admission control: board promotions share ONE per-cycle budget
# (budget.max_issues_per_cycle) with the GROOM_LIST path above — never a
# second, independent budget. `grooming_promoted_count` is however many the
# GROOM_LIST loop already promoted this cycle (via `eligible`); the board may
# promote at most `budget - grooming_promoted_count`. board_json's `items`
# list is already ranked (ready-first, then priority, null-priority last),
# so processing it in order and stopping at the remaining budget always
# keeps the top of the ranking — never an arbitrary slice. Anything past the
# cutoff is left untouched (no mutation, no write-back) and counted in
# board_truncated so the envelope reports the truncation instead of reading
# as "nothing else was ready".
grooming_promoted_count=0
if [ -n "$promoted_nums" ]; then
    grooming_promoted_count="$(printf '%s\n' $promoted_nums | awk 'NF' | wc -l | tr -d ' ')"
fi
board_budget_remaining=$((budget - grooming_promoted_count))
if [ "$board_budget_remaining" -lt 0 ]; then
    board_budget_remaining=0
fi

board_apply_item() {
    bai_item="$1"; bai_num="$2"; bai_ready="$3"; bai_labels="$4"

    case ",${bai_labels}," in
        *",autospec:needs-human,"*)
            board_writeback "$bai_item" "Blocked"
            return 0
            ;;
    esac
    case ",${bai_labels}," in
        *",in-progress-by-bot,"*)
            board_writeback "$bai_item" "Implementation"
            return 0
            ;;
    esac
    if [ "$bai_ready" != "true" ]; then
        board_writeback "$bai_item" "Blocked"
        return 0
    fi

    if [ "$board_budget_remaining" -le 0 ]; then
        board_truncated=$((board_truncated + 1))
        return 0
    fi

    rust_safety_result="hold"
    ensure_label "auto-implement"
    if review_admitted_issue "$bai_num"; then
        audit_comment "$bai_num" "grooming: promoted to auto-implement — board-ready."
        promoted_nums="${promoted_nums}${promoted_nums:+ }${bai_num}"
        board_writeback "$bai_item" "Ready"
        board_budget_remaining=$((board_budget_remaining - 1))
    else
        record_rust_safety_result "$bai_num"
    fi
}

if [ "$apply_enabled" = "1" ]; then
    board_own_count="$(printf '%s' "$board_json" | jq '.items | length' 2>/dev/null || printf '0')"
    case "$board_own_count" in
        ''|*[!0-9]*) board_own_count=0 ;;
    esac
    bi=0
    while [ "$bi" -lt "$board_own_count" ]; do
        b_item="$(printf '%s' "$board_json" | jq -r ".items[$bi].item_id // empty" 2>/dev/null || printf '')"
        b_num="$(printf '%s' "$board_json" | jq -r ".items[$bi].number // empty" 2>/dev/null || printf '')"
        b_ready="$(printf '%s' "$board_json" | jq -r ".items[$bi].ready // false" 2>/dev/null || printf 'false')"
        b_labels="$(printf '%s' "$board_json" | jq -r ".items[$bi].labels // \"\"" 2>/dev/null || printf '')"
        bi=$((bi + 1))
        if [ -z "$b_item" ] || [ -z "$b_num" ]; then
            continue
        fi
        board_apply_item "$b_item" "$b_num" "$b_ready" "$b_labels"
    done
fi

# ── Emit result ───────────────────────────────────────────────────────────────
if [ -n "$promoted_nums" ]; then
    emit_json "false" "groomed backlog: promoted eligible candidate(s)"
else
    emit_json "true" "groomed backlog: no candidate met promotion bar"
fi
