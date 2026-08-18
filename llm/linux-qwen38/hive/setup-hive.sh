#!/bin/bash
#SBATCH --job-name=qwen-setup
#SBATCH --partition=low
#SBATCH --account=publicgrp
#SBATCH --gres=gpu:6000_blackwell:1
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
nvidia-smi --query-gpu=name,memory.total,compute_cap --format=csv,noheader

# --- llama.cpp ---------------------------------------------------------------
# Built for every GPU generation on this cluster, because the serving job may
# land on any of them and a binary without the right arch fails at load with a
# "no kernel image is available" error that does not name the cause:
#   80  A100        86  A6000        89  L40S / RTX 5000 Ada      120  Blackwell
ARCHS="80;86;89;120"

# BUILD_ROOT is per-job scratch, so this is always a fresh shallow clone --
# no reuse to invalidate and nothing in the tree to reset.
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

echo "== built =="
"${PREFIX}/bin/llama-server" --version 2>&1 | head -2

# --- weights -----------------------------------------------------------------
# 96 GiB of VRAM removes the capacity pressure that forced Q5 on a 24 GiB card,
# so this stages the 8-bit build. Compute nodes have outbound internet
# (verified), so the download runs here rather than on the login node.
export HF_HOME="${ROOT}/hf-cache"
export PATH="${HOME}/.local/bin:${PATH}"
uvx --from huggingface_hub hf download unsloth/Qwen3.8-27B-GGUF \
    --include "*UD-Q8_K_XL*.gguf" "mmproj-F16.gguf" \
    --local-dir "${MODELS}"

echo "== staged =="
ls -lh "${MODELS}" | head
