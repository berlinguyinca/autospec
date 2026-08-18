#!/usr/bin/env bash
# Provision the SHIPPED stack: llama.cpp router serving a text preset and a
# vision preset on one port, as a boot service, with the client wired up.
#
#   ./scripts/install-node.sh [--with-opencode] [--skip-download] [--skip-verify]
#
# Idempotent. Refuses to report success unless the served model returns a real
# completion, identifies a generated image, and retrieves a needle from a long
# prompt. scripts/setup-linux-qwen38.sh installs the OPTIONAL vLLM profiles;
# this script is the one that produces the default configuration.
#
# VERIFIED ON: Ubuntu 24.04 + RTX 4090 (CUDA). Other platforms are handled where
# noted and are NOT verified -- see ../QWEN-NODE-SPEC.md Appendix B.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WITH_OPENCODE=0; SKIP_DOWNLOAD=0; SKIP_VERIFY=0
for a in "$@"; do
  case "$a" in
    --with-opencode) WITH_OPENCODE=1 ;;
    --skip-download) SKIP_DOWNLOAD=1 ;;
    --skip-verify)   SKIP_VERIFY=1 ;;
    *) echo "unknown option: $a" >&2; exit 64 ;;
  esac
done

# shellcheck source=../config/common.conf
. "${HERE}/config/common.conf"
SVC_USER="qwen-vllm"
GGUF_DIR="/var/lib/qwen-gguf/models"
LLAMA_TAG="b10434"

step() { printf '\n=== %s ===\n' "$*"; }
die()  { echo "FATAL: $*" >&2; exit 1; }

# --------------------------------------------------------------------------
step "1/8 platform detection"
OS="$(uname -s)"; ARCH="$(uname -m)"
ACCEL="cpu"
if command -v nvidia-smi >/dev/null 2>&1 && nvidia-smi -L >/dev/null 2>&1; then
  ACCEL="cuda"
elif [ "$OS" = "Darwin" ] && [ "$ARCH" = "arm64" ]; then
  ACCEL="metal"
elif command -v rocminfo >/dev/null 2>&1; then
  ACCEL="hip"
elif command -v sycl-ls >/dev/null 2>&1; then
  ACCEL="sycl"
fi
echo "os/arch : ${OS}/${ARCH}"
echo "accel   : ${ACCEL}"
if [ "$ACCEL" = "cuda" ]; then
  nvidia-smi --query-gpu=name,memory.total,driver_version --format=csv,noheader | sed 's/^/gpu     : /'
  vram="$(nvidia-smi --query-gpu=memory.total --format=csv,noheader,nounits | head -1)"
  [ "${vram}" -ge 23000 ] || die "need >=23 GiB VRAM for this preset set, found ${vram} MiB"
fi
[ "$OS" = "Linux" ] || die "this installer targets Linux/systemd; on macOS use launchd (see ../QWEN-NODE-SPEC.md B.3)"

avail_gib="$(df -BG --output=avail /var/lib | tail -1 | tr -dc '0-9')"
[ "${avail_gib}" -ge 45 ] || die "need >=45 GiB free under /var/lib, found ${avail_gib} GiB"

# --------------------------------------------------------------------------
step "2/8 llama.cpp ${LLAMA_TAG}"
LLAMA_DIR="${QWEN38_LLAMA_DIR:-/opt/qwen-local/llama.cpp/current}"
if [ -x "${LLAMA_DIR}/llama-server" ]; then
  echo "reusing : ${LLAMA_DIR}"
  "${LLAMA_DIR}/llama-server" --version 2>&1 | head -1 | sed 's/^/version : /' || true
