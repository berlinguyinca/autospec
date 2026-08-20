#!/usr/bin/env python3
"""The registry of servers this node can route to. Parsing and validation only.

No I/O beyond being handed text: health probing belongs to whatever has a
background timer, and routing belongs to the gateway. Keeping this module pure
means every validation rule below is testable without a socket.

Routing is by PATH (`/u/<id>/v1/...`) rather than by model name, because
resolving a model name would mean parsing the request body -- and not parsing the
body is what makes the gateway a pass-through. See upstreams.yaml.example.

`/v1` -- the plain, default base URL -- IS the virtual server: a request that
names no server is balanced, and `/u/local/v1` is how a caller pins this node.
So every client already configured against this node gets the load balancer with
no reconfiguration, which is the only way "the default" can mean anything.
"""
from __future__ import annotations

import re
from dataclasses import dataclass, field

import scheduler as _sched

try:
    import yaml
except ImportError:  # pragma: no cover
    yaml = None

# Same shape as a path segment, because that is what it becomes.
ID_RE = re.compile(r"^[a-z0-9][a-z0-9-]{0,30}$")

LOCAL = "local"
AUTO = "auto"
RESERVED = (LOCAL, AUTO)

# Only these endpoints may be balanced. An ALLOW-LIST, not a deny-list: nginx
# funnels llama.cpp's whole surface through the gateway -- /health, /props,
# /metrics, /slots, /completion -- and those describe THIS machine, so answering
# them from another one would be a lie. A deny-list would silently start
# balancing the next endpoint llama.cpp grows.
BALANCED_PATHS = ("/v1/chat/completions", "/v1/completions", "/v1/embeddings",
                  "/v1/rerank", "/v1/reranking")


# How the node reaches a server. The distinction is not cosmetic: a tunnelled
# server has NO address, because it dialled in, and asking for one would mean
# dialling a box that expects to dial out.
KIND_FILE = "file"          # from the registry file; configuration, not owned
KIND_STATIC = "static"      # attached through the dashboard, dialled by the node
KIND_TUNNEL = "tunnel"      # attached through the dashboard, holds pipes open
DIRECT_KINDS = (KIND_FILE, KIND_STATIC)


@dataclass
class Upstream:
    id: str
    base_url: str
    key_file: str | None = None
    enabled: bool = True
    note: str | None = None
    gpus: str | None = None
    problems: list[str] = field(default_factory=list)
    kind: str = KIND_FILE
    # The operator's tier. Absolute in the scheduler, so it is bounded where it
    # is set rather than here.
    priority: int = 0
    # File entries default to IN the pool, which preserves the behaviour they
    # already had; anything attached from the dashboard starts out of it, because
    # a server declares its own model ids and inserting a stranger's hardware
    # into everyone's default route is a different act from attaching it.
    pool_member: bool = True
    owner: str | None = None

    @property
    def usable(self) -> bool:
        return self.enabled and not self.problems

    @property
    def direct(self) -> bool:
        """Does this node dial it? False for a tunnelled server, whose capacity
        arrives as pipes instead."""
        return self.kind in DIRECT_KINDS

    def public(self) -> dict:
        """What the dashboard may show. Excludes key_file: a path is not a
        secret, but it is also of no use on a page and every field that stays
        server-side is a field that cannot leak."""
        return {"id": self.id, "base_url": self.base_url if self.direct else None,
                "enabled": self.enabled, "note": self.note, "gpus": self.gpus,
                "problems": self.problems, "needs_key": bool(self.key_file),
                "kind": self.kind, "priority": self.priority,
                "pool_member": self.pool_member, "owner": self.owner}


def balanceable(path: str) -> bool:
    """May a request for this path be sent to another server?

    True only for the endpoints that carry a `model` in the body and mean the
    same thing on any machine. Everything else pins to this node.
    """
    return path.rstrip("/") in BALANCED_PATHS


