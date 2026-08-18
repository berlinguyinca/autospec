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

module load cuda/13.3.0

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