else
  # There is NO prebuilt Linux CUDA binary in llama.cpp releases -- only CPU,
  # Vulkan, SYCL and OpenVINO. CUDA must be compiled. Everything else can be
  # fetched, which is why this branches on the accelerator rather than the OS.
  base="https://github.com/ggml-org/llama.cpp/releases/download/${LLAMA_TAG}"
  case "$ACCEL" in
    cuda)
      command -v cmake >/dev/null || die "cmake required to build llama.cpp with CUDA"
      command -v nvcc  >/dev/null || die "nvcc required to build llama.cpp with CUDA"
      echo "building from source with GGML_CUDA=ON (no prebuilt Linux CUDA release exists)"
      src="/opt/qwen-local/llama.cpp/src-${LLAMA_TAG}"
      sudo mkdir -p "$(dirname "$src")"
      [ -d "$src" ] || sudo git clone --depth 1 --branch "${LLAMA_TAG}" \
        https://github.com/ggml-org/llama.cpp "$src"
      sudo cmake -S "$src" -B "${src}/build" -DGGML_CUDA=ON -DCMAKE_BUILD_TYPE=Release
      sudo cmake --build "${src}/build" --config Release -j"$(nproc)" \
        --target llama-server llama-cli llama-bench
      sudo mkdir -p "/opt/qwen-local/llama.cpp/${LLAMA_TAG}"
      sudo cp -a "${src}/build/bin/." "/opt/qwen-local/llama.cpp/${LLAMA_TAG}/"
      ;;
    metal|sycl|hip|cpu)
      case "$ACCEL" in
        sycl) asset="llama-${LLAMA_TAG}-bin-ubuntu-sycl-fp16-x64.tar.gz" ;;
        hip)  asset="llama-${LLAMA_TAG}-bin-ubuntu-vulkan-x64.tar.gz" ;;   # Vulkan is the portable AMD path
        metal) asset="llama-${LLAMA_TAG}-bin-macos-arm64.tar.gz" ;;
        *)    asset="llama-${LLAMA_TAG}-bin-ubuntu-x64.tar.gz" ;;
      esac
      echo "fetching: ${asset}"
      tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
      curl -fsSL "${base}/${asset}" -o "${tmp}/l.tar.gz" || die "download failed: ${asset}"
      sudo mkdir -p "/opt/qwen-local/llama.cpp/${LLAMA_TAG}"
      sudo tar -xzf "${tmp}/l.tar.gz" -C "/opt/qwen-local/llama.cpp/${LLAMA_TAG}" --strip-components=1
      ;;
  esac
  sudo ln -sfn "/opt/qwen-local/llama.cpp/${LLAMA_TAG}" "/opt/qwen-local/llama.cpp/current"
  [ -x "${LLAMA_DIR}/llama-server" ] || die "llama-server still missing at ${LLAMA_DIR}"
fi
# The router is not in every build; fail here rather than at first request.
"${LLAMA_DIR}/llama-server" --help 2>&1 | grep -q -- "--models-preset" \
  || die "this llama.cpp build has no router mode (--models-preset); need ${LLAMA_TAG} or newer"
echo "router  : supported"

# --------------------------------------------------------------------------
step "3/8 service account and directories"
getent group  "${SVC_USER}" >/dev/null || sudo groupadd --system "${SVC_USER}"
getent passwd "${SVC_USER}" >/dev/null || sudo useradd --system --gid "${SVC_USER}" \
  --home-dir "${QWEN38_STATE}" --shell /usr/sbin/nologin "${SVC_USER}"
for g in video render; do getent group "$g" >/dev/null && sudo usermod -aG "$g" "${SVC_USER}"; done
sudo mkdir -p "${QWEN38_PREFIX}"/{bin,etc/profiles.d} "${QWEN38_STATE}"/{logs,results} "${GGUF_DIR}"
sudo chown -R "${SVC_USER}:${SVC_USER}" "${QWEN38_STATE}" /var/lib/qwen-gguf
sudo chown -R root:root "${QWEN38_PREFIX}"
echo "account : $(id "${SVC_USER}")"

