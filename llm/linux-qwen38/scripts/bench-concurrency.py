#!/usr/bin/env python3
"""Measure what happens when several clients hit one server at once.

    bench-concurrency.py --base URL --model ID --concurrency 1,2,4,8

Fires N chat completions simultaneously and reports, per level:

    per-stream tok/s   what ONE session feels
    aggregate tok/s    what the machine delivers in total
    TTFT               how long a session waits for its first token

The two numbers answer different questions and both matter. A server with one
slot does not reject the second request -- it queues it, so aggregate stays flat
while TTFT grows without bound. That failure is invisible if you only measure a
single stream, which is why this script exists alongside benchmark.py.

Every stream gets a DIFFERENT prompt. Identical prompts would share the prefix
cache and report a throughput the real workload never sees.
"""
from __future__ import annotations

import argparse
import json
import statistics
import sys
import threading
import time
import urllib.error
import urllib.request

_REC = "Record {i:05d} of set {s:02d}: ordinary archival entry, no code here."


def build_prompt(stream: int, records: int) -> str:
    """A prompt of `records` filler lines, unique to this stream."""
    body = "\n".join(_REC.format(i=i, s=stream) for i in range(records))
    return (body + f"\n\nSummarise set {stream:02d} in detail, then describe "
            "how you would index these records for retrieval.")


