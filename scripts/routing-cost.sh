#!/usr/bin/env bash
# scripts/routing-cost.sh — score candidate profiles on measured EFFECTIVE cost.
#
# Derived-scoring half of the routing pair, mirroring the
# explore-ledger.sh / explore-source-weights.sh split: routing-ledger.sh owns
# the data, this script owns the formula and consumes only its --stats output.
#
# The central idea, which is counterintuitive enough to state plainly: a local
# model that fails 60% of the time and escalates to Opus costs MORE than
# dispatching Sonnet once. Free-per-token is not free-per-merged-PR.
#
#   effective_cost = unit_cost * (1 + E[retries]) * cache_penalty
#                  + P(escalate) * advisor_unit_cost
#                  + P(fail)     * fallback_unit_cost
#
# Rate estimation is Bayesian-smoothed exactly as explore-source-weights.sh does
# (alpha pseudo-count toward a prior) so small-N cells are not overconfident and
# weights CONVERGE rather than oscillate. The priors differ from explore's in one
# deliberate way: cost-increasing terms shrink toward a PESSIMISTIC prior
# (retries -> 1, escalation -> 0.5, failure -> 0.5) while the quality term
# shrinks toward 0.5. An unproven profile therefore looks EXPENSIVE, never cheap
# — the opposite choice would let a brand-new local model win every cell on zero
# evidence, which is the failure this whole design exists to prevent.
#
# cache_penalty makes prompt-cache behaviour first class, and is the other half
# of "token effective". Autospec pre-stages large context into every dispatch;
# a cloud tier reuses that prefix across dispatches while most local runtimes
# have no cross-request prompt cache at all, so the same prefix is re-processed
# every time. A profile that breaks caching costs materially more than its
# per-token price implies:
#
#   cache_penalty = 1 + CACHE_BETA * (1 - cache_hit_ratio)
#
# Unit costs come from the profile catalog (model-profiles.yml):
#   cost_in / cost_out    USD per million input / output tokens (cloud)
#   cost_minute           USD-equivalent opportunity cost per wall-clock minute
#                         (local; GPU-minutes are not free, see R9)
#   max_wall_clock_ms     per-profile latency ceiling; a profile whose MEASURED mean
#                         exceeds it is ineligible however cheap it looks (R9)
#   cache_min_tokens      the model's prompt-cache MINIMUM. Below it nothing is
#                         cached, so a hit ratio measured under a larger prefix
#                         must not be credited to a dispatch that cannot cache.
#                         These differ sharply — Haiku 4.5 needs 4096 tokens where
#                         Opus 5 needs 512 — which makes the cheapest per-token
#                         profile the easiest one to fall under.
# A profile with NO cost keys is NOT scoreable and is reported ineligible, so the
# caller falls back to its existing choice rather than guessing a price.
#
# Usage:
#   routing-cost.sh --kind <dispatch_kind> --ctx <32k|64k|120k>
#                   --reasoning <shallow|medium|deep>
#                   --candidates <p1,p2,...>
#                   [--profiles-file <path>] [--stats-file <path>]
#                   [--alpha N] [--min-samples N] [--floor F] [--json|--explain]
#
# Output (--json): array sorted by effective_cost ascending, each entry
#   {profile, unit_cost, n, first_pass_rate, mean_retries, escalation_rate,
#    cache_hit_ratio, effective_cost, eligible, reason}
#
# Environment:
#   AUTOSPEC_MODEL_PROFILES        profile catalog (default ~/.autospec/model-profiles.yml)
#   AUTOSPEC_ROUTING_LEDGER        forwarded to routing-ledger.sh
#   AUTOSPEC_ROUTING_ALPHA         smoothing pseudo-count (default 5)
#   AUTOSPEC_ROUTING_MIN_SAMPLES   samples before a profile may win (default 10)
#   AUTOSPEC_ROUTING_FIRST_PASS_FLOOR  quality floor (default 0.6)
#   AUTOSPEC_ROUTING_CACHE_BETA    cache-penalty strength (default 0.5)
#   AUTOSPEC_ROUTING_CLOUD_MULTIPLIER  penalty on token-priced (cloud) profiles as
#                         the token budget runs down; from routing-budget-hint.sh (R10).
#                         Default 1.0 = no distortion. Local profiles are priced in
#                         wall clock and consume no token budget, so they are exempt.
#   AUTOSPEC_ROUTING_PREFIX_TOKENS  size of the prefix this dispatch will stage,
#                         used only to test it against each profile's
#                         cache_min_tokens. 0 (default) = unknown, which FAILS OPEN
#                         and scores exactly as before; a telemetry gap must never
#                         invent a penalty.
#   AUTOSPEC_ROUTING_MAX_WALL_CLOCK_MS  global latency ceiling, 0 = none (default 0).
#                         For a weeks-long autonomous run, 40 GPU-minutes on an issue a
#                         cheap cloud tier finishes in 90s is a throughput REGRESSION,
#                         not a saving. A per-profile max_wall_clock_ms overrides this.
#
# Exit codes: 0 ok | 1 bad arguments | 2 jq missing (fail-closed)

