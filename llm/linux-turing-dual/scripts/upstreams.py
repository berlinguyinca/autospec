#!/usr/bin/env python3
"""The registry of servers this node can route to. Parsing and validation only.

No I/O beyond being handed text: health probing belongs to whatever has a
background timer, and routing belongs to the gateway. Keeping this module pure
means every validation rule below is testable without a socket.

Routing is by PATH (`/u/<id>/v1/...`) rather than by model name, because
resolving a model name would mean parsing the request body -- and not parsing the
body is what makes the gateway a pass-through. See upstreams.yaml.example.
"""
from __future__ import annotations

import re
from dataclasses import dataclass, field

try:
    import yaml
except ImportError:  # pragma: no cover
    yaml = None

# Same shape as a path segment, because that is what it becomes.
ID_RE = re.compile(r"^[a-z0-9][a-z0-9-]{0,30}$")

LOCAL = "local"
AUTO = "auto"
RESERVED = (LOCAL, AUTO)


@dataclass
class Upstream:
    id: str
    base_url: str
    key_file: str | None = None
    enabled: bool = True
    note: str | None = None
    gpus: str | None = None
    problems: list[str] = field(default_factory=list)

    @property
    def usable(self) -> bool:
        return self.enabled and not self.problems

    def public(self) -> dict:
        """What the dashboard may show. Excludes key_file: a path is not a
        secret, but it is also of no use on a page and every field that stays
        server-side is a field that cannot leak."""
        return {"id": self.id, "base_url": self.base_url, "enabled": self.enabled,
                "note": self.note, "gpus": self.gpus, "problems": self.problems,
                "needs_key": bool(self.key_file)}


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


def route(path: str, ups: list[Upstream]) -> tuple[Upstream | None, str] | None:
    """Split a request path into (upstream, remaining path).

    Returns (None, path) for a local request, (upstream, rewritten) for a remote
    one, and None when the path names an upstream that is not usable -- which the
    caller answers as a clean refusal rather than by guessing.
    """
    if not path.startswith("/u/"):
        return None, path
    rest = path[3:]
    uid, slash, tail = rest.partition("/")
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


def pick_auto(ups: list[Upstream], state: dict, last_used: str | None,
              local_online: bool = True) -> str | None:
    """Which server `/u/auto/` should send this caller to, or None if none can.

    THE ORDER IS DELIBERATE, and the first rule is the whole point:

    1. **The server this caller used last**, if it is still online. Not for
       tidiness -- for the PREFIX CACHE. A warm slot was measured on this project
       at roughly a tenfold saving on prompt processing, so sending someone back
       to the machine that already holds their conversation is the largest
       performance lever available. Sending them elsewhere silently throws it away.
    2. **This node**, when nothing is remembered. Its state is known first-hand
       rather than inferred from a poll, and it costs no extra network hop.
    3. **Any online remote**, in registry order, so a node that is merely busy
       still gets used rather than the request failing.

    Returns None only when nothing is online, which the caller must answer as a
    refusal rather than by sending the request somewhere and hoping.
    """
    online = [u.id for u in ups
              if u.usable and (state.get(u.id) or {}).get("state") == "online"]
    candidates = ([LOCAL] if local_online else []) + online
    if not candidates:
        return None
    if last_used and last_used in candidates:
        return last_used
    if LOCAL in candidates:
        return LOCAL
    return online[0]
