#!/usr/bin/env python3
"""Extract EXACT token counts from a model response. No I/O, no network.

Measured on this node's llama.cpp build (see the design's section 1.1): the
terminal streaming chunk carries a `timings` block UNCONDITIONALLY, with
`prompt_n` and `predicted_n` as exact token counts. When the client asks for it
via `stream_options.include_usage`, a conventional `usage` object appears
alongside. Non-streaming responses carry both.

That is what lets this be a pass-through: the proxy reads the last chunk and
never has to rewrite the client's request body to inject an option. If a future
build stops emitting `timings` unprompted, THIS is the module that notices --
counts become None and rows are marked truncated, rather than silently zero.
"""
from __future__ import annotations

import json
from dataclasses import dataclass

# Only the tail is retained. A 100k-token exchange must not be accumulated in
# memory merely to read its final chunk. Generous enough to hold one terminal
# chunk plus a partial predecessor.
MAX_TAIL_BYTES = 65536

_DATA = b"data: "


@dataclass
class Usage:
    prompt_tokens: int | None = None
    completion_tokens: int | None = None
    cached_tokens: int | None = None
    prompt_ms: float | None = None
    predicted_ms: float | None = None
    model: str | None = None
    truncated: bool = False

    def as_row(self) -> dict:
        return {
            "prompt_tokens": self.prompt_tokens,
            "completion_tokens": self.completion_tokens,
            "cached_tokens": self.cached_tokens,
            "prompt_ms": self.prompt_ms,
            "predicted_ms": self.predicted_ms,
            "model": self.model,
            "truncated": self.truncated,
        }


def _from_obj(obj: dict) -> Usage | None:
    """One JSON object -> Usage, or None if it carries no accounting at all."""
    if not isinstance(obj, dict):
        return None
    u = obj.get("usage") if isinstance(obj.get("usage"), dict) else None
    t = obj.get("timings") if isinstance(obj.get("timings"), dict) else None
    if not u and not t:
        return None

    out = Usage()
    # The model is echoed on the response, so usage attribution never has to
    # parse -- or buffer -- the client's REQUEST body. A 100k prompt is ~400 KB;
    # reading it to learn one field would undo the whole pass-through design.
    if isinstance(obj.get("model"), str):
        out.model = obj["model"]
    if t:
        # Always present on this build; the fallback that makes pass-through work.
        out.prompt_tokens = t.get("prompt_n")
        out.completion_tokens = t.get("predicted_n")
        out.prompt_ms = t.get("prompt_ms")
        out.predicted_ms = t.get("predicted_ms")
        cache_n = t.get("cache_n")
        if isinstance(cache_n, int):
            out.cached_tokens = cache_n
    if u:
        # Preferred when present: it is the interoperable field, and it is the
        # one that carries cached_tokens explicitly.
        if isinstance(u.get("prompt_tokens"), int):
            out.prompt_tokens = u["prompt_tokens"]
        if isinstance(u.get("completion_tokens"), int):
            out.completion_tokens = u["completion_tokens"]
        det = u.get("prompt_tokens_details")
        if isinstance(det, dict) and isinstance(det.get("cached_tokens"), int):
            out.cached_tokens = det["cached_tokens"]
    return out


def from_json_body(obj: dict) -> Usage:
    """A complete non-streaming response body."""
    got = _from_obj(obj)
    if got is None:
        # Unknown, NOT zero. A zero here would silently under-account.
        return Usage(truncated=True)
    return got


class StreamAccountant:
    """Feed response bytes as they stream past; read the counts at the end.

    Keeps only a bounded tail, and parses lazily on result() rather than on every
    chunk -- a per-chunk JSON parse would put a decode on the hot path of every
    token.
    """

    def __init__(self) -> None:
        self._tail = bytearray()

    def feed(self, chunk: bytes) -> None:
        if not chunk:
            return
        self._tail.extend(chunk)
        if len(self._tail) > MAX_TAIL_BYTES:
            del self._tail[:len(self._tail) - MAX_TAIL_BYTES]

    def buffered_bytes(self) -> int:
        return len(self._tail)

    def result(self) -> Usage:
        """Scan the retained tail backwards for the last chunk carrying counts.

        Backwards because the accounting block is on the LAST data chunk, and
        because a truncated stream must fall through to `truncated=True` rather
        than pick up a mid-stream object that has none.
        """
        model = None
        for line in reversed(bytes(self._tail).split(b"\n")):
            line = line.strip()
            if not line.startswith(_DATA):
                continue
            payload = line[len(_DATA):].strip()
            if payload == b"[DONE]":
                continue
            try:
                obj = json.loads(payload)
            except (ValueError, UnicodeDecodeError):
                # A partial first line is expected once the tail has been
                # trimmed; it is not an error.
                continue
            got = _from_obj(obj)
            if got is not None:
                return got
            # No accounting on this chunk, but mid-stream chunks still name the
            # model. Keep it: a truncated request has unknown COUNTS, and that is
            # no reason to also lose which model served it.
            if model is None and isinstance(obj.get("model"), str):
                model = obj["model"]

        # No SSE framing found. A NON-STREAMING response is a plain JSON body,
        # and the caller cannot know in advance which it will get -- so try the
        # whole retained tail as one object before giving up. Without this every
        # non-streaming request was recorded as truncated with no counts.
        #
        # If the body exceeded the retained tail the parse fails and the row is
        # marked truncated, which is the honest answer: the counts really are not
        # available from what was kept.
        try:
            obj = json.loads(bytes(self._tail))
        except (ValueError, UnicodeDecodeError):
            return Usage(truncated=True, model=model)
        got = _from_obj(obj)
        if got is not None:
            return got
        if model is None and isinstance(obj, dict) and isinstance(obj.get("model"), str):
            model = obj["model"]
        return Usage(truncated=True, model=model)
