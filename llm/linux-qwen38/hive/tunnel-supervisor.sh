#!/usr/bin/env bash
# Keep a forward alive from this workstation to whichever node is serving.
#
# Driven entirely by environment, so it can be read and tested on its own
# rather than through a layer of heredoc escaping:
#
#   HIVE_USER HIVE_HOST ROOT FORWARD_PORT REMOTE_PORT NODE STATE
#   GPU NGPU WALLTIME ACCOUNT PARTITION   (for re-acquiring a node)
#   API_KEY                               (for the capability probe)
#
# The job is to survive things that are NORMAL on a shared cluster: a login
# node that throttles connections, a network that blinks, a preemptible job
# that comes back on a different host, and a walltime that eventually expires.
#
# When the job really is gone this does NOT stop. It submits another one, waits
# for the scheduler, and re-points the forward. Combined with hive-proxy.py
# holding the client side, an expired allocation becomes a long pause rather
# than a dead endpoint -- and OpenCode's own state lives on the workstation, so
# there is nothing on the cluster to lose.
set -uo pipefail

HIVE_USER="${HIVE_USER:?}"; HIVE_HOST="${HIVE_HOST:?}"
ROOT="${ROOT:?}"; STATE="${STATE:?}"
FORWARD_PORT="${FORWARD_PORT:?}"; REMOTE_PORT="${REMOTE_PORT:?}"
node="${NODE:?}"
GPU="${GPU:-6000_blackwell}"; NGPU="${NGPU:-1}"
# Recovery must not be pinned to the card the session started on. If that type
# is congested when the allocation expires, re-acquiring with it means waiting
# hours for a GPU while others sit free -- the same stale-estimate trap the
# driver hit, except here nobody is watching.
GPU_CANDIDATES="${GPU_CANDIDATES:-${GPU}}"
# Empty unless the serving job was started with QWEN_REQUIRE_KEY=1. The
# capability probe below omits the header entirely when it is empty.
API_KEY="${API_KEY:-}"
WALLTIME="${WALLTIME:-12:00:00}"
ACCOUNT="${ACCOUNT:-publicgrp}"; PARTITION="${PARTITION:-low}"

# How many times the scheduler must AUTHORITATIVELY report no running job
# before we believe it. The previous version exited on the first empty answer,
# which meant one throttled ssh -- a thing this login node demonstrably does --
# permanently killed the tunnel mid-session.
GONE_LIMIT="${GONE_LIMIT:-5}"
MAX_BACKOFF="${MAX_BACKOFF:-30}"
# How long to wait for a replacement allocation before trying again. The queue
# is the slow part here, not us.
REACQUIRE_WAIT="${REACQUIRE_WAIT:-2400}"
# How often to re-check that the MODEL, not just the router, still answers.
# Once per connection is not enough: the child can die mid-session -- observed,
# "instance name=qwen3.8-27b exited with status 1" with no other diagnostic --
# and from outside that is indistinguishable from health, because /health keeps
# returning 200. A probe also heals it, since the request makes the router load
# the model again.
DEEP_PROBE_EVERY="${DEEP_PROBE_EVERY:-180}"

log() { printf '%s %s\n' "$(date -Is)" "$*"; }

# Submit a replacement allocation and wait for it to publish an endpoint. The
# previous behaviour -- exit and let the human notice -- meant a walltime
# expiry silently ended the session.
# Whichever candidate the scheduler says can start soonest. --test-only
# allocates nothing, so probing costs only a few seconds.
soonest_gpu() {
  local best="$GPU" best_ts="" t out ts
  for t in ${GPU_CANDIDATES//,/ }; do
    out="$(q "${HIVE_USER}@${HIVE_HOST}" "bash -lc 'bash -s'" <<REMOTE 2>/dev/null || true
sbatch --test-only -p ${PARTITION} -A ${ACCOUNT} --gres=gpu:${t}:${NGPU} \
  -c 8 --mem=8G -t ${WALLTIME} --wrap=true 2>&1 | head -1
REMOTE
)"
    ts="$(printf '%s' "$out" | grep -oE '[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9:]{8}' | head -1 || true)"
    [ -z "$ts" ] && continue
    if [ -z "$best_ts" ] || [ "$ts" \< "$best_ts" ]; then best="$t"; best_ts="$ts"; fi
  done
  printf '%s' "$best"
}

