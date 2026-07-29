#!/usr/bin/env bash
# skills/autospec-shared/scripts/gen-telemetry-dashboard.sh — Generate telemetry HTML dashboard.
#
# Usage:
#   gen-telemetry-dashboard.sh --input <path> --output <path|-|default>
#   gen-telemetry-dashboard.sh --help
#
# Reads telemetry JSONL from <path> (one record per line) and emits an HTML dashboard with:
#   - Cache hit-rate trend (Chart.js line chart, canvas id="cache-hit-rate")
#   - Per-role token-cost breakdown (implementer, reviewer, decomposer, classifier)
#   - LGTM first-pass rate (reviewer dispatches where cache_read > 0 on first call)
#   - Top-10 cost outliers table (by total token cost per issue, descending)
#
# Options:
#   --input <path>    Path to telemetry JSONL file. Required.
#   --output <path>   Output HTML file path. Use '-' for stdout. Default: ~/.autospec/telemetry-dashboard.html
#
# Environment overrides (for testing):
#   AUTOSPEC_TELEMETRY_FILE — default input path (overridden by --input)
#
# Exit codes:
#   0  Success
#   1  Argument/file error
#
# Requires: jq (for aggregations), bash 3.2+

set -eu

HELP_TEXT='Usage:
gen-telemetry-dashboard.sh --input <path> --output <path|-|default>
gen-telemetry-dashboard.sh --help

Generate HTML telemetry dashboard from autospec telemetry.jsonl.
Output includes cache hit-rate trend, per-role token costs,
LGTM first-pass rate, and top-10 cost outlier table.'

INPUT_FILE=""
OUTPUT_FILE=""

while [ $# -gt 0 ]; do
  case "$1" in
    --help|-h)
      printf '%s\n' "$HELP_TEXT"
      exit 0
      ;;
    --input)
      if [ $# -lt 2 ]; then
        printf 'gen-telemetry-dashboard.sh: --input requires an argument\n' >&2
        exit 1
      fi
      INPUT_FILE="$2"
      shift 2
      ;;
    --output)
      if [ $# -lt 2 ]; then
        printf 'gen-telemetry-dashboard.sh: --output requires an argument\n' >&2
        exit 1
      fi
      OUTPUT_FILE="$2"
      shift 2
      ;;
    -*)
      printf 'gen-telemetry-dashboard.sh: unknown option: %s\n' "$1" >&2
      exit 1
      ;;
    *)
      printf 'gen-telemetry-dashboard.sh: unexpected argument: %s\n' "$1" >&2
      exit 1
      ;;
  esac
done

# Validate required args
if [ -z "$INPUT_FILE" ]; then
  printf 'gen-telemetry-dashboard.sh: --input is required\n' >&2
  printf '%s\n' "$HELP_TEXT" >&2
  exit 1
fi

if [ ! -f "$INPUT_FILE" ]; then
  printf 'gen-telemetry-dashboard.sh: input file not found: %s\n' "$INPUT_FILE" >&2
  exit 1
fi

# Default output path
if [ -z "$OUTPUT_FILE" ]; then
  OUTPUT_FILE="${AUTOSPEC_TELEMETRY_FILE:-$HOME/.autospec/telemetry-dashboard.html}"
  OUTPUT_FILE="${OUTPUT_FILE%.jsonl}.html"
  OUTPUT_FILE="$HOME/.autospec/telemetry-dashboard.html"
fi

# ── aggregate data via jq ─────────────────────────────────────────────────────

