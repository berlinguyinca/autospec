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

# A RELEASE, not master. The previous build was `--depth 1` of whatever master
# happened to be that day -- unrecorded, unreproducible, and bleeding edge. A
# child process crashed mid-session with `instance ... exited with status 1`,
# healthy at 44.7 tok/s one line earlier and no diagnostic, which is the kind of
# thing a release tag exists to avoid.
#
# v0.1.2 was checked for every flag this deployment relies on: --models-preset,
# --models-max, --kv-unified, --image-min-tokens. Override to chase a fix or to
# go back to master deliberately.
LLAMA_REF="${LLAMA_REF:-v0.1.2}"
STAMP="${PREFIX}/BUILD_REF"

# Runtime linkage, needed by anything that EXECUTES a built binary. llama.cpp
# splits each tool into a thin driver plus a libllama-<tool>-impl.so beside it,
# and CUDA's own runtime comes from the module -- so both paths are required.
# Getting this wrong looks like a broken build ("error while loading shared
# libraries: libllama-server-impl.so") when the build was in fact fine.
export LD_LIBRARY_PATH="${PREFIX}/lib${LD_LIBRARY_PATH:+:${LD_LIBRARY_PATH}}"

# Skip only if the existing build both RUNS and is the ref we were asked for.
# Testing runnability alone meant changing LLAMA_REF silently kept the old
# binary, which is the sort of thing that makes a pin worthless.
if [ -x "${PREFIX}/bin/llama-server" ] \
   && "${PREFIX}/bin/llama-server" --version >/dev/null 2>&1 \
   && [ "$(cat "${STAMP}" 2>/dev/null)" = "${LLAMA_REF}" ]; then
  echo "== llama.cpp ${LLAMA_REF} already built and runnable, skipping rebuild =="
  SKIP_BUILD=1
fi

# BUILD_ROOT is per-job scratch, so this is always a fresh shallow clone --
# no reuse to invalidate and nothing in the tree to reset.
if [ -z "${SKIP_BUILD:-}" ]; then
git clone --depth 1 --branch "${LLAMA_REF}" \
    https://github.com/ggml-org/llama.cpp "${SRC}"
cd "${SRC}"
echo "llama.cpp ${LLAMA_REF} at $(git rev-parse --short HEAD)"

cmake -B build \
  -DCMAKE_BUILD_TYPE=Release \
  -DGGML_CUDA=ON \
  -DCMAKE_CUDA_ARCHITECTURES="${ARCHS}" \
  -DLLAMA_CURL=ON \
  -DCMAKE_INSTALL_PREFIX="${PREFIX}"
cmake --build build --config Release -j "$(nproc)"
cmake --install build
# Record the provenance next to the binary, so a later run can tell whether it
# is looking at the build it asked for.
printf '%s' "${LLAMA_REF}" > "${STAMP}"
printf '%s %s\n' "${LLAMA_REF}" "$(git -C "${SRC}" rev-parse HEAD)" \
    >> "${ROOT}/logs/build-history.txt"
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
# matrix (text or vision, standard or uncensored) from one node.
#
# Filenames are given POSITIONALLY, not as --include globs. `hf download REPO
# --include A B` silently treats B as an explicit filename and then discards
# --include entirely -- "Ignoring `--include` since filenames have been
# explicitly set" -- so a run that looked successful fetched only the 885 MiB
# projector and none of the 29 GiB it was asked for. Explicit names are also
# exactly what this needs: the set is small, known, and worth pinning.
fetch() {
  local repo="$1"; shift
  echo "-- ${repo}"
  uvx --from huggingface_hub hf download "${repo}" "$@" --local-dir "${MODELS}"
}
fetch unsloth/Qwen3.8-27B-GGUF \
      Qwen3.8-27B-UD-Q8_K_XL.gguf mmproj-F16.gguf
fetch Blackfrost-AI/Qwen3.8-27B-ABLITERATED-GGUF \
      Qwen3.8-27B-ABLITERATED-Q8_0.gguf mmproj-Qwen3.8-27B-ABLITERATED-F16.gguf

# Size floors, because "the command exited 0" is not the same claim as "the
# weights are here". A truncated or skipped download leaves a plausible-looking
# tree that only fails much later, at model load, on another node.
missing=0
check_size() {
  local f="${MODELS}/$1" min_mib="$2" got
  got=$(stat -c %s "$f" 2>/dev/null || echo 0)
  got=$((got / 1048576))
  if [ "$got" -lt "$min_mib" ]; then
    echo "MISSING/SHORT: $1 is ${got} MiB, expected >= ${min_mib}" >&2
    missing=1
  else
    printf '  ok  %-52s %6s MiB\n' "$1" "$got"
  fi
}
check_size Qwen3.8-27B-UD-Q8_K_XL.gguf                    25000
check_size Qwen3.8-27B-ABLITERATED-Q8_0.gguf              25000
check_size mmproj-F16.gguf                                  800
check_size mmproj-Qwen3.8-27B-ABLITERATED-F16.gguf          800
[ "$missing" = 0 ] || { echo "weight staging incomplete" >&2; exit 1; }

echo "== staged =="
ls -lh "${MODELS}" | head
