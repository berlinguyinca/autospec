# hive.hpc.ucdavis.edu — site playbook

Measured on 2026-08-18 from `gw@hive.hpc.ucdavis.edu`. Everything here was read
off the live cluster, not from documentation.

## The three facts that decide everything

**1. You cannot submit to any `gpu-*` partition.** They exist, they are
`AllowAccounts=ALL`, and every request is still rejected:

```
sbatch --test-only -p gpu-a100 -A publicgrp --gres=gpu:1 ...
  -> allocation failure: Invalid account or account/partition combination
```

Your associations cover only `high` and `low`:

| account | partition | QOS |
|---|---|---|
| metabolomicsgrp | high | metabolomicsgrp-high-qos |
| publicgrp | high | publicgrp-high-qos |
| publicgrp | low | publicgrp-low-qos |

GPUs are reached through `low` (86 GPUs) or `high` (12 GPUs) with `--gres`.

**2. GPUs must be charged to `publicgrp`.** The metabolomics QOS carries
`gres/gpu=0`, so a GPU request under it is rejected at submit time:

```
sbatch -p high -A metabolomicsgrp --gres=gpu:1 ...  -> error: QOSGrpGRES
```

| QOS | limits |
|---|---|
| publicgrp-low-qos | **none** |
| publicgrp-high-qos | cpu=128, gres/gpu=5, mem=2000G |
| metabolomicsgrp-high-qos | cpu=512, **gres/gpu=0**, mem=8000G |

**3. `low` is preemptible; `high` is unreachable in practice.**

| | low | high |
|---|---|---|
| walltime | 7 days | 30 days |
| GPUs | 86 | 12 |
| PriorityTier | 10 | 50 |
| PreemptMode | **REQUEUE** | OFF |
| GraceTime | 130 s | 0 |
| est. start for `--gres=gpu:1` | same hour | **2026-09-20** |

So `low` is the only workable option, and a job there can be requeued with about
two minutes' notice when a `high` job wants the node. Design for it: `--requeue`,
no state worth losing, and re-read the endpoint after a restart because the job
may return on a different node.

## GPU inventory

Request by the **exact** GRES name — they are not consistent across nodes, and a
wrong name is an allocation failure rather than a fallback.

| GRES name | per node | nodes | VRAM |
|---|---:|---:|---:|
| `nvidia_a100-sxm4-80gb` | 4, 8 | 4 | 80 GB |
| `a100` | 4, 8 | 3 | 80 GB |
| `nvidia_a100_80gb_pcie` | 4 | 2 | 80 GB |
| `nvidia_a100-pcie-40gb` | 4 | 1 | 40 GB |
| `6000_blackwell` | 4 | 3 | **96 GB** |
| `nvidia_rtx_pro_6000_blackwell_max-q_workstation_edition` | 2 | 1 | **96 GB** |
| `nvidia_l40s` | 4 | 1 | 48 GB |
| `a6000` | 1, 4 | 5 | 48 GB |
| `nvidia_rtx_5000_ada_generation` | 4 | 1 | 32 GB |

Verified on a Blackwell node:

```
NVIDIA RTX PRO 6000 Blackwell Max-Q Workstation Edition, 97887 MiB, cc 12.0, driver 580.167.08
```

`compute_cap 12.0` is sm_120 — FP8 **and** NVFP4 are available, unlike the A100
(sm_80), where neither is.

### Which card to ask for

Predicted decode speed is the bandwidth roofline scaled by the 79.7% efficiency
the RTX 4090 reference build actually achieved. **Arithmetic, not benchmarks.**

| GPU | GB/s | Q5_K_M | Q8_0 | BF16 | VRAM |
|---|---:|---:|---:|---:|---:|
| A100 80 GB SXM | 2039 | ~82 | ~56 | ~30 | 80 |
| RTX PRO 6000 Blackwell | 1792 | ~72 | ~49 | ~26 | **96** |
| L40S | 864 | ~35 | ~24 | — | 48 |
| A6000 | 768 | ~31 | ~21 | — | 48 |
| RTX 5000 Ada | 576 | ~23 | — | — | 32 |

