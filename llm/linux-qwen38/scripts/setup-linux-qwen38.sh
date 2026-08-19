#!/usr/bin/env bash
# Reproducible installer for the Qwen3.8-27B vLLM inference node.
#
#   sudo -v && ./scripts/setup-linux-qwen38.sh [--skip-download] [--skip-smoke]
#
# Idempotent: safe to re-run. Refuses to report success unless a real completion
# comes back from the served model.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SKIP_DOWNLOAD=0
SKIP_SMOKE=0
for a in "$@"; do
  case "$a" in
    --skip-download) SKIP_DOWNLOAD=1 ;;
    --skip-smoke)    SKIP_SMOKE=1 ;;
    *) echo "unknown option: $a" >&2; exit 64 ;;
  esac
done

# shellcheck source=../config/common.conf
. "${HERE}/config/common.conf"

SVC_USER="qwen-vllm"
step() { printf '\n=== %s ===\n' "$*"; }
die()  { echo "FATAL: $*" >&2; exit 1; }

# --------------------------------------------------------------------------
step "1/8 preflight"
command -v nvidia-smi >/dev/null || die "nvidia-smi not found; install the NVIDIA driver"
# Resolved to an absolute path because the steps below run it under sudo, and
# uv normally lives in ~/.local/bin, which is not on root's PATH.
UV="$(command -v uv || true)"
[ -n "$UV" ] || die "uv not found: https://docs.astral.sh/uv/"
# FlashInfer's JIT compiles CUDA sources at first inference, not at install, so
# a missing host compiler surfaces as a dead engine minutes in. Check it here.
command -v g++        >/dev/null || die "g++ not found; apt install build-essential"
sudo -n true 2>/dev/null || die "passwordless sudo required (or run 'sudo -v' first)"

driver="$(nvidia-smi --query-gpu=driver_version --format=csv,noheader | head -1)"
gpu="$(nvidia-smi --query-gpu=name --format=csv,noheader | head -1)"
vram="$(nvidia-smi --query-gpu=memory.total --format=csv,noheader,nounits | awk '{s+=$1} END{print s+0}')"
echo "gpu      : ${gpu} (${vram} MiB, driver ${driver})"

[ "${vram}" -ge 23000 ] || die "need >=23 GiB VRAM, found ${vram} MiB"

# Weights (~20 GiB) + venv with the CUDA wheels (~20 GiB) + room to breathe.
avail_gib="$(df -BG --output=avail /var/lib | tail -1 | tr -dc '0-9')"
[ "${avail_gib}" -ge 60 ] || die "need >=60 GiB free under /var/lib, found ${avail_gib} GiB"

# --------------------------------------------------------------------------
step "2/8 service account"
if ! getent group  "${SVC_USER}" >/dev/null; then sudo groupadd --system "${SVC_USER}"; fi
if ! getent passwd "${SVC_USER}" >/dev/null; then
  sudo useradd --system --gid "${SVC_USER}" --home-dir "${QWEN38_STATE}" \
       --shell /usr/sbin/nologin "${SVC_USER}"
fi
# GPU device access. render/video only exist on some hosts; add only what does.
for g in video render; do
  getent group "$g" >/dev/null && sudo usermod -aG "$g" "${SVC_USER}"
done
echo "service account: $(id "${SVC_USER}")"

# --------------------------------------------------------------------------
step "3/8 directories"
sudo mkdir -p "${QWEN38_PREFIX}"/{bin,etc} "${QWEN38_STATE}"/{models,results,logs,vllm-cache}
# State is writable by the service; binaries and config are not.
sudo chown -R "${SVC_USER}:${SVC_USER}" "${QWEN38_STATE}"
sudo chown -R root:root "${QWEN38_PREFIX}"
sudo chmod 0755 "${QWEN38_PREFIX}" "${QWEN38_PREFIX}/bin" "${QWEN38_PREFIX}/etc"

# --------------------------------------------------------------------------
step "4/8 python runtime (vllm==${QWEN38_VLLM_VERSION})"
if [ ! -x "${QWEN38_VENV}/bin/python" ]; then
  sudo "$UV" venv --python 3.12 "${QWEN38_VENV}"
