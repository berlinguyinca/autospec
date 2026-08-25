#!/usr/bin/env bash
# Launch the dual-Turing node in llama.cpp router mode.
#
#   serve-router.sh <profile>        (profile is "router"; the unit passes %i)
#
# Invoked by qwen-turing@.service. Safe to run by hand for debugging, but note
# that by hand there is no CREDENTIALS_DIRECTORY, so pass QT_API_KEY_FILE.
set -euo pipefail

CONF_DIR="${QT_CONF_DIR:-/opt/qwen-turing/etc}"
PROFILE="${1:-router}"

# shellcheck source=../config/common.conf
. "${CONF_DIR}/common.conf"

PROFILE_CONF="${CONF_DIR}/profiles.d/${PROFILE}.conf"
[ -r "$PROFILE_CONF" ] || { echo "no such profile: ${PROFILE} (${PROFILE_CONF})" >&2; exit 64; }
# shellcheck source=../config/profiles.d/router.conf
. "$PROFILE_CONF"

# Site coordinates. require_site returns 78 naming the file; it is sourced, so
# it never exits on our behalf.
# shellcheck source=./site.sh
. "${CONF_DIR}/site.sh"
require_site || exit $?

[ -x "${QT_LLAMA_DIR}/llama-server" ] || {
  echo "llama-server not found at ${QT_LLAMA_DIR}" >&2; exit 69; }

# --- the API key ------------------------------------------------------------
# systemd hands it over via LoadCredential, which puts it in a private tmpfs
# readable only by this unit. It is deliberately NOT an Environment= value:
# anyone local can read those out of `systemctl show`.
if [ -n "${CREDENTIALS_DIRECTORY:-}" ] && [ -r "${CREDENTIALS_DIRECTORY}/apikey" ]; then
  API_KEY_FILE="${CREDENTIALS_DIRECTORY}/apikey"
elif [ -n "${QT_API_KEY_FILE:-}" ] && [ -r "${QT_API_KEY_FILE}" ]; then
  API_KEY_FILE="${QT_API_KEY_FILE}"
else
  echo "no API key available (expected CREDENTIALS_DIRECTORY/apikey)" >&2
  exit 78
fi

# --- render the presets ----------------------------------------------------
# The committed presets carry <QT_MODELS_DIR> because this repository is public.
# Rendering at START rather than at install means a site.conf change takes
# effect on restart instead of needing a reinstall.
RENDERED="${QT_STATE}/router-presets.rendered.ini"
install -d -m 0750 "${QT_STATE}"
sed "s|<QT_MODELS_DIR>|${QT_MODELS_DIR}|g" "${QT_ROUTER_PRESETS}" > "${RENDERED}"

# Fail loudly if a placeholder survived, rather than letting llama-server try to
# open a file literally named "<QT_MODELS_DIR>/...".
if grep -q '<[A-Z_]*>' "${RENDERED}"; then
  echo "unsubstituted placeholder left in ${RENDERED}:" >&2
  grep -n '<[A-Z_]*>' "${RENDERED}" >&2
  exit 78
fi

# Every model the presets name must exist before we start. Otherwise the first
# request for that id fails at load time, minutes later, looking like a runtime
# fault rather than a missing file.
missing=0
while read -r m; do
  [ -r "$m" ] || { echo "model not readable: $m" >&2; missing=1; }
done < <(sed -n 's/^[[:space:]]*model[[:space:]]*=[[:space:]]*//p' "${RENDERED}")
[ "$missing" -eq 0 ] || exit 69

# --- VRAM guard ------------------------------------------------------------
# Also wired as ExecStartPre in the unit; harmless here and useful by hand.
if [ -x "${QT_VRAM_GUARD}" ]; then
  "${QT_VRAM_GUARD}" --min-total "${QT_MIN_FREE_VRAM_MIB}" \
                     --min-per-card "${QT_MIN_FREE_PER_CARD_MIB}" || exit $?
fi

# The build ships its ggml backends beside the binaries.
export LD_LIBRARY_PATH="${QT_LLAMA_DIR}${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export HOME="${QT_STATE}"

echo "starting profile=${PROFILE} version=${QT_PROFILE_VERSION} runtime=${QT_RUNTIME}"
echo "  presets=${RENDERED} max-loaded=${QT_ROUTER_MAX_LOADED}"
echo "  bind=127.0.0.1:${QT_LLAMA_PORT} (public port is nginx's business)"

# --- devices ----------------------------------------------------------------
# Refuse to start unless every device named in QT_DEVICES is actually present.
# --device would catch a missing device on its own (exit 1), but doing the check
# here names WHICH device vanished and prints what llama.cpp could see, which is
# the difference between a five-second diagnosis and a long one.
AVAIL="$("${QT_LLAMA_DIR}/llama-server" --list-devices 2>&1 || true)"
missing=""
ndev=0
IFS=',' read -r -a _want <<< "${QT_DEVICES:?QT_DEVICES unset in common.conf}"
for d in "${_want[@]}"; do
  if printf '%s\n' "$AVAIL" | grep -qE "^[[:space:]]*${d}:"; then
    ndev=$(( ndev + 1 ))
  else
    missing="${missing}${missing:+,}${d}"
  fi
done
if [ -n "$missing" ]; then
  echo "refusing to start: GPU device(s) not present: ${missing}" >&2
  echo "llama-server --list-devices reported:" >&2
  printf '%s\n' "$AVAIL" >&2
  echo "NOT starting on CPU: this node is sized for GPU offload only." >&2
  exit 69
fi
# Config consistency, NOT a hardware count: every name in QT_DEVICES was found
# above, so ndev can only disagree with QT_EXPECT_DEVICES when the two settings
# contradict each other. The hardware count is vram-guard.sh's job (it counts
# nvidia-smi rows) and the dashboard's gpu_gate. Catching the contradiction here
# stops a node booting with a guard that silently means something else.
if [ -n "${QT_EXPECT_DEVICES:-}" ] && [ "${QT_EXPECT_DEVICES}" -gt 0 ] \
   && [ "$ndev" -ne "${QT_EXPECT_DEVICES}" ]; then
  echo "refusing to start: QT_DEVICES names ${ndev} device(s) but" >&2
  echo "QT_EXPECT_DEVICES says ${QT_EXPECT_DEVICES}; fix common.conf" >&2
  exit 78
fi
echo "  devices=${QT_DEVICES} (${ndev} present)"

# --models-preset, NOT --models. There is no --models flag; the earlier value
# was arrived at by grepping the docs for "--models", which matches inside
# --models-preset, --models-dir and --models-max. The binary rejected it with
# "invalid argument: --models" and restart-looped. Verify long options against
# the BUILT BINARY by exact match -- install-node.sh now does exactly that.

exec "${QT_LLAMA_DIR}/llama-server" \
  --device "${QT_DEVICES}" \
  --models-preset "${RENDERED}" \
  --models-max "${QT_ROUTER_MAX_LOADED}" \
  --host 127.0.0.1 \
  --port "${QT_LLAMA_PORT}" \
  --api-key-file "${API_KEY_FILE}" \
  --no-webui \
  --metrics \
  --slot-prompt-similarity "${QT_SLOT_PROMPT_SIMILARITY}"
