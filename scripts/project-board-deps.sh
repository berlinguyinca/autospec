#!/usr/bin/env bash
# scripts/project-board-deps.sh — extract dependency edges from a board plan.
#
# Reads a normalized board plan on stdin, writes the same plan with
# .items[].blocked_by = [{"repo","number"}] added (plus .items[].deps_unresolvable
# and .items[].deps_reason — see below).
#
# Source precedence per item, first non-empty wins:
#   1. the project's `dependencies` custom field
#   2. the native `parent_issue` relation
#   3. the issue body's `## Dependencies` section(s)
#
# Issue bodies are untrusted DATA: only text following a dependency marker
# phrase (default "Blocked by" / "Depends on", case-insensitive, colon
# optional) inside a `## Dependencies` section is read. A "#123" anywhere
# else in the body — a problem statement, a comment, a fenced code block —
# has no effect. Bare `#N` resolves against the item's own repo; a bare ref
# is NEVER guessed against some other repo — an edge that points at an
# issue not on the board is handled downstream as unresolvable and fails
# closed, which is the desired behavior.
#
# The marker phrase set is configurable via AUTOSPEC_PROJECT_BOARD_DEP_MARKERS
# (comma-separated, e.g. "Blocked by,Depends on,Waiting on") so board-specific
# prose is a config change, not a code change.
#
# Unresolvable declared dependencies: if a `## Dependencies` section contains
# a marker phrase, does NOT say "none", and yields zero parseable #N
# references (e.g. prose like "Blocked by the whole IW-WB-001..078
# portfolio"), the item is NOT treated as unblocked. Instead
# .items[].deps_unresolvable is set true with a short .items[].deps_reason,
# so downstream readiness computation (project-board-deps.sh --resolve, a
# later stage) can fail it closed rather than promoting it. We deliberately
# do not try to parse such prose into edges — bodies are untrusted data and
# inferring edges from free text is exactly what this script refuses to do.
#
# --resolve turns on a second stage that computes readiness from
# .items[].blocked_by (populated by the extraction stage above, or already
# present on the input plan): adds .items[].ready (bool), .items[].reason
# (string), and a top-level .cycles array of participant-key lists.
#
#   - An edge is satisfied only when the referenced item's .state is
#     "closed". A reference that does not resolve to any item on the board
#     is unsatisfied (fails closed), never treated as satisfied.
#   - An item with .deps_unresolvable == true (set by the extraction stage
#     when a body declared a dependency marker but no #N could be parsed
#     from it) is never ready, regardless of its (empty) .blocked_by — an
#     unparseable declared dependency is not the same thing as no
#     dependency, and must not be promoted as if it were.
#   - Cycle detection is a fixpoint over resolvable (index-hit) edges only:
#     repeatedly mark items whose blockers are all already marked, until
#     nothing new gets marked. Anything left unmarked is either an actual
#     cycle member or depends (transitively) on one — both are genuinely
#     unschedulable, so both are reported as not-ready with a cycle reason.
#     This is deliberately imprecise about which is which; the reason text
#     never names a specific item as "in" the cycle, only that its
#     dependency chain contains one, so it never asserts something false
#     about an item that is merely downstream of a cycle.
#
# Pure filter: no network, no `gh`, no mutation. Never fails on degenerate
# input — malformed/missing shapes degrade gracefully rather than crashing.
#
# Usage: project-board-deps.sh [--resolve] < plan.json

set -eu

resolve=0
while [ "$#" -gt 0 ]; do
    case "$1" in
        --resolve) resolve=1; shift ;;
        --help|-h) printf 'project-board-deps.sh [--resolve] < plan.json\n'; exit 0 ;;
        *) printf 'project-board-deps: unknown option: %s\n' "$1" >&2; exit 2 ;;
    esac
done

# Dependency marker phrases, comma-separated, overridable per board.
markers_csv="${AUTOSPEC_PROJECT_BOARD_DEP_MARKERS:-Blocked by,Depends on}"

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

