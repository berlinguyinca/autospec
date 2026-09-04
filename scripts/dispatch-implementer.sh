#!/usr/bin/env bash
# scripts/dispatch-implementer.sh — canonical helper for parallel-safe
# Phase 4 implementer dispatch.
#
# Per issue #690: when the orchestrator dispatches multiple background
# implementer Agents in the same git workdir, they collide on `git checkout`
# even when file-level scopes are disjoint. This helper enforces worktree
# isolation by delegating to worktree-guard.sh (create/resolve-branch) and
# pre-pending a worktree directive to the implementer prompt so the LLM cannot
# stray into the main checkout.
#
# Per issue #960 (D2): the silent-reuse fallback has been removed. Worktree
# creation is fully delegated to `worktree-guard.sh create`, which refuses dirty
# or wrong-branch reuse with exit 4 / code_health:worktree_dirty_reuse_refused.
# `worktree-guard.sh resolve-branch` is called first so the orchestrator can act
# on open-pr / branch-only / fresh state before dispatch.
#
# Model/provider routing (issue #3381): this script is the LIVE Phase 4 dispatch
# path, so it is where a routing decision has to become visible to the agent that
# actually runs. Issue #3179 was closed by documenting `route-decide.sh` as
# advisory precisely because this script had no model surface at all; without one,
# the local-model guardrail wave (#3344 and children) lands in scripts nothing
# calls. `--model`/`--provider`/`--kind` state a decision made upstream, and
# `--labels` resolves one through `select-model-profile.sh --print-model`.
#
# Two rules govern the routing surface, and both are load-bearing:
#
#   FAIL CLOSED. If a model cannot be resolved, NO routing is emitted and the
#   reason is stated on stderr. The dispatch still succeeds and the implementer
#   keeps its harness-detected tier. A guessed model id is worse than none: it
#   would be recorded by the routing ledger as if it had been chosen on evidence.
#
#   BACKWARD COMPATIBLE. With no routing arguments and no routing env vars,
#   stdout is byte-identical to the pre-#3381 script. Both routing surfaces are
#   suffixes appended INSIDE an existing line's interpolation, so an empty
#   routing value adds no whitespace of its own. tests/dispatch-implementer-
#   routing.bats diffs against output captured from the pre-change script.
#
# Usage:
#   dispatch-implementer.sh --issue <N> --branch <name> --prompt-file <path>
#                            [--repo <O/R>] [--base <ref>]
#                            [--model <id>] [--provider <name>]
#                            [--kind <dispatch_kind>] [--labels <comma-list>]
#   dispatch-implementer.sh --issue <N> --branch <name> --cleanup
#
# Modes:
#   default: resolve branch, create worktree, emit augmented prompt on stdout.
#            BRANCH_VERDICT env is printed as a comment in the output header.
#   --cleanup: remove the worktree at /tmp/wt-<branch> and exit.
#
# Environment (mirrors the existing AUTOSPEC_DISPATCH_* style):
#   AUTOSPEC_DISPATCH_BASE_REF  base ref for the new worktree (default origin/main)
#   AUTOSPEC_DISPATCH_REPO      <owner>/<repo> for the open-PR rung
#   AUTOSPEC_DISPATCH_MODEL     model id to route this dispatch to
#   AUTOSPEC_DISPATCH_PROVIDER  provider/harness (claude|codex|opencode)
#   AUTOSPEC_DISPATCH_KIND      dispatch kind (default implementer)
#   AUTOSPEC_MODEL_PROFILES     catalog consulted by --labels resolution
#
# Exit codes:
#   0  success
#   1  bad arguments / worktree-guard.sh not found / invalid routing value
#   2  worktree creation/removal failed (propagated from worktree-guard.sh)
#   3  prompt-file missing or unreadable
#   4  dirty/wrong-branch reuse refused (propagated from worktree-guard.sh)

set -eu

