#!/usr/bin/env bash
# scripts/project-board-deps.sh — extract dependency edges from a board plan.
#
# Reads a normalized board plan on stdin, writes the same plan with
# .items[].blocked_by = [{"repo","number"}] added.
#
# Source precedence per item, first non-empty wins:
#   1. the project's `dependencies` custom field
#   2. the native `parent_issue` relation
#   3. the issue body's `## Dependencies` section
#
# Issue bodies are untrusted DATA: only the text following a `Blocked by:`
# marker inside the `## Dependencies` section is read. A "#123" anywhere
# else in the body — a problem statement, a comment, a code block — has no
# effect. Bare `#N` resolves against the item's own repo; `owner/repo#N`
# is cross-repo.
#
# Pure filter: no network, no `gh`, no mutation. Never fails on degenerate
# input — malformed/missing shapes degrade gracefully rather than crashing.
#
# Usage: project-board-deps.sh < plan.json
#
# NOTE for Task 6: argument parsing lives in the loop below so a --resolve
# flag can be added here without restructuring.

set -eu

while [ "$#" -gt 0 ]; do
    case "$1" in
        --help|-h) printf 'project-board-deps.sh < plan.json\n'; exit 0 ;;
        *) printf 'project-board-deps: unknown option: %s\n' "$1" >&2; exit 2 ;;
    esac
done

# Parse stdin and guard degenerate cases. Never fail on malformed input.
stdin_data="$(cat)"

# Try to parse JSON. If it fails, exit 0 silently with no output.
if ! parsed="$(printf '%s' "$stdin_data" | jq '.' 2>/dev/null)"; then
    exit 0
fi

# Ensure .items exists and is an array. If missing or wrong type, pass through unchanged.
if ! printf '%s' "$parsed" | jq -e '.items | type == "array"' >/dev/null 2>&1; then
    printf '%s' "$parsed"
    exit 0
fi

printf '%s' "$parsed" | jq '
  # Extract every #N (optionally owner/repo#N) reference from arbitrary text.
  # Never applied to raw body text directly — only to already-isolated
  # dependency-field values or the Blocked-by line inside the Dependencies
  # section.
  def refs($text; $self_repo):
    (if ($text | type) == "string" then $text else "" end) as $t
    | [$t | scan("(?:([A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+))?#([0-9]+)")]
    | map({repo: (.[0] // $self_repo), number: (.[1] | tonumber)})
    | unique;

  # Slice out just the "## Dependencies" section of the body, up to the
  # next "## " heading. Guards non-string / missing / null bodies.
  def deps_section:
    (if (. | type) == "string" then . else "" end) as $b
    | ($b | split("\n## "))
    | (map(select(startswith("Dependencies") or startswith("## Dependencies"))) | first) // "";

  # Within the Dependencies section, only the text after a "Blocked by:"
  # marker (up to end of line) is dependency data. Anything else in the
  # section — surrounding prose, footnotes — is inert.
  def blocked_text:
    . as $sec
    | ([$sec | match("(?i)blocked\\s+by\\s*:\\s*([^\\n]*)")]) as $m
    | if ($m | length) > 0 then ($m[0].captures[0].string // "") else "" end;

  def from_body($item):
    ($item.body | deps_section) as $sec
    | ($sec | blocked_text) as $bt
    | if ($bt == "" or ($bt | test("^\\s*none\\b"; "i"))) then []
      else refs($bt; $item.repo) end;

  def field_refs($item; $key):
    ($item[$key]) as $v
    | if ($v | type) == "string" then refs($v; $item.repo) else [] end;

  .items |= map(
    . as $item
    | (field_refs($item; "dependencies")) as $f
    | (field_refs($item; "parent_issue")) as $p
    | . + {blocked_by:
        (if ($f | length) > 0 then $f
         elif ($p | length) > 0 then $p
         else from_body($item) end)})'
