#!/usr/bin/env bash
# scripts/explore-qa-gate.sh — autospec-explore QA sandbox-promotion gate
# runner (issue #1113).
#
# Stands the sandbox-HEAD app up in a fresh, throwaway worktree (NEVER the
# operator's primary checkout), runs `autospec-qa --no-heal` against it (the
# gate proves; it does not mutate the sandbox), and persists the promotion
# verdict to `.autospec/explore-qa-gate.json` per
# `schemas/autospec-explore-qa-gate.schema.json`.
#
# Fail-open SKIP vs fail-closed error:
#   - No QA config (no .autospec/test.yml and no autospec-qa config) → SKIP:
#     verdict "skipped", emit code_health:explore_qa_gate_skipped_no_config,
#     exit 0. A repo with no QA setup is NOT penalized.
#   - An *attempted* gate that crashes (missing autospec-qa, QA crash, jq
#     error, no verdict file) → verdict "error", BLOCK (non-zero). Distinct
#     from skip.
#   - QA FAIL or PARTIAL → block (non-zero). PASS → exit 0.
#   - No sandbox (.autospec/explore-mode.json absent) → operator error:
#     emit code_health:explore_qa_gate_no_sandbox, exit non-zero.
#
# Exit codes:
#   0  PASS or skipped (promotable / not penalized)
#   1  FAIL / PARTIAL / error (blocks promotion)
#   2  no sandbox (explore-mode.json absent — gate requested with no sandbox)
#
# Conventions (AGENTS.md): set -u (branch on exits explicitly, no -e
# short-circuit traps); bash 3.2 safe; no RETURN traps (inline cleanup);
# if/then/fi for one-sided conditionals; no `git add -A`.

set -u

PROG="explore-qa-gate.sh"

emit() { printf '%s\n' "$*" >&2; }
err()  { printf '%s: %s\n' "$PROG" "$*" >&2; }

REPO_ROOT="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
MODE_FILE="$REPO_ROOT/.autospec/explore-mode.json"
GATE_FILE="$REPO_ROOT/.autospec/explore-qa-gate.json"
RAN_AT="$(date -u +'%Y-%m-%dT%H:%M:%SZ')"

# worktree-guard.sh lives next to this script (repo) or in the installed
# scripts dir.
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
WORKTREE_GUARD="$SCRIPT_DIR/worktree-guard.sh"

# ---------------------------------------------------------------------------
# 1. No sandbox → operator error (fail-closed, exit 2).
# ---------------------------------------------------------------------------
if [ ! -f "$MODE_FILE" ]; then
    emit "code_health:explore_qa_gate_no_sandbox"
    err "no sandbox: $MODE_FILE absent (run /autospec-explore to create one)"
    exit 2
fi

# Read {branch, head_sha} from explore-mode.json.
SANDBOX_BRANCH="$(jq -r '.branch // empty' "$MODE_FILE" 2>/dev/null || true)"
SANDBOX_HEAD_SHA="$(jq -r '.head_sha // empty' "$MODE_FILE" 2>/dev/null || true)"

# Helper: write the gate artifact and (optionally) a worktree cleanup.
# write_gate <verdict> <blocking_findings_json> [reason]
write_gate() {
    local verdict="$1"
    local findings="$2"
    local reason="${3:-}"
    mkdir -p "$REPO_ROOT/.autospec"
    local tmp="$GATE_FILE.tmp.$$"
    if [ -n "$reason" ]; then
        jq -n \
            --arg verdict "$verdict" \
            --arg branch "$SANDBOX_BRANCH" \
            --arg head "$SANDBOX_HEAD_SHA" \
            --arg qvp ".autospec/qa-verdict.json" \
            --arg ran "$RAN_AT" \
            --arg reason "$reason" \
            --argjson findings "$findings" \
            '{verdict:$verdict, sandbox_branch:$branch, sandbox_head_sha:$head, qa_verdict_path:$qvp, blocking_findings:$findings, ran_at:$ran, reason:$reason}' \
            > "$tmp" 2>/dev/null
    else
        jq -n \
            --arg verdict "$verdict" \
            --arg branch "$SANDBOX_BRANCH" \
            --arg head "$SANDBOX_HEAD_SHA" \
            --arg qvp ".autospec/qa-verdict.json" \
            --arg ran "$RAN_AT" \
            --argjson findings "$findings" \
            '{verdict:$verdict, sandbox_branch:$branch, sandbox_head_sha:$head, qa_verdict_path:$qvp, blocking_findings:$findings, ran_at:$ran}' \
            > "$tmp" 2>/dev/null
    fi
    if [ ! -s "$tmp" ]; then
        # jq itself failed — fall back to a minimal hand-written object so the
        # artifact is never left missing.
        printf '{"verdict":"error","sandbox_branch":"%s","sandbox_head_sha":"%s","qa_verdict_path":".autospec/qa-verdict.json","blocking_findings":[],"ran_at":"%s","reason":"gate jq write failed"}\n' \
            "$SANDBOX_BRANCH" "$SANDBOX_HEAD_SHA" "$RAN_AT" > "$tmp"
    fi
    mv "$tmp" "$GATE_FILE"
    chmod 644 "$GATE_FILE" 2>/dev/null || true
}

