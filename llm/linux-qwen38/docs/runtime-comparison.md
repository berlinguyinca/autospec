# Runtime and quantisation comparison

All measured on this host (RTX 4090 24 GiB, Ubuntu 24.04, driver 580.173.02).
Every row names the **exact checkpoint** — "4-bit" alone is not a specification.

## What is actually deployed

**`unsloth/Qwen3.8-27B-GGUF` Q5_K_M**, served by llama.cpp b10434 on port 8081
(`longcontext` profile, enabled at boot).

It replaced `cyankiwi/Qwen3.8-27B-AWQ-INT4` on vLLM, which remains available as
the `interactive` profile on port 8000. The switch was made on measurement, not
preference: the vLLM profiles top out at a verified 56,448 tokens, which is not
enough for work on a large repository, and the Q5 GGUF is *also* the higher
quality checkpoint (5-bit vs 4-bit) at essentially the same file size.

The cost is ~20% single-stream speed at short context (33.0 vs 41.3 tok/s).

## Context-depth sweep — unsloth Q5_K_M (the deployed default)

`llama-bench`, q4_0 KV, generation and prefill measured **at depth**, which is
the number that matters for large-repository work:

| depth | prefill tok/s | generation tok/s | % of empty |
|---:|---:|---:|---:|
| 0 | 2,727 | 41.87 | 100% |
| 16,384 | 2,427 | 39.76 | 95% |
| 32,768 | 2,148 | 38.21 | 91% |
| 65,536 | 1,774 | 35.15 | 84% |
| 131,072 | 1,290 | 30.18 | 72% |
| **196,608** | 1,013 | **27.04** | 65% |

Generation decays gently because only 16 of 64 layers carry a growing KV cache;
the other 48 are linear-attention layers whose state is constant per sequence.
A dense 27B would fall off far harder.

Through the OpenAI API at ctx 196,608 the sustained rate is **33.0 tok/s**, and
a **153,038-token needle retrieval completed correctly in 89 s**. The llama-bench
figures are a synthetic best case (no HTTP, no full-context allocation); the API
number is the honest one.

## Measured results

| runtime | checkpoint | quant | weights on disk | VRAM resident | gen tok/s | max ctx verified |
|---|---|---|---:|---:|---:|---:|
| **vLLM 0.27.1** | `cyankiwi/Qwen3.8-27B-AWQ-INT4` | W4A16 compressed-tensors, group 32 | 19.57 GiB | 18.37 GiB | **41.3** | 32,928 (28,020-token retrieval) |
| vLLM 0.27.1 | same, `extended` profile (eager) | same | 19.57 GiB | 18.37 GiB | 14.3 | 56,448 (44,238-token retrieval) |
| **ExLlamaV3 1.4.2** | `turboderp/Qwen3.8-27B-exl3` @ `4.00bpw` | exl3 4.0 bpw (QTIP-derived) | 15.70 GiB | 18.5 GiB w/ 32k cache | **27.3** | **98,304 (83,581-token retrieval)** |
| llama.cpp b10434 | `ggml-org/Qwen3.8-27B-GGUF` | Q4_K_M | 17.67 GiB | ~18 GiB | ~30 (operator-reported) | 204,800 configured, q4_0 KV |

### The two results that matter

1. **vLLM W4A16 is the fastest single-stream option** at 41.3 tok/s, and is
   already at ~81% of this card's memory-bandwidth roofline (1008 GB/s ÷
   18.37 GiB = 51.1 tok/s theoretical). There is very little tuning headroom
   left; only smaller weights or speculative decoding beat that ceiling, and
   speculative decoding does not fit (see measured-ceilings.md).

2. **exl3 dominates the `extended` profile on both axes.** Against
   vLLM-eager's 56,448 ctx at 14.3 tok/s, exl3 delivers **98,304 ctx at
   27.3 tok/s** — roughly double the context at roughly double the speed. If
   long-context work matters, exl3 is the better runtime for it.

   exl3 is *not* faster than vLLM for short-context work (27.3 vs 41.3).

## Quantisations evaluated and rejected

| checkpoint | quant | size | why not |
|---|---|---:|---|
| `unsloth/Qwen3.8-27B-NVFP4` | NVFP4 | 21.81 GiB | **Blackwell only** (sm_100/120). This is an Ada card (sm_89). Unsloth reports ~1.5x faster — unavailable to us. |
| `RadixArk/Qwen3.8-27B-NVFP4` | NVFP4 | 20.42 GiB | same |
| `unsloth/Qwen3.8-27B-GGUF` | UD-Q4_K_XL | 17.92 GiB | Unsloth Dynamic is a **quality** optimisation, not speed — and it runs on llama.cpp, which measured slower than vLLM here. Worth it only if quality regression is the concern. |
| `goldhub/…-INT4-W4A16-AutoRound` | W4A16 AutoRound | 26.37 GiB | exceeds 24 GiB before any KV |
| `lued/Qwen3.8-27B-INT8-W8A16-MTP` | W8A16 | 29.44 GiB | far too large |
| `Qwen/Qwen3.8-27B-FP8` | FP8 | ~27 GiB | too large |
| `ggml-org/…-GGUF` Q8_0 | Q8_0 | 26.63 GiB | too large |
| `turboderp/Qwen3.8-27B-exl3` @ 3.50bpw | exl3 3.5 bpw | 14.29 GiB | **not yet tested** — smaller and should be faster still; the obvious next experiment |

## On "optimised" builds

Unsloth's speed claim for this model is **NVFP4-specific**, and NVFP4 needs
Blackwell tensor cores. Their GGUF line (UD-Q4_K_XL etc.) is Dynamic-quant
work aimed at *accuracy at a given size*, not throughput. So on an RTX 4090
there is no Unsloth build that makes this model faster; the speed lever here is
the runtime and the bit width, not the publisher.

## Missing flags worth adding (functionality, not speed)

The official [vLLM recipe](https://recipes.vllm.ai/Qwen/Qwen3.8-27B) serves this
model with parsers we do not currently pass:

```
--reasoning-parser qwen3
--enable-auto-tool-choice --tool-call-parser qwen3_coder
```

Without them the model's reasoning block is not separated from its answer and
**tool calls are not parsed at all** — which matters for an agentic coding
worker. The same recipe independently recommends `--enforce-eager` and
`--max-model-len 32768` for single 24-32 GiB cards, which matches the 32,928
measured here.

## Caveats

- The exl3 numbers come from `scripts/bench-exl3.py`, which drives the library
  directly with no CUDA graphs and a Python iteration loop. A TabbyAPI
  deployment could plausibly be faster; treat 27.3 tok/s as a floor.
- exl3 has no OpenAI-compatible server of its own. Adopting it means running
  TabbyAPI as a **second runtime**, not a config change to the vLLM node.
- No quality comparison has been run between any of these. Speed and context
  numbers alone must not decide a promotion.