The A100 SXM is **faster** than the Blackwell here — this workload is bandwidth
bound and HBM2e beats GDDR7 at these capacities. Blackwell wins on capacity
(96 vs 80 GB), on NVFP4, and above all on **availability**: A100 requests sat at
`PLANNED` with next-day start estimates while Blackwell allocated in seconds.
Ask for Blackwell unless you need the last 15% of speed.

## Filesystems

| path | size | notes |
|---|---:|---|
| `/home/gw` | 20 GB | **3.3 GB free — will not hold a model** |
| `/quobyte/metabolomicsgrp` | 8.5 PB | shared, writable, visible from compute nodes |
| `/scratch`, `/tmp` on compute | 3.5 TB NVMe | node-local, per-job, fast |

Work lives in `/quobyte/metabolomicsgrp/it/llm`.

Two traps:

- **The login node's `/tmp` is not the compute node's `/tmp`.** A script written
  to `/tmp` on login2 fails with `No such file or directory` inside the job.
  Stage scripts on `/quobyte`.
- **Stage weights to node-local NVMe at job start.** `/quobyte` is fine for one
  sequential read but every other user shares it; the node has 3.5 TB of idle
  local flash.

## Environment

- Ubuntu 22.04, software from CVMFS + Spack.
- **Slurm is not on `PATH` in a non-login shell.** `ssh host 'sinfo'` gives
  `command not found`; `ssh host 'bash -lc sinfo'` works. This bites any
  automation that shells in non-interactively.
- Modules: `cuda/13.3.0` (default), `gcc/13.2.0`, `cmake/3.28.1`,
  `python/3.11.9`. `uv` is installed at `~/.local/bin/uv`; `apptainer` is
  available; there is no `docker`.
- **Compute nodes have outbound internet** — `huggingface.co` returns 200, DNS
  resolves, no proxy needed. Weights can be fetched inside the job.
- There is no prebuilt llama.cpp; `setup-hive.sh` builds it for
  `sm_80;86;89;120` so the binary runs on any GPU here.

## Running it

```bash
sbatch setup-hive.sh                 # once: build llama.cpp, stage weights
sbatch serve-qwen.sbatch             # start the server
cat logs/endpoint.txt                # node, port, and the ssh tunnel command
```

Then from your workstation, using the node named in `endpoint.txt`:

```bash
ssh -N -L 8080:${NODE}:8080 gw@hive.hpc.ucdavis.edu
```

The server binds `0.0.0.0` on a shared cluster network, so `serve-qwen.sbatch`
generates a per-deployment API key into `logs/api-key.txt` (mode 600) and
requires it. Do not remove that.

### Scheduling

Short jobs backfill; long ones wait. A 3-hour, 32-CPU request sat at `(Priority)`
with a two-hour estimate while GPUs were visibly idle; the same job at 1 hour and
16 CPUs started immediately. Ask for what you need and no more.

## What this hardware changes

The 24 GiB reference build is one long fight with capacity. At 96 GiB, three of
its compromises are simply dropped — see `router-presets-hive.ini`:

| | RTX 4090 (24 GiB) | hive Blackwell (96 GiB) |
|---|---|---|
| quantisation | Q5_K_M | **UD-Q8_K_XL** |
| KV cache | q4_0 | **f16** |
| context pool | 180,224 (measured) | **262,144** (the trained maximum) |
| slots | 6 | 8 |

Budget: 95.6 GiB − 29.3 weights − 8.0 KV at full context − ~4 compute ≈ **54 GiB
spare**. Nothing here is tight, which is exactly the point.

The one thing that does **not** change is client-side rationing. llama.cpp still
accepts more sessions than the pool can fund and then fails all of them, so the
`-NNk` tiers still matter. And no tier is below 40k, because this operator's p90
session carries 37,873 tokens before any work is done.