# ---------------------------------------------------------------------------
# 2. QA-config probe (fail-open SKIP).
# Probe the PRIMARY repo: an explore sandbox built off it shares config layout.
# No .autospec/test.yml and no autospec-qa config → SKIP, exit 0.
# ---------------------------------------------------------------------------
HAS_QA_CONFIG=0
if [ -f "$REPO_ROOT/.autospec/test.yml" ]; then
    HAS_QA_CONFIG=1
fi
if [ -f "$REPO_ROOT/.autospec/qa.yml" ] || [ -f "$REPO_ROOT/.autospec/reliability.yml" ]; then
    HAS_QA_CONFIG=1
fi

if [ "$HAS_QA_CONFIG" -eq 0 ]; then
    emit "code_health:explore_qa_gate_skipped_no_config"
    write_gate "skipped" "[]" "no QA config (.autospec/test.yml / autospec-qa config) present; not penalized"
    printf '%s: skipped (no QA config) — promotion not penalized\n' "$PROG"
    exit 0
fi

# ---------------------------------------------------------------------------
# 3. Create a fresh worktree on the sandbox branch at head_sha.
# NEVER the primary checkout. Removed inline on every exit path below.
# ---------------------------------------------------------------------------
WT_PATH="${AUTOSPEC_QA_GATE_WORKTREE:-/tmp/wt-explore-qa-gate-$$}"

# Inline cleanup (no RETURN trap): remove the worktree we created.
cleanup_worktree() {
    local notify_pid=""
    if [ -n "${WT_PATH:-}" ] && [ -d "$WT_PATH" ]; then
        if [ -f "$WT_PATH/.omx/state/notify-fallback.pid" ]; then
            notify_pid="$(cat "$WT_PATH/.omx/state/notify-fallback.pid" 2>/dev/null || true)"
            case "$notify_pid" in
                ''|*[!0-9]*) notify_pid="" ;;
            esac
            if [ -n "$notify_pid" ]; then
                kill "$notify_pid" >/dev/null 2>&1 || true
            fi
        fi
        git -C "$REPO_ROOT" worktree remove --force "$WT_PATH" >/dev/null 2>&1 || true
        git -C "$REPO_ROOT" worktree prune >/dev/null 2>&1 || true
        for _cleanup_try in 1 2 3 4 5; do
            rm -rf "$WT_PATH" 2>/dev/null || true
            [ ! -d "$WT_PATH" ] && break
            sleep 0.2
        done
    fi
}

# Branch the worktree off the recorded sandbox HEAD sha (detached-safe via a
# throwaway local branch name). worktree-guard create refuses the primary
# checkout and dirty reuse.
WT_BRANCH="explore-qa-gate-$$"
WT_CREATE_OK=0
if [ -f "$WORKTREE_GUARD" ]; then
    if bash "$WORKTREE_GUARD" create \
        --branch "$WT_BRANCH" \
        --base "$SANDBOX_HEAD_SHA" \
        --path "$WT_PATH" >/dev/null 2>&1; then
        WT_CREATE_OK=1
    fi
fi
if [ "$WT_CREATE_OK" -ne 1 ]; then
    # Fallback: direct worktree add off the sandbox HEAD sha.
    if git -C "$REPO_ROOT" worktree add -b "$WT_BRANCH" "$WT_PATH" "$SANDBOX_HEAD_SHA" >/dev/null 2>&1; then
        WT_CREATE_OK=1
    fi
