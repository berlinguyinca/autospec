#!/usr/bin/env python3
"""The dashboard chat panel's request rules.

The panel is the one place on this node where inference is bought with a browser
SESSION rather than with an API key. That is a deliberate convenience -- a signed
-in person should be able to try the models without first minting a credential --
and it brings one hazard a key does not have: a cookie travels automatically, so
any page on the internet could make the browser spend this node's GPU time.

Hence `same_origin()`, which is checked BEFORE anything is read or routed.

Everything here is pure: no sockets, no store, no handler. The gateway supplies
the header lookup and the raw body, and gets back either a request to relay or a
refusal to report.
"""
from __future__ import annotations

import json

# A panel, not an API. These bounds are what keep it one: someone who wants long
# conversations, many turns, or big context has a key and the real endpoint.
MAX_MESSAGES = 24
MAX_CHARS = 8000
MAX_TOKENS = 2048
DEFAULT_MAX_TOKENS = 1024
ROLES = ("system", "user", "assistant")

# The accounting identity for chat traffic. NOT a minted key: creating a real
# credential that its owner cannot see or revoke would invert the whole point of
# the key surface. `usage_events.key_id` is NOT NULL with no foreign key, and the
# leaderboard groups by `sub`, so a sentinel attributes correctly and costs
# nothing. It shows up in the per-key table as this label, which is honest.
USAGE_KEY_ID = "dashboard-chat"


def same_origin(get_header, host: str) -> bool:
    """Did this request come from this node's own page?

    `Sec-Fetch-Site` is the reliable signal and every browser that can run the
    panel sends it. `Origin` is the fallback, compared against the Host the
    request arrived on -- nginx is the only thing that reaches this process, so
    Host is what the client actually asked for.

    A request carrying NEITHER header is refused. That is not a browser fetch,
    and something that is not a browser should be presenting a key instead.
    """
    site = (get_header("Sec-Fetch-Site") or "").strip().lower()
    if site:
        return site == "same-origin"
    origin = (get_header("Origin") or "").strip()
    if not origin or not host:
        return False
    for scheme in ("https://", "http://"):
        if origin == scheme + host:
            return True
    return False


def validate(raw: bytes, models) -> tuple[dict | None, str | None]:
    """Turn a panel payload into a chat-completions body, or say what is wrong.

    `models` is the set of ids this node will serve. The model is checked HERE
    rather than left to the upstream, because llama.cpp answers for a model it
    does not have -- measured on this fleet -- so an unchecked id comes back as
    somebody else's weights with a 200 and no error.
    """
    try:
        payload = json.loads(raw or b"{}")
    except ValueError:
        return None, "that is not JSON"
    if not isinstance(payload, dict):
        return None, "expected a JSON object"

    model = payload.get("model")
    if not isinstance(model, str) or not model:
        return None, "name a model"
    if models is not None and model not in models:
        return None, f"this node does not serve {model}"

    messages = payload.get("messages")
    if not isinstance(messages, list) or not messages:
        return None, "send at least one message"
    if len(messages) > MAX_MESSAGES:
        return None, f"the panel keeps {MAX_MESSAGES} messages at most"

    total = 0
    clean = []
    for m in messages:
        if not isinstance(m, dict):
            return None, "each message must be an object"
        role, content = m.get("role"), m.get("content")
        if role not in ROLES:
            return None, f"role must be one of {', '.join(ROLES)}"
        if not isinstance(content, str):
            return None, "message content must be a string"
        total += len(content)
        clean.append({"role": role, "content": content})
    if total > MAX_CHARS:
        return None, f"the panel takes {MAX_CHARS} characters at most"

    want = payload.get("max_tokens", DEFAULT_MAX_TOKENS)
    if not isinstance(want, int) or isinstance(want, bool) or want < 1:
        return None, "max_tokens must be a positive whole number"
    # Clamped rather than refused: a caller asking for more than the panel gives
    # is not making a mistake, they are asking for the real endpoint.
    want = min(want, MAX_TOKENS)

    body = {"model": model, "messages": clean, "max_tokens": want,
            "stream": True}
    # Reasoning tokens run FIRST on these models, so a small budget returns empty
    # content -- measured: max_tokens 16 yields nothing at all. The panel is a
    # conversation, so thinking is off by default and the budget buys an answer.
    if payload.get("thinking") is not True:
        body["chat_template_kwargs"] = {"enable_thinking": False}
    return body, None
