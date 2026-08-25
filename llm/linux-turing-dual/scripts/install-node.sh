#!/usr/bin/env bash
# Provision the dual-Turing inference node, then VERIFY it.
#
#   install-node.sh [--skip-build] [--skip-weights]
#
# This script refuses to claim success without a real completion from the served
# model. "The port is open" is not a verification.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
NODE="$(dirname "$HERE")"

SKIP_BUILD=0
SKIP_WEIGHTS=0
for a in "$@"; do
  case "$a" in
    --skip-build)   SKIP_BUILD=1 ;;
    --skip-weights) SKIP_WEIGHTS=1 ;;
    *) echo "usage: $(basename "$0") [--skip-build] [--skip-weights]" >&2; exit 64 ;;
  esac
done

# shellcheck source=../config/common.conf
. "${NODE}/config/common.conf"
# shellcheck source=./site.sh
. "${HERE}/site.sh"
require_site || exit $?

say() { printf '\n=== %s\n' "$*"; }

# --- phase 1: build dependencies -------------------------------------------
# CUDA 12.0 is what Ubuntu 24.04 ships, and its nvcc rejects gcc 13 (it supports
# up to 12.2). gcc-12 is therefore installed and passed as the CUDA host
# compiler. An older toolkit against a newer driver is fine -- the 580 driver
# provides a CUDA 13 runtime and runs code built by 12.0 without complaint.
# sm_75 is old enough that no recent toolkit feature is needed.
if [ "$SKIP_BUILD" -eq 0 ]; then
  say "build dependencies (cuda toolkit + gcc-12 host compiler)"
  sudo apt-get update -qq
  sudo apt-get install -y --no-install-recommends \
    nvidia-cuda-toolkit gcc-12 g++-12 cmake build-essential \
    libcurl4-openssl-dev git ccache

  say "toolchain versions"
  nvcc --version | tail -2
  gcc-12 --version | head -1

  # --- phase 2: build ------------------------------------------------------
  say "build llama.cpp ${QT_LLAMA_TAG} for sm_${QT_CUDA_ARCHS}"
  src="/usr/local/src/llama.cpp"
  if [ -d "${src}/.git" ]; then
    sudo git -C "$src" fetch --depth 1 origin "tag ${QT_LLAMA_TAG}" || true
    sudo git -C "$src" checkout -q "${QT_LLAMA_TAG}"
  else
    sudo mkdir -p "$(dirname "$src")"
    sudo git clone --depth 1 --branch "${QT_LLAMA_TAG}" \
      https://github.com/ggml-org/llama.cpp "$src"
  fi
  echo "built from: $(sudo git -C "$src" rev-parse --short HEAD) (tag ${QT_LLAMA_TAG})"

  # GGML_NATIVE=OFF: a native build targets the build host's CPU and is not
  # portable, which this project has already been bitten by.
  sudo cmake -S "$src" -B "${src}/build" \
    -DGGML_CUDA=ON \
    -DCMAKE_CUDA_ARCHITECTURES="${QT_CUDA_ARCHS}" \
    -DCMAKE_CUDA_HOST_COMPILER=/usr/bin/gcc-12 \
    -DGGML_NATIVE="${QT_GGML_NATIVE}" \
    -DLLAMA_CURL=ON \
    -DCMAKE_BUILD_TYPE=Release
  sudo cmake --build "${src}/build" --config Release -j"$(nproc)"

  # --- phase 3: install ----------------------------------------------------
  say "install binaries to ${QT_LLAMA_DIR}"
  sudo install -d "${QT_LLAMA_DIR}" "${QT_PREFIX}/bin" "${QT_PREFIX}/etc/profiles.d"
  sudo find "${src}/build/bin" -maxdepth 1 -type f -name 'llama-*' \
      -exec install -m 0755 {} "${QT_LLAMA_DIR}/" \;
  # ggml ships its backends as shared objects beside the binaries.
  sudo find "${src}/build/bin" -maxdepth 1 -name '*.so*' \
      -exec install -m 0755 {} "${QT_LLAMA_DIR}/" \; 2>/dev/null || true
fi

