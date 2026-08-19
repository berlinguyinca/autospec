# Measured on this hardware

Everything here was observed on the target host. Anything not measured says so.

**Host:** Ubuntu 24.04.4 (kernel 6.8.0-137) · NVIDIA GeForce RTX 4090, 24564 MiB,
driver 580.173.02 · 503 GiB system RAM · vLLM 0.27.1, torch 2.13.0+cu130 ·
model `cyankiwi/Qwen3.8-27B-AWQ-INT4` @ `63768c10`.

## The headline

**This node cannot serve the 100–150K context the spec asks for, and no amount
of tuning changes that.** The verified maximum is **56,448 tokens** — proven with
a real 44,238-token retrieval — and reaching it costs CUDA graphs, all
concurrency, and ~3x generation speed.

The reason is simple arithmetic: the checkpoint occupies **18.37 GiB of a
24 GiB card**.

### Three numbers looked like the ceiling. Only the smallest was real.

This is the single most important thing in this document, because two of the
three are what vLLM itself reports:

| number | source | what it actually is |
|---:|---|---|
| 109,760 | vLLM's own "estimated maximum model length", probed at 262144 | **wrong.** Starts, serves a 6-token request, then dies with `torch.OutOfMemoryError` on a ~70k prompt. |
| 66,446 | vLLM's `GPU KV cache size: N tokens` | **not comparable.** An aggregate pool figure, not a per-request limit. |
| **39,200** | the estimate iterated to a fixed point, then run at length | **real.** Verified with a 33,358-token retrieval. |

Three claims get confused easily and are not the same: a context can be
**allocatable**, the server can **start** at it, and it can **work at length**.
Only the third is quoted as verified anywhere in this tree.

Two traps produced the wrong numbers:

1. **vLLM's available-KV figure depends on the `max_model_len` you asked for.**
   Asking 262144 reports 3.51 GiB available; asking 109760 reports 1.36 GiB.
   So a single probe returns a ceiling that is invalid at its own answer, and
   the estimate has to be iterated to a fixed point.
2. **Measuring on a GPU that has not fully released the previous process
   under-reports badly.** An early probe with a 3-second settle read a
   25,486-token pool where the true figure was 66,446.

What is left over after the weights is the entire budget for KV cache,
activations and CUDA graphs.

## Where the VRAM goes

| consumer | cost | how it was measured |
|---|---|---|
| weights, text-only | **18.37 GiB** | `Model loading took 18.37 GiB memory` |
| checkpoint on disk | 19.57 GiB | vLLM startup log |
| CUDA graphs | **~2.5 GiB** | KV pool 25,486 → 73,728 tokens when disabled — but costs ~3x generation speed |
| FlashInfer workspace | **394 MiB** | allocated lazily on first request, NOT reserved by profiling |
| desktop / other processes | ~0.7 GiB | why `--gpu-memory-utilization 0.985` is refused |

`--gpu-memory-utilization 0.97` is the practical maximum. 0.985 fails outright:

```
ValueError: Free memory on device cuda:0 (22.78/23.48 GiB)
```

## KV cost per token

vLLM's own refusal confirms the arithmetic derived from `config.json`:

```
To serve at least one request with the model's max seq len (262144),
8.18 GiB KV cache is needed
```

8.18 GiB ÷ 262,144 = **32.7 KiB/token**, matching
`2 × 16 full-attn layers × 4 kv_heads × 256 head_dim = 32 KiB`.

The *effective* cost is roughly double, because the hybrid allocator pads the
mamba page size to match the attention page size and serves both from one pool:

```
Setting attention block size to 1568 tokens to ensure that attention
page size is >= mamba page size.
Padding mamba page size by 0.13% to ensure that mamba page size and
attention page size are exactly equal.
```

## Measured configurations

All at `--gpu-memory-utilization 0.97`, fp8 KV, text-only.

**These are POOL figures, not per-request context limits** — see the headline
table. They are recorded because they show where the VRAM goes, not because any
of them is a context you can configure.