reacquire() {
  # Do not pile up allocations. A job already queued IS the replacement; the
  # previous version resubmitted on every cycle because it never noticed, and
  # left a queue of duplicate jobs behind.
  local pending
  pending="$(q "${HIVE_USER}@${HIVE_HOST}" \
      "bash -lc 'squeue -u ${HIVE_USER} -n qwen-serve -h -o \"%i %T\"'" 2>/dev/null \
      | awk '{print $1}' | head -1 || true)"

  local newjob=""
  if [ -n "$pending" ]; then
    log "a replacement (${pending}) is already queued; waiting for it"
    newjob="$pending"
  else
    local pick; pick="$(soonest_gpu)"
    if [ "$pick" != "$GPU" ]; then
      log "switching replacement GPU ${GPU} -> ${pick} (starts sooner)"
      GPU="$pick"
    fi
    log "no job; requesting a replacement (${NGPU}x ${GPU}, ${WALLTIME})"
    printf 'reacquiring' > "${STATE}/tunnel.state"
    newjob="$(q "${HIVE_USER}@${HIVE_HOST}" "bash -lc 'bash -s'" <<REMOTE 2>/dev/null | grep -oE '[0-9]+$' | head -1 || true
cd ${ROOT} && sbatch --gres=gpu:${GPU}:${NGPU} --time=${WALLTIME} \
  -p ${PARTITION} -A ${ACCOUNT} -o ${ROOT}/logs/serve-%j.out serve-qwen.sbatch
REMOTE
)"
    if [ -z "$newjob" ]; then
      log "sbatch did not return a job id"
      return 1
    fi
    log "submitted ${newjob}"
  fi

  # Wait for an endpoint published BY THAT JOB. Matching on the node line alone
  # matches the endpoint file the DEAD job left behind, which is how this
  # previously declared success in 30 seconds and forwarded to a host that was
  # no longer serving -- then did it again, and again.
  local waited=0 fresh
  while [ "$waited" -lt "$REACQUIRE_WAIT" ]; do
    sleep 30; waited=$((waited + 30))
    fresh="$(q "${HIVE_USER}@${HIVE_HOST}" "bash -lc 'bash -s'" <<REMOTE 2>/dev/null || true
awk -v want="${newjob}" '
  /^job/  { j = \$3 }
  /^node/ { n = \$3 }
  END     { if (j == want && n != "") print n }
' ${ROOT}/logs/endpoint.txt 2>/dev/null
REMOTE
)"
    fresh="$(printf '%s' "$fresh" | tr -d '[:space:]')"
    if [ -n "$fresh" ]; then
      log "job ${newjob} is serving on ${fresh} (waited ${waited}s)"
      node="$fresh"
      printf 'up' > "${STATE}/tunnel.state"
      return 0
    fi
    log "waiting for ${newjob} to publish an endpoint (${waited}s)"
  done
  return 1
}

# A dedicated connection for the DATA path. It deliberately does not share the
# driver's ControlMaster: a multiplexed master that dies takes every channel on
# it down at once, so the one connection that must not drop gets its own.
forward_opts=(-N -o BatchMode=yes -o ExitOnForwardFailure=yes
              -o ServerAliveInterval=15 -o ServerAliveCountMax=4
              -o TCPKeepAlive=yes -o ConnectTimeout=20)
query_opts=(-o BatchMode=yes -o ConnectTimeout=20
            -o ControlMaster=no -o ControlPath=none)
# ConnectTimeout bounds the handshake, not the remote command. A login node
# under load can accept a connection and then leave squeue hanging, which
# freezes this loop -- and this loop is the only thing keeping the session
# alive. Every remote call is bounded.
q() { timeout "${QUERY_TIMEOUT:-45}" ssh "${query_opts[@]}" "$@"; }

# An `ssh -N -L` does NOT exit when the far end of the forward dies. The
# connection to the login node is still perfectly healthy; only the per-request
# channel fails, and it fails one channel at a time:
#
#   channel 2: open failed: connect failed: Connection refused
#
# So the forward cannot be used as the liveness signal, and a loop that waits
# for ssh to exit before re-checking the job will wait forever while the client
# gets refused on every request. The forward is held in the BACKGROUND and its
# health is probed instead.
probe() {
  # Cheap liveness: any HTTP status means something answered; 000 means the
  # channel is dead. Note what this does NOT prove -- see deep_probe.
  local code
  code="$(curl -s -o /dev/null -m 5 -w '%{http_code}' \
          "http://127.0.0.1:${FORWARD_PORT}/health" 2>/dev/null || true)"
  [ -n "$code" ] && [ "$code" != "000" ]
}

# /health is the ROUTER's health, not the model's. A router whose child died --
# because the preset did not fit the card, say -- still answers /health with
# 200 while every request gets
#     500 proxy error: Could not establish connection
# and every model reports "unloaded". A whole benchmark run failed that way
# while the supervisor reported the endpoint healthy throughout.
#
# So liveness is checked cheaply and CAPABILITY is checked separately: ask for
# one token and require a real answer. Run once after each (re)connect, which
# is when "the job started but the model cannot load" actually happens, rather
# than on a timer -- forcing a load on an idle node costs minutes of GPU.
deep_probe() {
  local model="${DEEP_PROBE_MODEL:-qwen3.8-27b}" code
  code="$(curl -s -o /dev/null -m "${DEEP_PROBE_TIMEOUT:-900}" -w '%{http_code}' \
      -H 'Content-Type: application/json' \
      ${API_KEY:+-H "Authorization: Bearer ${API_KEY}"} \
      -d "{\"model\":\"${model}\",\"max_tokens\":1,\"messages\":[{\"role\":\"user\",\"content\":\"hi\"}]}" \
      "http://127.0.0.1:${FORWARD_PORT}/v1/chat/completions" 2>/dev/null || true)"
  [ "$code" = "200" ]
}

