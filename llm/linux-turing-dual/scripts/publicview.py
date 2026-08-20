#!/usr/bin/env python3
"""What an unauthenticated visitor may see.

Anyone may read this node: how to reach the models, what it is serving, how busy
it is, how fast it is, and who is on the leaderboard. Signing in is required only
to mint a key and to use the chat panel.

EVERY projection here is built by iterating an allow-list, never by deleting
private keys from a full payload. The failure mode of a forgotten field is then a
MISSING value rather than a disclosure -- the same rule the queue payload in
collect-stats.py already follows. A new field added upstream is invisible here
until someone names it, which is the safe direction to fail.

Two things are deliberately absent and must stay absent:

  * ADDRESSES. `base_url`, and any upstream error string -- those quote hosts and
    ports, and this repository is public. The whole node is built so that no
    identifier of this site appears anywhere but /etc/qwen-turing/site.conf.
  * IDENTITIES. Email addresses and Cognito subs. The leaderboard is a lab
    scoreboard, so display names stay: a leaderboard nobody can find themselves
    on is not one. Contact details are a different question from a nickname, and
    a public URL is scraped.
"""
from __future__ import annotations

# --- the node's own numbers -------------------------------------------------
STATS_FIELDS = (
    "llama_up", "model", "catalog", "queue",
    "gpu_count", "gpu_total_mem_mib", "gpu_used_mem_mib",
    "kv_cache_usage_ratio", "requests_processing", "requests_deferred",
    "tokens_per_second", "prompt_tokens_total", "generated_tokens_total",
)
# Per-card telemetry. `name` is a product name ("NVIDIA GeForce RTX 4090"), not
# an identifier of this machine. No UUID or serial is carried, and none may be.
GPU_FIELDS = ("index", "name", "mem_total_mib", "mem_used_mib",
              "util_pct", "temp_c", "power_w")

# --- the fleet --------------------------------------------------------------
SERVER_FIELDS = (
    "id", "kind", "enabled", "state", "models", "priority", "pool_member",
    "in_flight", "idle_pipes", "route", "note", "gpus", "slots", "last_seen",
    # Measured, never declared: what this node has actually seen the server do.
    "prefill_rate", "mean_service", "samples",
)
# `agent_version` is deliberately NOT here. It is software inventory, which is
# operator information rather than capability.
FLEET_FIELDS = ("poll_seconds", "default_route", "auto_route",
                "balanced_paths", "peek_bytes", "pipe_wait_seconds",
                "heartbeat_seconds")

# --- the leaderboard --------------------------------------------------------
LEADER_FIELDS = ("display_name", "requests", "prompt_tokens",
                 "completion_tokens", "cached_tokens", "total_tokens")


def _pick(src: dict, fields) -> dict:
    return {k: src.get(k) for k in fields}


def stats(full: dict) -> dict:
    """The node's numbers, minus the operator's diagnostics.

    `config_health` is excluded on purpose: it reports journal readability and
    recent evictions, which is what an operator needs and not what a visitor
    asked for.
    """
    out = _pick(full or {}, STATS_FIELDS)
    out["gpus"] = [_pick(g or {}, GPU_FIELDS) for g in (full or {}).get("gpus") or []]
    out["public"] = True
    return out


def servers(payload: dict) -> dict:
    """The fleet, by capability and load, with nothing that says where it lives.

    A server's `id` stays -- it is the nickname the operator chose at enrolment
    and it is what a caller types into `/u/<id>/v1`, so the panel is useless
    without it. `base_url` and `error` do not: the first is an address, and the
    second quotes one whenever a connection fails.
    """
    payload = payload or {}
    out = _pick(payload, FLEET_FIELDS)
    out["servers"] = [_pick(s or {}, SERVER_FIELDS)
                      for s in payload.get("servers") or []]
    out["routing"] = dict(payload.get("routing") or {})
    out["public"] = True
    return out


def leaderboard(rows) -> list[dict]:
    """Ranks by display name. No email, no sub.

    A row whose owner has never signed in has no display name -- keys can be
    minted for a subject the registry has not met. Such a row still ranks, and
    renders as unattributed rather than being dropped: dropping it would make
    the totals disagree with the node's own usage figures.
    """
    return [_pick(r or {}, LEADER_FIELDS) for r in rows or []]
