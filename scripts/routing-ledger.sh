#!/usr/bin/env bash
# scripts/routing-ledger.sh — append-only JSONL outcome ledger for model routing.
#
# Records what each dispatch COST and how it turned out, so routing can be scored
# on measured effective cost instead of sticker price. Contract deliberately
# mirrors skills/autospec-shared/scripts/explore-ledger.sh (append-only, outcome
# enum, --stats, --show, --validate, --rebuild-shaped reader semantics) so there
# is one ledger idiom in the repo rather than two.
#
# Record schema (all keys required for --append):
#   {dispatch_id, ts, dispatch_kind, profile, model, harness, issue,
#    cell_ctx, cell_reasoning, input_tokens, output_tokens, cached_tokens,
#    wall_clock_ms, retries, escalated, outcome, reason}
#
#     dispatch_id     string, unique per dispatch attempt (NOT per issue — an
#                     issue can be dispatched many times)
#     dispatch_kind   implementer | lgtm-reviewer | explore-researcher |
#                     verify-voter | refine-lens | qa-sweep | secaudit-pass |
#                     growth-lens | spec-decompose
#                     Present from the FIRST record on purpose: retrofitting a
#                     key dimension into an append-only ledger would invalidate
#                     every historical row and every stats query.
#     profile         model-profiles.yml profile name that served the dispatch
#     model           concrete model id actually dispatched
#     cell_ctx        32k | 64k | 120k        (the routing cell's ctx ordinal)
#     cell_reasoning  shallow | medium | deep  (the routing cell's tier)
#     cached_tokens   prompt-cache hits; feeds the cache-penalty term, which is
#                     the difference between a cheap model and a cheap dispatch
#     escalated       true when the dispatch pulled in a stronger advisor/tier
#     outcome         pending | merged_clean | lgtm_first_pass | retried_ok |
#                     escalated | qa_failed | reverted | abandoned
#
# Append-only audit trail: --update-outcome appends a NEW copy of the record with
# an updated outcome/reason/ts rather than rewriting history. Readers (--show /
# --stats) take the LATEST line per dispatch_id.
#
# Usage:
#   routing-ledger.sh --append '<json-object>'
#   routing-ledger.sh --update-outcome <dispatch_id> <outcome> [reason]
#   routing-ledger.sh --stats [--json]
#   routing-ledger.sh --stats --group-by <dim>[,<dim>...] [--cell k=v[,k=v...]] [--json]
#   routing-ledger.sh --show [--profile <name>] [--kind <dispatch_kind>] [--json]
#   routing-ledger.sh --validate [<file>]
#   routing-ledger.sh -h | --help
#
# Multidimensional stats (§24-§28): --group-by takes §24 dimensions comma-separated
# in nesting order, so `--group-by model,context_band` reports each (model, band)
# cell inside its model parent. Every §26 metric comes back per cell with an
# `evidence` label — observed (n >= AUTOSPEC_ROUTING_MIN_SAMPLES, raw rate),
# smoothed (0 < n < min_samples, backed off one level to the parent by
# (hits + a*parent_rate)/(n + a)), or unknown (n == 0 — an empty cell is never a
# prior and no vendor claim outranks an observation, §27). The label states the
# sample-size class, so with alpha=0 a smoothed cell's rate is its raw rate.
# Medians, percentiles, tok/s and the cache ratio stay RAW: smoothing a median
# invents a number the ledger never saw. Un-landed #3174 fields group as
# "unknown" over a 0 denominator. alpha/min_samples come from routing-cost.sh
# --jq-prelude unless AUTOSPEC_ROUTING_ALPHA / AUTOSPEC_ROUTING_MIN_SAMPLES say so.
#
# Ledger path (precedence): --ledger <path> > $AUTOSPEC_ROUTING_LEDGER
#   > .autospec/routing-ledger.jsonl
#
# Exit codes:
#   0  ok / valid
#   1  invalid object/line, bad arguments, or --update-outcome id not found
#   2  jq missing (fail-closed — this is a data-integrity tool)
#
# Requires bash 3.2+ and jq. jq is MANDATORY and the script fails closed without
# it: silently degrading a data-integrity tool is worse than refusing to run.

