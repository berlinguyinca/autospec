#!/usr/bin/env bash
# Thin wrapper so the unit does not have to know about site.conf.
set -euo pipefail
CONF_DIR="${QT_CONF_DIR:-/opt/qwen-turing/etc}"
# shellcheck source=../config/common.conf
. "${CONF_DIR}/common.conf"
# shellcheck source=./site.sh
. "${CONF_DIR}/site.sh"
# The gateway needs the identity-provider and registry values as well as the
# core ones; require_gateway_site validates AND exports both sets.
require_gateway_site || exit $?

# The runtime's own key, which ONLY this process knows. Distinct from every user
# key, so a request that somehow reaches llama.cpp directly fails closed.
INTERNAL="${CREDENTIALS_DIRECTORY:-}/internalkey"
[ -r "$INTERNAL" ] || { echo "no internal key credential" >&2; exit 78; }

# The registry password arrives as a credential too, never in the environment:
# `systemctl show` would print an Environment= value to any local user.
DBPW="${CREDENTIALS_DIRECTORY:-}/dbpassword"
DBARG=()
if [ -r "$DBPW" ]; then
  DBARG=(--db-password-file "$DBPW")
else
  echo "no registry credential -- running mirror-only (keys still work)" >&2
fi

# The pre-gateway shared key, while it still has users. Its traffic shows up in
# the usage panel under a reserved key id, which is how you know when it is safe
# to drop this credential from the unit.
LEGACY="${CREDENTIALS_DIRECTORY:-}/legacykey"
if [ -r "$LEGACY" ]; then
  DBARG+=(--legacy-key-file "$LEGACY")
fi

export QT_UPSTREAM_HOST="127.0.0.1"
export QT_UPSTREAM_PORT="${QT_LLAMA_PORT}"

exec /opt/qwen-turing/bin/gateway.py \
  --host 127.0.0.1 \
  --port "${QT_GATEWAY_PORT}" \
  --mirror /var/lib/qwen-turing/keys.sqlite3 \
  --internal-key-file "$INTERNAL" \
  "${DBARG[@]}"