| CUDA graphs | batched tokens | max seqs | KV pool | reported concurrency |
|---|---:|---:|---:|---|
| on | 2048 | 1 | 1.36 GiB / 25,486 tok — *under-measured, GPU not fully released* | — |
| on | 2048 | 4 | 1.29 GiB / 30,427 tok | 1.86x @ 16,384 |
| **off** | 1568 | 1 | 3.88 GiB / 73,728 tok | starts, then OOMs on first request |
| **off** | 1568 | 8 | 4.00 GiB / 75,548 tok | 9.22x @ 8,192 — also OOMs on first request |

### Correction: concurrency is not bound by recurrent state

It is natural to assume — and the spec does assume — that the 48 linear-attention
layers' per-sequence recurrent state limits concurrency independently of context.
**Measured, it does not.** The pool was 73,728 tokens at `seqs=1` and 75,548 at
`seqs=8`: essentially unchanged. Because mamba pages are padded to the attention
page size and drawn from the same block pool, the real bound is just

```
concurrency ≈ pool_tokens / max_model_len
```

### CUDA graphs are the one big tuning lever

Disabling them nearly triples usable context (25,486 → 73,728 tokens) at the cost
of generation speed. That trade is why `interactive` and `extended` exist as
separate profiles rather than one compromise.

## The trap: healthy is not working

At `--gpu-memory-utilization 0.97` the eager profiles start, report healthy, and
then **500 on every request**:

```
torch.OutOfMemoryError: CUDA out of memory. Tried to allocate 394.00 MiB.
  ... flashinfer.py, in _get_workspace_buffer: self._workspace_buffer = torch.zeros(
vllm.v1.engine.exceptions.EngineDeadError
```

FlashInfer allocates a **394 MiB workspace buffer lazily on the first prefill**,
and vLLM's memory profiling does not reserve it — so utilisation that looks fine
at startup is fatal on first use. `interactive` survived only because CUDA graphs
had already left it several GiB of slack.

Both eager profiles therefore run at **util 0.94**, keeping ~1 GiB back. This is
why a health check is not an acceptance test, and why `tests/test_smoke.sh`
issues real completions.

## Resulting profile values — all verified by real inference

| profile | graphs | util | context | seqs | verified by |
|---|---|---:|---:|---:|---|
| `interactive` | on | 0.97 | **39,200** | 1 | 6/6 smoke + **33,358-token retrieval** |
| `concurrent` | off | 0.94 | 8,192 | 6 | 6/6 smoke |
| `extended` | off | 0.94 | **56,448** | 1 | 6/6 smoke + **44,238-token retrieval** |

`extended` exceeds `interactive` because eager execution returns the ~2.5 GiB
that CUDA graphs hold — the same trade, seen from the context side.

## Throughput

Measured with `scripts/benchmark.py`, ~512-token prompt, 256 max tokens.

| profile | concurrency | aggregate tok/s | per-request tok/s | TTFT (s) |
|---|---:|---:|---:|---:|
| `interactive` | 1 | **41.5** | 41.5 | 0.26 |
| `interactive` | 2 | 42.2 | 31.7 | 0.93 |
| `concurrent` | 1 | 14.3 | 14.3 | 0.29 |
| `concurrent` | 2 | 20.9 | 10.5 | 0.89 |
| `concurrent` | 4 | 46.0 | 11.7 | 0.90 |
| `concurrent` | 6 | **52.7** | 10.3 | 1.30 |

**Eager mode costs about 3x single-stream generation speed** (41.5 → 14.3 tok/s).
That is a much steeper price than the VRAM saving suggests, and it means the
`extended` profile generates at roughly a third of `interactive` speed. Six-way
concurrency recovers only ~25% aggregate over `interactive` (52.7 vs 42.2).

For reference, the llama.cpp node on this same host runs Q4_K_M at roughly
30 tok/s single-stream, so `interactive` at 41.5 tok/s is a genuine improvement —
but `extended` at 14.3 tok/s is materially slower than llama.cpp.

## Verified long context

A real 44,238-token prompt, not an extrapolation:

