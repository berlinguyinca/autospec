#!/usr/bin/env python3
"""Find the model a request asks for, from a BOUNDED prefix of its body.

The gateway routes by path precisely so it never has to parse a request body:
a 100k prompt is ~400 KB and streams straight through. But a load balancer that
does not know which model was asked for cannot avoid sending the request to a
server that does not serve it -- and llama.cpp does not refuse in that case, it
ANSWERS WITH WHATEVER IT HAS LOADED. Measured on this fleet: a request naming
`qwen3.5-9b-vision` was answered by `qwen3.8-27b`, with no error. A vision
request silently served by a text model, or an uncensored one by the aligned
model, is worse than a refusal.

So the body is peeked, not parsed:

  * At most PEEK_BYTES are buffered, then forwarded unchanged. Either `model`
    is near the front -- every OpenAI SDK, the Vercel ai-sdk and our own
    snippets put it first -- or it sits behind a 400 KB `messages` array where
    no affordable buffer would reach it.
  * The scan is inconclusive rather than wrong when the buffer runs out. The
    caller treats "I could not tell" as "keep it local", never as "any server
    will do".
  * `model` is accepted ONLY as a key of the top-level object. A `"model"`
    inside a message, or the plain string `"model"` used as some other key's
    value, must not be mistaken for the routing key.

Pure: no I/O, so every rule above is testable on a byte string.
"""
from __future__ import annotations

import json

# 8 KB. Big enough for any real request's leading fields, small enough that two
# concurrent slots cost 16 KB of buffer -- the pass-through cost of this gateway
# was measured at 1.7 MB of RSS growth across a 100k-token request and that
# figure is worth keeping.
PEEK_BYTES = 8192

_WS = b" \t\r\n"
_END = b",}] \t\r\n"


def _skip_ws(b: bytes, i: int) -> int:
    while i < len(b) and b[i:i + 1] in _WS:
        i += 1
    return i


def _scan_string(b: bytes, i: int) -> tuple[bytes | None, int]:
    """Scan the JSON string starting at the quote at `i`.

    Returns (raw contents without the quotes, index after the closing quote), or
    (None, i) when the buffer ends first -- which is inconclusive, not invalid.
    """
    n = len(b)
    j = i + 1
    while j < n:
        c = b[j:j + 1]
        if c == b"\\":
            j += 2
            continue
        if c == b'"':
            return b[i + 1:j], j + 1
        j += 1
    return None, i


def _decode(raw: bytes) -> str | None:
    """A JSON string body back into text, escapes and all. None if it will not
    decode -- an unusable id is reported as "unknown", never guessed at."""
    try:
        out = json.loads(b'"' + raw + b'"')
    except Exception:
        return None
    return out if isinstance(out, str) and out else None


def peek_model(prefix: bytes) -> tuple[str | None, bool]:
    """(model id, conclusive) from a prefix of a JSON request body.

    conclusive says whether the answer is final. (None, True) means the body was
    read far enough to know it names no top-level `model` -- a body that is not
    a JSON object at all, or one whose object closed without the key.
    (None, False) means the prefix ran out first.
    """
    n = len(prefix)
    i = _skip_ws(prefix, 0)
    if i >= n:
        return None, False
    if prefix[i:i + 1] != b"{":
        # Not a JSON object: form data, multipart audio, a bare array. There is
        # no top-level model field to find, and that is a final answer.
        return None, True
    i += 1
    depth = 1
    key: bytes | None = None      # the depth-1 key whose value comes next
    while True:
        i = _skip_ws(prefix, i)
        if i >= n:
            return None, False
        c = prefix[i:i + 1]

        if c == b'"':
            raw, j = _scan_string(prefix, i)
            if raw is None:
                return None, False
            # KEY POSITION is what makes this safe: a string is a key only at
            # depth 1 with no key already pending, and only if a colon follows.
            if depth == 1 and key is None:
                k = _skip_ws(prefix, j)
                if k >= n:
                    return None, False
                if prefix[k:k + 1] != b":":
                    return None, True          # malformed; stop guessing
                key = raw
                i = k + 1
                continue
            if depth == 1 and key == b"model":
                return _decode(raw), True
            if depth == 1:
                key = None
            i = j
            continue

        if c in b"{[":
            depth += 1
            i += 1
            continue

        if c in b"}]":
            depth -= 1
            i += 1
            if depth == 0:
                return None, True              # the object closed: no model
            if depth == 1:
                key = None                     # a nested value finished
            continue

        if c == b",":
            if depth == 1:
                key = None
            i += 1
            continue

        if c == b":":
            i += 1
            continue

        # A bare scalar: number, true, false, null.
        j = i
        while j < n and prefix[j:j + 1] not in _END:
            j += 1
        if j >= n:
            # It may continue past the buffer, so nothing after it can be read.
            return None, False
        if depth == 1:
            key = None
        i = j
