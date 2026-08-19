---
name: GPU capability registry — measured, observed and assumed kept apart
description: llm/linux-qwen38/config/gpu-registry.json records every GPU this project has run on; serving jobs self-register new cards, and bandwidth/weight_bytes x 0.80 predicts an untouched machine to ~10%
type: reference
wing: synthesis
drawer_class: fact
---
`llm/linux-qwen38/config/gpu-registry.json`, driven by
`scripts/gpu-registry.py`:

```
gpu-registry.py record --site NAME [--out LOG]   observe this machine
gpu-registry.py merge OBSERVED.jsonl             fold observations in
gpu-registry.py show                             what do we know
gpu-registry.py predict --weights-gib N          expected tok/s
```

**Why it exists:** once the GPU is selected by earliest queue start, the
hardware is unknown until the job runs — four different cards in a
single session — and their specs were otherwise being retyped into three
places with nothing recording what any of them actually achieved.

**Three kinds of claim, deliberately separate fields:**

| field | status |
|---|---|
| `vram_mib`, `compute_cap` | read from the device |
| `measured_tps` | benchmarked, per quantisation |
| `bandwidth_gbs` | vendor figure — the only assumption |

A card that is not recognised gets `bandwidth_gbs: null`, never a guess.

**Benchmarks belong in it too.** `gpu-registry.py benchmark --base URL
--model ID --quant FILE` measures single-stream decode and records it
under the weights filename. It times from the FIRST token (prefill and
queueing must not contaminate a decode rate) and forces a fixed length
with `ignore_eos` (a model that stops after three tokens reports a
fine-looking rate computed over nothing). `--tps` records a figure
measured elsewhere; `--gpu` names the card when driving the benchmark
through a tunnel.

**Reproducibility check:** the Blackwell measured 45.57 tok/s (80.0% of
roofline) and, hours later through a different allocation on a different
node, 44.99 (79.0%). Two independent measurements 1.3% apart.

**Self-populating:** every serving job runs `record --out` (a compute
node cannot write the repo, so observations append to a JSONL on shared
storage) and the driver `merge`s them on the next run.

**The 0.80 constant is measured, not chosen.** RTX 4090 (Ada, 24 GiB,
Q5_K_M) hit **79.7%** of its memory-bandwidth roofline; RTX PRO 6000
Blackwell (96 GiB, UD-Q8_K_XL) hit **80.0%**. Different architecture,
4x the memory, different quantisation, same ratio — so it belongs to the
runtime and the model rather than the card, and
`bandwidth / resident_weight_bytes x 0.80` predicts an untouched machine
to within ~10%. Asked to predict the Blackwell it returns 45.6 against
45.57 observed.

Related: [[reference_slurm_hpc_cluster]] — the cluster where dynamic GPU
selection made this necessary.
