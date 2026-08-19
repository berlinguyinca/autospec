#!/usr/bin/env python3
"""Benchmark an ExLlamaV3 (exl3) checkpoint for comparison against the vLLM node.

    bench-exl3.py --model <dir> [--ctx N] [--gen N] [--prompt-tokens N]

Reports the same two things the vLLM benchmark does -- single-stream generation
rate and a long-prompt needle retrieval -- so the numbers are comparable.

This is a COMPARISON harness, not a deployment. exl3 needs ExLlamaV3, not vLLM,
so adopting it would mean a second runtime; the point here is to find out
whether that is worth doing before anyone builds it.
"""
from __future__ import annotations

import argparse
import sys
import time
from pathlib import Path


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--model", required=True)
    ap.add_argument("--ctx", type=int, default=32768)
    ap.add_argument("--gen", type=int, default=256)
    ap.add_argument("--prompt-tokens", type=int, default=512)
    ap.add_argument("--needle", action="store_true",
                    help="run a needle-in-haystack retrieval at ~85%% of --ctx")
    ap.add_argument("--kv-bits", type=int, default=0,
                    help="quantised K/V cache at N bits (e.g. 4 or 8); 0 = fp16. "
                         "fp16 KV costs ~64 KiB/token here, so six-figure "
                         "contexts need this.")
    args = ap.parse_args()

    from exllamav3 import (Config, Model, Cache, Tokenizer, Generator, Job,
                           CacheLayer_quant, CacheLayer_fp16)
    import torch
    from functools import partial

    model_dir = Path(args.model)
    if not model_dir.is_dir():
        print(f"no such model dir: {model_dir}", file=sys.stderr)
        return 1

    t0 = time.perf_counter()
    config = Config.from_directory(str(model_dir))
    model = Model.from_config(config)
    tokenizer = Tokenizer.from_config(config)
    if args.kv_bits:
        layer_type = partial(CacheLayer_quant,
                             k_bits=args.kv_bits, v_bits=args.kv_bits)
    else:
        layer_type = CacheLayer_fp16
    cache = Cache(model, max_num_tokens=args.ctx, layer_type=layer_type)
    model.load()
    load_s = time.perf_counter() - t0

    free, total = torch.cuda.mem_get_info()
    vram_used_mib = (total - free) // (1024 * 1024)
    print(f"kv             : {str(args.kv_bits) + '-bit' if args.kv_bits else 'fp16'}")
    print(f"load           : {load_s:.1f}s")
    print(f"vram after load: {vram_used_mib} MiB")

    generator = Generator(model=model, cache=cache, tokenizer=tokenizer)

    if args.needle:
        n = max(1, int(args.ctx * 0.85) // 17)
        recs = [f"Record {i:05d}: ordinary archival entry with no authorization code."
                for i in range(n)]
        at = n // 2
        recs[at] = (f"Record {at:05d}: authorization code COBALT-719 applies to "
                    "the lunar inventory.")
        prompt = ("\n".join(recs) +
                  f"\n\nWhat authorization code appears in record {at:05d}? "
                  "Respond with only the code.")
        gen_tokens = 32
    else:
        filler = "\n".join(f"Record {i:04d}: ordinary archival entry, no code."
                           for i in range(max(1, args.prompt_tokens // 12)))
        prompt = (f"{filler}\n\nSummarise the structure of the records above in "
                  "one short paragraph, then stop.")
        gen_tokens = args.gen

    ids = tokenizer.encode(prompt)
    n_prompt = ids.shape[-1]

    t0 = time.perf_counter()
    job = Job(input_ids=ids, max_new_tokens=gen_tokens)
    generator.enqueue(job)
    out_text, produced = "", 0
    while generator.num_remaining_jobs():
        for result in generator.iterate():
            chunk = result.get("text", "")
            out_text += chunk
            if result.get("eos"):
                produced = result.get("new_tokens", produced)
    elapsed = time.perf_counter() - t0
    produced = produced or gen_tokens

    print(f"prompt tokens  : {n_prompt:,}")
    print(f"generated      : {produced}")
    print(f"wall           : {elapsed:.2f}s")
    print(f"generation     : {produced / elapsed:.2f} tok/s")
    if args.needle:
        ok = "COBALT-719" in out_text
        print(f"needle         : {'PASS' if ok else 'FAIL'}  {out_text.strip()[:60]!r}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