set -u

_die() { printf 'routing-cost: %s\n' "$1" >&2; exit "${2:-1}"; }

if ! command -v jq >/dev/null 2>&1; then
    _die 'jq is required (fails closed)' 2
fi

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd -P)"
PROFILES_FILE="${AUTOSPEC_MODEL_PROFILES:-$HOME/.autospec/model-profiles.yml}"
STATS_FILE=""
KIND=""; CTX=""; REASONING=""; CANDIDATES=""
ALPHA="${AUTOSPEC_ROUTING_ALPHA:-5}"
MIN_SAMPLES="${AUTOSPEC_ROUTING_MIN_SAMPLES:-10}"
FLOOR="${AUTOSPEC_ROUTING_FIRST_PASS_FLOOR:-0.6}"
CACHE_BETA="${AUTOSPEC_ROUTING_CACHE_BETA:-0.5}"
MAX_WALL_MS="${AUTOSPEC_ROUTING_MAX_WALL_CLOCK_MS:-0}"
CLOUD_MULT="${AUTOSPEC_ROUTING_CLOUD_MULTIPLIER:-1.0}"
PREFIX_TOKENS="${AUTOSPEC_ROUTING_PREFIX_TOKENS:-0}"
JSON_OUT=0
EXPLAIN=0

while [ $# -gt 0 ]; do
    case "$1" in
        -h|--help) sed -n '2,/^$/p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        --kind)          KIND="${2:-}"; shift 2 ;;
        --ctx)           CTX="${2:-}"; shift 2 ;;
        --reasoning)     REASONING="${2:-}"; shift 2 ;;
        --candidates)    CANDIDATES="${2:-}"; shift 2 ;;
        --profiles-file) PROFILES_FILE="${2:-}"; shift 2 ;;
        --stats-file)    STATS_FILE="${2:-}"; shift 2 ;;
        --alpha)         ALPHA="${2:-}"; shift 2 ;;
        --min-samples)   MIN_SAMPLES="${2:-}"; shift 2 ;;
        --floor)         FLOOR="${2:-}"; shift 2 ;;
        --cache-beta)    CACHE_BETA="${2:-}"; shift 2 ;;
        --max-wall-clock-ms) MAX_WALL_MS="${2:-}"; shift 2 ;;
        --cloud-multiplier)  CLOUD_MULT="${2:-}"; shift 2 ;;
        --prefix-tokens)     PREFIX_TOKENS="${2:-}"; shift 2 ;;
        --json)          JSON_OUT=1; shift ;;
        --explain)       EXPLAIN=1; shift ;;
        *) _die "unknown option: $1" ;;
    esac
done

if [ -z "$KIND" ] || [ -z "$CTX" ] || [ -z "$REASONING" ] || [ -z "$CANDIDATES" ]; then
    _die 'required: --kind --ctx --reasoning --candidates'
fi