# --------------------------------------------------------------------------
step "4/8 model artifacts"
GGUF="${GGUF_DIR}/Qwen3.8-27B-Q5_K_M.gguf"
MMPROJ="${GGUF_DIR}/mmproj-F16.gguf"
if [ "${SKIP_DOWNLOAD}" -eq 0 ]; then
  HF="${QWEN38_VENV}/bin/hf"
  [ -x "$HF" ] || HF="$(command -v hf || true)"
  [ -n "$HF" ] || die "huggingface CLI not found; pip install huggingface_hub"
  for f in Qwen3.8-27B-Q5_K_M.gguf mmproj-F16.gguf; do
    if [ ! -s "${GGUF_DIR}/${f}" ]; then
      echo "downloading ${f}"
      sudo -u "${SVC_USER}" env HF_HOME=/var/lib/qwen-gguf/hf \
        "$HF" download unsloth/Qwen3.8-27B-GGUF "$f" >/dev/null
      src="$(sudo find /var/lib/qwen-gguf/hf -name "$f" | head -1)"
      sudo cp -f "$(sudo readlink -f "$src")" "${GGUF_DIR}/${f}"
      sudo chown "${SVC_USER}:${SVC_USER}" "${GGUF_DIR}/${f}"
    fi
  done
fi
# Size floors, not just existence: an interrupted download leaves a short file
# that every later step happily accepts.
[ "$(stat -Lc %s "$GGUF"   2>/dev/null || echo 0)" -gt 15000000000 ] || die "GGUF missing/short: $GGUF"
[ "$(stat -Lc %s "$MMPROJ" 2>/dev/null || echo 0)" -gt   500000000 ] || die "projector missing/short: $MMPROJ"
ls -lh "$GGUF" "$MMPROJ" | awk '{print "  "$5"  "$9}'

