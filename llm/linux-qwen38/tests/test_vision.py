#!/usr/bin/env python3
"""Prove the vision profile actually processes an image.

    test_vision.py <base-url> <model>

Generates a deterministic 256x256 PNG with the standard library only -- no
downloaded asset, so the test cannot silently start passing because some URL
changed -- and asks the model what it depicts.

Also checks the image token accounting: with --image-min-tokens 1024 the prompt
must grow by roughly that much. A server that quietly ignored --mmproj would
still answer the question plausibly from the text alone, so the token count is
what distinguishes "looked at the image" from "guessed".
"""
from __future__ import annotations

import base64
import json
import struct
import sys
import time
import urllib.error
import urllib.request
import zlib


def chessboard_png(size: int = 256, squares: int = 8) -> bytes:
    """An 8x8 light/dark chessboard as a PNG, built by hand."""
    cell = size // squares
    raw = bytearray()
    for y in range(size):
        raw.append(0)  # filter type 0 for each scanline
        for x in range(size):
            dark = ((x // cell) + (y // cell)) % 2 == 1
            v = 40 if dark else 225
            raw += bytes((v, v, v))

    def chunk(tag: bytes, data: bytes) -> bytes:
        return (struct.pack(">I", len(data)) + tag + data
                + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF))

    return (b"\x89PNG\r\n\x1a\n"
            + chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(bytes(raw), 9))
            + chunk(b"IEND", b""))


def ask(base: str, model: str, content: list, max_tokens: int = 64) -> dict:
    body = json.dumps({
        "model": model, "temperature": 0, "max_tokens": max_tokens,
        "chat_template_kwargs": {"enable_thinking": False},
        "messages": [{"role": "user", "content": content}],
    }).encode()
    req = urllib.request.Request(f"{base}/v1/chat/completions", data=body,
                                 headers={"Content-Type": "application/json"})
    t0 = time.perf_counter()
    with urllib.request.urlopen(req, timeout=600) as resp:
        data = json.load(resp)
    data["_elapsed"] = time.perf_counter() - t0
    return data


def main() -> int:
    base = sys.argv[1] if len(sys.argv) > 1 else "http://127.0.0.1:8082"
    model = sys.argv[2] if len(sys.argv) > 2 else "qwen3.8-27b"

    png = chessboard_png()
    url = "data:image/png;base64," + base64.b64encode(png).decode()
    print(f"image: {len(png)} bytes, 256x256, 8x8 chessboard")

    passed = failed = 0

    def check(ok: bool, label: str, detail: str = "") -> None:
        nonlocal passed, failed
        print(f"  {'PASS' if ok else 'FAIL'}  {label}{'  — ' + detail if detail else ''}")
        if ok:
            passed += 1
        else:
            failed += 1

    # Baseline: the same question with no image at all. Its prompt size is the
    # reference the image run is compared against.
    text_only = ask(base, model,
                    [{"type": "text",
                      "text": "What board game uses the board pattern shown? "
                              "Respond with exactly one lowercase word."}])
    base_tokens = text_only["usage"]["prompt_tokens"]

    got = ask(base, model, [
        {"type": "text",
         "text": "What board game uses the board pattern shown? "
                 "Respond with exactly one lowercase word."},
        {"type": "image_url", "image_url": {"url": url}},
    ])
    answer = got["choices"][0]["message"]["content"].strip().lower()
    img_tokens = got["usage"]["prompt_tokens"]
    delta = img_tokens - base_tokens

    print(f"prompt tokens: {base_tokens} text-only -> {img_tokens} with image "
          f"(+{delta})")
    print(f"answer       : {answer!r}   ({got['_elapsed']:.1f}s)")
    print()

    check(delta >= 900, "image consumed ~1024+ prompt tokens",
          f"+{delta}; below this the projector is not being used")
    check("chess" in answer, "model identified the board", repr(answer))

    print(f"== vision: {passed} passed, {failed} failed ==")
    return 0 if failed == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
