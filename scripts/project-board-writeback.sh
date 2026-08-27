#!/usr/bin/env bash
# scripts/project-board-writeback.sh — mirror autospec progress into the board's
# `AutoSpec state` single-select field.
#
# FAIL-OPEN BY CONTRACT: always exits 0. A write-back failure must never block a
# promotion or a merge — this script decorates a board, it does not gate work.
# Failures emit code_health:project_board_writeback_failed on stderr. Every skip
# and every failure prints a reason; nothing is swallowed silently.
#
# The two live boards name their state options differently (measured):
#   p2 "AutoSpec state":  Blocked, Ready, Planning, Implementation, Review, Testing, Done
#   p1 "Delivery status": Backlog, Ready, In progress, In review, Verify, Blocked, Done
# --state takes a CANONICAL state name and is resolved through an ordered
# candidate list of board option names (same mechanism project-board-resolve.sh
# uses to resolve the field name itself, applied one level down to option
# names). The first candidate present in the board's options wins. A canonical
# state not in the table falls back to a single candidate: its own literal
# name. This script NEVER creates a field or an option — when no candidate
# exists on the board, it skips.
#
# Usage: project-board-writeback.sh --plan FILE --item ITEM_ID --state NAME
#
# Exit codes: always 0 (fail-open by contract).

set -u

plan=""; item=""; state=""
while [ "$#" -gt 0 ]; do
    case "$1" in
        --plan)  plan="${2:-}";  shift 2 ;;
        --item)  item="${2:-}";  shift 2 ;;
        --state) state="${2:-}"; shift 2 ;;
        --help|-h)
            printf 'project-board-writeback.sh --plan FILE --item ID --state NAME\n'
            exit 0
            ;;
        *)
            printf 'project-board-writeback: unknown option: %s\n' "$1" >&2
            exit 0
            ;;
    esac
done

skip() { printf 'project-board-writeback: %s\n' "$1"; exit 0; }
fail() { printf 'code_health:project_board_writeback_failed %s\n' "$1" >&2; exit 0; }

if [ -z "$plan" ]; then
    skip "missing --plan"
fi
if [ ! -f "$plan" ]; then
    skip "plan file not found: $plan"
fi
if [ -z "$item" ]; then
    skip "missing --item"
fi
if [ -z "$state" ]; then
    skip "missing --state"
fi

command -v gh >/dev/null 2>&1 || skip "gh not found"
command -v jq >/dev/null 2>&1 || skip "jq not found"

# ── project_board.write_back gate — the single choke point ────────────────
# Both current callers (autonomous-promote-open-issues.sh's board_writeback()
# wrapper, and scripts/lib/autospec-loop.sh's _autospec_conductor_board_state
# at PR lifecycle points) reach a board mutation only by invoking this
# script — so the gate lives HERE, not duplicated in each caller. A future
# caller that forgets to check a flag still can't re-enable writes, because
# there is nothing to forget: this script always checks itself before it
# ever reaches `gh project item-edit` below.
#
# AUTOSPEC_PROJECT_BOARD_WRITE_BACK, when explicitly "0" or "1", wins (same
# override precedence every other AUTOSPEC_PROJECT_BOARD_* field uses — the
# promoter sets this once per run from the bridged YAML so this script does
# not need to re-invoke the Rust binary on every item). When unset (e.g. the
# conductor loop, which does not run the promoter's bridge step), this
# script re-derives the value itself from the same validated config via
# `autospec autonomous project-board-config`. Any failure along that path
# (binary missing, stale install without the subcommand, unreadable config)
# degrades to enabled — write-back was unconditional before this fix, so an
# absent/unreachable config must not silently start suppressing writes.
write_back_ok="${AUTOSPEC_PROJECT_BOARD_WRITE_BACK:-}"
if [ "$write_back_ok" != "0" ] && [ "$write_back_ok" != "1" ]; then
    wb_bin="${AUTOSPEC_PROJECT_BOARD_CONFIG_BIN:-${AUTOSPEC_BIN:-autospec}}"
    wb_repo_dir="${AUTOSPEC_REPO_DIR:-.}"
    wb_json="$("$wb_bin" autonomous project-board-config --repo-dir "$wb_repo_dir" 2>/dev/null || true)"
    write_back_ok=1
    if [ -n "$wb_json" ]; then
        # NOT `.write_back // true`: jq's `//` treats a literal `false` as
        # falsy too, so that expression would silently coerce an explicit
        # `write_back: false` back to "true". `if/then/else` tests the
        # value directly instead of falling through on any falsy value.
        wb_flag="$(printf '%s' "$wb_json" | jq -r 'if .write_back == false then "false" else "true" end' 2>/dev/null || true)"
        if [ "$wb_flag" = "false" ]; then
            write_back_ok=0
        fi
    fi
fi
if [ "$write_back_ok" = "0" ]; then
    skip "project_board.write_back is false; board mutation suppressed"
fi