# ── profile catalog: unit cost per profile ────────────────────────────────────
# Emits {profile: {cost_in, cost_out, cost_minute}} for the candidates only.
# Parses the same two layouts select-model-profile.sh handles (top-level blocks
# and profiles: nesting), scoped to each profile's own block.
_catalog() {
    if [ ! -f "$PROFILES_FILE" ]; then
        printf '{}'
        return 0
    fi
    awk '
        function ind(s) { match(s, /^ */); return RLENGTH }
        {
            line = $0
            sub(/[[:space:]]*#.*$/, "", line)
            if (line ~ /^[[:space:]]*$/) next
            i = ind(line)
            key = line
            sub(/^[[:space:]]+/, "", key)
            if (cur != "" && i <= blocki) { cur = "" }
            if (key ~ /^[^:]+:[[:space:]]*$/) {
                name = key; sub(/:[[:space:]]*$/, "", name)
                if (name != "profiles") { cur = name; blocki = i }
                next
            }
            if (cur == "") next
            split(key, kv, ":")
            k = kv[1]
            v = key; sub(/^[^:]*:[[:space:]]*/, "", v)
            gsub(/[[:space:]]+$/, "", v)
            if (k == "cost_in" || k == "cost_out" || k == "cost_minute" || k == "max_wall_clock_ms" || k == "cache_min_tokens") {
                printf "%s\t%s\t%s\n", cur, k, v
            }
        }
    ' "$PROFILES_FILE" | jq -R -s '
        [ split("\n")[] | select(length>0) | split("\t") ]
        | reduce .[] as $r ({}; .[$r[0]] = ((.[$r[0]] // {}) + { ($r[1]): ($r[2]|tonumber?) }))'
}

# ── ledger stats for this cell ────────────────────────────────────────────────
_stats() {
    if [ -n "$STATS_FILE" ]; then
        if [ -f "$STATS_FILE" ]; then cat "$STATS_FILE"; else printf '[]'; fi
        return 0
    fi
    _ledger_sh="$SCRIPT_DIR/routing-ledger.sh"
    if [ ! -f "$_ledger_sh" ]; then
        printf '[]'
        return 0
    fi
    bash "$_ledger_sh" --stats --json 2>/dev/null || printf '[]'
}

catalog_json="$(_catalog)"
stats_json="$(_stats)"
cand_json="$(printf '%s' "$CANDIDATES" | jq -R 'split(",") | map(gsub("^\\s+|\\s+$";"")) | map(select(length>0))')"

# ── score ─────────────────────────────────────────────────────────────────────
# Reference unit costs for the escalation/fallback terms: the most expensive
# scoreable candidate stands in for the stronger tier a failure escalates to.
result="$(jq -n \
    --argjson candidates "$cand_json" \
    --argjson catalog "$catalog_json" \
    --argjson stats "$stats_json" \
    --arg kind "$KIND" --arg ctx "$CTX" --arg reasoning "$REASONING" \
    --argjson alpha "$ALPHA" --argjson min_samples "$MIN_SAMPLES" \
    --argjson floor "$FLOOR" --argjson cache_beta "$CACHE_BETA" \
    --argjson max_wall_ms "$MAX_WALL_MS" --argjson cloud_mult "$CLOUD_MULT" \
    --argjson prefix_tokens "$PREFIX_TOKENS" '
    def cell($p): ($stats | map(select(
        .dispatch_kind == $kind and .profile == $p and
        .cell_ctx == $ctx and .cell_reasoning == $reasoning)) | first);

    # A dispatch is ~1 unit of work; price it per-dispatch so cloud (per-token)
    # and local (per-minute) profiles land on one comparable scale.
    def unit($p):
        ($catalog[$p] // {}) as $c
        | if ($c.cost_in != null and $c.cost_out != null)
          then (($c.cost_in + $c.cost_out) * $cloud_mult)
          elif ($c.cost_minute != null)
          then ($c.cost_minute * 10)   # a dispatch is priced at ten GPU-minutes
          else null end;

    # Bayesian smoothing. Quality shrinks toward 0.5; every cost-increasing term
    # shrinks toward a pessimistic prior so no-data never looks cheap.
    def smooth($hits; $n; $prior): (($hits + ($alpha * $prior)) / ($n + $alpha));

    ($candidates | map({ profile: ., unit: unit(.), row: cell(.) })
      | map(. + { n: (.row.dispatches // 0) })
      | map(. + {
          first_pass_rate: smooth(((.row.first_pass_rate // 0) * .n); .n; 0.5),
          mean_retries:    smooth(((.row.mean_retries // 0) * .n); .n; 1.0),
          escalation_rate: smooth(((.row.escalation_rate // 0) * .n); .n; 0.5),
          failure_rate:    smooth(((.row.failure_rate // 0) * .n); .n; 0.5),
          # A prompt cache has a per-model MINIMUM: below it, nothing is cached
          # however large the prefix feels. Haiku 4.5 needs 4096 tokens where
          # Opus 5 needs 512, so the cheapest per-token profile is the easiest one
          # to fall under — and a measured cache_hit_ratio recorded under a LARGER
          # prefix would otherwise be credited to a dispatch that cannot cache at
          # all. Zero it when the prefix provably cannot clear the floor.
          # Fails open: prefix_tokens 0 (unknown) leaves the measured value alone,
          # so a host that does not report prefix size scores exactly as before.
          cache_hit_ratio: (
            (($catalog[.profile] // {}).cache_min_tokens) as $floor_tok
            | if ($prefix_tokens > 0 and $floor_tok != null and $prefix_tokens < $floor_tok)
              then 0
              else (.row.cache_hit_ratio // 0) end),
          cache_floor_unmet: (
            (($catalog[.profile] // {}).cache_min_tokens) as $floor_tok
            | ($prefix_tokens > 0 and $floor_tok != null and $prefix_tokens < $floor_tok)),
          mean_wall_clock_ms: (.row.mean_wall_clock_ms // 0),
          wall_clock_ceiling_ms: ((($catalog[.profile] // {}).max_wall_clock_ms) // $max_wall_ms)
        })
      | map(. + { cache_penalty: (1 + ($cache_beta * (1 - .cache_hit_ratio))) })
    ) as $scored
    | ([$scored[] | select(.unit != null) | .unit] | max) as $strongest
    | $scored
      | map(. + {
          effective_cost: (
            if .unit == null then null
            else (.unit * (1 + .mean_retries) * .cache_penalty)
                 + (.escalation_rate * ($strongest // .unit))
                 + (.failure_rate * ($strongest // .unit))
            end)
        })
      | map(. + {
          eligible: (
            .unit != null and .effective_cost != null and
            .n >= $min_samples and .first_pass_rate >= $floor and
            (.wall_clock_ceiling_ms == 0 or .mean_wall_clock_ms <= .wall_clock_ceiling_ms)),
          reason: (
            if .unit == null then "no cost keys in profile catalog"
            elif .n < $min_samples then "insufficient samples (\(.n)/\($min_samples))"
            elif .first_pass_rate < $floor then "first-pass rate \(.first_pass_rate) below floor \($floor)"
            elif (.wall_clock_ceiling_ms != 0 and .mean_wall_clock_ms > .wall_clock_ceiling_ms)
              then "mean wall clock \(.mean_wall_clock_ms)ms exceeds ceiling \(.wall_clock_ceiling_ms)ms"
            else "" end)
        })
      | map(del(.row))
      | sort_by(if .effective_cost == null then 1e18 else .effective_cost end)')"

if [ "$EXPLAIN" -eq 1 ]; then
    printf '%s' "$result" | jq -r '.[] |
        "\(.profile): n=\(.n) unit=\(.unit // "-") retries=\(.mean_retries|.*1000|round/1000) esc=\(.escalation_rate|.*1000|round/1000) cache_pen=\(.cache_penalty|.*1000|round/1000) -> cost=\(if .effective_cost == null then "-" else (.effective_cost*1000|round/1000) end) eligible=\(.eligible)\(if .reason != "" then " (\(.reason))" else "" end)"' >&2
fi

if [ "$JSON_OUT" -eq 1 ] || [ "$EXPLAIN" -eq 0 ]; then
    printf '%s\n' "$result" | jq '.'
fi