set -u

ALLOWED_OUTCOMES="pending merged_clean lgtm_first_pass retried_ok escalated qa_failed reverted abandoned"
ALLOWED_KINDS="implementer lgtm-reviewer explore-researcher verify-voter refine-lens qa-sweep secaudit-pass growth-lens spec-decompose"
ALLOWED_CTX="32k 64k 120k"
ALLOWED_REASONING="shallow medium deep"

# §24 grouping dimensions. `runtime` and `concurrency` are logical names resolved
# to the fields that exist today (harness, concurrency_at_start); the rest are
# record fields, present now or landing with #3174. Nothing is invented: a record
# without the field groups into "unknown" instead of being dropped.
GROUP_DIMS="provider model model_version hardware_fingerprint runtime quantization role dispatch_kind language repository context_band concurrency risk"
SCRIPT_DIR="$(cd "$(dirname "$0")" 2>/dev/null && pwd)" || SCRIPT_DIR="."

REQUIRED_KEYS="dispatch_id ts dispatch_kind profile model harness issue cell_ctx cell_reasoning input_tokens output_tokens cached_tokens wall_clock_ms retries escalated outcome reason"

_usage() { sed -n '2,/^$/p' "$0" | sed 's/^# \{0,1\}//'; }
_die() { printf 'routing-ledger: %s\n' "$1" >&2; exit "${2:-1}"; }

if ! command -v jq >/dev/null 2>&1; then
    _die 'jq is required (data-integrity tool, fails closed)' 2
fi

LEDGER="${AUTOSPEC_ROUTING_LEDGER:-.autospec/routing-ledger.jsonl}"
MODE=""
JSON_OUT=0
FILTER_PROFILE=""
FILTER_KIND=""
GROUP_BY=""
CELL_SPEC=""
ARG1=""; ARG2=""; ARG3=""

while [ $# -gt 0 ]; do
    case "$1" in
        -h|--help) _usage; exit 0 ;;
        --ledger)
            if [ $# -lt 2 ]; then _die '--ledger requires a path'; fi
            LEDGER="$2"; shift 2 ;;
        --json) JSON_OUT=1; shift ;;
        --profile)
            if [ $# -lt 2 ]; then _die '--profile requires a name'; fi
            FILTER_PROFILE="$2"; shift 2 ;;
        --kind)
            if [ $# -lt 2 ]; then _die '--kind requires a dispatch_kind'; fi
            FILTER_KIND="$2"; shift 2 ;;
        --append)
            if [ $# -lt 2 ]; then _die '--append requires a JSON object'; fi
            MODE="append"; ARG1="$2"; shift 2 ;;
        --update-outcome)
            if [ $# -lt 3 ]; then _die '--update-outcome requires <dispatch_id> <outcome>'; fi
            MODE="update"; ARG1="$2"; ARG2="$3"
            shift 3
            if [ $# -gt 0 ]; then
                case "$1" in -*) ;; *) ARG3="$1"; shift ;; esac
            fi ;;
        --stats)    MODE="stats"; shift ;;
        --group-by)
            if [ $# -lt 2 ]; then _die '--group-by requires a comma-separated dimension list'; fi
            GROUP_BY="$2"; shift 2 ;;
        --cell)
            if [ $# -lt 2 ]; then _die '--cell requires k=v[,k=v...]'; fi
            CELL_SPEC="$2"; shift 2 ;;
        --show)     MODE="show"; shift ;;
        --validate)
            MODE="validate"
            shift
            if [ $# -gt 0 ]; then
                case "$1" in -*) ;; *) ARG1="$1"; shift ;; esac
            fi ;;
        *) _die "unknown option: $1" ;;
    esac
done

if [ -z "$MODE" ]; then
    _usage >&2
    exit 1
fi
if [ -n "$GROUP_BY" ] && [ "$MODE" != "stats" ]; then
    _die '--group-by requires --stats'
fi
if [ -n "$CELL_SPEC" ] && [ -z "$GROUP_BY" ]; then
    _die '--cell requires --group-by'
fi

_in_list() {
    for _w in $2; do
        if [ "$_w" = "$1" ]; then return 0; fi
    done
    return 1
}

