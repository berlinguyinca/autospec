#!/usr/bin/env bash
# scripts/pi-performance-dashboard.sh — replay a fixed execution ledger and
# snapshot the Pi performance dashboard plus its routing advice (issue #3327).
#
# Provider-neutral by contract: the live card, the historical summary and the
# advisory read the same record shapes autospec_core::aar::dashboard consumes,
# and they stay in lockstep with it on the percentile rule (nearest-rank, ceil,
# integer arithmetic) and the Wilson success lower bound. The locked model
# family (qwen3.8 for this deployment) is an input, not a rule of the tool, and
# the advice never reports a different family.
#
# Ledger record (JSONL, one completed issue per line; required keys first):
#   {"issue_id":"3300","ts":"2026-08-01T00:00:00Z",
#    "model_family":"qwen3.8","profile":"qwen3.8-27b-q4/rtx4090",
#    "duration_ms":61000,"succeeded":true,
#    "node_id":"node-1","state":"merged",
#    "wall_ms":61000,"cost_micros":0,
#    "time_to_passing_change_ms":56000}
#   required: issue_id ts model_family profile duration_ms succeeded
#   optional: node_id state wall_ms cost_micros time_to_passing_change_ms
#   issue ids must be unique; a second row for the same issue double-counts it.
#
# Live record (one JSON object, every key required):
#   issue_id state node_id profile turns context_tokens ttft_ms
#   decode_tokens_per_second cache_hit_rate tool_ms test_ms repair_count
#   queue_ms
#
# Usage:
#   pi-performance-dashboard.sh --ledger <jsonl> [--live <json>]
#   [--window-hours <n>] [--min-samples <n>] [--static-profile <name>]
#   [--model-family <family>]
#
# Advice: a profile is eligible when it is inside the locked family, has at
# least the configured sample count, and clears the 0.80 Wilson success lower
# bound; the cheapest eligible profile (then shortest wall, then key) wins.
# With no eligible profile the static selection is kept, and the reported
# model family never leaves the locked one.
#
# Exit codes:
#   0  ok
#   1  bad arguments or invalid ledger/live record
#   2  jq missing (fail-closed — the numbers must not be approximated)

set -u

usage() { sed -n '2,/^$/p' "$0" | sed 's/^# \{0,1\}//'; }
die() { printf 'pi-performance-dashboard: %s\n' "$1" >&2; exit "${2:-1}"; }

command -v jq >/dev/null 2>&1 || die 'jq is required (fail-closed)' 2

ledger=""
live=""
window_hours=24
min_samples=20
static_profile="qwen3.8-coding-local"
model_family="qwen3.8"

while [ $# -gt 0 ]; do
    case "$1" in
        --help) usage; exit 0 ;;
        -h) usage; exit 0 ;;
        --ledger) [ $# -ge 2 ] || die "missing value for --ledger"; ledger="$2"; shift 2 ;;
        --live) [ $# -ge 2 ] || die "missing value for --live"; live="$2"; shift 2 ;;
        --window-hours) [ $# -ge 2 ] || die "missing value for --window-hours"; window_hours="$2"; shift 2 ;;
        --min-samples) [ $# -ge 2 ] || die "missing value for --min-samples"; min_samples="$2"; shift 2 ;;
        --static-profile) [ $# -ge 2 ] || die "missing value for --static-profile"; static_profile="$2"; shift 2 ;;
        --model-family) [ $# -ge 2 ] || die "missing value for --model-family"; model_family="$2"; shift 2 ;;
        *) die "unknown option: $1" ;;
    esac
done

[ -n "$ledger" ] || die "ledger is required (--ledger)"
[ -f "$ledger" ] && [ -r "$ledger" ] || die "ledger not found: $ledger"
if [ -n "$live" ]; then
    [ -f "$live" ] && [ -r "$live" ] || die "live record not found: $live"
fi
case "$window_hours" in '' | *[!0-9]*) die "window-hours must be a positive integer" ;; esac
[ "$window_hours" -gt 0 ] || die "window-hours must be a positive integer"
case "$min_samples" in '' | *[!0-9]*) die "min-samples must be a positive integer" ;; esac
[ "$min_samples" -gt 0 ] || die "min-samples must be a positive integer"
[ -n "$static_profile" ] || die "static-profile must not be empty"
[ -n "$model_family" ] || die "model-family must not be empty"