fwd_pid=""
start_forward() {
  stop_forward
  log "forwarding 127.0.0.1:${FORWARD_PORT} -> ${node}:${REMOTE_PORT}"
  ssh "${forward_opts[@]}" -L "${FORWARD_PORT}:${node}:${REMOTE_PORT}" \
      "${HIVE_USER}@${HIVE_HOST}" &
  fwd_pid=$!
  printf '%s' "$node" > "${STATE}/tunnel.node"
  verified=0
}
stop_forward() {
  if [ -n "$fwd_pid" ] && kill -0 "$fwd_pid" 2>/dev/null; then
    kill "$fwd_pid" 2>/dev/null || true
    wait "$fwd_pid" 2>/dev/null || true
  fi
  fwd_pid=""
}
trap 'stop_forward; exit 0' TERM INT

gone=0
sick=0
# Survive our own restarts. The counter is what an operator reads to answer
# "has this tunnel been stable?", so a supervisor that is itself restarted --
# by a fresh `opencode_hive up`, or by the session it runs under -- must not
# silently reset the history to zero and report a quiet night.
reconnects="$(cat "${STATE}/tunnel.reconnects" 2>/dev/null || true)"
case "$reconnects" in ''|*[!0-9]*) reconnects=0 ;; esac
last_deep=0
start_forward

while true; do
  sleep 10

  if probe; then
    sick=0; gone=0
    now=$(date +%s)
    # Re-verify on a schedule as well as after each connect: a child that dies
    # mid-session leaves /health answering 200 and every request failing.
    if [ "${verified:-0}" = 1 ] \
       && [ $((now - last_deep)) -ge "$DEEP_PROBE_EVERY" ]; then
      verified=0
    fi
    if [ "${verified:-0}" = 0 ]; then
      last_deep=$now
      if deep_probe; then
        if [ "${announced_ok:-0}" = 0 ]; then
          log "endpoint verified: the model answers"
          announced_ok=1
        fi
        verified=1
        printf 'up' > "${STATE}/tunnel.state"
      else
        announced_ok=0
        log "router answers but the model does not; treating as a dead job"
        printf 'degraded' > "${STATE}/tunnel.state"
        gone=$((gone + 1))
        if [ "$gone" -ge 2 ]; then
          stop_forward
          if reacquire; then start_forward; fi
          gone=0
        fi
      fi
      continue
    fi
    printf 'up' > "${STATE}/tunnel.state"
    continue
  fi

  sick=$((sick + 1))
  log "endpoint not answering (${sick})"
  printf 'degraded' > "${STATE}/tunnel.state"
  # One missed probe is a blip; several mean the far side is really gone.
  [ "$sick" -lt 3 ] && continue

  # Is the ssh itself dead, or is the far end gone? Restarting the forward is
  # cheap and fixes the first case, so try it before troubling the scheduler.
  if ! kill -0 "$fwd_pid" 2>/dev/null; then
    reconnects=$((reconnects + 1))
    printf '%s' "$reconnects" > "${STATE}/tunnel.reconnects"
    log "forward process died; restarting"
    start_forward; sick=0; continue
  fi

  # The forward is up but nothing answers through it. Ask the scheduler why.
  out="$(q "${HIVE_USER}@${HIVE_HOST}" \
        "bash -lc 'squeue -u ${HIVE_USER} -n qwen-serve -h -o \"%i %T\"'" 2>/dev/null)"
  rc=$?
  if [ "$rc" -ne 0 ]; then
    log "job query unreachable (ssh rc=${rc}); assuming the job is fine"
    continue
  fi

  jid="$(printf '%s\n' "$out" | awk '$2=="RUNNING"{print $1; exit}')"
  if [ -z "$jid" ]; then
    gone=$((gone + 1))
    log "scheduler reports no running job (${gone}/${GONE_LIMIT})"
    if [ "$gone" -ge "$GONE_LIMIT" ]; then
      stop_forward
      if reacquire; then
        start_forward
      else
        log "re-acquire did not complete; will try again"
      fi
      gone=0; sick=0
    fi
    continue
  fi

  # A job IS running -- most likely a requeue onto a different host, or the
  # model is still loading after a restart.
  gone=0
  fresh="$(q "${HIVE_USER}@${HIVE_HOST}" \
          "awk '/^node/{print \$3}' ${ROOT}/logs/endpoint.txt" 2>/dev/null)"
  if [ -n "$fresh" ] && [ "$fresh" != "$node" ]; then
    log "node moved ${node} -> ${fresh}"
    node="$fresh"
    reconnects=$((reconnects + 1))
    printf '%s' "$reconnects" > "${STATE}/tunnel.reconnects"
    start_forward; sick=0
  fi
done
