# Qwen3.8-27B — Linux RTX 4090 inference node

A reproducible vLLM deployment of Qwen3.8-27B on a single NVIDIA RTX 4090
(24 GiB), serving an OpenAI-compatible API for AutoSpec workers.

```
scripts/setup-linux-qwen38.sh     # install, validate, enable
qwen38ctl status                  # what is running
qwen38ctl switch extended         # change profile
```

## Why this model is cheaper to serve than it looks

Qwen3.8-27B (`Qwen3_5ForConditionalGeneration`) is a **hybrid** model. Of its
64 layers only 16 are full attention — `full_attention_interval` is 4 — and the
other 48 are linear-attention (gated-delta) layers that carry a **constant-size
recurrent state** instead of a per-token KV cache.

That changes the arithmetic that governs everything else here:

```
fp8 KV per token = 2 × 16 full-attn layers × 4 kv_heads × 256 head_dim
                 = 32 KiB/token
```

A dense 27B would cost roughly four times that — which is what makes tens of
thousands of tokens reachable here at all. It is still not enough for the
spec's six-figure target; see below.

vLLM confirms the figure during startup: 262,144 tokens "needs 8.18 GiB",
which is 32.7 KiB/token.

**But context and concurrency come out of one pool.** It is tempting to assume
the recurrent state bounds concurrency separately — the spec's Profile B does.
Measured, it does not: the KV pool was **73,728 tokens at `seqs=1` and 75,548 at
`seqs=8`**, essentially unchanged. vLLM pads the mamba page size to equal the
attention page size and serves both from the same block pool, so what binds is
simply:

```
concurrency ≈ pool_tokens / max_model_len
```

Raising `--max-num-seqs` past that ratio buys nothing; requests queue.

## The two things that actually cost VRAM

Weights dominate, and CUDA graphs are a distant but very real second:

| consumer | cost | note |
|---|---|---|
| weights (text-only) | **18.37 GiB** | of a 24 GiB card |
| CUDA graphs | **~2.5 GiB** | disabling them took the KV pool from 25,486 → 73,728 tokens — and cost ~3x generation speed |
| FlashInfer workspace | **394 MiB** | allocated on the *first request*, not reserved by profiling — the cause of a start-healthy-then-die failure |
| desktop / other processes | ~0.7 GiB | why util 0.985 is refused outright |
| everything left for KV | 1.3–3.2 GiB | depending on the above |

That ratio is why the profiles below look the way they do, and why none of them
reaches the spec's 100–150K target. It is not a tuning failure: 18.37 GiB of
weights on a 24 GiB card leaves too little, and the fix is a smaller checkpoint,
not a smaller batch. See `config/model-artifacts.yaml` → `wanted`.

## Measured, not assumed

Every context and concurrency number in `config/profiles.d/` was measured on
this host, and every one was then **confirmed by a real completion** — not by a
successful startup. That distinction is not pedantic: a configuration that
allocates a 73,728-token pool and passes `/health` still dies on its first
request here. The measurements and the reasoning behind each value are in
**[docs/measured-ceilings.md](docs/measured-ceilings.md)**.

`tests/test_structural.sh` fails if any profile still carries an unresolved
`__PLACEHOLDER__`, so a profile can never ship with a guessed context size.

## Which model, exactly

Deployed: **`cyankiwi/Qwen3.8-27B-AWQ-INT4`** @ `63768c10` (W4A16
compressed-tensors, group size 32), served by vLLM 0.27.1. Chosen for runtime
compatibility, **not** speed — no Unsloth / NVFP4 / exl3 build is in use.
Measured alternatives, and why NVFP4 is unavailable on this card, are in
[docs/runtime-comparison.md](docs/runtime-comparison.md).

## Profiles

| profile | purpose | CUDA graphs | context | concurrency |
|---|---|---|---|---|
| `interactive` | one worker, interactive coding — **starts at boot** | on (fast) | 39,200 | 1 |
| `concurrent` | several agents in parallel | off | 8,192 | 6 |
| `extended` | repo-wide analysis, architectural review | off | **56,448** | 1 |
| `quality` | quantisation-regression control (llama.cpp Q6_K) | n/a | see qwen-local | 1 |

The `interactive` / `extended` split is the honest expression of the trade above:
you can have generation speed or you can have context, and on this card you
cannot have both. Measured: **41.5 tok/s at 39,200 context**, or **14.3 tok/s at
56,448** — disabling CUDA graphs buys context and costs ~3x speed.

