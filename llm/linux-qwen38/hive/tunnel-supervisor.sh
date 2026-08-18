#!/usr/bin/env bash
# Keep a forward alive from this workstation to whichever node is serving.
#
# Driven entirely by environment, so it can be read and tested on its own
# rather than through a layer of heredoc escaping:
#
#   HIVE_USER HIVE_HOST ROOT LOCAL_PORT REMOTE_PORT NODE STATE
#
# The job is to survive things that are NORMAL on a shared cluster: a login
# node that throttles connections, a network that blinks, and a preemptible
# job that comes back on a different host.
set -uo pipefail

HIVE_USER="${HIVE_USER:?}"; HIVE_HOST="${HIVE_HOST:?}"
ROOT="${ROOT:?}"; STATE="${STATE:?}"
LOCAL_PORT="${LOCAL_PORT:?}"; REMOTE_PORT="${REMOTE_PORT:?}"
node="${NODE:?}"

# How many times the scheduler must AUTHORITATIVELY report no running job
# before we believe it. The previous version exited on the first empty answer,
# which meant one throttled ssh -- a thing this login node demonstrably does --
# permanently killed the tunnel mid-session.
GONE_LIMIT="${GONE_LIMIT:-5}"
MAX_BACKOFF="${MAX_BACKOFF:-30}"

log() { printf '%s %s\n' "$(date -Is)" "$*"; }

# A dedicated connection for the DATA path. It deliberately does not share the
# driver's ControlMaster: a multiplexed master that dies takes every channel on
# it down at once, so the one connection that must not drop gets its own.
forward_opts=(-N -o BatchMode=yes -o ExitOnForwardFailure=yes
              -o ServerAliveInterval=15 -o ServerAliveCountMax=4
              -o TCPKeepAlive=yes -o ConnectTimeout=20)
query_opts=(-o BatchMode=yes -o ConnectTimeout=20
            -o ControlMaster=no -o ControlPath=none)

gone=0
backoff=1
reconnects=0

while true; do
  # --- is the job still there? -----------------------------------------------
  # ssh's own exit status separates "the scheduler says no job" from "I could
  # not ask". Only the first is evidence.
  out="$(ssh "${query_opts[@]}" "${HIVE_USER}@${HIVE_HOST}" \
        "bash -lc 'squeue -u ${HIVE_USER} -n qwen-serve -h -o \"%i %T\"'" 2>/dev/null)"
  rc=$?
  if [ "$rc" -ne 0 ]; then
    log "job query unreachable (ssh rc=${rc}); assuming the job is fine"
  else
    jid="$(printf '%s\n' "$out" | awk '$2=="RUNNING"{print $1; exit}')"
    if [ -z "$jid" ]; then
      gone=$((gone + 1))
      log "scheduler reports no running job (${gone}/${GONE_LIMIT})"
      if [ "$gone" -ge "$GONE_LIMIT" ]; then
        log "job is gone; supervisor exiting"
        exit 0
      fi
    else
      gone=0
      # A requeued job can come back on a different host, and the old forward
      # then points at a node that is no longer serving.
      fresh="$(ssh "${query_opts[@]}" "${HIVE_USER}@${HIVE_HOST}" \
              "awk '/^node/{print \$3}' ${ROOT}/logs/endpoint.txt" 2>/dev/null)"
      if [ -n "$fresh" ] && [ "$fresh" != "$node" ]; then
        log "node moved ${node} -> ${fresh}"
        node="$fresh"
      fi
    fi
  fi

  # --- hold the forward ------------------------------------------------------
  log "forwarding 127.0.0.1:${LOCAL_PORT} -> ${node}:${REMOTE_PORT}"
  printf '%s' "$node" > "${STATE}/tunnel.node"
  start=$(date +%s)
  ssh "${forward_opts[@]}" -L "${LOCAL_PORT}:${node}:${REMOTE_PORT}" \
      "${HIVE_USER}@${HIVE_HOST}"
  held=$(( $(date +%s) - start ))
  reconnects=$((reconnects + 1))
  printf '%s' "$reconnects" > "${STATE}/tunnel.reconnects"

  # A forward that stood for a while was healthy; treat its loss as a blip and
  # come back immediately. One that dies instantly is failing for a reason
  # (port already bound, host unreachable) and deserves backoff instead of a
  # tight loop hammering the login node.
  if [ "$held" -ge 60 ]; then
    backoff=1
    log "forward dropped after ${held}s; reconnecting immediately"
  else
    log "forward failed after ${held}s; retrying in ${backoff}s"
    sleep "$backoff"
    backoff=$(( backoff * 2 ))
    if [ "$backoff" -gt "$MAX_BACKOFF" ]; then backoff="$MAX_BACKOFF"; fi
  fi
done
