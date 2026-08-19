# Slurm GPU cluster — site playbook

Measured on 2026-08-18 from `<user>@<login-host>`. Everything here was read
off the live cluster, not from documentation.

## The three facts that decide everything

**1. You cannot submit to any `gpu-*` partition.** They exist, they are
`AllowAccounts=ALL`, and every request is still rejected:

```
sbatch --test-only -p gpu-a100 -A <gpu-account> --gres=gpu:1 ...
  -> allocation failure: Invalid account or account/partition combination
```

Your associations cover only `high` and `low`:

| account | partition | QOS |
|---|---|---|
| `<group-account>` | high | `<group-account>-high-qos` |
| `<gpu-account>` | high | `<gpu-account>-high-qos` |
| `<gpu-account>` | low | `<gpu-account>-low-qos` |

GPUs are reached through `low` (86 GPUs) or `high` (12 GPUs) with `--gres`.

**2. GPUs must be charged to `<gpu-account>`.** The group's own QOS carries
`gres/gpu=0`, so a GPU request under it is rejected at submit time:

```
sbatch -p high -A <group-account> --gres=gpu:1 ...  -> error: QOSGrpGRES
```

| QOS | limits |
|---|---|
| `<gpu-account>-low-qos` | **none** |
| `<gpu-account>-high-qos` | cpu=128, gres/gpu=5, mem=2000G |
| `<group-account>-high-qos` | cpu=512, **gres/gpu=0**, mem=8000G |

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

Concurrency, measured on the cluster at a ~4,000-token prompt:

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
| `/home/<user>` | 20 GB | **3.3 GB free — will not hold a model** |
| `<shared-storage>` | 8.5 PB | shared, writable, visible from compute nodes |
| `/scratch`, `/tmp` on compute | 3.5 TB NVMe | node-local, per-job, fast |

Work lives in `<shared-storage>/llm`.

Two traps:

- **The login node's `/tmp` is not the compute node's `/tmp`.** A script written
  to `/tmp` on login2 fails with `No such file or directory` inside the job.
  Stage scripts on `<shared-storage>`.
- **`<shared-storage>` takes a few seconds to propagate between nodes.** A file a
  compute node has just written and can `ls` is briefly absent from the login
  node. Don't read a job's output the instant it claims to have written it.
- **Stage weights to node-local NVMe at job start.** `<shared-storage>` is fine for one
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
- There is no prebuilt llama.cpp; `setup-slurm.sh` builds it for
  `sm_80;86;89;120` so the binary runs on any GPU here.

## Running it

### First, name your site

Every placeholder in this document — `<login-host>`, `<user>`,
`<shared-storage>`, `<gpu-account>` — is a real value that is deliberately not
committed, because this repository is public and a login host, an account name
and a group's storage path are your institution's details to publish rather than
ours. None of them is a secret; the tunnel authenticates with your existing SSH
key.

They live in one local file:

```bash
mkdir -p ~/.config/opencode-slurm
cp site.conf.example ~/.config/opencode-slurm/site.conf
$EDITOR ~/.config/opencode-slurm/site.conf
```

Any subcommand run before that stops with `EX_CONFIG` and prints what is
missing, rather than reaching `ssh` and failing on a hostname that does not
resolve. An environment variable wins over the file, so
`OPENCODE_SLURM_PARTITION=high opencode_slurm up` is a one-off, not an edit.

### Then, one command from the workstation

```bash
opencode_slurm                  # acquire a GPU, serve, tunnel, configure, launch
opencode_slurm status           # where things stand
opencode_slurm stop             # drop the tunnel, cancel the job
opencode_slurm --gpu nvidia_a100-sxm4-80gb --time 04:00:00
```

It reuses a running job rather than starting a second one, and runs the setup
job for you if llama.cpp or the weights are missing.

### Staying connected

Three processes, because the middle one is allowed to fail:

```
OpenCode -> 127.0.0.1:11111   slurm-proxy.py      always listening
         -> 127.0.0.1:11112   ssh forward        comes and goes
         -> login node -> compute node:8080      llama-server
```

**Local port 11111, not 8080.** The local RTX 4090 router owns 8080; a tunnel
onto it would either refuse to bind or silently shadow the local node, so every
"local" request would quietly execute on the cluster.

**The proxy exists so an outage is latency, not an error.** If the listening
socket were the ssh forward itself — as it was — then a reconnect, a preempted
job landing on another node, or an expired allocation all reach OpenCode as
`Connection refused`. `slurm-proxy.py` owns 11111 permanently and simply *holds*
a client until the upstream returns. Measured: kill the forward, send a request
immediately, get an answer 6 s later.

It is a byte relay, not an HTTP proxy, so it carries streamed SSE and long
keep-alive connections without opinions about framing. Its honest limit: a
connection lost **mid-response** cannot be replayed from there, because bytes
have already reached the client. Requests that had not started are covered.