Both are verified with real long prompts (33,358 and 44,238 tokens), not just
allocations — two larger figures vLLM itself reported turned out not to survive
a long request. Full numbers in [docs/measured-ceilings.md](docs/measured-ceilings.md).

Only one profile runs at a time. `qwen38ctl` stops the current one and waits for
the GPU to actually be released before starting the next — systemd reports a
unit inactive as soon as the process exits, but the driver can hold the
allocation a moment longer, which is long enough to fail the next start.

## Coexistence with the llama.cpp node

This host also runs `qwen-local.service`, the llama.cpp deployment of the same
model. It holds ~23 GiB of the 24 GiB card when resident. **The two cannot run
at once.**

- vLLM listens on **8000**, llama.cpp on **8080**, so neither unit needs editing
  to run the other.
- The unit declares `Conflicts=qwen-local.service`, which systemd applies in
  both directions: starting either runtime stops the other, instead of the
  newcomer OOMing several minutes into vLLM's memory-profiling pass.

**Rolling back to llama.cpp is one command:**

```
qwen-localctl resume        # stops vLLM, restores the llama.cpp node on :8080
```

That path is the known-good configuration on this machine and is worth keeping
in reach — the vLLM profiles here are newer and less proven.

## Profile versioning

Weights and quantisation are pinned by immutable revision in
`config/model-artifacts.yaml`. Changing either is a **new profile version**
(`…-v1` → `…-v2`), never an edit in place, so a benchmark result always names a
configuration that can be reconstructed. `tests/test_structural.sh` enforces
that the pin is a full commit sha rather than a branch name.

## Layout

```
config/
  common.conf              shared settings — the single source of truth
  profiles.d/*.conf        per-profile overrides
  model-artifacts.yaml     provenance, and what was rejected and why
scripts/
  setup-linux-qwen38.sh    idempotent installer; refuses to claim success
                           without a real completion from the served model
  serve-profile.sh         launcher (ExecStart)
  qwen38ctl                control CLI
  measure-ceiling.sh       find and prove the real context ceiling
  benchmark.py             concurrency sweep → JSON + Markdown
systemd/
  autospec-qwen38@.service template unit, instance name = profile
tests/
  test_structural.sh       no GPU needed; safe in CI
  test_smoke.sh            the gate the installer will not skip
docs/
  measured-ceilings.md     what this hardware actually does
runtime-descriptor.json    machine-readable, for the AutoSpec model router
benchmark-profile.yaml     what the qualification benchmark must run
```

## Simplifications

Recorded so they are visible rather than discovered later.

1. **One template unit instead of two named units.** The spec sketched
   `autospec-qwen38-interactive.service` and `-concurrent.service`.
   `autospec-qwen38@.service` covers all three vLLM profiles from one file, and
   makes mutual exclusion a property of the unit rather than of a script.
   `autospec-qwen38@interactive.service` is the instance the spec named.
2. **INT8 `lm_head` / INT8 `embed_tokens` are not implemented.** No public
   checkpoint quantises them, so this would require self-quantisation with
   llm-compressor. It is the single highest-value follow-up: those two tensors
   cost ~4.8 GiB at fp16, and returning that VRAM buys roughly 75k tokens of
   additional context. See `config/model-artifacts.yaml` → `wanted`.
3. **Prometheus metrics are not wired up.** vLLM exposes `/metrics` natively;
   nothing scrapes it here yet. The spec called this preferred, not mandatory.
4. **MTP is off in every profile.** The `qwen3_5_mtp` method string is verified
   valid in the installed vLLM, but its benefit and — more importantly on this
   card — its VRAM cost are unmeasured. The A/B is specified in
   `benchmark-profile.yaml` and has not been run.
5. **Two upstream workarounds are configuration, not patches.** The FlashInfer
   sampler is disabled (CUDA 13 CUB incompatibility) and the venv `bin` is forced
   onto `PATH`. Both are documented with removal conditions in
   [patches/README.md](patches/README.md) and asserted by the structural tests.

## Sandbox settings that break CUDA

The unit deliberately does **not** set `PrivateDevices=true`, `DevicePolicy=closed`,
`MemoryDenyWriteExecute=true`, or `ProtectSystem=strict` — each breaks the CUDA
stack, the first two by hiding `/dev/nvidia*`. `ProtectSystem=full` is the
strongest setting that leaves CUDA working. `tests/test_structural.sh` asserts
none of the four come back.