fi
if [ "$WT_CREATE_OK" -ne 1 ]; then
    emit "code_health:explore_qa_gate_worktree_failed branch=$SANDBOX_BRANCH head=$SANDBOX_HEAD_SHA"
    write_gate "error" "[]" "could not create sandbox-HEAD worktree at $WT_PATH"
    err "worktree creation failed for $SANDBOX_HEAD_SHA at $WT_PATH"
    exit 1
fi

# ---------------------------------------------------------------------------
# 4. Run autospec-qa --no-heal against the sandbox-HEAD worktree.
# Missing skill / crash / jq error → verdict "error" (fail-closed block).
# ---------------------------------------------------------------------------
QA_VERDICT_FILE="$WT_PATH/.autospec/qa-verdict.json"

# AUTOSPEC_QA_CMD test/operator seam; default to the autospec-qa binary.
QA_CMD="${AUTOSPEC_QA_CMD:-autospec-qa --no-heal}"

# Resolve the binary name (first word) to detect a missing skill up front.
QA_BIN="$(printf '%s\n' "$QA_CMD" | awk '{print $1}')"
if ! command -v "$QA_BIN" >/dev/null 2>&1; then
    emit "code_health:explore_qa_gate_failed missing autospec-qa ($QA_BIN)"
    write_gate "error" "[]" "autospec-qa not found on PATH ($QA_BIN)"
    cleanup_worktree
    err "autospec-qa not found ($QA_BIN); fail-closed error"
    exit 1
fi

QA_RC=0
( cd "$WT_PATH" && sh -c "$QA_CMD" ) >/dev/null 2>&1 || QA_RC=$?

if [ "$QA_RC" -ne 0 ] && [ ! -f "$QA_VERDICT_FILE" ]; then
    emit "code_health:explore_qa_gate_failed qa crash rc=$QA_RC"
    write_gate "error" "[]" "autospec-qa crashed (rc=$QA_RC) and wrote no verdict"
    cleanup_worktree
    err "autospec-qa crashed (rc=$QA_RC); fail-closed error"
    exit 1
fi

if [ ! -f "$QA_VERDICT_FILE" ]; then
    emit "code_health:explore_qa_gate_failed no qa-verdict.json produced"
    write_gate "error" "[]" "autospec-qa produced no .autospec/qa-verdict.json"
    cleanup_worktree
    err "no qa-verdict.json after autospec-qa; fail-closed error"
    exit 1
fi

# Read the verdict + blocking findings. A jq error here → fail-closed error.
QA_VERDICT="$(jq -r '.verdict // empty' "$QA_VERDICT_FILE" 2>/dev/null || true)"
if [ -z "$QA_VERDICT" ]; then
    emit "code_health:explore_qa_gate_failed unreadable qa-verdict.json"
    write_gate "error" "[]" "could not read .verdict from qa-verdict.json (jq error?)"
    cleanup_worktree
    err "unreadable qa-verdict.json; fail-closed error"
    exit 1
fi

BLOCKING_FINDINGS="$(jq -c '[(.findings // [])[] | select(.release_blocking == true)]' "$QA_VERDICT_FILE" 2>/dev/null || true)"
if [ -z "$BLOCKING_FINDINGS" ]; then
    BLOCKING_FINDINGS="[]"
fi

# ---------------------------------------------------------------------------
# 5. Persist the gate artifact and translate the verdict to an exit code.
# ---------------------------------------------------------------------------
write_gate "$QA_VERDICT" "$BLOCKING_FINDINGS"
cleanup_worktree

case "$QA_VERDICT" in
    PASS)
        printf '%s: PASS — sandbox %s promotable\n' "$PROG" "$SANDBOX_BRANCH"
        exit 0
        ;;
    FAIL|PARTIAL)
        emit "code_health:explore_qa_gate_failed verdict=$QA_VERDICT"
        printf '%s: %s — promotion blocked\n' "$PROG" "$QA_VERDICT" >&2
        exit 1
        ;;
    *)
        # Any other verdict string is treated conservatively as a block.
        emit "code_health:explore_qa_gate_failed verdict=$QA_VERDICT (unrecognized)"
        printf '%s: unrecognized verdict %s — promotion blocked\n' "$PROG" "$QA_VERDICT" >&2
        exit 1
        ;;
esac
