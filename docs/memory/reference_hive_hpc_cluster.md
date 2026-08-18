---
name: hive.hpc.ucdavis.edu — GPU access, queues and layout
description: Measured layout of the UC Davis hive cluster; GPU jobs must go to partition low with account publicgrp, and the gpu-* partitions are not submittable
type: reference
wing: synthesis
drawer_class: fact
---
Measured 2026-08-18 from `gw@hive.hpc.ucdavis.edu`. Full playbook:
`llm/linux-qwen38/hive/README.md`.

**Submitting a GPU job:**

```bash
sbatch -p low -A publicgrp --gres=gpu:6000_blackwell:1 -t 01:00:00 ...
```

- The six `gpu-*` partitions (`gpu-a100`, `gpu-6000-blackwell`, ...) are
  `AllowAccounts=ALL` and still reject everything: *"Invalid account or
  account/partition combination"*. The associations only cover `high`
  and `low`. GPUs are reached from those with `--gres`.
- **`metabolomicsgrp` cannot have a GPU** — its QOS carries
  `gres/gpu=0` and requests fail with `QOSGrpGRES`. Use `publicgrp`.
  `publicgrp-low-qos` has no limits; `publicgrp-high-qos` allows 5 GPUs.
- `low` = 7 days, 86 GPUs, **preemptible** (PriorityTier=10,
  PreemptMode=REQUEUE, GraceTime=130s). `high` = 30 days but only 12
  GPUs and `--gres=gpu:1` estimated a start **a month out**.
- GRES names are inconsistent across nodes of the same model: `a100`,
  `nvidia_a100-sxm4-80gb`, `nvidia_a100_80gb_pcie`,
  `nvidia_a100-pcie-40gb`, `6000_blackwell`,
  `nvidia_rtx_pro_6000_blackwell_max-q_workstation_edition`,
  `nvidia_l40s`, `a6000`, `nvidia_rtx_5000_ada_generation`. A wrong name
  is an allocation failure, not a fallback.
- Short jobs backfill; long ones do not. 3h/32cpu queued for hours next
  to idle GPUs; 1h/16cpu started immediately.
- `sbatch --test-only` establishes all of this in minutes and allocates
  nothing.

**Hardware:** RTX PRO 6000 Blackwell Max-Q = 97,887 MiB, cc 12.0
(sm_120, so FP8 and NVFP4). A100 80GB SXM is *faster* for
bandwidth-bound inference (2039 vs 1792 GB/s) but sits `PLANNED` while
Blackwell allocates in seconds.

**Environment traps:**

- **Slurm is not on `PATH` in a non-login shell.** `ssh host 'sinfo'`
  fails; `ssh host 'bash -lc "sinfo"'` works. This breaks automation
  that shells in non-interactively.
- `$HOME` is 20 GB with ~3 GB free — it cannot hold a model. Group space
  is `/quobyte/metabolomicsgrp`; this user's work lives in
  `/quobyte/metabolomicsgrp/it/llm`.
- **The login node's `/tmp` is not the compute node's `/tmp`** — a
  script staged there fails with `No such file or directory` in the job.
- Compute nodes **do** have outbound internet (huggingface.co returns
  200, no proxy) and see `/quobyte`, plus 3.5 TB of node-local NVMe at
  `/scratch`. Clone and build there, install to `/quobyte`: a source
  checkout on the parallel filesystem spends minutes in `git clone`.
- Modules: `cuda/13.3.0` (default), `gcc/13.2.0`, `cmake/3.28.1`,
  `python/3.11.9`. `uv` is at `~/.local/bin`; `apptainer` exists; there
  is no docker and no prebuilt llama.cpp.

**One-command driver:** `llm/linux-qwen38/hive/opencode_hive` acquires a
GPU, serves, tunnels, configures OpenCode, and launches it. The tunnel is
outbound from the workstation, so NAT/firewall there is irrelevant:
`127.0.0.1:11111 -> hive login -> compute node:8080`. It uses local port
**11111**, deliberately not 8080, because the local RTX 4090 router owns
8080 and a forward onto it would silently shadow the local node — every
"local" request would quietly execute on the cluster. The tunnel is
supervised: `low` is preemptible, so the job can be requeued onto a
different node and the supervisor re-reads `logs/endpoint.txt` and
re-forwards. The hive provider is added to OpenCode without becoming the
default, so a lost job never leaves the client on a dead endpoint.