def _problems_for(raw: dict, seen: set[str]) -> list[str]:
    """Every reason this entry cannot be used, collected rather than raised.

    Collected because one bad entry must not hide the others, and because the
    Servers panel should be able to SAY what is wrong with a server instead of
    silently omitting it -- an omitted server looks like a server nobody
    configured.
    """
    out = []
    uid = str(raw.get("id") or "")
    base = str(raw.get("base_url") or "")

    if not uid:
        out.append("no id")
    elif not ID_RE.match(uid):
        out.append("id must be lowercase letters, digits and dashes")
    elif uid in seen:
        out.append("duplicate id")
    if uid in RESERVED:
        out.append(f"'{uid}' is reserved")

    if not base:
        out.append("no base_url")
    elif "<" in base or ">" in base:
        out.append("base_url is still a placeholder")
    elif not base.startswith(("http://", "https://")):
        out.append("base_url must start with http:// or https://")

    kf = raw.get("key_file")
    if kf and ("<" in str(kf) or ">" in str(kf)):
        out.append("key_file is still a placeholder")
    return out


def load(text: str) -> list[Upstream]:
    """Parse registry text into entries, each carrying its own problems."""
    if yaml is None:
        raise RuntimeError("pyyaml is required to read the upstream registry")
    doc = yaml.safe_load(text) or {}
    raw = doc.get("upstreams") or []
    if not isinstance(raw, list):
        raise ValueError("'upstreams' must be a list")

    out: list[Upstream] = []
    seen: set[str] = set()
    for item in raw:
        if not isinstance(item, dict):
            continue
        problems = _problems_for(item, seen)
        uid = str(item.get("id") or "")
        if uid:
            seen.add(uid)
        out.append(Upstream(
            id=uid, base_url=str(item.get("base_url") or ""),
            key_file=(str(item["key_file"]) if item.get("key_file") else None),
            enabled=bool(item.get("enabled", True)),
            note=(str(item["note"]) if item.get("note") else None),
            gpus=(str(item["gpus"]) if item.get("gpus") else None),
            problems=problems))
    return out


def from_records(rows: list[dict]) -> list[Upstream]:
    """Attached servers (from the store) as routable entries.

    The same type as a file entry deliberately: one list, one scheduler, one
    report. A second type would mean every routing decision asking which kind it
    was holding.
    """
    out = []
    for r in rows:
        kind = r.get("kind") or KIND_STATIC
        base = r.get("base_url") or ""
        problems = []
        if kind in DIRECT_KINDS and not base.startswith(("http://", "https://")):
            problems.append("no usable base_url")
        out.append(Upstream(
            id=r.get("server_id") or "", base_url=base, key_file=None,
            enabled=True, note=r.get("note"), gpus=r.get("gpus"),
            problems=problems, kind=kind, priority=int(r.get("priority") or 0),
            pool_member=bool(r.get("pool_member")), owner=r.get("sub")))
    return out


def route(path: str, ups: list[Upstream]) -> tuple[Upstream | None, str] | None:
    """Split a request path into (upstream, remaining path).

    Returns (None, path) for a local request, (LOCAL, rewritten) when the caller
    explicitly pinned this node, (upstream, rewritten) for a remote one, and None
    when the path names an upstream that is not usable -- which the caller answers
    as a clean refusal rather than by guessing.
    """
    if not path.startswith("/u/"):
        return None, path
    rest = path[3:]
    uid, slash, tail = rest.partition("/")
    if uid == LOCAL:
        # Now that the plain path is balanced, this is the ONLY way to say "this
        # machine and no other". It used to refuse, because `local` was reserved
        # and never a destination.
        return LOCAL, ("/" + tail if slash else "/")
    for u in ups:
        if u.id == uid:
            if not u.usable:
                return None
            return u, ("/" + tail if slash else "/")
    return None


def target(u: Upstream, path: str) -> tuple[str, str, int, str]:
    """(scheme, host, port, full path) for a request to this upstream.

    base_url carries a path prefix (conventionally /v1), and the incoming path
    already begins with /v1 -- so the prefix is taken from base_url and the
    duplicate dropped, rather than producing /v1/v1.
    """
    from urllib.parse import urlsplit
    parts = urlsplit(u.base_url)
    scheme = parts.scheme or "http"
    host = parts.hostname or ""
    port = parts.port or (443 if scheme == "https" else 80)
    prefix = (parts.path or "").rstrip("/")
    if prefix and path.startswith(prefix):
        path = path[len(prefix):] or "/"
    return scheme, host, port, prefix + path