# ── ledger validation: a bad row would poison every derived number ───────────
ledger_err="$(jq -rs '
  [ to_entries[] | . as $e |
    (if ($e.value | type) != "object"
       then "line \($e.key + 1): not a JSON object"
       else
         ([ "issue_id", "ts", "model_family", "profile", "duration_ms", "succeeded" ]
          | map(select(. as $k | ($e.value | has($k)) | not))) as $missing |
         (if ($missing | length) > 0
            then "line \($e.key + 1): missing required key \($missing | join(","))"
            elif (($e.value.issue_id | type) != "string" or ($e.value.issue_id | length) == 0
                  or ($e.value.ts | type) != "string" or ($e.value.ts | length) == 0
                  or ($e.value.model_family | type) != "string" or ($e.value.model_family | length) == 0
                  or ($e.value.profile | type) != "string" or ($e.value.profile | length) == 0)
            then "line \($e.key + 1): issue_id/ts/model_family/profile must be non-empty strings"
            elif ($e.value.duration_ms | type) != "number" or ($e.value.duration_ms < 0)
            then "line \($e.key + 1): duration_ms must be a non-negative number"
            elif ($e.value.succeeded | type) != "boolean"
            then "line \($e.key + 1): succeeded must be a boolean"
            elif ($e.value | has("time_to_passing_change_ms"))
                 and (($e.value.time_to_passing_change_ms | type) != "number"
                      or ($e.value.time_to_passing_change_ms < 0))
            then "line \($e.key + 1): time_to_passing_change_ms must be null or a non-negative number"
            elif (($e.value | has("wall_ms")) and (($e.value.wall_ms | type) != "number" or ($e.value.wall_ms < 0)))
                 or (($e.value | has("cost_micros")) and (($e.value.cost_micros | type) != "number" or ($e.value.cost_micros < 0)))
            then "line \($e.key + 1): wall_ms and cost_micros must be non-negative numbers"
            else empty
            end)
       end)
  ] | .[]' "$ledger" 2>/dev/null)" || die "ledger is not valid JSON: $ledger"
[ -z "$ledger_err" ] || die "$(printf '%s\n' "$ledger_err" | head -n 1)"

dupes="$(jq -rs '
  [ .[] | select(type == "object") | .issue_id ]
  | group_by(.) | map(select(length > 1) | .[0]) | .[]
  | "duplicate issue_id \(.)"' "$ledger" 2>/dev/null || true)"
[ -z "$dupes" ] || die "$(printf '%s\n' "$dupes" | head -n 1)"

# ── live record validation ───────────────────────────────────────────────────
if [ -n "$live" ]; then
    live_err="$(jq -r '
      . as $row |
      (["issue_id", "state", "node_id", "profile", "turns", "context_tokens", "ttft_ms",
        "decode_tokens_per_second", "cache_hit_rate", "tool_ms", "test_ms",
        "repair_count", "queue_ms"]) as $keys |
      (if ($row | type) != "object"
         then "live record is not a JSON object"
         elif ([$keys[] | . as $k | select($row | has($k) | not)] | length) > 0
         then "live record missing required key \([$keys[] | . as $k | select($row | has($k) | not)] | join(","))"
         elif ([(($row.turns | type) != "number" or ($row.turns < 0)),
                (($row.context_tokens | type) != "number" or ($row.context_tokens < 0)),
                (($row.ttft_ms | type) != "number" or ($row.ttft_ms < 0)),
                (($row.tool_ms | type) != "number" or ($row.tool_ms < 0)),
                (($row.test_ms | type) != "number" or ($row.test_ms < 0)),
                (($row.repair_count | type) != "number" or ($row.repair_count < 0)),
                (($row.queue_ms | type) != "number" or ($row.queue_ms < 0))] | any)
         then "live record numeric fields must be non-negative numbers"
         elif (($row.decode_tokens_per_second | type) != "number" or ($row.decode_tokens_per_second < 0))
         then "decode_tokens_per_second must be a non-negative number"
         elif (($row.cache_hit_rate | type) != "number" or ($row.cache_hit_rate < 0) or ($row.cache_hit_rate > 1))
         then "cache_hit_rate must be within 0.0..=1.0"
         else empty
         end)' "$live" 2>/dev/null)" || die "live record is not valid JSON: $live"
    [ -z "$live_err" ] || die "$live_err"
fi

