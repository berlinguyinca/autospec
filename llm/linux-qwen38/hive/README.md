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

Decode speed is the bandwidth roofline scaled by measured efficiency. The
Blackwell row is now **measured**; the rest remain arithmetic.

The calibration held remarkably well across two very different cards:

| card | quant | roofline | measured | % of roofline |
|---|---|---:|---:|---:|
| RTX 4090 | Q5_K_M | 50.9 | **40.55** | 79.7% |
| RTX PRO 6000 Blackwell | UD-Q8_K_XL | 57.0 | **45.57** | 80.0% |

Ada and Blackwell, 24 GiB and 96 GiB, a 5-bit and an 8-bit quant — and both land
at 80% of their memory-bandwidth ceiling. That is what makes the predictions in
this table worth anything: the constant is a property of the runtime and the
model, not of the card. The prediction for this machine was ~49 t/s against
45.57 measured, about 7% optimistic — the Max-Q part is power-limited, and the
tunnel adds a little.

Concurrency, measured on hive at a ~4,000-token prompt:

| clients | per-stream | aggregate | worst TTFT |
|---:|---:|---:|---:|
| 1 | 45.57 | 28.10 | 1.74 s |
| 4 | 32.49 | **53.22** | 5.90 s |

Worth putting beside the 4090, which reached 56.72 aggregate at four clients on
Q5: the cluster card does not serve *more* throughput, it serves **8-bit weights
at the full 262,144 context** for about the same. Capacity and quality are what
you go there for, not speed.

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
- **Environment Modules 5.5.0** (Tcl), *not* Lmod — so `module`/`ml` work but
  Lmod-only syntax (`module spider`, hierarchical `ml gcc cuda`) does not.
  `MODULEPATH` has seven roots, all on CVMFS, which is mounted on compute nodes
  as well. Useful modules: `cuda/13.3.0` (default), `gcc/13.2.0`,
  `cmake/3.28.1`, `python/3.11.9`, plus a large `conda/*` tree. `uv` is at
  `~/.local/bin/uv`; `apptainer` is available; there is no `docker`.
- **Built binaries need two runtime paths.** llama.cpp splits each tool into a
  thin driver plus `libllama-<tool>-impl.so`, so `LD_LIBRARY_PATH` must include
  the install `lib/`, and `module load cuda` must be in effect for
  `libcudart.so.13`. Missing either looks like a broken build — "error while
  loading shared libraries" — when the build was fine.
- **Compute nodes have outbound internet** — `huggingface.co` returns 200, DNS
  resolves, no proxy needed. Weights can be fetched inside the job.
- There is no prebuilt llama.cpp; `setup-hive.sh` builds it for
  `sm_80;86;89;120` so the binary runs on any GPU here.

## Running it

One command, from the workstation:

```bash
opencode_hive                  # acquire a GPU, serve, tunnel, configure, launch
opencode_hive status           # where things stand
opencode_hive stop             # drop the tunnel, cancel the job
opencode_hive --gpu nvidia_a100-sxm4-80gb --time 04:00:00
```

It reuses a running job rather than starting a second one, and runs the setup
job for you if llama.cpp or the weights are missing.

### The tunnel

Outbound from the workstation, so a router, NAT, or firewall on that end is
irrelevant and the compute node never needs to be reachable from outside:

```
127.0.0.1:8081  ->  hive login node  ->  compute node:8080
```

**Local port 8081, not 8080.** The local RTX 4090 router already owns 8080; a
tunnel onto it would either refuse to bind or silently shadow the local node, so
every "local" request would quietly execute on the cluster.

The tunnel is **supervised, not started once**. `low` is preemptible, so the job
can be requeued onto a different node mid-session; the supervisor re-reads
`logs/endpoint.txt`, notices the move, and re-forwards.

The server binds `0.0.0.0` on a shared cluster network, so `serve-qwen.sbatch`
generates a per-deployment API key into `logs/api-key.txt` (mode 600) and
requires it. Do not remove that.

The hive provider is added to OpenCode as `qwen-hive/...` **without** becoming
the default: the local 4090 stays default, so losing the cluster job never
leaves the client pointed at a dead endpoint. Pick a `qwen-hive/` model when you
want the cluster.

### Setup jobs ask for no GPU

Compiling CUDA needs `nvcc`, not a device, and downloading weights needs
neither. `setup-hive.sh` therefore requests **no** `--gres`, so it schedules
against all 168 nodes in `low` instead of queueing behind the 86 GPUs that the
serving job actually wants. The first version asked for a Blackwell and sat
`PENDING`; without it, the same job started immediately.

## Git

`git` 2.34.1, with `url.git@github.com:.insteadOf https://github.com/` already
set globally — so HTTPS GitHub URLs are rewritten to SSH and authenticate with
`~/.ssh/id_rsa`. Verified: GitHub answers `Hi berlinguyinca!`. There is no `gh`
CLI and no credential helper.

`user.name` and `user.email` were **unset**, which fails any commit made on the
cluster; they are now configured. Clone work into `/quobyte/metabolomicsgrp/it`,
never `$HOME` (3.3 GB free).

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