usage() {
    cat <<'EOF'
Usage: dispatch-implementer.sh --issue <N> --branch <name> --prompt-file <path>
                                [--repo <O/R>] [--base <ref>]
                                [--model <id>] [--provider <name>]
                                [--kind <dispatch_kind>] [--labels <comma-list>]
       dispatch-implementer.sh --issue <N> --branch <name> --cleanup

Resolves the branch ladder verdict via worktree-guard.sh resolve-branch, then
creates /tmp/wt-<branch> via worktree-guard.sh create (no silent dirty reuse),
and pre-pends a worktree directive + verdict to the implementer prompt.
Pass --cleanup to remove the worktree after the implementer has completed.

Routing (optional):
  --model <id>        route this dispatch to a specific model id.
  --provider <name>   route it to a specific harness (claude|codex|opencode).
  --kind <kind>       dispatch kind recorded with the routing (default: implementer).
  --labels <list>     comma-separated issue labels; with no --model, the model is
                      resolved via select-model-profile.sh --print-model.

An explicit --model always beats --labels resolution. If no model can be
resolved, NO routing is emitted, the reason is printed on stderr, and the
implementer keeps its harness-detected tier — a model id is never guessed.
With no routing arguments and no AUTOSPEC_DISPATCH_MODEL/_PROVIDER/_KIND, stdout
is byte-identical to a dispatch from before routing existed.
EOF
}

ISSUE=""
BRANCH=""
PROMPT_FILE=""
CLEANUP=0
BASE_REF="${AUTOSPEC_DISPATCH_BASE_REF:-origin/main}"
REPO="${AUTOSPEC_DISPATCH_REPO:-}"
MODEL="${AUTOSPEC_DISPATCH_MODEL:-}"
PROVIDER="${AUTOSPEC_DISPATCH_PROVIDER:-}"
KIND="${AUTOSPEC_DISPATCH_KIND:-implementer}"
LABELS=""

while [ $# -gt 0 ]; do
    case "$1" in
        --issue)       ISSUE="$2"; shift 2 ;;
        --branch)      BRANCH="$2"; shift 2 ;;
        --prompt-file) PROMPT_FILE="$2"; shift 2 ;;
        --cleanup)     CLEANUP=1; shift ;;
        --base)        BASE_REF="$2"; shift 2 ;;
        --repo)        REPO="$2"; shift 2 ;;
        --model)       MODEL="$2"; shift 2 ;;
        --provider)    PROVIDER="$2"; shift 2 ;;
        --kind)        KIND="$2"; shift 2 ;;
        --labels)      LABELS="$2"; shift 2 ;;
        -h|--help)     usage; exit 0 ;;
        *)             echo "dispatch-implementer.sh: unknown arg: $1" >&2; usage >&2; exit 1 ;;
    esac
done

if [ -z "$ISSUE" ] || [ -z "$BRANCH" ]; then
    echo "dispatch-implementer.sh: --issue and --branch are required" >&2
    usage >&2
    exit 1
fi

# Routing values are interpolated into an HTML comment AND into JSON inside it.
# A value containing `-->` would close the machine-readable comment early and
# spill the remainder into the prompt as prose; a value containing `"` would
# break the JSON. Model ids, provider names, dispatch kinds and labels are all
# drawn from a narrow character set already, so restricting them is free and
# removes the injection surface rather than escaping around it.
_routing_charset_ok() {
    case "$1" in
        '')                          return 0 ;;
        *[!]A-Za-z0-9._:@/+[-]*)     return 1 ;;
    esac
    return 0
}

_routing_label_charset_ok() {
    case "$1" in
        '')                          return 0 ;;
        *[!]A-Za-z0-9._:@/+,[-]*)    return 1 ;;
    esac
    return 0
}

_reject_routing_value() {
    printf 'dispatch-implementer.sh: invalid %s value: %s\n' "$1" "$2" >&2
    printf 'dispatch-implementer.sh: routing values are limited to [A-Za-z0-9._:@/+[]-] (labels may also contain commas) so they cannot break out of the routing comment or its JSON\n' >&2
    exit 1
}

# Locate worktree-guard.sh: prefer the repo-local copy, then ~/.autospec/scripts/.
SCRIPT_DIR="$(cd "$(dirname "$0")" 2>/dev/null && pwd -P)" || SCRIPT_DIR=""
GUARD=""
if [ -n "$SCRIPT_DIR" ] && [ -x "$SCRIPT_DIR/worktree-guard.sh" ]; then
    GUARD="$SCRIPT_DIR/worktree-guard.sh"
elif [ -x "${HOME}/.autospec/scripts/worktree-guard.sh" ]; then
    GUARD="${HOME}/.autospec/scripts/worktree-guard.sh"
