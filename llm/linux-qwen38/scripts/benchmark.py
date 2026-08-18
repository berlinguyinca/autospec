#!/usr/bin/env python3
"""Benchmark the running Qwen3.8-27B vLLM node.

    benchmark.py --concurrency 1,2,4,8 [--out DIR] [--prompt-tokens N]

Measures, per concurrency level: time to first token, per-request generation
rate, and aggregate generation rate across the batch. Writes the raw JSON and a
Markdown summary under the results directory.

Aggregate throughput is measured over the wall-clock window in which the batch
was actually in flight, not as a sum of per-request rates -- summing per-request
rates double-counts the overlap and reports numbers the server never produced.
"""
from __future__ import annotations

import argparse
import json
import os
import re
import statistics
import sys
import threading
import time
import urllib.error
import urllib.request
from datetime import datetime, timezone
from pathlib import Path


def load_conf(path: str) -> dict[str, str]:
    """Read the shell conf without sourcing it.

    Values may reference earlier keys (`QWEN38_RESULTS="${QWEN38_STATE}/results"`),
    so ${...} references are expanded against what has been parsed so far --
    otherwise results get written to a directory literally named "${QWEN38_STATE}".
    """
    conf: dict[str, str] = {}
    for line in Path(path).read_text().splitlines():
        line = line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, _, val = line.partition("=")
        if not key.startswith("QWEN38_"):
            continue
        val = val.strip().strip('"')
        conf[key] = re.sub(r"\$\{(\w+)\}", lambda m: conf.get(m.group(1), m.group(0)), val)
    return conf


def post(base: str, payload: dict, api_key: str, timeout: float) -> tuple[dict, float, float]:
    """POST a streaming chat completion. Returns (usage, ttft_s, total_s)."""
    body = json.dumps({**payload, "stream": True,
                       "stream_options": {"include_usage": True}}).encode()
    req = urllib.request.Request(
        f"{base}/v1/chat/completions", data=body,
        headers={"Content-Type": "application/json",
                 **({"Authorization": f"Bearer {api_key}"} if api_key else {})})
    started = time.perf_counter()
    ttft = None
    usage: dict = {}
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        for raw in resp:
            if not raw.startswith(b"data: "):
                continue
            chunk = raw[6:].strip()
            if chunk == b"[DONE]":
                break
            obj = json.loads(chunk)
            if obj.get("usage"):
                usage = obj["usage"]
            choices = obj.get("choices") or []
            if ttft is None and choices and choices[0].get("delta", {}).get("content"):
                ttft = time.perf_counter() - started
    return usage, (ttft if ttft is not None else float("nan")), time.perf_counter() - started