# _validate_object <json> — echo nothing on success; print reason + fail otherwise.
_validate_object() {
    _obj="$1"
    if ! printf '%s' "$_obj" | jq -e 'type == "object"' >/dev/null 2>&1; then
        printf 'not a JSON object\n'
        return 1
    fi
    for _k in $REQUIRED_KEYS; do
        if ! printf '%s' "$_obj" | jq -e --arg k "$_k" 'has($k)' >/dev/null 2>&1; then
            printf 'missing required key: %s\n' "$_k"
            return 1
        fi
    done
    _oc="$(printf '%s' "$_obj" | jq -r '.outcome')"
    if ! _in_list "$_oc" "$ALLOWED_OUTCOMES"; then
        printf 'invalid outcome: %s\n' "$_oc"
        return 1
    fi
    _dk="$(printf '%s' "$_obj" | jq -r '.dispatch_kind')"
    if ! _in_list "$_dk" "$ALLOWED_KINDS"; then
        printf 'invalid dispatch_kind: %s\n' "$_dk"
        return 1
    fi
    _cx="$(printf '%s' "$_obj" | jq -r '.cell_ctx')"
    if ! _in_list "$_cx" "$ALLOWED_CTX"; then
        printf 'invalid cell_ctx: %s\n' "$_cx"
        return 1
    fi
    _cr="$(printf '%s' "$_obj" | jq -r '.cell_reasoning')"
    if ! _in_list "$_cr" "$ALLOWED_REASONING"; then
        printf 'invalid cell_reasoning: %s\n' "$_cr"
        return 1
    fi
    _validate_counters "$_obj"
}

# _validate_counters <json> — numeric/boolean half of the record contract.
_validate_counters() {
    _obj="$1"
    # Counters must be non-negative numbers, never strings: the cost formula
    # divides by them and a string would silently poison every derived weight.
    if ! printf '%s' "$_obj" | jq -e '
        (.input_tokens|type=="number") and (.output_tokens|type=="number") and
        (.cached_tokens|type=="number") and (.wall_clock_ms|type=="number") and
        (.retries|type=="number") and
        (.input_tokens>=0) and (.output_tokens>=0) and (.cached_tokens>=0) and
        (.wall_clock_ms>=0) and (.retries>=0)' >/dev/null 2>&1; then
        printf 'token/wall-clock/retry counters must be non-negative numbers\n'
        return 1
    fi
    if ! printf '%s' "$_obj" | jq -e '.escalated|type=="boolean"' >/dev/null 2>&1; then
        printf 'escalated must be a boolean\n'
        return 1
    fi
    # cached_tokens is a subset of input_tokens; a ratio above 1 means the caller
    # is double-counting and would produce a cache penalty below its true floor.
    if ! printf '%s' "$_obj" | jq -e '.cached_tokens <= .input_tokens' >/dev/null 2>&1; then
        printf 'cached_tokens may not exceed input_tokens\n'
        return 1
    fi
    return 0
}

# Latest line per dispatch_id, preserving first-seen order.
_latest_records() {
    if [ ! -f "$LEDGER" ]; then
        printf '[]'
        return 0
    fi
    jq -s 'map(select(type=="object" and has("dispatch_id")))
           | group_by(.dispatch_id)
           | map(.[-1])' "$LEDGER" 2>/dev/null || printf '[]'
}

# ---------------------------------------------------------------------------
# Multidimensional stats (--stats --group-by, spec §24-§28)
# ---------------------------------------------------------------------------

# _group_by_error <csv> — echo why the dimension list is bad (empty, outside §24,
# duplicated); empty stdout means valid. jq does the set maths, not bash.
_group_by_error() {
    printf '%s' "$1" | jq -Rr --arg allow "$GROUP_DIMS" '
        ($allow | split(" ")) as $ok | split(",") as $d
        | ($d | map(select(. as $x | $ok | index($x) | not))) as $bad
        | ($d | group_by(.) | map(select(length > 1) | .[0])) as $dup
        | if ($d | length) == 0 or ($d | any(. == "")) then "empty dimension list"
          elif ($bad | length) > 0 then "unknown dimension(s): " + ($bad | join(","))
          elif ($dup | length) > 0 then "duplicate dimension(s): " + ($dup | join(","))
          else empty end'
}

