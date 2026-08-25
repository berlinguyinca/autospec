# What this node should serve, and why

Same rule as everything else here: **no number in this file that a real request
did not produce.** Predictions are labelled as predictions and stay that way
until a measurement replaces them.

---

## The constraints, measured

| | value | how it was established |
|---|---:|---|
| Cards | 2 × RTX 2080 Ti | `nvidia-smi` |
| VRAM per card | 11,264 MiB (10,820 usable headless) | `llama-server --list-devices` |
| Usable pair budget | ~21.1 GiB | both cards, headless |
| Memory bandwidth | 616 GB/s per card | vendor figure — the one number here not measured |
| Compute capability | **7.5 (Turing)** | `gpu-registry.json` |
| Interconnect | PCIe, **no NVLink** | `nvidia-smi nvlink -s`: all links inactive |

**sm_75 is the binding constraint on runtime choice.** No bf16, no FP8, no
NVFP4, no Marlin — which rules out the W4A16 vLLM recipes the 4090 node uses.
GGUF on llama.cpp is not a preference here, it is the option that exists.

### The two numbers that decide model choice

Measured 2026-08-25 on this node, and they point in opposite directions:

- **Decode is bandwidth-bound and gets ONE card.** Layer-split does not sum
  bandwidth. The 27B's 28.7 t/s is what a single card's 616 GB/s yields against
  16.46 GB of weights. The second card buys capacity to fit the model, not speed.
- **Prefill is compute-bound and uses BOTH.** The 27B prefilled *faster* than the
  9B on one card — 2,787 vs 2,175 tok/s at 9k context.

So decode speed is governed by **bytes read per token**, and that is the lever.

---

## What runs today

| model | on disk | measured decode | measured prefill |
|---|---:|---:|---:|
| Qwen3.8-27B UD-Q4_K_M | 16.46 GB | 28.7 t/s short-gen, 37.0 t/s sustained | 2,450–2,787 tok/s |
| Qwen3.5-9B Q4_K_M | ~7.1 GiB | 75.7 t/s | 2,040–2,175 tok/s |

Both verified by real completions this week.

---

## The one change most likely to pay: a sparse MoE

Decode reads weights per token. A **mixture-of-experts model activates a fraction
of its parameters per token**, so it reads far less — which is precisely the
constraint this hardware has.

**Candidate: Qwen3-Coder-30B-A3B-Instruct**, ~30.5B total but **~3.3B active per
token**, Q4_K_M ≈ 18.6 GB
([unsloth](https://huggingface.co/unsloth/Qwen3-Coder-30B-A3B-Instruct-GGUF),
[apxml](https://apxml.com/models/qwen3-30b-a3b)).

- **PREDICTION, unverified:** decode materially faster than the dense 27B's
  28.7 t/s, because per-token bytes read fall by roughly an order of magnitude
  even though total weights are larger.
- **PREDICTION, unverified:** it fits, but only just. 18.6 GB of weights against
  a ~21.1 GiB budget leaves ~2–3 GiB for KV and compute buffers — far less slack
  than the 27B's ~4.6 GiB. Context tiers would have to shrink, and the KV rate
  of this architecture is **not** the 18 KiB/token the hybrid-attention Qwen3.8
  enjoys. That must be computed from its `config.json` before any tier is
  advertised.
- **RISK, specific to this host:** MoE routing sends different tokens to
  different experts. With experts layer-split across two cards and **no NVLink**,
  per-token routing may cross PCIe. That could erase the sparsity win entirely.
  This is the measurement that decides it, and it cannot be predicted from the
  model card.

**Alternative, denser and safer:** Devstral Small 2 (30B MoE, ~3B active,
50.3% SWE-bench Verified) — same architectural bet, lower reported coding score
than the Qwen coder line
([kilo.ai](https://blog.kilo.ai/p/the-best-local-coding-models-for)).

Other 24 GB-tier recommendations circulating
([KDnuggets](https://www.kdnuggets.com/top-7-coding-models-you-can-run-locally-in-2026),
[Tembo](https://www.tembo.io/blog/best-local-llm-for-coding)) are written for a
single 24 GB card. They do not account for split-without-NVLink, so their
throughput figures do not transfer to this host.

---

## Single-card operation

**This is not hypothetical: one card is off the bus as of 2026-08-25**, and the
node currently serves nothing because the 27B needs both.

Within one card's 10,820 MiB, the best-grounded option is the model already
here: **Qwen3.5-9B Q4_K_M, ~7.1 GiB, measured 75.7 t/s** — it fits with room for
useful context, and its numbers come from this hardware rather than a table.

Search surfaces other 11 GB-tier candidates (e.g. "Ornith 1.0 9B",
[willitrunai](https://willitrunai.com/gpus/rtx-2080-ti-11gb)). **Unverified, and
not recommended on that basis alone** — a claimed tok/s from an unnamed harness
is not evidence about this node.

---

## What to measure, in order

1. **KV bytes/token from `config.json`** for any candidate, before advertising a
   context tier. This is arithmetic, not a benchmark, and getting it wrong is
   what caused 191 Metal failures on the operator's other host.
2. **Decode at fixed generation length** (`ignore_eos`, 200 tokens) against the
   dense 27B baseline of 28.7 / 37.0 t/s. Short generations understate decode by
   ~29% — measure both or neither.
3. **MoE cross-card penalty:** the same model pinned to one card versus split
   across two. If split is not faster, the sparsity win is being spent on PCIe.
4. **Prefill at 9k**, against 2,787 tok/s.
5. **Perplexity**, before believing any quality claim. On this operator's other
   host Q4/Q5/Q6 landed within ±0.112 and Q5 nominally beat Q6 — bigger quant did
   not mean better, and it was only knowable by measuring.

Pin every candidate by **repository + revision** in `model-artifacts.yaml`
before benchmarking. A bit-width nickname is not an identity.

---

## Recommendation

1. **Now, while one card is down:** serve Qwen3.5-9B on the surviving card. It is
   the only measured model that fits, and half a node beats none.
2. **When both cards return:** benchmark Qwen3-Coder-30B-A3B against the dense
   27B using the order above. The sparsity argument is strong enough to justify
   the disk and the afternoon; the no-NVLink routing risk is real enough that it
   must be measured rather than assumed.
3. **Do not replace the 27B until step 2 produces numbers.** It is measured,
   pinned, and its context tiers are verified to 99,710 tokens by retrieval.
   Trading that for a model that is better on someone else's hardware is how
   this node ends up slower and unable to say why.
