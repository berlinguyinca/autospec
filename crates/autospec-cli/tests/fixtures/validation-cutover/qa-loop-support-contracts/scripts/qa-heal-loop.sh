#!/usr/bin/env bash
set -euo pipefail

source "$(dirname "$0")/lib/autospec-harness-detect.sh"

parent_fingerprint='fixture'
printf '%s\n' "$parent_fingerprint"
