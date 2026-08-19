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

# --- phase 4: install config and helpers -----------------------------------
say "install config, helpers and units"
sudo install -d "${QT_PREFIX}/bin" "${QT_PREFIX}/etc/profiles.d" "${QT_STATE}"
sudo install -m 0755 "${HERE}/vram-guard.sh"    "${QT_PREFIX}/bin/"
sudo install -m 0755 "${HERE}/serve-router.sh"  "${QT_PREFIX}/bin/"
sudo install -m 0755 "${HERE}/collect-stats.py" "${QT_PREFIX}/bin/"
sudo install -m 0755 "${HERE}/queue_window.py"  "${QT_PREFIX}/bin/"
sudo install -m 0755 "${HERE}/dashboard.py"     "${QT_PREFIX}/bin/"
sudo install -m 0755 "${HERE}/dashboard-run.sh" "${QT_PREFIX}/bin/"
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