def run_level(base: str, model: str, api_key: str, n: int,
              prompt: str, max_tokens: int, timeout: float) -> dict:
    """Fire n concurrent requests and measure the batch."""
    results: list[dict] = []
    lock = threading.Lock()
    gate = threading.Barrier(n)

    def worker(idx: int) -> None:
        payload = {"model": model, "temperature": 0, "max_tokens": max_tokens,
                   "chat_template_kwargs": {"enable_thinking": False},
                   "messages": [{"role": "user", "content": prompt}]}
        gate.wait()  # release all requests together, so the batch is a real batch
        t0 = time.perf_counter()
        try:
            usage, ttft, total = post(base, payload, api_key, timeout)
            row = {"i": idx, "ok": True, "ttft_s": ttft, "total_s": total,
                   "start": t0, "end": time.perf_counter(),
                   "prompt_tokens": usage.get("prompt_tokens"),
                   "completion_tokens": usage.get("completion_tokens")}
        except (urllib.error.URLError, OSError, ValueError) as exc:
            row = {"i": idx, "ok": False, "error": f"{type(exc).__name__}: {exc}",
                   "start": t0, "end": time.perf_counter()}
        with lock:
            results.append(row)

    threads = [threading.Thread(target=worker, args=(i,)) for i in range(n)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()

    good = [r for r in results if r.get("ok") and r.get("completion_tokens")]
    if not good:
        return {"concurrency": n, "ok": 0, "failed": len(results),
                "errors": [r.get("error") for r in results][:3]}

    gen_tokens = sum(r["completion_tokens"] for r in good)
    window = max(r["end"] for r in good) - min(r["start"] for r in good)
    per_req = [r["completion_tokens"] / r["total_s"] for r in good if r["total_s"] > 0]
    ttfts = [r["ttft_s"] for r in good if r["ttft_s"] == r["ttft_s"]]  # drop NaN

    return {
        "concurrency": n,
        "ok": len(good),
        "failed": len(results) - len(good),
        "prompt_tokens": good[0].get("prompt_tokens"),
        "generated_tokens_total": gen_tokens,
        "window_s": round(window, 3),
        "aggregate_gen_tok_s": round(gen_tokens / window, 2) if window > 0 else None,
        "per_request_gen_tok_s_median": round(statistics.median(per_req), 2) if per_req else None,
        "ttft_s_median": round(statistics.median(ttfts), 3) if ttfts else None,
        "ttft_s_p95": round(sorted(ttfts)[int(len(ttfts) * 0.95) - 1], 3) if len(ttfts) >= 2 else None,
    }


def gpu_snapshot() -> dict:
    import subprocess
    try:
        out = subprocess.run(
            ["nvidia-smi", "--query-gpu=memory.used,memory.total,utilization.gpu,power.draw",
             "--format=csv,noheader,nounits"],
            capture_output=True, text=True, timeout=10, check=True).stdout.strip().split(", ")
        return {"vram_used_mib": int(out[0]), "vram_total_mib": int(out[1]),
                "gpu_util_pct": int(out[2]), "power_w": float(out[3])}
    except Exception as exc:  # nvidia-smi absent or output shape changed
        return {"error": str(exc)}


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--conf", default=os.environ.get("QWEN38_CONF_DIR", "/opt/qwen-vllm/etc") + "/common.conf")
    ap.add_argument("--concurrency", default="1,2,4,8")
    ap.add_argument("--max-tokens", type=int, default=256)
    ap.add_argument("--prompt-tokens", type=int, default=512,
                    help="approximate prompt size, padded with filler records")
    ap.add_argument("--timeout", type=float, default=900.0)
    ap.add_argument("--out", default=None)
    ap.add_argument("--label", default="")
    args = ap.parse_args()

    conf = load_conf(args.conf)
    base = f"http://{conf['QWEN38_HOST']}:{conf['QWEN38_PORT']}"
    model = conf["QWEN38_SERVED_NAME"]
    api_key = conf.get("QWEN38_API_KEY", "")

    # ~7 tokens per filler line; deterministic so runs are comparable.
    filler = "\n".join(f"Record {i:04d}: ordinary archival entry, no code."
                       for i in range(max(1, args.prompt_tokens // 12)))
    prompt = (f"{filler}\n\nSummarise the structure of the records above in one "
              "short paragraph, then stop.")

    levels = [int(x) for x in args.concurrency.split(",") if x.strip()]
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    out_dir = Path(args.out or f"{conf['QWEN38_RESULTS']}/{stamp}")
    out_dir.mkdir(parents=True, exist_ok=True)

    try:
        with urllib.request.urlopen(f"{base}/v1/models", timeout=10) as r:
            served = json.load(r)
    except OSError as exc:
        print(f"no server on {base}: {exc}", file=sys.stderr)
        return 1

    print(f"benchmarking {model} on {base}")
    print(f"prompt ~{args.prompt_tokens} tokens, max_tokens={args.max_tokens}\n")

    rows = []
    for n in levels:
        print(f"  concurrency {n:>3} ... ", end="", flush=True)
        row = run_level(base, model, api_key, n, prompt, args.max_tokens, args.timeout)
        row["gpu"] = gpu_snapshot()
        rows.append(row)
        if row.get("aggregate_gen_tok_s"):
            print(f"agg {row['aggregate_gen_tok_s']:>7.2f} tok/s   "
                  f"per-req {row['per_request_gen_tok_s_median']:>6.2f} tok/s   "
                  f"ttft {row['ttft_s_median']}s")
        else:
            print(f"FAILED ({row.get('failed')} requests): {row.get('errors')}")

    payload = {"timestamp_utc": stamp, "label": args.label, "endpoint": base,
               "served_models": served, "config": conf, "levels": rows}
    (out_dir / "benchmark.json").write_text(json.dumps(payload, indent=2))

    md = [f"# Benchmark {stamp}", ""]
    if args.label:
        md += [f"**{args.label}**", ""]
    md += [f"- endpoint: `{base}`", f"- model: `{model}`",
           f"- prompt: ~{args.prompt_tokens} tokens, max_tokens {args.max_tokens}", "",
           "| concurrency | ok | aggregate tok/s | per-request tok/s (median) "
           "| TTFT median (s) | TTFT p95 (s) | VRAM (MiB) |",
           "|---:|---:|---:|---:|---:|---:|---:|"]
    for r in rows:
        md.append(f"| {r['concurrency']} | {r.get('ok', 0)}/{r.get('ok', 0) + r.get('failed', 0)} "
                  f"| {r.get('aggregate_gen_tok_s') or '—'} "
                  f"| {r.get('per_request_gen_tok_s_median') or '—'} "
                  f"| {r.get('ttft_s_median') or '—'} | {r.get('ttft_s_p95') or '—'} "
                  f"| {r.get('gpu', {}).get('vram_used_mib', '—')} |")
    (out_dir / "benchmark.md").write_text("\n".join(md) + "\n")

    print(f"\nwrote {out_dir}/benchmark.json and benchmark.md")
    return 0 if any(r.get("ok") for r in rows) else 1


if __name__ == "__main__":
    sys.exit(main())
