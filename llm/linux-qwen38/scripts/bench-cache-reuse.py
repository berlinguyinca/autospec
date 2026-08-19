#!/usr/bin/env python3
"""Measure what --cache-reuse actually buys, on two different shapes of prompt.

    bench-cache-reuse.py [--base URL] [--model ID] [--preamble-tokens N]

`--cache-reuse` is NOT ordinary prefix caching. An exact prefix is reused
without it. What it adds is reuse across a DIVERGENCE: when a prompt differs
somewhere early and then re-converges, it shifts the surviving KV chunks
instead of recomputing them. So whether it does anything at all depends on the
shape of the prompt, and a benchmark that only sends identical prefixes will
measure nothing and report a confident zero.

Two shapes, therefore:

  shared    one long preamble, different question each time. This is the case
            ordinary caching already handles; --cache-reuse should change
            little, and a large gain here would mean the baseline was broken.

  diverged  the same preamble with a small varying field NEAR THE START -- a
            session id, a timestamp, a rotating memory block -- then a long
            identical remainder. Without reuse everything after the varying
            field is recomputed; with it, those chunks can be shifted.

Reported per request: `cache_n` (tokens taken from cache), `prompt_n` (tokens
actually evaluated) and `prompt_ms`. The second request of each pair is the
interesting one; the first only populates the slot.
"""
from __future__ import annotations

import argparse
import json
import sys
import urllib.request

FILLER = ("The implementer must not edit files outside the issue's declared "
          "surface, and every change needs a test that fails without it. ")


def post(base: str, path: str, body: dict, timeout: float = 600.0) -> dict:
    req = urllib.request.Request(
        base.rstrip("/") + path, method="POST",
        data=json.dumps(body).encode(),
        headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=timeout) as fh:
        return json.load(fh)


def run(base: str, prompt: str) -> dict:
    out = post(base, "/completion",
               {"prompt": prompt, "n_predict": 1, "cache_prompt": True,
                "temperature": 0})
    t = out.get("timings", {})
    return {"cache_n": t.get("cache_n", 0), "prompt_n": t.get("prompt_n", 0),
            "prompt_ms": round(t.get("prompt_ms", 0.0), 1)}


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--base", default="http://127.0.0.1:8080")
    ap.add_argument("--preamble-tokens", type=int, default=6000,
                    help="approximate size of the shared body, in tokens")
    ap.add_argument("--json", action="store_true")
    args = ap.parse_args()

    # ~4 tokens per repetition of FILLER; close enough, and the exact count is
    # reported back by the server anyway.
    body = FILLER * max(1, args.preamble_tokens // 4)

    shapes = {
        # Same preamble both times; only the trailing question differs.
        "shared": [f"SYSTEM:\n{body}\nUSER: What is 2+2?\n",
                   f"SYSTEM:\n{body}\nUSER: Name one colour.\n"],
        # A varying field near the START, then the same long remainder. This is
        # the shape --cache-reuse exists for.
        "diverged": [f"SESSION 1111-aaaa\nSYSTEM:\n{body}\nUSER: What is 2+2?\n",
                     f"SESSION 2222-bbbb\nSYSTEM:\n{body}\nUSER: What is 2+2?\n"],
    }

    results = {}
    for name, (first, second) in shapes.items():
        run(args.base, first)                  # populate the slot
        results[name] = run(args.base, second)  # the measurement

    if args.json:
        print(json.dumps(results, indent=2, sort_keys=True))
        return 0

    print(f"{'shape':<10} {'cached':>8} {'evaluated':>10} {'prefill ms':>11}")
    for name, r in results.items():
        print(f"{name:<10} {r['cache_n']:>8} {r['prompt_n']:>10} "
              f"{r['prompt_ms']:>11}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
