#!/usr/bin/env python3
"""Send a needle-in-haystack prompt that fills most of a served context.

    long-prompt-probe.py <base-url> <model> <ctx> [api-key]

Prints one of:
    CEILING_OK <prompt_tokens>       needle retrieved at that length
    WRONG_ANSWER <prompt_tokens>     served, but did not find the needle
    REQUEST_FAILED <reason>          rejected, or the engine died

Used by measure-ceiling.sh. It lives in its own file rather than inline because
a short "reply with X" request proves only that the server is up: a context can
start, serve a 6-token request, and still OOM on a prompt that actually fills
it. Verifying at length is the entire point, so it gets a real script.
"""
from __future__ import annotations

import json
import sys
import urllib.error
import urllib.request


def main() -> int:
    base, model, ctx = sys.argv[1], sys.argv[2], int(sys.argv[3])
    api_key = sys.argv[4] if len(sys.argv) > 4 else ""

    # These records tokenise to ~15.5 tokens each; dividing by 17 keeps the
    # prompt safely under the limit. Overshooting only earns a 400 and wastes a
    # whole model load, which is minutes.
    n = max(1, int(ctx * 0.85) // 17)
    recs = [f"Record {i:05d}: ordinary archival entry with no authorization code."
            for i in range(n)]
    at = n // 2
    recs[at] = (f"Record {at:05d}: authorization code COBALT-719 applies to the "
                "lunar inventory.")
    prompt = ("\n".join(recs) +
              f"\n\nWhat authorization code appears in record {at:05d}? "
              "Respond with only the code.")

    body = json.dumps({
        "model": model, "temperature": 0, "max_tokens": 32,
        "chat_template_kwargs": {"enable_thinking": False},
        "messages": [{"role": "user", "content": prompt}],
    }).encode()
    headers = {"Content-Type": "application/json"}
    if api_key:
        headers["Authorization"] = f"Bearer {api_key}"

    req = urllib.request.Request(f"{base}/v1/chat/completions",
                                 data=body, headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=1800) as resp:
            data = json.load(resp)
    except (urllib.error.HTTPError, urllib.error.URLError, OSError) as exc:
        print(f"REQUEST_FAILED {exc}")
        return 0

    used = data["usage"]["prompt_tokens"]
    text = data["choices"][0]["message"]["content"]
    print(f"CEILING_OK {used}" if "COBALT-719" in text else f"WRONG_ANSWER {used}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