def calibrate(base: str, api_key: str, target_tokens: int) -> int:
    """Records needed for a prompt of target_tokens, per the server's tokenizer.

    Assuming a tokens-per-record constant is how this test lied to me: a filler
    line I had eyeballed at 17 tokens was 21, so a "4 x 40k" run really sent
    4 x 51,800 and blew a pool that the configuration fits comfortably. The
    server owns the tokenizer, so ask it.
    """
    probe = 64
    body = json.dumps({"content": build_prompt(0, probe)}).encode()
    headers = {"Content-Type": "application/json"}
    if api_key:
        headers["Authorization"] = f"Bearer {api_key}"
    req = urllib.request.Request(f"{base}/tokenize", data=body, headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=120) as resp:
            n_tok = len(json.load(resp)["tokens"])
    except (urllib.error.HTTPError, urllib.error.URLError, KeyError, OSError):
        return max(1, target_tokens // 21)   # measured fallback, not a guess
    per_record = n_tok / probe
    return max(1, int(target_tokens / per_record))


class Result:
    __slots__ = ("stream", "t_start", "t_first", "t_end", "tokens", "error")

    def __init__(self, stream: int) -> None:
        self.stream = stream
        self.t_start = self.t_first = self.t_end = 0.0
        self.tokens = 0
        self.error = ""

    @property
    def ttft(self) -> float:
        return self.t_first - self.t_start

    @property
    def decode_tps(self) -> float:
        span = self.t_end - self.t_first
        return self.tokens / span if span > 0 and self.tokens else 0.0


def one_stream(base: str, model: str, api_key: str, stream: int,
               records: int, max_tokens: int, out: Result) -> None:
    body = json.dumps({
        "model": model, "temperature": 0, "max_tokens": max_tokens,
        "stream": True,
        # llama.cpp extension. Without it a stream ends whenever the model is
        # done talking, and short answers make the decode rate meaningless --
        # a 3-token reply reports a fine per-stream rate and a nonsense
        # aggregate. Every stream must decode exactly max_tokens.
        "ignore_eos": True,
        "chat_template_kwargs": {"enable_thinking": False},
        "messages": [{"role": "user",
                      "content": build_prompt(stream, records)}],
    }).encode()
    headers = {"Content-Type": "application/json"}
    if api_key:
        headers["Authorization"] = f"Bearer {api_key}"
    req = urllib.request.Request(f"{base}/v1/chat/completions",
                                 data=body, headers=headers)

    out.t_start = time.perf_counter()
    try:
        with urllib.request.urlopen(req, timeout=1800) as resp:
            for raw in resp:
                line = raw.decode("utf-8", "replace").strip()
                if not line.startswith("data:"):
                    continue
                payload = line[5:].strip()
                if payload == "[DONE]":
                    break
                try:
                    chunk = json.loads(payload)
                except json.JSONDecodeError:
                    continue
                choices = chunk.get("choices") or []
                if not choices:
                    continue
                delta = choices[0].get("delta") or {}
                # A reasoning model emits reasoning_content deltas too; both are
                # decoded tokens, so both count toward throughput.
                if not (delta.get("content") or delta.get("reasoning_content")):
                    continue
                if out.tokens == 0:
                    out.t_first = time.perf_counter()
                out.tokens += 1
    except (urllib.error.HTTPError, urllib.error.URLError, OSError) as exc:
        out.error = str(exc)
    out.t_end = time.perf_counter()
    if out.t_first == 0.0:
        out.t_first = out.t_end


def run_level(base: str, model: str, api_key: str, n: int,
              records: int, max_tokens: int) -> dict:
    results = [Result(i) for i in range(n)]
    threads = [threading.Thread(target=one_stream,
                                args=(base, model, api_key, i, records,
                                      max_tokens, results[i]))
               for i in range(n)]
    wall0 = time.perf_counter()
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    wall = time.perf_counter() - wall0

    failed = [r for r in results if r.error]
    ok = [r for r in results if not r.error and r.tokens]
    if not ok:
        return {"concurrency": n, "failed": len(failed),
                "error": failed[0].error if failed else "no tokens"}

    total_tokens = sum(r.tokens for r in ok)
    return {
        "concurrency": n,
        "failed": len(failed),
        "streams_ok": len(ok),
        "wall_s": wall,
        "per_stream_tps": statistics.median(r.decode_tps for r in ok),
        "per_stream_tps_min": min(r.decode_tps for r in ok),
        "aggregate_tps": total_tokens / wall,
        "ttft_median_s": statistics.median(r.ttft for r in ok),
        "ttft_max_s": max(r.ttft for r in ok),
        "tokens": total_tokens,
    }


def run_mix(args) -> int:
    """One round of heterogeneous sessions, each with its own id and size."""
    spec = []
    for item in args.mix.split(","):
        model_id, _, tokens = item.strip().partition(":")
        spec.append((model_id, int(tokens)))

    results = [Result(i) for i in range(len(spec))]
    threads = []
    for i, (model_id, tokens) in enumerate(spec):
        records = calibrate(args.base, args.api_key, tokens)
        threads.append(threading.Thread(
            target=one_stream,
            args=(args.base, model_id, args.api_key, i, records,
                  args.max_tokens, results[i])))
    wall0 = time.perf_counter()
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    wall = time.perf_counter() - wall0

    print(f"\n{'session':<28} {'asked':>9} {'TTFT':>9} {'tok/s':>8}  status")
    print("-" * 72)
    failures = 0
    for (model_id, tokens), r in zip(spec, results):
        status = "ok" if not r.error and r.tokens else (r.error or "no output")
        if status != "ok":
            failures += 1
        print(f"{model_id:<28} {tokens:>9,} {r.ttft:>8.1f}s "
              f"{r.decode_tps:>7.2f}  {status}")
    print(f"\ntotal requested: {sum(t for _, t in spec):,} tokens "
          f"in {wall:.1f}s wall\n")
    return 1 if failures else 0


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--base", default="http://127.0.0.1:8080")
    ap.add_argument("--model", default="qwen3.8-27b")
    ap.add_argument("--concurrency", default="1,2,4,8",
                    help="comma-separated levels to sweep")
    ap.add_argument("--prompt-tokens", type=int, default=4000,
                    help="approximate prompt size per stream")
    ap.add_argument("--max-tokens", type=int, default=192)
    ap.add_argument("--mix",
                    help="run ONE round of differently-sized sessions instead "
                         "of a sweep, as id:tokens,id:tokens,... — this is the "
                         "case a shared KV pool exists for, and the case that "
                         "over-subscribes it")
    ap.add_argument("--api-key", default="")
    ap.add_argument("--json", action="store_true")
    args = ap.parse_args()

    if args.mix:
        return run_mix(args)

    levels = [int(x) for x in args.concurrency.split(",") if x.strip()]
    records = calibrate(args.base, args.api_key, args.prompt_tokens)
    if not args.json:
        print(f"  {records} filler records ~= {args.prompt_tokens} tokens "
              f"(server tokenizer)", file=sys.stderr)
    rows = []
    for n in levels:
        if not args.json:
            print(f"  running concurrency={n} ...", file=sys.stderr, flush=True)
        rows.append(run_level(args.base, args.model, args.api_key, n,
                              records, args.max_tokens))

    if args.json:
        print(json.dumps({"model": args.model,
                          "prompt_tokens": args.prompt_tokens,
                          "levels": rows}, indent=2))
        return 0

    print(f"\nmodel={args.model} prompt~{args.prompt_tokens} tok "
          f"max_tokens={args.max_tokens}\n")
    print(f"{'N':>3}  {'per-stream t/s':>14}  {'aggregate t/s':>13}  "
          f"{'TTFT med':>9}  {'TTFT max':>9}  {'wall':>7}")
    print("-" * 68)
    base_agg = None
    for r in rows:
        if "error" in r:
            print(f"{r['concurrency']:>3}  FAILED: {r['error']}")
            continue
        if base_agg is None:
            base_agg = r["aggregate_tps"]
        print(f"{r['concurrency']:>3}  {r['per_stream_tps']:>14.2f}  "
              f"{r['aggregate_tps']:>13.2f}  {r['ttft_median_s']:>8.2f}s  "
              f"{r['ttft_max_s']:>8.2f}s  {r['wall_s']:>6.1f}s"
              + (f"   {r['aggregate_tps'] / base_agg:.2f}x" if base_agg else ""))
    print()
    return 0


if __name__ == "__main__":
    sys.exit(main())