AGGREGATED=$(jq -rs '
  (. // []) as $rows
  | ($rows | length) as $total

  # Daily cache hit-rate: group by date prefix of ts
  | ($rows
      | group_by(.ts[0:10])
      | map({
          date: .[0].ts[0:10],
          total: length,
          hits: (map(select(.cache_read_input_tokens > 0)) | length),
          hit_rate: (if length > 0 then ((map(select(.cache_read_input_tokens > 0)) | length) * 100.0 / length) else 0 end)
        })
      | sort_by(.date)
    ) as $daily

  # Per-role breakdown
  | ($rows
      | group_by(.role)
      | map({
          role: (.[0].role // "unknown"),
          dispatches: length,
          total_tokens: (map((.input_tokens // 0) + (.output_tokens // 0)) | add // 0),
          hits: (map(select(.cache_read_input_tokens > 0)) | length),
          hit_rate: (if length > 0 then ((map(select(.cache_read_input_tokens > 0)) | length) * 100.0 / length) else 0 end)
        })
    ) as $by_role

  # LGTM first-pass rate:
  # For each issue, a "first-pass" LGTM means the first reviewer dispatch had cache_read > 0
  # (i.e., cache was warm). We approximate: reviewer dispatches sorted by ts; first per issue.
  | ($rows
      | map(select(.role == "reviewer"))
      | group_by(.issue)
      | map(sort_by(.ts) | .[0])
      | length
    ) as $reviewer_issues
  | ($rows
      | map(select(.role == "reviewer"))
      | group_by(.issue)
      | map(sort_by(.ts) | .[0])
      | map(select(.cache_read_input_tokens > 0))
      | length
    ) as $lgtm_first_pass_hits
  | (if $reviewer_issues > 0 then ($lgtm_first_pass_hits * 100.0 / $reviewer_issues) else 0 end) as $lgtm_rate

  # Top-10 cost outliers by total tokens per issue
  | ($rows
      | group_by(.issue)
      | map({
          issue: .[0].issue,
          total_tokens: (map((.input_tokens // 0) + (.output_tokens // 0)) | add // 0),
          dispatches: length
        })
      | sort_by(-.total_tokens)
      | .[0:10]
    ) as $outliers

  | {
      total: $total,
      daily: $daily,
      by_role: $by_role,
      lgtm_rate: ($lgtm_rate | . * 10 | round / 10),
      reviewer_issues: $reviewer_issues,
      lgtm_first_pass_hits: $lgtm_first_pass_hits,
      outliers: $outliers
    }
' "$INPUT_FILE") || {
  printf 'gen-telemetry-dashboard.sh: jq failed to parse input\n' >&2
  exit 1
}

# ── extract values for template ───────────────────────────────────────────────

TOTAL=$(printf '%s' "$AGGREGATED" | jq -r '.total')
LGTM_RATE=$(printf '%s' "$AGGREGATED" | jq -r '.lgtm_rate')
REVIEWER_ISSUES=$(printf '%s' "$AGGREGATED" | jq -r '.reviewer_issues')
LGTM_HITS=$(printf '%s' "$AGGREGATED" | jq -r '.lgtm_first_pass_hits')

# Daily chart data: dates and hit rates as JSON arrays
DAILY_LABELS=$(printf '%s' "$AGGREGATED" | jq -r '[.daily[].date] | @json')
DAILY_RATES=$(printf '%s' "$AGGREGATED" | jq -r '[.daily[].hit_rate | . * 10 | round / 10] | @json')

# Per-role rows as HTML table rows
ROLE_ROWS=$(printf '%s' "$AGGREGATED" | jq -r '
  .by_role[]
  | "<tr><td>" + .role + "</td><td>" + (.dispatches | tostring) + "</td><td>"
    + (.total_tokens | tostring) + "</td><td>"
    + (.hits | tostring) + "</td><td>"
    + (.hit_rate | . * 10 | round / 10 | tostring) + "%</td></tr>"
')

# Outlier rows
OUTLIER_ROWS=$(printf '%s' "$AGGREGATED" | jq -r '
  .outliers[]
  | "<tr><td>#" + .issue + "</td><td>" + (.total_tokens | tostring) + "</td><td>" + (.dispatches | tostring) + "</td></tr>"
')

# ── emit HTML ─────────────────────────────────────────────────────────────────

HTML=$(cat <<HTMLEOF
<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>Autospec Telemetry Dashboard</title>
  <!-- Chart.js CDN -->
  <script src="https://cdn.jsdelivr.net/npm/chart.js@4/dist/chart.umd.min.js"></script>
  <style>
    body { font-family: system-ui, sans-serif; margin: 2rem; background: #f9f9f9; color: #222; }
    h1 { font-size: 1.5rem; border-bottom: 2px solid #ddd; padding-bottom: 0.5rem; }
    h2 { font-size: 1.1rem; margin-top: 2rem; color: #444; }
    .kpi-row { display: flex; gap: 1.5rem; flex-wrap: wrap; margin: 1rem 0; }
    .kpi { background: #fff; border: 1px solid #e0e0e0; border-radius: 6px; padding: 1rem 1.5rem; min-width: 160px; }
    .kpi .value { font-size: 2rem; font-weight: bold; color: #2563eb; }
    .kpi .label { font-size: 0.85rem; color: #666; }
    .chart-box { background: #fff; border: 1px solid #e0e0e0; border-radius: 6px; padding: 1rem; max-width: 700px; margin: 1rem 0; }
    table { border-collapse: collapse; width: 100%; max-width: 700px; background: #fff; }
    th, td { border: 1px solid #e0e0e0; padding: 0.4rem 0.75rem; text-align: left; font-size: 0.9rem; }
    th { background: #f1f5f9; font-weight: 600; }
    .empty-state { color: #888; font-style: italic; margin: 1rem 0; }
  </style>
</head>
<body>
  <h1>Autospec Telemetry Dashboard</h1>

  <div class="kpi-row">
    <div class="kpi">
      <div class="value">${TOTAL}</div>
      <div class="label">Total Dispatches</div>
    </div>
    <div class="kpi">
      <div class="value" id="lgtm-first-pass-rate">${LGTM_RATE}%</div>
      <div class="label">LGTM First-Pass Rate</div>
    </div>
    <div class="kpi">
      <div class="value">${LGTM_HITS} / ${REVIEWER_ISSUES}</div>
      <div class="label">Reviewer Issues (first-pass hits)</div>
    </div>
  </div>

  <h2>Cache Hit-Rate Trend</h2>
  <div class="chart-box">
    <canvas id="cache-hit-rate" height="120"></canvas>
  </div>

  <h2>Per-Role Token Cost Breakdown</h2>
  <table id="role-cost-table">
    <thead>
      <tr>
        <th>Role</th>
        <th>Dispatches</th>
        <th>Total Tokens</th>
        <th>Cache Hits</th>
        <th>Hit Rate</th>
      </tr>
    </thead>
    <tbody>
${ROLE_ROWS}
    </tbody>
  </table>

  <h2>Top-10 Cost Outliers (by total tokens per issue)</h2>
  <table id="outliers-table">
    <thead>
      <tr>
        <th>Issue</th>
        <th>Total Tokens</th>
        <th>Dispatches</th>
      </tr>
    </thead>
    <tbody>
${OUTLIER_ROWS}
    </tbody>
  </table>

  <script>
    var dailyLabels = ${DAILY_LABELS};
    var dailyRates  = ${DAILY_RATES};
    if (dailyLabels.length > 0) {
      new Chart(document.getElementById('cache-hit-rate'), {
        type: 'line',
        data: {
          labels: dailyLabels,
          datasets: [{
            label: 'Cache Hit Rate (%)',
            data: dailyRates,
            borderColor: '#2563eb',
            backgroundColor: 'rgba(37,99,235,0.08)',
            tension: 0.3,
            fill: true,
            pointRadius: 4
          }]
        },
        options: {
          plugins: { legend: { display: true } },
          scales: {
            y: { min: 0, max: 100, title: { display: true, text: 'Hit Rate (%)' } },
            x: { title: { display: true, text: 'Date' } }
          }
        }
      });
    } else {
      document.getElementById('cache-hit-rate').insertAdjacentHTML(
        'afterend', '<p class="empty-state">No data available for chart.</p>'
      );
    }
  </script>
</body>
</html>
HTMLEOF
)

# ── write output ──────────────────────────────────────────────────────────────

if [ "$OUTPUT_FILE" = "-" ]; then
  printf '%s\n' "$HTML"
else
  mkdir -p "$(dirname "$OUTPUT_FILE")"
  printf '%s\n' "$HTML" > "$OUTPUT_FILE"
fi
