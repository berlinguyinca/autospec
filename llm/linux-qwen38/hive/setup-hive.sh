#!/bin/bash
#SBATCH --job-name=qwen-setup
#SBATCH --partition=low
#SBATCH --account=publicgrp
#SBATCH --cpus-per-task=32
#SBATCH --mem=64G
#SBATCH --time=03:00:00
#SBATCH --requeue
#
# Build llama.cpp and stage the weights into the group's shared space.
#
#   sbatch setup-hive.sh
#
# Run once. Everything it produces lives on /quobyte and survives the job, so
# the serving job (serve-qwen.sbatch) starts in seconds afterwards.
#
# Deliberately requests NO GPU. Compiling CUDA code needs nvcc, not a device,
# and downloading weights needs neither -- so this schedules against all 168
# nodes in `low` instead of queueing for one of the 86 GPUs that the serving
# job actually wants.
#
# Account note: GPUs must be charged to publicgrp. The metabolomicsgrp
# association carries gres/gpu=0 and a GPU request under it is rejected at
# submit time with QOSGrpGRES.
set -euo pipefail

ROOT="${QWEN_HIVE_ROOT:-/quobyte/metabolomicsgrp/it/llm}"
# Build on node-local NVMe, install to /quobyte. A source checkout is tens of
# thousands of small files and a parallel filesystem is the worst possible place
# for that -- the first run spent minutes inside `git clone` alone. Only the
# installed binaries need to persist.
BUILD_ROOT="${SLURM_TMPDIR:-/scratch/${USER}-${SLURM_JOB_ID:-$$}}"
SRC="${BUILD_ROOT}/llama.cpp"
PREFIX="${ROOT}/opt/llama.cpp"
MODELS="${ROOT}/models"
mkdir -p "${BUILD_ROOT}" "${PREFIX}" "${MODELS}" "${ROOT}/logs"

module load cuda/13.3.0 gcc/13.2.0 cmake/3.28.1

echo "== toolchain =="
nvcc --version | tail -2
cmake --version | head -1
nvidia-smi --query-gpu=name,memory.total,compute_cap --format=csv,noheader 2>/dev/null \
  || echo "no GPU on this node (expected: none was requested)"

# --- llama.cpp ---------------------------------------------------------------
# Built for every GPU generation on this cluster, because the serving job may
# land on any of them and a binary without the right arch fails at load with a
# "no kernel image is available" error that does not name the cause:
#   80  A100        86  A6000        89  L40S / RTX 5000 Ada      120  Blackwell
ARCHS="80;86;89;120"

# Runtime linkage, needed by anything that EXECUTES a built binary. llama.cpp
# splits each tool into a thin driver plus a libllama-<tool>-impl.so beside it,
# and CUDA's own runtime comes from the module -- so both paths are required.
# Getting this wrong looks like a broken build ("error while loading shared
# libraries: libllama-server-impl.so") when the build was in fact fine.
export LD_LIBRARY_PATH="${PREFIX}/lib${LD_LIBRARY_PATH:+:${LD_LIBRARY_PATH}}"

if [ -x "${PREFIX}/bin/llama-server" ] && "${PREFIX}/bin/llama-server" --version >/dev/null 2>&1; then
  echo "== llama.cpp already built and runnable, skipping rebuild =="
  SKIP_BUILD=1
fi

# BUILD_ROOT is per-job scratch, so this is always a fresh shallow clone --
# no reuse to invalidate and nothing in the tree to reset.
if [ -z "${SKIP_BUILD:-}" ]; then
git clone --depth 1 https://github.com/ggml-org/llama.cpp "${SRC}"
cd "${SRC}"
echo "llama.cpp at $(git rev-parse --short HEAD)"

cmake -B build \
  -DCMAKE_BUILD_TYPE=Release \
  -DGGML_CUDA=ON \
  -DCMAKE_CUDA_ARCHITECTURES="${ARCHS}" \
  -DLLAMA_CURL=ON \
  -DCMAKE_INSTALL_PREFIX="${PREFIX}"
cmake --build build --config Release -j "$(nproc)"
cmake --install build
fi

echo "== built =="
# Informational only. This ran under `set -e` before and a linkage problem here
# aborted the script BEFORE the weights download, turning a working build into a
# failed job with no weights to show for it.
"${PREFIX}/bin/llama-server" --version 2>&1 | head -2 || true

# --- weights -----------------------------------------------------------------
# 96 GiB of VRAM removes the capacity pressure that forced Q5 on a 24 GiB card,
# so this stages the 8-bit build. Compute nodes have outbound internet
# (verified), so the download runs here rather than on the login node.
export HF_HOME="${ROOT}/hf-cache"
export PATH="${HOME}/.local/bin:${PATH}"

# Two model families, each with its projector, so the router can serve the full
# matrix (text or vision, standard or uncensored) from one node. Re-running is
# cheap: hf download skips files that already match.
fetch() {
  echo "-- ${1}"
  uvx --from huggingface_hub hf download "$1" \
      --include "${@:2}" --local-dir "${MODELS}"
}
fetch unsloth/Qwen3.8-27B-GGUF "*UD-Q8_K_XL*.gguf" "mmproj-F16.gguf"
fetch Blackfrost-AI/Qwen3.8-27B-ABLITERATED-GGUF \
      "Qwen3.8-27B-ABLITERATED-Q8_0.gguf" "mmproj-Qwen3.8-27B-ABLITERATED-F16.gguf"

echo "== staged =="
ls -lh "${MODELS}" | head