else
    echo "dispatch-implementer.sh: worktree-guard.sh not found (checked $SCRIPT_DIR and ~/.autospec/scripts/)" >&2
    exit 1
fi

WT_PATH="/tmp/wt-${BRANCH}"

if [ "$CLEANUP" -eq 1 ]; then
    if [ -d "$WT_PATH" ]; then
        git worktree remove --force "$WT_PATH" 2>/dev/null \
            || rm -rf "$WT_PATH" \
            || { echo "dispatch-implementer.sh: failed to remove $WT_PATH" >&2; exit 2; }
        git worktree prune 2>/dev/null || true
    fi
    exit 0
fi

# Validate routing values only on the dispatch path. --cleanup emits no routing,
# so a malformed AUTOSPEC_DISPATCH_* left in the operator's environment must not
# be able to strand a worktree: cleanup exits above, before this gate.
_routing_charset_ok "$MODEL"          || _reject_routing_value "--model" "$MODEL"
_routing_charset_ok "$PROVIDER"       || _reject_routing_value "--provider" "$PROVIDER"
_routing_charset_ok "$KIND"           || _reject_routing_value "--kind" "$KIND"
_routing_label_charset_ok "$LABELS"   || _reject_routing_value "--labels" "$LABELS"

if [ -z "$PROMPT_FILE" ] || [ ! -r "$PROMPT_FILE" ]; then
    echo "dispatch-implementer.sh: --prompt-file must point to a readable file" >&2
    exit 3
fi

# Step 0: resolve the routing decision. An explicit --model (or
# AUTOSPEC_DISPATCH_MODEL) always wins; --labels is consulted only when no model
# was stated. Resolution runs before worktree creation so an invalid routing
# argument costs nothing.
ROUTING_SOURCE=""
if [ -n "$MODEL" ]; then
    ROUTING_SOURCE="explicit"
elif [ -n "$LABELS" ]; then
    # Locate select-model-profile.sh across the three layouts it lives in: the
    # installed tree is flat, while in the repo it is a per-skill script and this
    # one is a top-level script. Same candidate order as route-decide.sh.
    SELECTOR=""
    for _cand in \
        "$SCRIPT_DIR/select-model-profile.sh" \
        "$SCRIPT_DIR/../skills/autospec-run/scripts/select-model-profile.sh" \
        "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/select-model-profile.sh"
    do
        if [ -f "$_cand" ]; then SELECTOR="$_cand"; break; fi
    done

    if [ -z "$SELECTOR" ]; then
        echo "dispatch-implementer.sh: model routing unresolved for labels '$LABELS': select-model-profile.sh not found; emitting no routing block — the implementer keeps its harness-detected tier" >&2
    else
        # select-model-profile.sh exits 3 and prints nothing when the catalog
        # states no model id for the resolved profile. Its documented contract is
        # that the caller then KEEPS ITS CLOUD TIER rather than guessing, so a
        # non-zero exit or empty output must produce no routing at all.
        _resolved=""
        _sel_exit=0
        _resolved="$(bash "$SELECTOR" --labels "$LABELS" --print-model 2>/dev/null)" || _sel_exit=$?
        _resolved="$(printf '%s' "$_resolved" | tr -d '\r\n')"
        if [ "$_sel_exit" -ne 0 ] || [ -z "$_resolved" ]; then
            echo "dispatch-implementer.sh: model routing unresolved for labels '$LABELS' (select-model-profile.sh --print-model exited $_sel_exit with no model id); emitting no routing block — the implementer keeps its harness-detected tier" >&2
        elif ! _routing_charset_ok "$_resolved"; then
            echo "dispatch-implementer.sh: model routing unresolved for labels '$LABELS': the catalog returned an unusable model id '$_resolved'; emitting no routing block — the implementer keeps its harness-detected tier" >&2
        else
            MODEL="$_resolved"
            ROUTING_SOURCE="profile"
        fi
    fi
fi