# Guard degenerate input up front: a plan file that is not valid JSON must
# never crash this script. Every jq query below also uses `?`-guarded
# indexing so a JSON document with an unexpected shape (missing .fields,
# missing .items, .fields.autospec_state.options being null or the wrong
# type) degrades to "not found" rather than erroring.
if ! jq -e . "$plan" >/dev/null 2>&1; then
    skip "plan file is not valid JSON: $plan"
fi

# Probe the token's project scope at most once per run, not once per item.
# The caller (autonomous-promote-open-issues.sh) invokes this script once per
# item, often against the SAME --plan file across an entire cycle AND across
# separate cycles (--plan is the persistent, URL-keyed board cache under
# ~/.autospec/board-cache/, reused for as long as it stays within its TTL —
# not a fresh per-run temp file). Keying a disk cache to the plan path, as an
# earlier fix did, therefore caches the scope result FOREVER on that host: a
# probe made before the token had the `project` scope permanently disables
# write-back for that board, with no operator-visible signal (this script's
# skip message goes to stdout, which the promoter discards).
#
# Fix: the promoter computes the probe result at most once per run, as a
# plain in-process shell variable (never written to disk), and passes it to
# every child invocation via AUTOSPEC_PROJECT_BOARD_AUTH_OK. That variable
# starts unset in every fresh process, so a new run always re-probes — there
# is no file to go stale and nothing to clean up. When this script is run
# standalone (no caller-supplied value), it just probes live every time,
# which is exactly today's correct-if-wasteful fallback behavior.
auth_ok="${AUTOSPEC_PROJECT_BOARD_AUTH_OK:-}"
if [ "$auth_ok" != "0" ] && [ "$auth_ok" != "1" ]; then
    if gh auth status 2>&1 | grep -q "'project'"; then
        auth_ok=1
    else
        auth_ok=0
    fi
fi
if [ "$auth_ok" != "1" ]; then
    skip "token lacks the project scope; write-back disabled for this run"
fi

project_id="$(jq -r '.project?.id? // empty' "$plan" 2>/dev/null)"
if [ -z "$project_id" ]; then
    skip "plan has no project id"
fi

field_id="$(jq -r '.fields?.autospec_state?.id? // empty' "$plan" 2>/dev/null)"
if [ -z "$field_id" ]; then
    skip "board has no AutoSpec state field"
fi

options_json="$(jq -c '(.fields?.autospec_state?.options? // {}) | if type == "object" then . else {} end' "$plan" 2>/dev/null)"
if [ -z "$options_json" ]; then
    options_json='{}'
fi

item_found="$(jq -r --arg i "$item" '(.items? // [])[]? | select((.item_id? // "") == $i) | .item_id' "$plan" 2>/dev/null | head -n 1)"
if [ -z "$item_found" ]; then
    skip "item $item not found in plan"
fi

current="$(jq -r --arg i "$item" '(.items? // [])[]? | select((.item_id? // "") == $i) | (.autospec_state? // "")' "$plan" 2>/dev/null | head -n 1)"

# Canonical state -> ordered candidate board-option names. Not a fallback
# shim: a literal option name is exactly the kind of target-board specific
# the spec forbids hardcoding, so every canonical lifecycle state resolves
# through this table instead. Override with AUTOSPEC_PROJECT_BOARD_STATE_OPTIONS
# as `Canonical=Cand1|Cand2` entries separated by commas.
default_table="Blocked=Blocked,Ready=Ready,Done=Done,Implementation=Implementation|In progress,Review=Review|In review,Testing=Testing|Verify"
table="${AUTOSPEC_PROJECT_BOARD_STATE_OPTIONS:-$default_table}"

candidates_json="$(printf '%s' "$table" | jq -R -c '
  split(",") | map(select(length > 0) | split("="))
  | map(select(length == 2) | {(.[0]): (.[1] | split("|"))})
  | add // {}' 2>/dev/null)"
if [ -z "$candidates_json" ]; then
    candidates_json='{}'
fi

cand_list="$(jq -n -c --arg s "$state" --argjson t "$candidates_json" '$t[$s] // [$s]' 2>/dev/null)"
if [ -z "$cand_list" ]; then
    cand_list="[]"
fi

matched="$(jq -n -c --argjson cands "$cand_list" --argjson opts "$options_json" '
  first($cands[] as $c | select(($opts[$c] // null) != null) | {name: $c, id: $opts[$c]}) // empty' 2>/dev/null)"

if [ -z "$matched" ] || [ "$matched" = "null" ]; then
    skip "board has no AutoSpec state option matching $state"
fi

matched_name="$(printf '%s' "$matched" | jq -r '.name')"
option_id="$(printf '%s' "$matched" | jq -r '.id')"

# Idempotent: re-read the item's current (literal, board-side) state and skip
# a no-op write rather than issuing a redundant mutation.
if [ "$current" = "$matched_name" ]; then
    skip "item $item already in state $matched_name"
fi

if ! gh project item-edit --id "$item" --project-id "$project_id" \
        --field-id "$field_id" --single-select-option-id "$option_id" >/dev/null 2>&1; then
    fail "item=$item state=$state option=$matched_name"
fi

printf 'project-board-writeback: %s -> %s (%s)\n' "$item" "$matched_name" "$state"
exit 0