# ── history + advisory in one deterministic pass ─────────────────────────────
stats="$(wh="$window_hours" ms="$min_samples" fam="$model_family" jq -s '
  ($ENV.wh | tonumber) as $window |
  ($ENV.ms | tonumber) as $min_samples |
  $ENV.fam as $family |
  def rankv($p): ((($p * length) + 99) / 100) | floor;
  def wilson($n; $k):
    if $n == 0 then 0.0
    else
      ($k / $n) as $p |
      (1.96 * 1.96) as $z2 |
      ((($p + ($z2 / (2 * $n))) - (1.96 * (((($p * (1 - $p)) / $n) + ($z2 / (4 * $n * $n))) | sqrt)))
       / (1 + ($z2 / $n)))
      | if . < 0 then 0.0 elif . > 1 then 1.0 else . end
    end;
  . as $rows |
  ($rows | map(.duration_ms) | sort) as $durations |
  ($rows | map(.time_to_passing_change_ms | select(type == "number")) | sort) as $passing |
  ([ $rows[] | select(.succeeded) ] | length) as $successes |
  ($rows | group_by([.model_family, .profile]) | map(
      (.[0].model_family) as $mf |
      (.[0].profile) as $pf |
      (length) as $n |
      ([ .[] | select(.succeeded) ] | length) as $k |
      {
        model_family: $mf,
        profile: $pf,
        samples: $n,
        successes: $k,
        mean_cost_micros: (([ .[] | (.cost_micros // 0) ] | add) / $n),
        mean_wall_ms: (([ .[] | (.wall_ms // 0) ] | add) / $n),
        lower_bound: wilson($n; $k),
        reason:
          (if $mf != $family then "family"
           elif $n < $min_samples then "samples"
           elif (wilson($n; $k) < 0.8) then "success"
           else "eligible"
           end)
      })) as $candidates |
  ($candidates | map(select(.reason == "eligible"))) as $eligible |
  {
    samples: ($rows | length),
    p50_ms: (if ($durations | length) > 0 then $durations[(rankv(50)) - 1] else null end),
    p90_ms: (if ($durations | length) > 0 then $durations[(rankv(90)) - 1] else null end),
    p95_ms: (if ($durations | length) > 0 then $durations[(rankv(95)) - 1] else null end),
    successes: $successes,
    per_hour: ($successes / $window),
    median_passing_ms: (if ($passing | length) > 0 then $passing[(rankv(50)) - 1] else null end),
    candidates: $candidates,
    winner: ($eligible | sort_by(.mean_cost_micros, .mean_wall_ms, .profile) | (.[0].profile // ""))
  }' "$ledger" 2>/dev/null)" || die "ledger failed the stats pass: $ledger"

printf 'pi-performance dashboard\n'
printf 'model-family: %s\n' "$model_family"
printf 'min-samples: %s\n' "$min_samples"
printf '\n'

if [ -n "$live" ]; then
    printf 'live:\n'
    jq -r '
      "  issue_id: \(.issue_id)"
      + "\n  state: \(.state)"
      + "\n  node_id: \(.node_id)"
      + "\n  profile: \(.profile)"
      + "\n  turns: \(.turns)"
      + "\n  context_tokens: \(.context_tokens)"
      + "\n  ttft_ms: \(.ttft_ms)"
      + "\n  decode_tokens_per_second: \((.decode_tokens_per_second * 10 | round) / 10)"
      + "\n  cache_hit_rate: \((.cache_hit_rate * 100 | round) / 100)"
      + "\n  tool_ms: \(.tool_ms)"
      + "\n  test_ms: \(.test_ms)"
      + "\n  repair_count: \(.repair_count)"
      + "\n  queue_ms: \(.queue_ms)"' "$live"
    printf '\n'
fi

printf 'history:\n'
printf '  samples: %s\n' "$(jq -r '.samples' <<<"$stats")"
printf '  p50_ms: %s\n' "$(jq -r '.p50_ms // "n/a"' <<<"$stats")"
printf '  p90_ms: %s\n' "$(jq -r '.p90_ms // "n/a"' <<<"$stats")"
printf '  p95_ms: %s\n' "$(jq -r '.p95_ms // "n/a"' <<<"$stats")"
printf '  successful_issues_per_hour: %s\n' "$(printf '%.2f' "$(jq -r '.per_hour' <<<"$stats")")"
printf '  median_time_to_passing_ms: %s\n' "$(jq -r '.median_passing_ms // "n/a"' <<<"$stats")"
printf '\n'

winner="$(jq -r '.winner' <<<"$stats")"
if [ -z "$winner" ]; then
    advice_source="static"
    advice_profile="$static_profile"
else
    advice_source="benchmark"
    advice_profile="$winner"
fi

printf 'advice:\n'
printf '  source: %s\n' "$advice_source"
printf '  model-family: %s\n' "$model_family"
printf '  profile: %s\n' "$advice_profile"
printf '  rationale:\n'
fam="$model_family" ms="$min_samples" jq -r '
  $ENV.ms as $min_samples |
  $ENV.fam as $family |
  .candidates[]
  | (if .reason == "family"
       then "\(.profile): model_family \(.model_family) is not the locked family \($family); excluded"
       elif .reason == "samples"
       then "\(.profile): \(.samples) samples below configured \($min_samples)"
       elif .reason == "success"
       then "\(.profile): success lower bound \((.lower_bound * 100 | round) / 100) below 0.80"
       else "\(.profile): eligible (lower bound \((.lower_bound * 100 | round) / 100), cost \(.mean_cost_micros | floor) micros, wall \(.mean_wall_ms | floor) ms)"
       end)' <<<"$stats" | sed 's/^/    /'
if [ -z "$winner" ]; then
    printf '    keeping static profile %s: no eligible benchmark candidate\n' "$static_profile"
else
    printf '    selected %s: cheapest eligible profile under locked family %s\n' "$winner" "$model_family"
fi