# _smoothing_contract — resolve alpha/min_samples and the smooth() helper from
# routing-cost.sh so ledger and scorer cannot disagree; env vars win, and a
# non-numeric value fails closed rather than falling back silently.
_smoothing_contract() {
    _sib=""
    for _cand in "$SCRIPT_DIR/routing-cost.sh" "$SCRIPT_DIR/../scripts/routing-cost.sh" \
                 "${AUTOSPEC_SCRIPTS_DIR:-}/routing-cost.sh"; do
        if [ -n "$_cand" ] && [ -f "$_cand" ]; then _sib="$_cand"; break; fi
    done
    if [ -z "$_sib" ]; then _die 'cannot locate routing-cost.sh (the smoothing contract lives there)' 2; fi
    _pre="$(bash "$_sib" --jq-prelude 2>/dev/null)" \
        || _die 'routing-cost.sh --jq-prelude failed (cannot resolve the smoothing contract)' 2
    SMOOTH_JQ="$(printf '%s' "$_pre" | jq -r '.jq // empty')"
    if [ -z "$SMOOTH_JQ" ]; then _die 'routing-cost.sh --jq-prelude returned no smooth() definition' 2; fi
    ALPHA="${AUTOSPEC_ROUTING_ALPHA:-$(printf '%s' "$_pre" | jq -r '.alpha')}"
    MIN_SAMPLES="${AUTOSPEC_ROUTING_MIN_SAMPLES:-$(printf '%s' "$_pre" | jq -r '.min_samples')}"
    if ! printf '%s' "$ALPHA" | jq -e 'type == "number"' >/dev/null 2>&1; then
        _die "AUTOSPEC_ROUTING_ALPHA is not a number: $ALPHA"
    fi
    if ! printf '%s' "$MIN_SAMPLES" | jq -e 'type == "number"' >/dev/null 2>&1; then
        _die "AUTOSPEC_ROUTING_MIN_SAMPLES is not a number: $MIN_SAMPLES"
    fi
}