# --- phase 3.5: weights ------------------------------------------------------
# The repository must be able to rebuild this node, not merely describe it. This
# phase existed only as a --skip-weights flag until now; the checkpoints were
# fetched by hand.
#
# Driven by model-artifacts.yaml, which already records provenance, so a new model
# is added in ONE place. Pinned revisions, never branches: the 27B repository was
# modified on the same day these weights were first downloaded.
if [ "$SKIP_WEIGHTS" -eq 0 ]; then
  say "fetch weights into ${QT_MODELS_DIR} (pinned revisions)"
  sudo install -d -m 0755 "${QT_MODELS_DIR}"
  plan="$(python3 "${HERE}/artifacts.py" --plan "${NODE}/config/model-artifacts.yaml")"
  if [ -z "$plan" ]; then
    echo "no artifacts in model-artifacts.yaml" >&2
    exit 78
  fi
  # dest is the LOCAL name and file the REMOTE one: two projectors share the
  # remote name mmproj-F16.gguf across repositories.
  printf '%s\n' "$plan" | while IFS=$'\t' read -r local file repo rev size; do
    [ -n "$local" ] || continue
    dest="${QT_MODELS_DIR}/${local}"
    have="$(stat -c%s "$dest" 2>/dev/null || echo 0)"
    if [ "$have" = "$size" ]; then
      echo "  present at expected size: ${local}"
      continue
    fi
    echo "  fetching ${local} <- ${file} from ${repo} @ ${rev:0:12}"
    # -C - resumes a partial file; a revision URL is immutable, so resuming is safe.
    sudo curl -fL --retry 3 --retry-delay 5 -C - -o "$dest" \
      "https://huggingface.co/${repo}/resolve/${rev}/${file}" || {
        echo "  DOWNLOAD FAILED: ${local}" >&2; exit 70; }
    got="$(stat -c%s "$dest" 2>/dev/null || echo 0)"
    if [ "$got" != "$size" ]; then
      # Fatal, not a warning. A truncated GGUF loads, answers, and is subtly wrong.
      echo "  SIZE MISMATCH ${local}: got ${got}, expected ${size}" >&2
      exit 70
    fi
    echo "  ok ${local} ${got} bytes"
  done
  # The subshell created by the pipe cannot fail the script, so re-verify here.
  printf '%s\n' "$plan" | while IFS=$'\t' read -r local file repo rev size; do
    [ -n "$local" ] || continue
    got="$(stat -c%s "${QT_MODELS_DIR}/${local}" 2>/dev/null || echo 0)"
    [ "$got" = "$size" ] || { echo "weights incomplete: ${local}" >&2; exit 70; }
  done || exit 70
  echo "every artifact present at its pinned size"
fi

