#!/usr/bin/env bash
# Thin wrapper so the unit does not have to know about site.conf.
set -euo pipefail
CONF_DIR="${QT_CONF_DIR:-/opt/qwen-turing/etc}"
# shellcheck source=../config/common.conf
. "${CONF_DIR}/common.conf"
# shellcheck source=./site.sh
. "${CONF_DIR}/site.sh"
require_site || exit $?

KEY="${CREDENTIALS_DIRECTORY:-}/apikey"
[ -r "$KEY" ] || { echo "no API key credential" >&2; exit 78; }

# The GPU gate's device-count check reads this from the ENVIRONMENT.
# Sourcing common.conf only makes it a shell variable: without the export
# the check silently becomes a no-op while still looking installed.
export QT_EXPECT_DEVICES="${QT_EXPECT_DEVICES:-0}"

exec /opt/qwen-turing/bin/dashboard.py \
  --host 127.0.0.1 \
  --port "${QT_DASH_PORT}" \
  --metrics-url "http://127.0.0.1:${QT_LLAMA_PORT}/metrics" \
  --api-key-file "$KEY"