# linter:allow-COMPLEXITY the body below is jq data, not bash flow
# _grouped_stats_program — the §24/§26 reducer, emitted raw. The caller prepends
# SMOOTH_JQ: jq-1.6 needs every def before the pipeline, smooth/3 included.
_grouped_stats_program() {
    cat <<'JQPROG'
# dimv/2 — one dimension value for one record (§24). `context_band` is the cell_ctx
# ordinal, falling back to a band DERIVED from context_used (#3174) at the same
# cell boundaries the router dispatches on. `runtime` falls back to `harness`.
# A value the record does not carry is "unknown", never a guess.
def dimv($r; $d):
  if $d == "context_band" then
    (if ($r.cell_ctx // "unknown") != "unknown" then $r.cell_ctx
     elif (($r.context_used // null) | type) == "number" then
       (if $r.context_used <= 32000 then "32k"
        elif $r.context_used <= 64000 then "64k"
        elif $r.context_used <= 120000 then "120k" else "256k" end)
     else "unknown" end)
  elif $d == "runtime" then (($r.runtime // $r.harness // "unknown") | tostring)
  elif $d == "concurrency" then (($r.concurrency_at_start // "unknown") | tostring)
  else (($r[$d] // "unknown") | tostring) end;

def keyf($r; $l): [ $dims[0:$l][] as $d | dimv($r; $d) ] | join("\u001f");

# median_of/1 — jq-1.6-safe (the half index is bound before use); jq's own
# `median` rejects empty arrays, and every median here may span an empty cell.
def median_of($a): ($a | length) as $n | ($n / 2 | floor) as $h
  | if $n == 0 then null
    elif ($n % 2) == 1 then $a[$h]
    else (($a[$h - 1] + $a[$h]) / 2) end;

# pctl/2 — nearest-rank, index clamped so P95 over 3 rows is the max, not null.
def pctl($a; $q): ($a | length) as $n
  | if $n == 0 then null
    else ((((($n * $q) | ceil) - 1)) as $i
          | (if $i < 0 then 0 elif $i >= $n then $n - 1 else $i end) as $j
          | $a[$j]) end;

def mean_unk($a): ($a | length) as $m | if $m == 0 then "unknown" else (($a | add) / $m) end;

def nums($rows; $f): [ $rows[] | select((.[$f] | type) == "number") | .[$f] ];
def cnt($rows; $f): [ $rows[] | (.[$f] // 0) ] | add // 0;
def strcount($rows; $f; $v): [ $rows[] | select((.[$f] | type) == "string" and .[$f] == $v) ] | length;
def known($rows; $f): [ $rows[] | select((.[$f] | type) == "string" and .[$f] != "" and .[$f] != "unknown") ] | length;

# agg/1 — raw counts for one cell; every rate derives from these, so a parent's
# aggregate has the same shape and can back a thin cell up.
def agg($rows):
  ($rows | length) as $n
  | {
      dispatches:       $n,
      first_pass:       [ $rows[] | select(.outcome == "merged_clean" or .outcome == "lgtm_first_pass") ] | length,
      eventual_success: [ $rows[] | select(.outcome == "merged_clean" or .outcome == "lgtm_first_pass" or .outcome == "retried_ok") ] | length,
      escalations:      [ $rows[] | select(.escalated == true) ] | length,
      reverts:          [ $rows[] | select(.reverted == true or .outcome == "reverted") ] | length,
      review_known:     known($rows; "review_outcome"),
      review_rejected:  strcount($rows; "review_outcome"; "rejected"),
      tests_known:      known($rows; "tests_outcome"),
      tests_failed:     strcount($rows; "tests_outcome"; "failed"),
      retries_total:    cnt($rows; "retries"),
      retries_sorted:   [ $rows[] | (.retries // 0) ] | sort,
      wall_sorted:      nums($rows; "wall_clock_ms") | sort,
      prompt_sorted:    nums($rows; "prompt_tok_s") | sort,
      decode_sorted:    nums($rows; "decode_tok_s") | sort,
      aggregate_sorted: nums($rows; "aggregate_decode_tok_s") | sort,
      cost_success:     [ $rows[] | select(.cost_usd != null) | select(.outcome == "merged_clean" or .outcome == "lgtm_first_pass" or .outcome == "retried_ok") ] | length,
      cost_total:       (nums($rows; "cost_usd") | add // 0),
      input_tokens:     cnt($rows; "input_tokens"),
      output_tokens:    cnt($rows; "output_tokens"),
      cached_tokens:    cnt($rows; "cached_tokens"),
      wall_clock_ms:    (nums($rows; "wall_clock_ms") | add // 0)
    };

# rate/6 — §26/§27's evidence rule: 0 observations -> "unknown" (an empty cell is
# never a prior); >= min_samples -> the raw rate; below that, backed off one level
# toward the PARENT's rate with $alpha pseudo-counts (a superset's denominator is
# always >= ours). $den is the metric's own denominator, which is smaller than
# the cell count when a #3174 telemetry field is missing on some rows.
def rate($n; $hits; $den; $phits; $pden):
  if $den == 0 then "unknown"
  elif $n >= $min_samples then ($hits / $den)
  elif $pden > 0 then smooth($hits; $den; ($phits / $pden))
  else ($hits / $den) end;

def cellobj($vals; $names):
  [ $names, $vals ] | transpose | map({ key: .[0], value: .[1] }) | from_entries;

# row($vals; $own; $par) — one §26 metric block for one cell. Rates go through
# rate/6 (smoothed when thin); medians, percentiles, tok/s and the cache ratio
# are raw: smoothing a median invents a number the ledger never saw.
def row($vals; $own; $par):
  $own.dispatches as $n
  | (if $par == null then agg([]) else $par end) as $p
  | ($dims | length) as $k
  | cellobj($vals; $dims) as $cell
  | (if $n == 0 then "unknown" elif $n >= $min_samples then "observed" else "smoothed" end) as $ev
  | $cell + {
      evidence: $ev,
      parent: (if ($n == 0 or $k <= 1) then null else cellobj($vals[0:$k - 1]; $dims[0:$k - 1]) end),
      parent_dispatches: (if $n == 0 then 0 else $p.dispatches end),
      dispatches: $n,
      first_pass: $own.first_pass,
      first_pass_rate: rate($n; $own.first_pass; $n; $p.first_pass; $p.dispatches),
      eventual_success: $own.eventual_success,
      eventual_success_rate: rate($n; $own.eventual_success; $n; $p.eventual_success; $p.dispatches),
      mean_retries: rate($n; $own.retries_total; $n; $p.retries_total; $p.dispatches),
      median_retries: (if ($own.retries_sorted | length) == 0 then "unknown"
                       else (median_of($own.retries_sorted) // "unknown") end),
      escalations: $own.escalations,
      escalation_rate: rate($n; $own.escalations; $n; $p.escalations; $p.dispatches),
      review_rejections: $own.review_rejected,
      review_rejection_rate: rate($n; $own.review_rejected; $own.review_known; $p.review_rejected; $p.review_known),
      test_failures: $own.tests_failed,
      test_failure_rate: rate($n; $own.tests_failed; $own.tests_known; $p.tests_failed; $p.tests_known),
      reverts: $own.reverts,
      revert_rate: rate($n; $own.reverts; $n; $p.reverts; $p.dispatches),
      median_completion_ms: (if ($own.wall_sorted | length) == 0 then "unknown"
                             else (median_of($own.wall_sorted) // "unknown") end),
      p95_completion_ms: (if ($own.wall_sorted | length) == 0 then "unknown"
                          else (pctl($own.wall_sorted; 0.95) // "unknown") end),
      prompt_tok_s: mean_unk($own.prompt_sorted),
      decode_tok_s: mean_unk($own.decode_sorted),
      aggregate_decode_tok_s: mean_unk($own.aggregate_sorted),
      cache_hit_ratio: (if $own.input_tokens > 0 then ($own.cached_tokens / $own.input_tokens)
                        else "unknown" end),
      # Denominator is the PRICED successes: cost on 2 of 6 rows divided by 6
      # successes would fabricate a unit cost three times cheaper than measured.
      cost_per_success: (if $own.cost_success == 0 then "unknown"
                         else ($own.cost_total / $own.cost_success) end),
      input_tokens:  $own.input_tokens,
      output_tokens: $own.output_tokens,
      cached_tokens: $own.cached_tokens,
      wall_clock_ms: $own.wall_clock_ms
    };

# parse_cell/1 — "k=v[,k=v]" -> [[k,v],...], parsed in jq rather than in bash so a
# value may contain a space. An entry that is not k=v, names a dimension outside
# --group-by, or leaves one unaddressed is an error the caller must see.
def parse_cell($spec):
  if $spec == "" then []
  else ([ $spec | split(",")[]
          | if (split("=") | length) < 2 then { bad: . }
            else { k: (split("=") | .[0]), v: (split("=") | .[1:] | join("=")) } end ]) as $kv
    | ($kv | map(select(has("bad"))) | map(.bad) | join(",")) as $bad
    | ($kv | map(select(has("k")) | .k)) as $ks
    | if $bad != "" then "invalid --cell entry (expected k=v): " + $bad
      elif ($ks - $dims | length) > 0 then "unknown --cell dimension(s): " + ($ks - $dims | join(","))
      elif ($dims - $ks | length) > 0 then
        "--cell must address every --group-by dimension; missing: " + ($dims - $ks | join(","))
      else ($kv | map([.k, .v])) end
  end;

. as $records
| ($dims | length) as $k
| parse_cell($cell_spec) as $cell
| if ($cell | type) == "string" then { __error: $cell } else
  # One aggregate per grouping LEVEL (0 = everything, k = the full cell) so a
  # thin cell can find the parent it backs off to in a single pass.
  (reduce [range(0; $k + 1)][] as $l ({};
      .[$l | tostring] = ($records
        | group_by([ $dims[0:$l][] as $d | dimv(.; $d) ])
        | map({ key: ($l as $ll | keyf(.[0]; $ll)), value: agg(.) })
        | from_entries))) as $levels
  | [ $levels[$k | tostring] | to_entries[]
      | .key as $key
      | .value as $own
      | ($key | split("\u001f")) as $vals
      | ($levels[($k - 1 | tostring)][($vals[0:$k - 1] | join("\u001f"))] // null) as $par
      | row($vals; $own; $par) ] as $rows
  | if ($cell | length) == 0 then $rows
    else
      ([ $rows[] | . as $r | select(all($cell[]; . as $kv | $r[$kv[0]] == $kv[1])) ]) as $hit
      | if ($hit | length) > 0 then $hit
        else
          # The requested cell simply has no dispatches: report the cell with
          # every metric "unknown" rather than reporting nothing at all.
          ([ $dims[] as $d | (([ $cell[] | select(.[0] == $d) | .[1] ]) | (.[0] // "unknown")) ]) as $cvals
          | [ row($cvals; agg([]); null) ]
        end
    end
  end
JQPROG
}

# _grouped_tsv <dims-json> — flattened dim columns first, then every §26 metric.
_grouped_tsv() {
    jq -r --argjson dims "$1" '
        def v($o; $c): ($o[$c]) as $x | if $x == null then "-" else ($x | tostring) end;
        ($dims + ["dispatches", "evidence", "first_pass_rate", "eventual_success_rate",
                  "mean_retries", "median_retries", "escalation_rate", "review_rejection_rate",
                  "test_failure_rate", "revert_rate", "median_completion_ms", "p95_completion_ms",
                  "prompt_tok_s", "decode_tok_s", "aggregate_decode_tok_s", "cache_hit_ratio",
                  "cost_per_success"]) as $cols  # every §26 metric, one column each
        | ($cols | @tsv),
          (.[] | ([ $cols[] as $c | v(.; $c) ] | @tsv))'
}

# _stats_grouped — run the reducer over the latest-per-dispatch_id records.
# Pending dispatches have no outcome to score, exactly as in plain --stats.
_stats_grouped() {
    _reason="$(_group_by_error "$GROUP_BY")"
    [ -z "$_reason" ] || _die "invalid --group-by: $_reason"
    _dims_json="$(printf '%s' "$GROUP_BY" | jq -Rc 'split(",")')"
    _smoothing_contract
    _recs="$(_latest_records | jq -c 'map(select(.outcome != "pending"))')"
    if ! _out="$(printf '%s' "$_recs" | jq -c \
            --argjson dims "$_dims_json" --arg cell_spec "$CELL_SPEC" \
            --argjson alpha "$ALPHA" --argjson min_samples "$MIN_SAMPLES" \
            "${SMOOTH_JQ}$(_grouped_stats_program)" 2>&1)"; then
        _die "grouped stats failed: $_out"
    fi
    _err="$(printf '%s' "$_out" | jq -r 'if type == "object" and has("__error") then ."__error" else empty end' 2>/dev/null)"
    if [ -n "${_err:-}" ]; then _die "$_err"; fi
    if [ "$JSON_OUT" -eq 1 ]; then
        printf '%s' "$_out" | jq '.'
    else
        printf '%s' "$_out" | _grouped_tsv "$_dims_json"
    fi
}

case "$MODE" in
    append)
        if ! _reason="$(_validate_object "$ARG1")"; then
            _die "$_reason"
        fi
        _dir="$(dirname "$LEDGER")"
        if [ ! -d "$_dir" ]; then mkdir -p "$_dir"; fi
        printf '%s\n' "$(printf '%s' "$ARG1" | jq -c '.')" >> "$LEDGER"
        exit 0
        ;;

    update)
        if ! _in_list "$ARG2" "$ALLOWED_OUTCOMES"; then
            _die "invalid outcome: $ARG2"
        fi
        if [ ! -f "$LEDGER" ]; then
            _die "--update-outcome: dispatch_id not found: $ARG1"
        fi
        _prev="$(_latest_records | jq -c --arg id "$ARG1" '.[] | select(.dispatch_id==$id)')"
        if [ -z "$_prev" ]; then
            _die "--update-outcome: dispatch_id not found: $ARG1"
        fi
        # Append a NEW record rather than rewriting: the ledger is an audit trail.
        printf '%s' "$_prev" | jq -c \
            --arg oc "$ARG2" --arg rs "$ARG3" \
            --arg ts "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
            '.outcome=$oc | .ts=$ts | (if $rs != "" then .reason=$rs else . end)' >> "$LEDGER"
        exit 0
        ;;

    validate)
        _file="${ARG1:-$LEDGER}"
        if [ ! -f "$_file" ]; then
            # A missing ledger is "no data yet", not corruption.
            exit 0
        fi
        _lineno=0
        _rc=0
        while IFS= read -r _line || [ -n "$_line" ]; do
            _lineno=$((_lineno + 1))
            if [ -z "$_line" ]; then continue; fi
            if ! _reason="$(_validate_object "$_line")"; then
                printf 'routing-ledger: %s:%d: %s\n' "$_file" "$_lineno" "$_reason" >&2
                _rc=1
            fi
        done < "$_file"
        exit "$_rc"
        ;;

    show)
        _recs="$(_latest_records)"
        if [ -n "$FILTER_PROFILE" ]; then
            _recs="$(printf '%s' "$_recs" | jq -c --arg p "$FILTER_PROFILE" '[.[]|select(.profile==$p)]')"
        fi
        if [ -n "$FILTER_KIND" ]; then
            _recs="$(printf '%s' "$_recs" | jq -c --arg k "$FILTER_KIND" '[.[]|select(.dispatch_kind==$k)]')"
        fi
        if [ "$JSON_OUT" -eq 1 ]; then
            printf '%s' "$_recs" | jq '.'
        else
            printf '%s' "$_recs" | jq -r '.[] |
                "\(.dispatch_kind)\t\(.profile)\t\(.cell_ctx)/\(.cell_reasoning)\t\(.outcome)\tretries=\(.retries)\tms=\(.wall_clock_ms)"'
        fi
        exit 0
        ;;

    stats)
        if [ -n "$GROUP_BY" ]; then _stats_grouped; exit 0; fi
        # One row per (dispatch_kind, profile, cell). These are exactly the
        # coordinates the effective-cost formula scores, so the shape is the
        # contract: routing-cost.sh consumes this and nothing else.
        _stats="$(_latest_records | jq '
            map(select(.outcome != "pending"))
            | group_by([.dispatch_kind, .profile, .cell_ctx, .cell_reasoning])
            | map({
                dispatch_kind: .[0].dispatch_kind,
                profile:       .[0].profile,
                cell_ctx:      .[0].cell_ctx,
                cell_reasoning:.[0].cell_reasoning,
                dispatches:    length,
                first_pass:    (map(select(.outcome=="merged_clean" or .outcome=="lgtm_first_pass")) | length),
                failed:        (map(select(.outcome=="qa_failed" or .outcome=="reverted" or .outcome=="abandoned")) | length),
                escalations:   (map(select(.escalated)) | length),
                retries_total: (map(.retries) | add // 0),
                input_tokens:  (map(.input_tokens) | add // 0),
                output_tokens: (map(.output_tokens) | add // 0),
                cached_tokens: (map(.cached_tokens) | add // 0),
                wall_clock_ms: (map(.wall_clock_ms) | add // 0)
              })
            | map(. + {
                first_pass_rate: (if .dispatches > 0 then (.first_pass / .dispatches) else 0 end),
                failure_rate:    (if .dispatches > 0 then (.failed / .dispatches) else 0 end),
                escalation_rate: (if .dispatches > 0 then (.escalations / .dispatches) else 0 end),
                mean_retries:    (if .dispatches > 0 then (.retries_total / .dispatches) else 0 end),
                cache_hit_ratio: (if .input_tokens > 0 then (.cached_tokens / .input_tokens) else 0 end),
                mean_wall_clock_ms: (if .dispatches > 0 then (.wall_clock_ms / .dispatches) else 0 end)
              })')"
        if [ "$JSON_OUT" -eq 1 ]; then
            printf '%s' "$_stats" | jq '.'
        else
            printf '%s' "$_stats" | jq -r '.[] |
                "\(.dispatch_kind)\t\(.profile)\t\(.cell_ctx)/\(.cell_reasoning)\tn=\(.dispatches)\tfirst_pass=\(.first_pass_rate)\tesc=\(.escalation_rate)\tcache=\(.cache_hit_ratio)"'
        fi
        exit 0
        ;;
esac