# Readiness resolution: reads the extracted plan (with .items[].blocked_by
# and .items[].deps_unresolvable) on stdin, adds .items[].ready,
# .items[].reason, and top-level .cycles.
#
# Cycle detection is an iterative fixpoint rather than unbounded recursion:
# repeatedly mark items whose (index-resolvable) blockers are all already
# marked, starting from items with no such blockers, for at most
# (item count + 1) rounds — a directed graph over N nodes cannot have a
# resolution chain longer than N, so that many rounds is always enough to
# reach the fixpoint. Anything never marked is in a cycle or depends on
# one. This avoids the unbounded/deep-recursion cost (and depth-cap
# guesswork) of a per-branch recursive walk on a board with many edges.
resolve_stage() {
jq '
  def key($r): "\($r.repo)#\($r.number)";

  (.items) as $items
  | (reduce $items[] as $i ({}; . + {(key($i)): $i})) as $index
  | def resolvable_refs($item):
      ($item.blocked_by // []) | map(key(.)) | map(select($index[.] != null));
  (reduce range(0; ($items | length) + 1) as $r
       ([];
         . as $resolved
         | ($items
            | map(select(. as $it | ($resolved | index(key($it))) | not))
            | map(select((resolvable_refs(.) - $resolved) | length == 0))
            | map(key(.))) as $newly
         | $resolved + $newly)
    ) as $resolved

  | ($items | map(key(.)) | map(select(. as $s | ($resolved | index($s)) | not))) as $cyclic

  | .cycles = (if ($cyclic | length) > 0 then [$cyclic] else [] end)
  | .items |= map(
      . as $item
      | key($item) as $k
      | (.blocked_by // []) as $bl
      | ($bl | map(select($index[key(.)] == null)) | map(key(.))) as $missing
      | ($bl | map(select($index[key(.)] != null and $index[key(.)].state != "closed")) | map(key(.))) as $open
      | if ($item.deps_unresolvable // false) then
          . + {ready: false,
               reason: "unresolvable declared dependency: \($item.deps_reason // "no #N reference could be parsed from a declared dependency")"}
        elif ($cyclic | index($k)) then
          . + {ready: false, reason: "dependency chain contains a cycle (unschedulable)"}
        elif ($missing | length) > 0 then
          . + {ready: false, reason: "unresolvable blocker: \($missing | join(", "))"}
        elif ($open | length) > 0 then
          . + {ready: false, reason: "blocked-by \($open | join(", "))"}
        else . + {ready: true, reason: "all blockers satisfied"} end)'
}

printf '%s' "$parsed" | jq --arg markers_csv "$markers_csv" '
  # Regex-escape a literal string (used for user-configured marker phrases,
  # never for board-derived data).
  def rxesc:
    explode
    | map(
        . as $c
        | if ([46,94,36,124,40,41,91,93,123,125,42,43,63,92] | index($c)) then [92, $c]
          else [$c] end)
    | flatten
    | implode;

  def marker_pats:
    $markers_csv
    | split(",")
    | map(gsub("^\\s+|\\s+$"; ""))
    | map(select(length > 0))
    | map(rxesc);

  def marker_alt: (marker_pats | join("|"));

  # Extract every #N (optionally owner/repo#N) reference from arbitrary text.
  # Never applied to raw body text directly — only to already-isolated
  # dependency-field values or a single matched marker line.
  def refs($text; $self_repo):
    (if ($text | type) == "string" then $text else "" end) as $t
    | [$t | scan("(?:([A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+))?#([0-9]+)")]
    | map({repo: (.[0] // $self_repo), number: (.[1] | tonumber)})
    | unique;

  # Strip fenced code blocks and HTML comments before anything else touches
  # the body, so a "#666" or "Blocked by:" sitting inside either has no
  # effect anywhere (Dependencies section or not). Non-greedy + dotall so
  # a single block does not swallow unrelated surrounding text.
  def strip_noise:
    (if (. | type) == "string" then . else "" end)
    | gsub("(?s)```.*?```"; "")
    | gsub("(?s)<!--.*?-->"; "");

  # All "## Dependencies" section bodies, up to their next "## " heading,
  # unioned (a body may have more than one such heading) and joined so a
  # later global marker scan sees every sections text.
  def deps_sections:
    (. | split("\n## "))
    | map(select(startswith("Dependencies") or startswith("## Dependencies")))
    | join("\n");

  # Every dependency-declaration outcome found in the section text: for each
  # marker-phrase occurrence, classify its trailing text as "none" (no
  # edges, not an error), parseable (contributes refs), or unresolvable (a
  # marker was used but no #N reference could be parsed from it).
  def dep_outcomes($sec; $self_repo):
    ($sec | [match("(?i)(?:" + marker_alt + ")\\s*:?\\s*([^\\n]*)"; "g")]) as $ms
    | $ms | map(
        (.captures[0].string // "") | gsub("^\\s+|\\s+$"; "") as $rest
        | if ($rest | test("^none\\b"; "i")) then
            {kind: "none", refs: []}
          else
            (refs($rest; $self_repo)) as $r
            | if ($r | length) > 0 then {kind: "parsed", refs: $r}
              else {kind: "unresolvable", refs: []} end
          end);

  def from_body($item):
    ($item.body | strip_noise | deps_sections) as $sec
    | (if (marker_alt | length) > 0 then dep_outcomes($sec; $item.repo) else [] end) as $outs
    | ([$outs[] | select(.kind == "parsed") | .refs[]] | unique) as $parsed_refs
    | (($outs | map(select(.kind == "unresolvable")) | length) > 0) as $saw_unresolvable
    | if ($parsed_refs | length) > 0 then
        {refs: $parsed_refs, unresolvable: false, reason: null}
      elif $saw_unresolvable then
        {refs: [], unresolvable: true,
         reason: "## Dependencies section names a marker phrase but no #N reference could be parsed from it"}
      else
        {refs: [], unresolvable: false, reason: null}
      end;

  def field_refs($item; $key):
    ($item[$key]) as $v
    | if ($v | type) == "string" then refs($v; $item.repo) else [] end;

  # An item with none of these three source keys at all gives extraction
  # nothing to derive from; in that case an already-populated blocked_by
  # (e.g. a plan piped straight into --resolve, or extraction run twice)
  # is preserved rather than clobbered with a freshly-derived empty result.
  def has_dep_source($item):
    ($item | has("dependencies")) or ($item | has("parent_issue")) or ($item | has("body"));

  .items |= map(
    . as $item
    | if has_dep_source($item) then
        (field_refs($item; "dependencies")) as $f
        | (field_refs($item; "parent_issue")) as $p
        | (if ($f | length) > 0 then {refs: $f, unresolvable: false, reason: null}
           elif ($p | length) > 0 then {refs: $p, unresolvable: false, reason: null}
           else from_body($item) end) as $result
        | . + {
            blocked_by: $result.refs,
            deps_unresolvable: $result.unresolvable,
            deps_reason: $result.reason
          }
      else
        . + {
            blocked_by: (.blocked_by // []),
            deps_unresolvable: (.deps_unresolvable // false),
            deps_reason: (.deps_reason // null)
          }
      end)' | if [ "$resolve" -eq 1 ]; then resolve_stage; else cat; fi
