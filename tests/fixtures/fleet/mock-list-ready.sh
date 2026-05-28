#!/usr/bin/env bash
# Mock list-ready-issues.sh for autospec-fleet dry-run E2E.

set -euo pipefail

fixture_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo=""

while [ $# -gt 0 ]; do
    case "$1" in
        --repo) repo="${2:-}"; shift 2 ;;
        --batch-size) shift 2 ;;
        *) shift ;;
    esac
done

case "$repo" in
    org/repo-a) cat "$fixture_dir/queue-org-repo-a.json" ;;
    org/repo-b) cat "$fixture_dir/queue-org-repo-b.json" ;;
    *) printf '{"ready":[],"blocked":[],"claimed":[],"conflicts":[],"batch":[]}\n' ;;
esac
