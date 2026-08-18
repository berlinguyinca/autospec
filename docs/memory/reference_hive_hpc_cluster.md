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

Related: [[feedback_shared_kv_pool_has_no_admission_control]] and
[[feedback_context_floor_kills_small_tiers]] — 96 GiB removes the
capacity fight but not the need for client-side tier rationing.