def servers_for(ups: list[Upstream], state: dict, model: str) -> list[str]:
    """Every server last known to serve this model, whatever its state now.

    Used only to EXPLAIN a refusal: "nothing online serves X" is a dead end,
    "X is on bender, which is not answering" is something a caller can act on.
    """
    out = []
    for u in ups:
        if model in ((state.get(u.id) or {}).get("models") or []):
            out.append(u.id)
    return out


def eligible(ups: list[Upstream], state: dict, model: str | None, *,
             local_online: bool = True,
             local_models: list[str] | None = None,
             pool_only: bool = True) -> list[str]:
    """Which servers may serve this request, in preference order.

    THIS IS A HARD FILTER, applied before any preference. It exists because
    llama.cpp does not refuse a model it has not got -- it answers with whatever
    is loaded. Measured on this fleet: a request for `qwen3.5-9b-vision` came
    back served by `qwen3.8-27b`, no error. So a server that does not advertise
    the model is not a slower choice, it is a WRONG one.

    Remote eligibility is a POSITIVE check against that server's own polled
    list, which is what makes the unknown cases safe: nothing is eligible by
    default, so a failed poll excludes a server rather than including it.

    `model is None` means the request's model could not be read (the peek buffer
    ran out, or the body is not JSON). Then only this node is eligible -- never a
    remote that might substitute silently.
    """
    out = []
    if local_online and (model is None or not local_models or model in local_models):
        # This node is eligible when its own list is UNKNOWN as well as when the
        # model is in it: it is the model authority here, and a broken probe
        # must not empty the fleet.
        out.append(LOCAL)
    if model is None:
        return out
    for u in ups:
        if not u.usable:
            continue
        # `pool_only` is what keeps an attached server out of the DEFAULT route
        # until an admin promotes it. A pin bypasses this by asking for the server
        # by name, which is a different question from "where should this go".
        if pool_only and not u.pool_member:
            continue
        st = state.get(u.id) or {}
        if st.get("state") == "online" and model in (st.get("models") or []):
            out.append(u.id)
    return out


def pick_auto(ups: list[Upstream], state: dict, last_used: str | None, *,
              model: str | None = None, local_online: bool = True,
              local_models: list[str] | None = None,
              stats: dict | None = None, load: dict | None = None,
              ready: dict | None = None, prompt_tokens: float = 0.0,
              pool_only: bool = True) -> tuple[str | None, str, float | None]:
    """(server, why, estimated seconds) for a balanced request.

    Two steps, and the order is the whole safety property:

    1. **Eligibility**, here. Absolute. A server that does not advertise the
       model is not a slower choice, it is a WRONG one -- llama.cpp answers with
       whatever it has loaded rather than refusing, measured on this fleet.
    2. **Ranking**, delegated to scheduler.choose, which knows nothing about
       models and everything about how fast these servers have actually been.

    `stats` is keyed (server, model) with a (server, None) fallback, as
    KeyStore.throughput returns it. `load` and `ready` are keyed by server.
    """
    elig = eligible(ups, state, model, local_online=local_online,
                    local_models=local_models, pool_only=pool_only)
    if not elig:
        return None, "none-eligible", None
    if model is None:
        # The model could not be read, so only this node is eligible (a remote
        # might substitute silently). Said as its own reason rather than dressed
        # up as a ranking decision.
        return elig[0], "blind", None

    by_id = {u.id: u for u in ups}
    stats = stats or {}
    load = load or {}
    ready = ready or {}
    candidates = []
    for sid in elig:
        u = by_id.get(sid)
        s = stats.get((sid, model)) or stats.get((sid, None)) or {}
        candidates.append(_sched.Candidate(
            server_id=sid,
            priority=(u.priority if u else 0),
            queued_ahead=float(load.get(sid, 0) or 0),
            prefill_rate=s.get("prefill_rate"),
            mean_service=s.get("mean_service"),
            warm=(last_used == sid),
            ready=bool(ready.get(sid, True))))
    return _sched.choose(candidates, prompt_tokens)