| profile | prompt tokens | wall time | prefill+gen | needle |
|---|---:|---:|---:|---|
| `extended` (56,448) | 44,238 | 18.91 s | ~2,339 tok/s | **correct** |
| `interactive` (39,200) | 33,358 | 12.62 s | ~2,643 tok/s | **correct** |

The limit is enforced exactly: a 56,449-token request against `extended` is
rejected with `This model's maximum context length is 56448 tokens`.

## MTP (speculative decoding): does not fit on 24 GB

Tested, not assumed. The checkpoint *does* contain the draft head (15 `mtp.*`
tensors, `mtp_num_hidden_layers: 1`) and `qwen3_5_mtp` is a valid vLLM method,
so this was a real attempt, not a config error.

It fails identically in three configurations:

| config | result |
|---|---|
| CUDA graphs, util 0.97, ctx 39200 | `OutOfMemoryError: Tried to allocate 2.37 GiB` |
| eager, util 0.95, ctx 8192 | same, 993 MiB free |
| eager, util 0.85, ctx 8192 | same — process still held 22.10 GiB |

The draft head is **unquantised fp16 and costs 2.37 GiB**, and vLLM loads it
*after* claiming the KV pool — so it sits outside `gpu_memory_utilization` and
lowering that does not make room. Even if it fit, the 2.37 GiB would have to
come out of the ~2.5 GiB that CUDA graphs occupy, and CUDA graphs are worth 3x
generation speed. **MTP is the wrong trade on this card.** It becomes viable
only with a checkpoint small enough to leave ~3 GiB spare.

## Attention backend: TRITON_ATTN does not buy context either

The theory was sound — FlashInfer's 394 MiB workspace forces ~1 GiB of headroom,
and TRITON_ATTN has no such workspace. Measured, it does not help: at util 0.95
the engine dies in the **linear-attention prefill kernel** instead.

```
chunk_gated_delta_rule_fwd_h -> torch.OutOfMemoryError:
Tried to allocate 20.00 MiB ... 19.62 MiB is free
```

The GDN prefill kernel allocates working memory outside the profiled budget too,
so ~1 GiB of headroom is required regardless of attention backend. **util ~0.94
is the practical ceiling for the eager profiles**, and which backend serves
attention is not what decides it.

This also exposed a wrong assumption worth stating plainly: **lowering
`max_model_len` frees no VRAM.** vLLM sizes the KV pool from
`gpu_memory_utilization`, so the identical OOM reproduced at 109760, 76832,
53312 and 36064. Utilisation is the only lever that returns memory;
`scripts/measure-ceiling.sh` descends on utilisation for exactly this reason.

## How to actually reach 100K+

One change would do it: **quantise `lm_head` and `embed_tokens` to INT8.** Each
is 248320 × 5120 and costs ~2.4 GiB at fp16. Recovering ~4.8 GiB roughly doubles
the KV pool. The spec asked for exactly this; no public checkpoint provides it,
so it means running llm-compressor over the base model. See
`config/model-artifacts.yaml` → `wanted`.

Rejected alternatives are recorded there too — notably that the AutoRound W4A16
checkpoint is *larger* (26.39 GiB) and NVFP4 needs Blackwell, not Ada.

## Not measured

Recorded so silence is not read as a result. All are specified in
`benchmark-profile.yaml`.

| area | status |
|---|---|
| concurrency sweep 16/32/64 | **not run.** At util 0.94 the pool supports ~7x at 8K; 16+ would need a much smaller context. |
| MTP on/off A/B | **run — MTP does not fit.** See below. |
| Profile D (GGUF Q6_K) | **not run.** The llama.cpp node holds Q4_K_M, not Q6_K. |
| quality / regression suite | **not run.** No promotion decision should be made without it. |
| long-prompt (100k+) throughput | **not reachable** on this node; 44,238 tokens was executed and is reported above. |
| vision profile | **not built.** All vLLM profiles are text-only. |
| attribution of the 19.57 → 18.37 GiB delta | **inferred, not A/B tested.** vLLM reports "running in text-only mode"; the 1.2 GiB difference is consistent with the vision tower but was not confirmed by a paired `--mm on` run. |