**Binaries built from modules need those modules at RUN time.** llama.cpp
compiled with `gcc/13.2.0` requires `GLIBCXX_3.4.32`, which Ubuntu
22.04's system libstdc++ does not provide (it stops at 3.4.30). The
serving job must `module load cuda/13.3.0 gcc/13.2.0` and put the install
`lib/` on `LD_LIBRARY_PATH`; loading only cuda gives *"version
`GLIBCXX_3.4.32' not found"*, which reads as a broken binary rather than
a missing runtime toolchain. Prove the binary runs BEFORE publishing an
endpoint for it — the first attempt advertised node, port and tunnel
command and only then died, so the client forwarded to a port nothing
ever listened on.

**Multiplex SSH, or the login node throttles you into a phantom bug.**
One connection per poll (job state, endpoint file) gets rate-limited on
a campus-shared login node: connections fail with
`kex_exchange_identification: read: Connection timed out`, helpers
return empty, and the driver reports "waiting for the scheduler" for a
job that is plainly RUNNING. Use `ControlMaster=auto` +
`ControlPersist`, and poll every 30s rather than every 10s.

**`hf download REPO --include A B` silently ignores `--include`.** A
second positional argument is read as an explicit filename, which
disables include-globbing entirely — *"Ignoring `--include` since
filenames have been explicitly set"* — so a job that exits 0 can fetch
one 885 MiB projector instead of the 29 GiB asked for. Pass exact
filenames positionally, and assert a size floor afterwards: "the command
exited 0" is not the claim "the weights are here".

**Setup jobs must not request a GPU.** Compiling CUDA needs `nvcc`, not a
device, and downloading weights needs neither; asking for a Blackwell
left the job `PENDING` while a GPU-free submission of the same work
started immediately against all 168 nodes.

**Modules are Environment Modules 5.5.0 (Tcl), not Lmod** — `module`/`ml`
work, but `module spider` and hierarchical Lmod syntax do not.
`MODULEPATH` has seven CVMFS roots, mounted on compute nodes too.

**Built llama.cpp binaries need two runtime paths**: `LD_LIBRARY_PATH`
must include the install `lib/` (each tool is a thin driver plus
`libllama-<tool>-impl.so`) *and* `module load cuda` must be in effect for
`libcudart.so.13`. Missing either reads as a broken build ("error while
loading shared libraries") when the build was fine. A `set -e` script
that verifies with `--version` before downloading weights will abort a
perfectly good build and leave nothing staged.

**Git** already has `url.git@github.com:.insteadOf https://github.com/`
globally, so HTTPS GitHub URLs authenticate over SSH with `~/.ssh/id_rsa`
(verified: *Hi berlinguyinca!*). No `gh` CLI, no credential helper.
`user.name`/`user.email` were unset — now set to Gert Wohlgemuth
<wohlgemuth@ucdavis.edu>. Clone into `/quobyte/metabolomicsgrp/it`, never
`$HOME`.

**Both routers publish the same four-way matrix**, as context tiers
aliased onto one loaded model (switching tier is free; switching family
costs one reload, since `--models-max 1`):

| preset | hive (96 GiB, Q8) | local 4090 (24 GiB, Q5) |
|---|---|---|
| `qwen3.8-27b` | 262,144 pool, 8 slots | 180,224 pool, 6 slots |
| `qwen3.8-27b-vision` | 196,608 | 98,304 |
| `qwen3.8-27b-uncensored` | 262,144 | 163,840 |
| `qwen3.8-27b-uncensored-vision` | 196,608 | 98,304 |

Read the smallest tier as "fast" and the largest as "deep" — same
weights and same per-token speed, but the small tier funds several
concurrent sessions and answers in seconds while the large one funds one
and spends minutes prefilling. `qwen3.8-27b-abliterated` is retained as
an alias of the uncensored text preset so pre-rename selections resolve.

**Default selection must break ties toward the plainest preset.** Ranking
ids as strings put `qwen3.8-27b-uncensored-40k` above `qwen3.8-27b-40k`
at equal context and silently made an abliterated model the default for
every new session.

Related: [[feedback_shared_kv_pool_has_no_admission_control]] and
[[feedback_context_floor_kills_small_tiers]] — 96 GiB removes the
capacity fight but not the need for client-side tier rationing.
