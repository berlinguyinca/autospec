#!/usr/bin/env bash
# Multi-GPU free-VRAM guard.
#
#   vram-guard.sh --min-total <MiB> [--min-per-card <MiB>]
#
# Exists because the single-card version of this check read nvidia-smi's FIRST
# ROW and compared it against a floor sized for one big card. On two 11 GiB
# cards, card 0 reports ~11000 against a 20000 floor, so it refused to start a
# node that fits -- and it blamed VRAM, which is the one thing that was fine.
#
# --min-total     the sum across every visible device; what a split model spends
# --min-per-card  the best single card; what a model PINNED to one card needs
#
# Exit codes are distinct on purpose, because "no driver" and "not enough VRAM"
# want different fixes:
#   0   enough
#   64  EX_USAGE        bad argument
#   69  EX_UNAVAILABLE  no nvidia-smi, or it reported no devices
#   75  EX_TEMPFAIL     not enough VRAM right now
set -euo pipefail

NVIDIA_SMI="${NVIDIA_SMI:-nvidia-smi}"
MIN_TOTAL=0
MIN_PER_CARD=0
EXPECT_DEVICES=0

while [ $# -gt 0 ]; do
  case "$1" in
    --min-total)    MIN_TOTAL="${2:?--min-total needs a value}";    shift 2 ;;
    --min-per-card) MIN_PER_CARD="${2:?--min-per-card needs a value}"; shift 2 ;;
    --expect-devices) EXPECT_DEVICES="${2:?--expect-devices needs a value}"; shift 2 ;;
    -h|--help)      sed -n '2,20p' "$0"; exit 0 ;;
    *) echo "vram-guard: unknown argument: $1" >&2; exit 64 ;;
  esac
done

# An absolute path that does not exist, or a name not on PATH, is 69 -- not a
# VRAM verdict. `command -v` covers both forms.
command -v "$NVIDIA_SMI" >/dev/null 2>&1 || {
  echo "vram-guard: ${NVIDIA_SMI} not found" >&2; exit 69; }

raw="$("$NVIDIA_SMI" --query-gpu=memory.free --format=csv,noheader,nounits 2>/dev/null)" || {
  echo "vram-guard: ${NVIDIA_SMI} failed to report memory" >&2; exit 69; }

total=0
best=0
n=0
while IFS= read -r mib; do
  mib="${mib//[[:space:]]/}"
  [ -n "$mib" ] || continue
  case "$mib" in *[!0-9]*) continue ;; esac
  total=$(( total + mib ))
  if [ "$mib" -gt "$best" ]; then best="$mib"; fi
  n=$(( n + 1 ))
done <<< "$raw"

if [ "$n" -eq 0 ]; then
  echo "vram-guard: ${NVIDIA_SMI} reported no devices" >&2
  exit 69
fi

# A device-count mismatch is 69 (unavailable hardware), not 75 (transient
# pressure): a card that is gone will not come back on a retry timer.
if [ "$EXPECT_DEVICES" -gt 0 ] && [ "$n" -ne "$EXPECT_DEVICES" ]; then
  echo "vram-guard: refusing to start: ${n} device(s) visible, expected ${EXPECT_DEVICES}" >&2
  exit 69
fi

if [ "$total" -lt "$MIN_TOTAL" ]; then
  echo "vram-guard: refusing to start: ${total} MiB free across ${n} device(s), need ${MIN_TOTAL} total" >&2
  exit 75
fi

if [ "$MIN_PER_CARD" -gt 0 ] && [ "$best" -lt "$MIN_PER_CARD" ]; then
  echo "vram-guard: refusing to start: best per-card free is ${best} MiB, need ${MIN_PER_CARD} on one card" >&2
  exit 75
fi

echo "vram-guard: ok -- ${total} MiB free across ${n} device(s), best card ${best} MiB"
