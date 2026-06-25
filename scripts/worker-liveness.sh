#!/usr/bin/env bash
# scripts/worker-liveness.sh — host-match + PID-liveness decision helper
#
# Given a worker_id of the form "host:user:harness:pid" (the format written by
# autospec into heartbeats and the autospec-run-state GitHub comment), decide
# whether that worker is provably alive, provably dead, or unknown.
#
# Outputs (stdout) — always exits 0; never error-exits the caller:
#   alive   — same host AND kill -0 <pid> succeeded (provably live)
#   dead    — same host AND kill -0 <pid> failed   (provably dead; safe to reclaim)
#   unknown — cross-host, malformed, or non-numeric pid
#             (caller falls back to GitHub-timestamp freshness per F1 rule)
#
# Usage:
#   worker-liveness.sh <worker_id>
#
# Mockable boundary:
#   WORKER_LIVENESS_HOSTNAME — override hostname for tests (default: $(hostname))
#
# Reuse decision: No reuse — no existing helper performs host-match + kill-0 on
# a colon-delimited worker_id; heartbeat-read.sh parses heartbeat JSON files,
# not run-state worker_id strings.

set -eu

worker_id="${1:-}"

# Resolve hostname; allow env override for testing.
this_host="${WORKER_LIVENESS_HOSTNAME:-$(hostname)}"

# Guard: empty input → unknown.
if [ -z "$worker_id" ]; then
    echo "unknown"
    exit 0
fi

# Require at least 3 colons (4 fields: host:user:harness:pid).
colon_count=$(printf '%s' "$worker_id" | tr -cd ':' | wc -c | tr -d ' ')
if [ "$colon_count" -lt 3 ]; then
    echo "unknown"
    exit 0
fi

# Parse the four fields. cut -f4 handles the pid even when extra colons exist
# in later fields (forward-compat).
whost=$(printf '%s' "$worker_id" | cut -d: -f1)
wpid=$(printf '%s' "$worker_id" | cut -d: -f4)

# Guard: host or pid field empty → unknown.
if [ -z "$whost" ] || [ -z "$wpid" ]; then
    echo "unknown"
    exit 0
fi

# Guard: pid must be numeric (kill -0 requires an integer).
case "$wpid" in
    ''|*[!0-9]*)
        echo "unknown"
        exit 0
        ;;
esac

# Cross-host: caller falls back to GitHub freshness (F1 rule).
if [ "$whost" != "$this_host" ]; then
    echo "unknown"
    exit 0
fi

# Same host: PID-liveness via kill -0.
if kill -0 "$wpid" 2>/dev/null; then
    echo "alive"
else
    echo "dead"
fi

exit 0