**`/health` is the router's health, not the model's.** A router whose child has
died still answers `/health` with 200 while every request gets
`500 proxy error: Could not establish connection` and every model reports
`unloaded`. An entire benchmark run failed that way while the supervisor
reported the endpoint healthy throughout. So liveness is checked cheaply on
`/health` and **capability** is checked separately, by asking for one token and
requiring a real answer — once after each connect, which is when "the job
started but the model cannot load" actually happens. Running it on a timer would
force a model load on an idle node, which costs minutes of GPU.

**A lost allocation is also recoverable.** `low` caps at 7 days and is
preemptible, so the job ending is a matter of when. The supervisor submits a
replacement, waits for the scheduler, and re-points the forward. Every piece of
session state lives on the workstation, so there is nothing on the cluster to
lose — the outage is a pause.

The tunnel is **supervised, not started once** (`tunnel-supervisor.sh`). `low`
is preemptible, so the job can be requeued onto a different node mid-session;
the supervisor re-reads `logs/endpoint.txt`, notices the move, and re-forwards.
Measured: kill the forward and the endpoint serves again in **4 seconds**.

Three things it has to get right, each learned the hard way:

- **An unanswered question is not an answer.** The first version exited when one
  `squeue` came back empty — indistinguishable from a throttled ssh, which this
  login node does. Only five *consecutive authoritative* "no running job"
  replies end supervision; a transport failure is ignored.
- **One supervisor, not one per `up`.** Two of them fight over the local port,
  and the loser backs off — that alone turned a 4s reconnect into 24s.
- **Reap the orphaned `ssh`.** Killing a supervisor leaves its child holding the
  port, which the replacement then cannot bind.

**Authentication is off by default**, at the operator's instruction — this is a
trusted network. What that means concretely, since it is not nothing:

- SSH to compute nodes is refused here (verified), so the server **cannot** bind
  loopback and be reached by a jump. It has to listen on the node's interface.
- That port is reachable from the login node — `</dev/tcp/NODE/8080` succeeds
  from there — so **any account that can log into this cluster can use the
  model**.

`QWEN_REQUIRE_KEY=1` turns it back on. The key is generated per deployment and
`opencode_slurm` picks it up and configures the client automatically, so it costs
no manual step either way; when it is off, any stale key file is removed so a
client cannot send a token the server no longer expects.

The cluster provider is added to OpenCode as `qwen-slurm/...` **without** becoming
the default: the local 4090 stays default, so losing the cluster job never
leaves the client pointed at a dead endpoint. Pick a `qwen-slurm/` model when you
want the cluster.

### Setup jobs ask for no GPU

Compiling CUDA needs `nvcc`, not a device, and downloading weights needs
neither. `setup-slurm.sh` therefore requests **no** `--gres`, so it schedules
against all 168 nodes in `low` instead of queueing behind the 86 GPUs that the
serving job actually wants. The first version asked for a Blackwell and sat
`PENDING`; without it, the same job started immediately.

## Git

`git` 2.34.1, with `url.git@github.com:.insteadOf https://github.com/` already
set globally — so HTTPS GitHub URLs are rewritten to SSH and authenticate with
`~/.ssh/id_rsa`. Verified: GitHub answers `Hi berlinguyinca!`. There is no `gh`
CLI and no credential helper.

`user.name` and `user.email` were **unset**, which fails any commit made on the
cluster; they are now configured. Clone work into `<shared-storage>`,
never `$HOME` (3.3 GB free).

### Scheduling

Short jobs backfill; long ones wait. A 3-hour, 32-CPU request sat at `(Priority)`
with a two-hour estimate while GPUs were visibly idle; the same job at 1 hour and
16 CPUs started immediately. Ask for what you need and no more.

**Ask for very little host RAM.** Memory, not GPUs, is what this cluster runs
short of. Sixteen Blackwell GPUs were idle while a `--mem=64G` request queued
with a four-hour estimate, because the two allocatable Blackwell nodes had ~5 GB
of RAM left between them. And the request was nonsense anyway: **`llama-server`'s
measured RSS is 154 MB**. The 67 GB that accounting reports for one job is page
cache from copying 57 GB of weights to local NVMe — reclaimable, and charged to a
job that died after 68 seconds without loading a model. The serving job now asks
for 8 GB and 8 CPUs.

**Let the scheduler pick the GPU.** `opencode_slurm` defaults to `--gpu auto`,
which probes each candidate with `sbatch --test-only` (allocating nothing) and
takes the earliest start. Measured inside one minute:

| GPU | could start |
|---|---|
| a6000 | 13:56 |
| nvidia_a100_80gb_pcie | 14:29 |
| nvidia_a100-sxm4-80gb | 16:52 |
| 6000_blackwell | 17:20 |
| a100 | 20:14 |
| nvidia_l40s | next day |