# Build both routing surfaces. They are EMPTY STRINGS when nothing routed, and
# each is interpolated as a suffix of an existing line, so an unrouted dispatch
# emits byte-identical output to the pre-routing script.
ROUTING_COMMENT_SUFFIX=""
ROUTING_BLOCK=""
if [ -n "$MODEL" ] || [ -n "$PROVIDER" ]; then
    _json_or_null() { if [ -n "$1" ]; then printf '"%s"' "$1"; else printf 'null'; fi; }
    ROUTING_JSON="$(printf '{"provider":%s,"model":%s,"kind":%s,"source":%s}' \
        "$(_json_or_null "$PROVIDER")" \
        "$(_json_or_null "$MODEL")" \
        "$(_json_or_null "$KIND")" \
        "$(_json_or_null "$ROUTING_SOURCE")")"
    ROUTING_COMMENT_SUFFIX="$(printf '\n<!-- dispatch-implementer: routing=%s -->' "$ROUTING_JSON")"

    _routing_lines=""
    if [ -n "$PROVIDER" ]; then
        _routing_lines="$_routing_lines$(printf '\n- provider: `%s`' "$PROVIDER")"
    fi
    if [ -n "$MODEL" ]; then
        _routing_lines="$_routing_lines$(printf '\n- model: `%s`' "$MODEL")"
    fi
    if [ -n "$KIND" ]; then
        _routing_lines="$_routing_lines$(printf '\n- kind: `%s`' "$KIND")"
    fi
    case "$ROUTING_SOURCE" in
        explicit) _routing_lines="$_routing_lines$(printf '\n- source: `explicit` (stated by the orchestrator)')" ;;
        profile)  _routing_lines="$_routing_lines$(printf '\n- source: `profile` (resolved from the issue labels via select-model-profile.sh)')" ;;
    esac

    # The machine comment alone is not enough: the agent reading this prompt is
    # what actually honours the routing, so it needs a human-readable directive
    # in the body — the same reason the branch verdict is surfaced twice.
    ROUTING_BLOCK="$(printf '\n\n**Model routing (authoritative):** dispatch this work with the routing below.\nDo NOT substitute a different model or provider, and do NOT silently fall back to\na default — if the named routing is unavailable, stop and report it.%s' "$_routing_lines")"
fi

# Step 1: resolve-branch ladder — surface the verdict BEFORE worktree creation
# so the orchestrator can decide open-pr / branch-only / fresh handling.
VERDICT_JSON=""
if [ -n "$REPO" ]; then
    VERDICT_JSON="$("$GUARD" resolve-branch --branch "$BRANCH" --repo "$REPO" 2>/dev/null || true)"
else
    # No --repo provided: skip the open-PR rung (requires gh), do branch-only check only.
    VERDICT_JSON='{"state":"fresh","pr":null}'
fi
# Normalise: if guard returned nothing, treat as fresh.
if [ -z "$VERDICT_JSON" ]; then
    VERDICT_JSON='{"state":"fresh","pr":null}'
fi

# Step 2: delegate worktree creation to worktree-guard.sh create.
# This enforces: no silent dirty reuse (exit 4 with code_health:worktree_dirty_reuse_refused),
# fetch-before-branch (G4), and idempotent clean reuse.
guard_exit=0
"$GUARD" create --branch "$BRANCH" --path "$WT_PATH" --base "$BASE_REF" 2>&1 || guard_exit=$?
if [ "$guard_exit" -ne 0 ]; then
    echo "dispatch-implementer.sh: worktree-guard.sh create exited $guard_exit for $WT_PATH (branch=$BRANCH)" >&2
    exit "$guard_exit"
fi

# Emit the augmented prompt: verdict header + worktree directive + original prompt body.
cat <<EOF
<!-- dispatch-implementer: branch_verdict=$VERDICT_JSON -->${ROUTING_COMMENT_SUFFIX}

**Branch verdict:** \`$VERDICT_JSON\`
The orchestrator should act on \`state\` before proceeding:
- \`open-pr\`: an open PR already exists — validate + merge the existing PR, no re-implementation.
- \`branch-only\`: branch exists remotely but no open PR — adopt in this worktree and continue.
- \`fresh\`: no prior work exists — proceed with new implementation.${ROUTING_BLOCK}

**Workdir:** \`$WT_PATH\` (worktree). All \`cd\`, \`git\`, \`gh\`, edit, and
test commands MUST run from this worktree. Do NOT touch the main checkout.
Do NOT \`git checkout\` other branches. This is parallel-safety isolation
per autospec-run Phase 4 worktree contract (issue #690).

Issue: #$ISSUE
Branch: $BRANCH

---

EOF
cat "$PROMPT_FILE"