fi
# huggingface_hub >=1.0 folded the `cli` extra into the base package and
# retired hf_transfer in favour of Xet, so neither extra is requested. Asking
# for `huggingface_hub[cli,hf_transfer]` only earns two warnings.
# ninja is named explicitly even though vLLM pulls it in transitively: FlashInfer
# JIT-compiles its sampling kernels on first request and shells out to `ninja`,
# so losing it to a future dependency change would break inference, not install.
sudo "$UV" pip install --python "${QWEN38_VENV}/bin/python" \
     "vllm==${QWEN38_VLLM_VERSION}" huggingface_hub ninja
"${QWEN38_VENV}/bin/python" - <<'PY'
import vllm, torch
print(f"vllm  {vllm.__version__}")
print(f"torch {torch.__version__} (cuda {torch.version.cuda})")
PY

# The architecture must be registered, or nothing below can work.
"${QWEN38_VENV}/bin/python" - <<'PY' || exit 1
from vllm.model_executor.models.registry import ModelRegistry
arch = "Qwen3_5ForConditionalGeneration"
assert arch in ModelRegistry.get_supported_archs(), f"{arch} not registered in this vLLM"
print(f"{arch}: registered")
PY

# --------------------------------------------------------------------------
step "5/8 install config, scripts and unit"
sudo install -m 0644 "${HERE}/config/common.conf" "${QWEN38_PREFIX}/etc/common.conf"
sudo mkdir -p "${QWEN38_PREFIX}/etc/profiles.d"
sudo install -m 0644 "${HERE}"/config/profiles.d/*.conf "${QWEN38_PREFIX}/etc/profiles.d/"
sudo install -m 0755 "${HERE}/scripts/serve-profile.sh" "${QWEN38_PREFIX}/bin/serve-profile.sh"
sudo install -m 0755 "${HERE}/scripts/qwen38ctl"        /usr/local/bin/qwen38ctl
sudo install -m 0644 "${HERE}/systemd/autospec-qwen38@.service" \
     /etc/systemd/system/autospec-qwen38@.service
sudo systemctl daemon-reload
sudo systemd-analyze verify /etc/systemd/system/autospec-qwen38@.service \
  || die "unit failed systemd-analyze verify"

# --------------------------------------------------------------------------
step "6/8 model artifacts"
if [ "${SKIP_DOWNLOAD}" -eq 0 ]; then
  # HF_HUB_ENABLE_HF_TRANSFER is deprecated in huggingface_hub 1.x; Xet is the
  # replacement accelerator and is on by default, so only the perf flag is set.
  sudo -u "${SVC_USER}" env \
    HF_HOME="${QWEN38_MODELS}" HF_XET_HIGH_PERFORMANCE=1 \
    "${QWEN38_VENV}/bin/hf" download "${QWEN38_MODEL_REPO}" \
      --revision "${QWEN38_MODEL_REVISION}"
fi
hub="${QWEN38_MODELS}/hub/models--${QWEN38_MODEL_REPO//\//--}"
snap="${hub}/snapshots/${QWEN38_MODEL_REVISION}"
[ -d "$snap" ] || die "model snapshot missing at ${snap}"
# Size the hub directory, not the snapshot: a snapshot is a tree of symlinks
# into ../../blobs, so `du` on it reports ~27K and would happily "verify" an
# empty download.
echo "snapshot: ${snap}"
echo "size    : $(du -sh "${hub}" | cut -f1)"
shards="$(find "$snap" -name '*.safetensors' | wc -l)"
[ "$shards" -ge 1 ] || die "no safetensors shards under ${snap}; download incomplete"
echo "shards  : ${shards}"

# --------------------------------------------------------------------------
step "7/8 enable default profile"
sudo systemctl enable "autospec-qwen38@interactive.service"

# --------------------------------------------------------------------------
step "8/8 smoke test"
if [ "${SKIP_SMOKE}" -eq 1 ]; then
  echo "skipped by request — installation is NOT validated"
  exit 0
fi
"${HERE}/tests/test_smoke.sh" || die "smoke test failed; installation is not usable"

echo
echo "OK — Qwen3.8-27B vLLM node installed and validated."
echo "   qwen38ctl status | qwen38ctl switch <profile> | qwen38ctl logs"