Pinning one type is how a session waits half a day while five other GPUs sit
idle. Every candidate has at least 46 GiB, which is what the Q8 build needs once
the KV cache is sized correctly; the 32 GiB RTX 5000 Ada is deliberately
excluded.

**An estimate is not a promise.** It is made once and the queue moves under it —
one job submitted on a 15-minute estimate drifted to "tomorrow 00:50" while a
fresh probe showed another type could start eight hours sooner. So the choice is
re-probed after ten minutes of queueing and switched if another type is
materially better, bounded to two switches: cancelling forfeits queue position,
and thrashing is worse than waiting. Recovery re-probes the same way, so an
expired allocation is not re-requested on a card that has since become
congested.

### The client is configured from what the server publishes

The serving job copies the preset it generated to
`logs/router-presets.active.ini`, and `opencode_slurm` configures OpenCode from
**that**, never from a template. Configuring from a template written for the
largest card is not a cosmetic mismatch: against a smaller one it advertises
models the server does not have (`400 model 'qwen3.8-27b-256k' not found`) and,
worse, a context the pool cannot fund — and over-committing a shared pool fails
*every* live session, not just the greedy one. Observed exactly that: client
told 262,144, server serving 131,072.

### The preset sizes itself to the card

Since `--gpu auto` takes whichever GPU can start soonest, the job does not know
its card until it runs — 96 GiB Blackwell one time, 46 GiB L40S the next.
`gen-preset.py` reads `nvidia-smi` and computes pool, slot count and KV type
from the VRAM it finds, drops tier aliases that no longer fit, and refuses
outright on a card too small for the weights.

| VRAM | pool | slots | KV |
|---:|---:|---:|---|
| 97,887 MiB (Blackwell) | 262,144 | 8 | f16 |
| 81,920 MiB (A100 80 GB) | 262,144 | 8 | f16 |
| 46,068 MiB (L40S) | 131,072 | 4 | f16 |
| 24,564 MiB | — | — | refused |

**Get the KV arithmetic right or this silently overcommits.** For this hybrid
architecture it is

```
KV bytes/token = 2 × 16 full-attn layers × 4 kv_heads × 256 head_dim × bytes/elem
```

— 32 KiB/token at fp8 and therefore **64 KiB at f16**. The full 262,144 context
is 16 GiB of KV, not 8. A 46 GiB card holding 28.6 GiB of weights has no room
left for the recurrent-state cache, and the failure reads as
`failed to allocate buffer for rs cache`, which looks like a corrupt model
rather than a budget that was never checked.

### A crash worth knowing about, and why the fix is permanent

The model child died twice with a silent `exit(1)` — no signal, no assert, no
CUDA error, while serving at 45 tok/s. Both times the preceding line was:

```
slot get_availabl: selected slot by LCP similarity, f_sim_best = 0.349 (> 0.100 thold)
```

Three runs, one variable at a time:

| build | `slot-prompt-similarity` | failures | child exits |
|---|---|---:|---:|
| master | 0.10 | 11 | yes |
| v0.1.2 + portable CPU | 0.10 | 8 | yes |
| v0.1.2 + portable CPU | **0.0** | **0** | **0** |

That was a mitigation with an unknown cost, and the docs promised a retest on a
newer build. The retest happened on 2026-08-19, and it answered a better
question than the one asked: **the feature does nothing here even when enabled.**

Driving b10434 with `--parallel 4`, `--slot-prompt-similarity 0.1` and 240
concurrent requests carrying deliberately similar 11k-token prompts:

```
0 selections by LCP similarity
264 selections by LRU
```

It never engaged, so the reproduction could not have crashed — and the cost of
turning it off, which these docs described for weeks as "losing cross-slot
prompt-cache reuse", is nothing. The reuse that matters comes from ordinary
exact-prefix caching, which this does not touch: 36,998 of 37,511 tokens served
from cache.

Upstream agrees the feature is a liability: [PR #22083][sps-pr] disables it when
`--cache-idle-slots` is on or `--parallel 1`, because it causes cache
*thrashing* rather than sharing, and [issue #17673][sps-issue] reports the same
"always LRU despite high similarity" behaviour seen here.

So `0.0` is not pending work. Revisit if the upstream default changes — not on a
schedule.

[sps-pr]: https://github.com/ggml-org/llama.cpp/pull/22083
[sps-issue]: https://github.com/ggml-org/llama.cpp/issues/17673

## What this hardware changes

The 24 GiB reference build is one long fight with capacity. At 96 GiB, three of
its compromises are simply dropped — `gen-preset.py` emits the preset below
for a 96 GiB card:

| | RTX 4090 (24 GiB) | RTX 6000 Blackwell (96 GiB) |
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