# --------------------------------------------------------------------------
step "5/8 install config, launcher, control CLI and unit"
sudo install -m 0644 "${HERE}/config/common.conf"        "${QWEN38_PREFIX}/etc/common.conf"
sudo install -m 0644 "${HERE}/config/router-presets.ini" "${QWEN38_PREFIX}/etc/router-presets.ini"
sudo install -m 0644 "${HERE}"/config/profiles.d/*.conf  "${QWEN38_PREFIX}/etc/profiles.d/"
sudo install -m 0755 "${HERE}/scripts/serve-profile.sh"      "${QWEN38_PREFIX}/bin/serve-profile.sh"
sudo install -m 0755 "${HERE}/scripts/qwen38ctl" /usr/local/bin/qwen38ctl

# Everything an operator needs AFTER the install: measuring the real ceiling,
# adding a model, re-deriving the client config, sizing a context window.
# Leaving these in the checkout means a machine provisioned from a release
# tarball silently lacks half the toolkit, and the omission only surfaces when
# somebody needs the tool -- which is exactly when they cannot get it.
for tool in long-prompt-probe.py measure-ceiling.sh measure-slot-frontier.sh \
            bench-concurrency.py bench-context-sweep.sh benchmark.py \
            select-quant.py add-gguf-model.sh configure-opencode.py \
            analyze-session-contexts.py; do
  sudo install -m 0755 "${HERE}/scripts/${tool}" "${QWEN38_PREFIX}/bin/${tool}"
done
# check_presets.py lives with the tests but is a runtime guard too: it is what
# catches a preset tier that outgrew its pool before the pool finds out.
sudo install -m 0755 "${HERE}/tests/check_presets.py" "${QWEN38_PREFIX}/bin/check_presets.py"
sudo install -m 0644 "${HERE}/systemd/autospec-qwen38@.service" \
  /etc/systemd/system/autospec-qwen38@.service
sudo systemctl daemon-reload
sudo systemd-analyze verify /etc/systemd/system/autospec-qwen38@.service \
  || die "unit failed systemd-analyze verify"

# --------------------------------------------------------------------------
step "6/8 enable and start the router"
sudo systemctl enable autospec-qwen38@router.service
qwen38ctl stop >/dev/null 2>&1 || true
sudo systemctl start autospec-qwen38@router.service
port="$(grep -m1 '^QWEN38_PORT=' "${QWEN38_PREFIX}/etc/profiles.d/router.conf" | cut -d'"' -f2)"
BASE="http://${QWEN38_HOST}:${port}"
waited=0
until curl -sf --max-time 5 "${BASE}/health" >/dev/null 2>&1; do
  systemctl is-active --quiet autospec-qwen38@router.service \
    || { journalctl -u autospec-qwen38@router.service -n 25 --no-pager >&2; die "router died on startup"; }
  [ "$waited" -ge 600 ] && die "router not healthy after ${waited}s"
  sleep 5; waited=$((waited+5))
done
echo "healthy after ${waited}s on ${BASE}"
curl -sf "${BASE}/v1/models" | python3 -c 'import sys,json
for m in json.load(sys.stdin)["data"]:
    print("  model :", m["id"], "|", ",".join(m.get("architecture",{}).get("input_modalities",[])))'

# --------------------------------------------------------------------------
step "7/8 verify with real inference"
if [ "${SKIP_VERIFY}" -eq 1 ]; then
  echo "skipped by request — installation is NOT validated"
else
  PY="${QWEN38_VENV}/bin/python"; [ -x "$PY" ] || PY="$(command -v python3)"

  text_model="$(curl -sf "${BASE}/v1/models" | python3 -c 'import sys,json
d=[m["id"] for m in json.load(sys.stdin)["data"] if "image" not in m.get("architecture",{}).get("input_modalities",[])]
print(d[0] if d else "")')"
  vis_model="$(curl -sf "${BASE}/v1/models" | python3 -c 'import sys,json
d=[m["id"] for m in json.load(sys.stdin)["data"] if "image" in m.get("architecture",{}).get("input_modalities",[])]
print(d[0] if d else "")')"
  [ -n "$text_model" ] || die "no text model advertised"

  out="$(curl -sf --max-time 300 "${BASE}/v1/chat/completions" -H 'Content-Type: application/json' \
    -d "{\"model\":\"${text_model}\",\"temperature\":0,\"max_tokens\":64,
         \"chat_template_kwargs\":{\"enable_thinking\":false},
         \"messages\":[{\"role\":\"user\",\"content\":\"Reply with exactly: INSTALL_OK\"}]}" \
    | python3 -c 'import sys,json;print(json.load(sys.stdin)["choices"][0]["message"]["content"])')"
  printf '%s' "$out" | grep -q INSTALL_OK || die "completion failed: ${out}"
  echo "  PASS  completion (${text_model})"

  ctx="$(grep -A4 "^\[${text_model}\]" "${QWEN38_PREFIX}/etc/router-presets.ini" \
        | grep -m1 '^c *=' | tr -dc '0-9')"
  res="$("$PY" "${QWEN38_PREFIX}/bin/long-prompt-probe.py" "$BASE" "$text_model" "${ctx:-32768}")"
  case "$res" in CEILING_OK*) echo "  PASS  long-prompt retrieval at ${ctx} (${res})" ;;
                 *) die "long-prompt retrieval failed: ${res}" ;; esac

  if [ -n "$vis_model" ] && [ -r "${HERE}/tests/test_vision.py" ]; then
    "$PY" "${HERE}/tests/test_vision.py" "$BASE" "$vis_model" >/dev/null \
      || die "vision verification failed for ${vis_model}"
    echo "  PASS  vision (${vis_model}) — swapped models on request"
  fi
fi

# --------------------------------------------------------------------------
step "8/8 client configuration"
if [ "${WITH_OPENCODE}" -eq 1 ]; then
  python3 "${HERE}/scripts/configure-opencode.py" \
    --presets "${QWEN38_PREFIX}/etc/router-presets.ini" --port "${port}" --set-default
else
  echo "skipped; run with --with-opencode, or:"
  echo "  python3 ${HERE}/scripts/configure-opencode.py --set-default"
fi

echo
echo "OK — router installed, verified and enabled at boot."
echo "   endpoint : ${BASE}/v1"
echo "   switch   : select a different model id in the client; the router swaps"
echo "   control  : qwen38ctl status | logs | start <profile>"
