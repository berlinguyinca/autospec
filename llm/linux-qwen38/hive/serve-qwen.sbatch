#!/bin/bash
#SBATCH --job-name=qwen-serve
#SBATCH --partition=low
#SBATCH --account=publicgrp
#SBATCH --gres=gpu:6000_blackwell:1
#SBATCH --cpus-per-task=16
#SBATCH --mem=64G
#SBATCH --time=7-00:00:00
#SBATCH --requeue
#SBATCH --signal=B:TERM@120
#
# Serve Qwen3.8-27B on a hive GPU node for the duration of the allocation.
#
#   sbatch serve-qwen.sbatch
#   cat  $ROOT/logs/endpoint.txt     # where it landed, and the tunnel command
#
# 7 days is the cap on `low`; `high` allows 30 but has 12 GPUs against low's 86
# and a queue measured in weeks, and its association still charges publicgrp.
#
# THIS JOB CAN BE PREEMPTED. `low` is PriorityTier=10 with PreemptMode=REQUEUE
# and GraceTime=130s, so a `high` job wanting the node will requeue this one
# with about two minutes' notice. Nothing here holds state that matters: the
# server is restarted from the same weights and presets, and clients reconnect.
# What DOES break is the endpoint -- the job may come back on another node, so
# re-read endpoint.txt after a requeue rather than assuming the old tunnel.
set -euo pipefail

ROOT="${QWEN_HIVE_ROOT:-/quobyte/metabolomicsgrp/it/llm}"
PREFIX="${ROOT}/opt/llama.cpp"
MODELS="${ROOT}/models"
PORT="${QWEN_PORT:-8080}"
mkdir -p "${ROOT}/logs"

# gcc as well as cuda, at RUN time. The binary is compiled by the gcc 13.2
# module and needs GLIBCXX_3.4.32, which Ubuntu 22.04's system libstdc++ does
# not provide -- it stops at 3.4.30. Loading only cuda gets you
# "version `GLIBCXX_3.4.32' not found", which looks like a broken binary rather
# than a missing runtime toolchain.
module load cuda/13.3.0 gcc/13.2.0
# Two runtime paths, both required. llama.cpp splits each tool into a thin
# driver plus libllama-<tool>-impl.so next to it, so the install lib/ must be on
# LD_LIBRARY_PATH; libcudart.so.13 comes from the module above. Missing either
# is exit 127 with "error while loading shared libraries", which reads like a
# broken build rather than a missing path.
export LD_LIBRARY_PATH="${PREFIX}/lib${LD_LIBRARY_PATH:+:${LD_LIBRARY_PATH}}"

# Prove the binary RUNS before publishing an endpoint for it. The first attempt
# advertised a node, port and tunnel command, and only then died on the missing
# library -- so the client dutifully forwarded to a port nothing ever listened
# on, and the failure surfaced as a timeout far from its cause.
if ! "${PREFIX}/bin/llama-server" --version >/dev/null 2>&1; then
  echo "llama-server will not run; rebuild with setup-hive.sh" >&2
  "${PREFIX}/bin/llama-server" --version || true
  exit 1
fi

# A stale endpoint file outlives the job that wrote it, and a driver reading it
# will tunnel confidently to a node that is no longer serving. Clear it first,
# and let the job that owns it be the one that publishes.
rm -f "${ROOT}/logs/endpoint.txt"

# Stage the weights onto the node's local NVMe. /quobyte is a shared parallel
# filesystem: fine for a one-time read, but 30 GiB of it competes with everyone
# else's I/O, and the node has 3.5 TB of local scratch sitting idle.
LOCAL="${SLURM_TMPDIR:-/scratch/${USER}-${SLURM_JOB_ID}}"
mkdir -p "${LOCAL}"
echo "staging weights to ${LOCAL} ..."
cp -n "${MODELS}"/*.gguf "${LOCAL}/" 2>/dev/null || true

# The presets ship with a placeholder rather than an absolute path, so the same
# file works from /quobyte and from node-local scratch.
PRESETS="${LOCAL}/router-presets.ini"
sed "s#MODELS_DIR#${LOCAL}#g" "${ROOT}/router-presets-hive.ini" > "${PRESETS}"

# An unauthenticated model server on a shared cluster network is reachable by
# every other user on it. The key is generated per job and written next to the
# endpoint, readable only by this account.
KEYFILE="${ROOT}/logs/api-key.txt"
if [ ! -s "${KEYFILE}" ]; then
  umask 077; head -c 24 /dev/urandom | base64 | tr -d '/+=' > "${KEYFILE}"
fi
API_KEY="$(cat "${KEYFILE}")"

NODE="$(hostname -f)"
cat > "${ROOT}/logs/endpoint.txt" <<EOF
job     : ${SLURM_JOB_ID}
node    : ${NODE}
port    : ${PORT}
started : $(date -Is)

# from your workstation:
ssh -N -L ${PORT}:${NODE}:${PORT} ${USER}@hive.hpc.ucdavis.edu
# then point the client at http://127.0.0.1:${PORT}/v1 with the key in
# ${KEYFILE}
EOF
cat "${ROOT}/logs/endpoint.txt"

# Forward the preemption signal so llama-server exits cleanly inside GraceTime.
term() { echo "preempted or cancelled, shutting down"; kill -TERM "${srv:-0}" 2>/dev/null || true; }
trap term TERM INT

"${PREFIX}/bin/llama-server" \
    --models-preset "${PRESETS}" \
    --models-max 1 \
    --api-key "${API_KEY}" \
    --host 0.0.0.0 \
    --port "${PORT}" &
srv=$!
wait "${srv}"