# --- phase 4: install config and helpers -----------------------------------
say "install config, helpers and units"
sudo install -d "${QT_PREFIX}/bin" "${QT_PREFIX}/etc/profiles.d" "${QT_STATE}"
sudo install -m 0755 "${HERE}/vram-guard.sh"    "${QT_PREFIX}/bin/"
sudo install -m 0755 "${HERE}/serve-router.sh"  "${QT_PREFIX}/bin/"
sudo install -m 0755 "${HERE}/collect-stats.py" "${QT_PREFIX}/bin/"
sudo install -m 0755 "${HERE}/queue_window.py"  "${QT_PREFIX}/bin/"
sudo install -m 0755 "${HERE}/dashboard.py"     "${QT_PREFIX}/bin/"
sudo install -m 0755 "${HERE}/dashboard-run.sh" "${QT_PREFIX}/bin/"
# The gateway and the modules it imports. gateway.py adds its own directory to
# sys.path, so these are plain modules rather than executables -- and they must
# ALL be here: a missing one is a unit that dies at import with
# ModuleNotFoundError, which is how usage.py was left out the first time -- and
# how upstreams.py was left out the second time, which only a clean install would
# have shown. tests/test_structural.sh now derives the list from gateway.py's own
# imports and fails the build when one is missing, so it cannot drift again.
sudo install -m 0755 "${HERE}/gpuhealth.py"     "${QT_PREFIX}/bin/"
sudo install -m 0755 "${HERE}/gateway.py"       "${QT_PREFIX}/bin/"
sudo install -m 0755 "${HERE}/gateway-run.sh"   "${QT_PREFIX}/bin/"
sudo install -m 0644 "${HERE}/keys.py"          "${QT_PREFIX}/bin/"
sudo install -m 0644 "${HERE}/keystore.py"      "${QT_PREFIX}/bin/"
sudo install -m 0644 "${HERE}/oidc.py"          "${QT_PREFIX}/bin/"
sudo install -m 0644 "${HERE}/usage.py"         "${QT_PREFIX}/bin/"
sudo install -m 0644 "${HERE}/upstreams.py"     "${QT_PREFIX}/bin/"
sudo install -m 0644 "${HERE}/modelpeek.py"     "${QT_PREFIX}/bin/"
sudo install -m 0644 "${HERE}/wsframe.py"       "${QT_PREFIX}/bin/"
sudo install -m 0644 "${HERE}/tunnel.py"        "${QT_PREFIX}/bin/"
sudo install -m 0644 "${HERE}/scheduler.py"     "${QT_PREFIX}/bin/"
sudo install -m 0644 "${HERE}/publicview.py"    "${QT_PREFIX}/bin/"
sudo install -m 0644 "${HERE}/chat.py"          "${QT_PREFIX}/bin/"
sudo install -m 0644 "${HERE}/health.py"        "${QT_PREFIX}/bin/"
sudo install -m 0644 "${HERE}/admission.py"     "${QT_PREFIX}/bin/"
sudo install -m 0644 "${HERE}/site.sh"          "${QT_PREFIX}/etc/"
sudo install -m 0644 "${NODE}/config/common.conf" "${QT_PREFIX}/etc/"
sudo install -m 0644 "${NODE}/config/router-presets.ini" "${QT_PREFIX}/etc/"
sudo install -m 0644 "${NODE}/config/profiles.d/router.conf" "${QT_PREFIX}/etc/profiles.d/"
# Install EVERY page, not a named one. The status page was added later and a
# hardcoded index.html left /status returning 500 -- a missing file is exactly
# what a glob would have shipped for free.
sudo install -d "${QT_PREFIX}/web"
for page in "${NODE}"/web/*.html; do
  [ -r "$page" ] || continue
  sudo install -m 0644 "$page" "${QT_PREFIX}/web/"
done

say "verify the toolchain produced a usable binary"
[ -x "${QT_LLAMA_DIR}/llama-server" ] || { echo "llama-server missing" >&2; exit 70; }
LD_LIBRARY_PATH="${QT_LLAMA_DIR}" "${QT_LLAMA_DIR}/llama-server" --version 2>&1 | head -3

say "verify it sees BOTH cards"
devs="$(LD_LIBRARY_PATH="${QT_LLAMA_DIR}" "${QT_LLAMA_DIR}/llama-server" --list-devices 2>&1 | grep -c 'CUDA' || true)"
echo "CUDA devices visible to llama-server: ${devs}"
[ "${devs:-0}" -ge 2 ] || {
  echo "expected 2 CUDA devices; a one-device build or driver is wrong" >&2; exit 70; }

# --- phase 5: verify every flag the launcher passes EXISTS in this binary ----
# Exact match against the built binary, not a substring search of the docs.
# "--models" appears inside --models-preset, --models-dir and --models-max, so a
# substring grep of the README reported it present; the binary then refused to
# start with "invalid argument: --models" and the unit restart-looped. A flag
# check that can pass for a flag that does not exist is worse than none.
# --- verify every path the UNITS reference actually exists -------------------
# dashboard-run.sh was missing from the install list, so the unit died with
# status=203/EXEC -- a message that names no file and reads like a permissions
# problem. Deriving the list from the units themselves means adding a helper to a
# unit without shipping it fails here, loudly, instead of at first start.
# --- TLS snippet, written BEFORE the site that includes it -------------------
# The site file includes this unconditionally, so it must always exist. Its
# CONTENT is conditional: `listen 443 ssl` against a missing certificate makes
# nginx refuse to start, which would take plain :80 down on any node that has
# no certificate yet.
say "write the nginx TLS snippet (content depends on whether a cert exists)"
sudo install -d -m 0755 /etc/nginx/snippets
QT_TLS_DIR="/etc/ssl/qwen-turing"
if [ -r "${QT_TLS_DIR}/fullchain.pem" ] && [ -r "${QT_TLS_DIR}/privkey.pem" ]; then
  sudo tee /etc/nginx/snippets/qwen-turing-tls.conf >/dev/null <<'TLSEOF'
# Written by install-node.sh because a certificate was found.
#
# `listen ... http2` rather than the newer standalone `http2 on;` directive:
# that form arrived in nginx 1.25.1 and this host runs 1.24, where it is an
# "unknown directive" that stops nginx from starting at all.
listen 443 ssl http2;

ssl_certificate     /etc/ssl/qwen-turing/fullchain.pem;
ssl_certificate_key /etc/ssl/qwen-turing/privkey.pem;
ssl_protocols       TLSv1.2 TLSv1.3;
ssl_prefer_server_ciphers off;
ssl_session_cache   shared:qwenturing:10m;
ssl_session_timeout 1d;

# OCSP stapling deliberately omitted: it needs a `resolver` to be effective at
# all, and Let's Encrypt is retiring OCSP. Enabling it buys a warning.
TLSEOF
  echo "TLS enabled: certificate found under ${QT_TLS_DIR}"
else
  sudo tee /etc/nginx/snippets/qwen-turing-tls.conf >/dev/null <<'TLSEOF'
# Written by install-node.sh because NO certificate was found.
# Populate /etc/ssl/qwen-turing/{fullchain,privkey}.pem -- symlinks to an ACME
# client's live/ directory are the intended arrangement -- then re-run the
# installer. Deliberately empty rather than absent: the site file includes it.
TLSEOF
  echo "TLS not enabled: no certificate under ${QT_TLS_DIR}"
fi

# --- install the nginx site --------------------------------------------------
if [ -r "${NODE}/nginx/qwen-turing.conf" ] && command -v nginx >/dev/null 2>&1; then
  say "install the nginx site and VALIDATE before reloading"
  sudo install -m 0644 "${NODE}/nginx/qwen-turing.conf" \
      /etc/nginx/sites-available/qwen-turing.conf
  sudo ln -sf /etc/nginx/sites-available/qwen-turing.conf /etc/nginx/sites-enabled/
  # The stock default site also claims :80 default_server, so both cannot be
  # enabled at once.
  sudo rm -f /etc/nginx/sites-enabled/default
  # nginx -t BEFORE reload: a bad config after removing the default site would
  # leave port 80 unserved.
  sudo nginx -t || { echo "nginx config rejected -- NOT reloading" >&2; exit 78; }
  # reload only works on a RUNNING nginx; on a first install it fails noisily and
  # looks like a config error when it is merely a stopped service.
  if systemctl is-active --quiet nginx; then
    sudo systemctl reload nginx
    echo "nginx site installed and reloaded"
  else
    sudo systemctl enable --now nginx
    echo "nginx site installed and started"
  fi
fi

# --- the runtime's internal key ----------------------------------------------
# Distinct from every user key and known only to the gateway, so a request that
# somehow reaches llama.cpp directly fails closed instead of being served.
say "ensure the runtime's internal key exists"
if [ ! -s /etc/qwen-turing/internal.key ]; then
  sudo install -d -m 0755 /etc/qwen-turing
  openssl rand -hex 24 | sudo tee /etc/qwen-turing/internal.key >/dev/null
  sudo chmod 600 /etc/qwen-turing/internal.key
  sudo chown root:root /etc/qwen-turing/internal.key
  echo "generated /etc/qwen-turing/internal.key"
else
  echo "internal key already present"
fi

# --- the gateway's imports must all be satisfied ----------------------------
# Verified by IMPORTING them, not by listing files: a syntax error or a missing
# dependency is exactly as fatal as a missing file, and both present as a unit
# that will not start.
say "verify the gateway's modules import"
if ! sudo python3 -c "
import sys; sys.path.insert(0, '${QT_PREFIX}/bin')
import keys, keystore, oidc, usage   # noqa
print('gateway modules import cleanly')
"; then
  echo "the gateway's modules do not import -- refusing to claim success" >&2
  exit 70
fi

say "verify unit ExecStart paths are installed"
missing_exec=""
for unit in "${NODE}"/systemd/*.service; do
  while read -r path; do
    [ -n "$path" ] || continue
    case "$path" in /opt/qwen-turing/*) ;; *) continue ;; esac
    [ -x "$path" ] || missing_exec="${missing_exec} ${path}"
  done < <(grep -hoE '^ExecStart(Pre)?=[^ ]+' "$unit" | sed 's/^ExecStart\(Pre\)\?=//')
done
if [ -n "$missing_exec" ]; then
  echo "units reference executables that are not installed:${missing_exec}" >&2
  exit 70
fi
echo "every unit ExecStart path is installed and executable"

say "verify launcher flags against the built binary (exact match)"
opts="$(LD_LIBRARY_PATH="${QT_LLAMA_DIR}" "${QT_LLAMA_DIR}/llama-server" --help 2>&1 \
        | grep -oE '\-\-[a-z0-9-]+' | sort -u)"
missing=""
for f in --models-preset --models-max --no-webui --metrics \
         --slot-prompt-similarity --api-key-file --host --port; do
  # printf/grep -Fx: whole-line exact match, so --models cannot match
  # --models-preset and a typo cannot pass.
  printf '%s\n' "$opts" | grep -Fxq -- "$f" || missing="${missing} ${f}"
done
if [ -n "$missing" ]; then
  echo "the built llama-server does not accept:${missing}" >&2
  echo "the pinned tag ${QT_LLAMA_TAG} may have renamed them; do NOT guess" >&2
  exit 70
fi
echo "every launcher flag exists in this build"

say "done -- weights and service are the next phases"
