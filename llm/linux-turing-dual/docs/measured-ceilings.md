# Dual-Turing node: what was measured, and what was only predicted

Nothing in this file may state a performance number that a real request did not
produce. Predictions are labelled as predictions until a measurement replaces
them. "Allocates", "starts" and "works at length" are three different claims and
only the third one counts.

---

## Multi-GPU defect ledger

A second card exposed a whole family of defects in tooling that had only ever
run on one. All of them shared one shape: read `nvidia-smi`, keep the first row,
treat it as the machine's answer.

| site | field | what it did on two 11 GiB cards |
|---|---|---|
| `select-quant.py` `detect_memory_mib` | `memory.total` | reported 11264 MiB and concluded no quantisation fits |
| `serve-profile.sh:48` (llama.cpp) | `memory.free` | refused to start: ~11000 free against a 20000 floor |
| `serve-profile.sh:107` (vLLM) | `memory.free` | same |
| `measure-ceiling.sh:48, :229` | `free`, `used` | would have measured the ceiling against half the VRAM |
| `measure-slot-frontier.sh:90` | `memory.free` | would have priced a seat against half the VRAM |
| `bench-context-sweep.sh:45` | `memory.free` | same |
| `install-node.sh:53` | `memory.total` | reported half the card capacity during provisioning |
| `setup-linux-qwen38.sh:42` | `memory.total` | same |

Two `head -1` reads were **left alone deliberately**:
`setup-linux-qwen38.sh:40` (`driver_version`) and `:41` (`name`). Every card on a
host shares one driver, and the first card's name is the key convention
`gpu-registry.json` uses.

**Why the fix is not "subtract more headroom".** The per-card cost — compute
buffers plus the CUDA context — is charged by scaling the existing
`--reserve-mib`, not by shrinking the detected budget. Doing both double-counts
headroom and silently costs a whole rung of context; this project has made that
mistake once already. One device leaves the reserve untouched, so the 24 GiB
node's measured numbers do not move.

**The aggregate is not always the right bound.** A model *pinned* to one card is
limited by the smallest card, not by the sum — 22 GiB across two cards does not
hold a 16 GiB model that may not be split. `per_card_ceiling()` and
`vram-guard.sh --min-per-card` exist for exactly that case.

---

## What gates this node in CI

`llm/` appeared in none of the twelve workflows before this node existed, so the
leak guard had never gated a commit. The `llm-node-checks` job in
`.github/workflows/python.yml` now runs, on every push:

| gate | command |
|---|---|
| structural checks (incl. leak guards) | `bash llm/linux-turing-dual/tests/test_structural.sh` |
| unit suites, no accelerator | `python3 -m pytest -q -p no:cacheprovider llm/linux-turing-dual/tests llm/linux-qwen38/tests` |

Deliberately **not** in CI: `test_smoke.sh` and `test_vision.py` need a live
server on a real GPU, and `QT_LEAK_PATTERNS` is left unarmed because arming it
would require committing the site's real identifiers to a workflow file — which
is the thing the guard exists to prevent. Check 7 of the structural suite is the
secret-free companion: no literal IPv4 under the node directory at all, since
every address comes from `site.conf` at runtime.

A note on collection: `llm/linux-qwen38/tests/conftest.py` used to ignore
`test_*.py` wholesale to keep its standalone script-suites out of pytest. That
silently swallowed a genuine pytest module added beside them — it passed when
named explicitly and never ran under directory collection. Ignoring is now by
name, with a `test_unit_*.py` convention for real pytest modules.

---

## Host cleanup — Tier A, measured

The host had been a Docker and compute server for years. Recorded before and
after so the cleanup is a measurement rather than a claim.

| | before | after Tier A |
|---|---:|---:|
| root filesystem used | 272 G (84%) | **251 G (78%)** |
| root filesystem free | 55 G | **75 G** |
| apt packages (`ii`) | 2875 | **2618** |
| snaps | 31 | **21** |
| kernels in `/boot` | 5 | **2** |
| journal | 3.1 G | 200 M cap |
| Xorg holding GPUs | yes, both cards | **no** |

`gh` is a snap and survived deliberately; `snapd` was kept for it rather than
removed wholesale.

### A bug this run found in its own cleanup script

The kernel policy protected `6.8.0-101`, `-111` and `-138`, and the explicit
purge honoured that list. `apt-get autoremove --purge`, two steps later,
removed `6.8.0-101` anyway — an auto-installed kernel that nothing depends on
is precisely what autoremove exists to collect.

Refusing to purge a package does not protect it from autoremove. The script now
runs `apt-mark manual` on every protected kernel *before* autoremove, and the
surviving kernels on this host were marked manual retroactively;
`autoremove --dry-run` now proposes no kernel removal.

The outcome was survivable by luck rather than design: `6.8.0-111` was running,
known-good, and still installed, so a fallback existed. `6.8.0-101` had never
been booted on this host either, so nothing verified was lost — but the barrier
did not hold, and a barrier that holds only when you are lucky is not a barrier.

## Host cleanup — Tier B and C, measured

| | before Tier A | after B/C |
|---|---:|---:|
| root filesystem used | 272 G (84%) | **150 G (46%)** |
| root filesystem free | 55 G | **177 G** |
| bulk array used | 9.5 T (69%) | **4.4 T (33%)** |
| Docker images | 97 (4.8 T) | **0** |
| Docker containers | 2 (522 G) | **0** |
| Docker volumes | 64 (70 G) | **64 (70 G) — untouched** |
| human accounts | 5 | **1** |

**122 GiB reclaimed on the root filesystem and 5.1 TiB on the bulk array.**
The acceptance criterion was root below 50%; it finished at 46%.

Docker volumes were deliberately not pruned. Sixty-four of them hold data, one is
Postgres-shaped, and volumes are where databases live — reclaiming another 70 GB
is not worth being the reason a database went missing. That is a separate,
per-volume review, not part of a cleanup.

Tier B moved the two 2019 libvirt qcow2 images (36 GiB) to
`<bulk-array>/archive/libvirt-images` with all twelve domains left defined, and
deleted ~20 GiB of stale JetBrains caches.

### LM Studio: removed, not adopted

The store held `Qwen3.5-9B-Q4_K_M.gguf` and a 35B-A3B MoE, and the first version
of this step relocated it to the fast array to save a download. The operator
asked for a clean slate instead, so all three LM Studio paths were removed and
both served models are fetched fresh against pinned revisions.

That is the better outcome on provenance grounds regardless: the on-disk 9B came
from a different uploader than `model-artifacts.yaml` pins, and a file with the
right name is not an identity. After removal there is no `.gguf` anywhere under
`/home`, `<nvme-array>` or `<bulk-array>` — the node starts from nothing.

`Qwen3.5-35B-A3B-Q4_K_M` (21.17 GB) is recorded here as a **rejected candidate,
not a loss**: a ~3B-active MoE would decode far faster than the 27B dense-hybrid,
but 19.7 GiB of ~20.5 GiB usable leaves under 1 GiB for KV plus per-card compute
buffers, so it cannot serve a 40k seat on two 11 GiB cards. It becomes the
interesting option at three cards.

### Two bugs this step found in itself

**The move aborted and the pipe hid it.** `<nvme-array>` is `root:root 0755`, so
an unprivileged `mv` onto it failed with EACCES after Tier B.1 had already
succeeded. `set -e` aborted correctly — but the run was piped to `tail`, so the
reported exit status was the pipe's `0`. A background pipeline reports the last
stage's status, so the gate's own final line is what must be read, never the
pipeline's exit code.

**The enumeration ran unprivileged.** `/var/lib/libvirt/images` is root-only, so
`ls .../*.qcow2` as a normal user found nothing and the script cheerfully
reported "no qcow2 images present" — silently skipping 36 GiB while claiming
success. It now enumerates under `sudo`. A cleanup that cannot see what it is
cleaning will always report success.

## Reboot: what it proved

| | value |
|---|---|
| kernel | `6.8.0-138-generic` (first boot of it on this host) |
| default target | `multi-user.target` |
| driver / CUDA | 580.173.02 / 13.0 — `nvidia-smi` works, the NVML mismatch is gone |
| cards | 2 x NVIDIA GeForce RTX 2080 Ti, 11264 MiB each, **compute_cap 7.5** |
| free VRAM | **10820 MiB on each card**, 1 MiB used |
| Xorg / compute apps | none |
| topology | `PHB` — both on one host bridge, NUMA node 0 |
| NVLink | **not fitted** (`nvidia-smi nvlink -s`: all links inActive) |

**Usable total is 21640 MiB = 21.13 GiB**, slightly more than the ~20.5 GiB the
budget assumed, because a headless host leaves only ~444 MiB per card to the
driver.

`--split-mode row` is therefore out of scope for good: with no bridge, cross-card
traffic crosses PCIe, where row-split typically costs more than it returns.

### The multi-GPU guard, proven on the real cards

This is the whole defect family, measured rather than argued:

| read | value | against the 19000 MiB floor |
|---|---:|---|
| sum across both cards | 21640 MiB | **PASS** |
| first card only (the old code) | 10820 MiB | **REFUSE** |

The unfixed launcher would have refused to start this node, and the message
would have blamed VRAM — the one thing that was fine.

### Barrier 2, in the end

Moot rather than exercised: `6.8.0-101` had already been lost to autoremove
before the reboot, so the fallback that actually stood behind this boot was
`6.8.0-111` — running, proven, pinned `manual`, and still installed. The boot
succeeded first time, so nothing needed it.

## Toolchain: why CUDA 12.0 and gcc-12

Ubuntu 24.04 ships `nvidia-cuda-toolkit` 12.0.140, and that `nvcc` refuses gcc
13 (it supports up to 12.2) — while gcc 13 is the distribution default. So the
build installs `gcc-12` and passes it as `CMAKE_CUDA_HOST_COMPILER`.

An older toolkit against a newer driver is the safe direction: the 580 driver
provides a CUDA 13 runtime and runs code built by 12.0 without complaint. `sm_75`
is old enough that no recent toolkit feature is wanted, so adding NVIDIA's own
repository to get CUDA 13 would be complexity bought for nothing.

`GGML_NATIVE=OFF` is set deliberately — a native build targets the build host's
CPU and produces a binary that runs only on the machine that compiled it.

## Model switch cost

_Filled by Task 12 Step 7._

## Measured throughput and verified ceilings

_Filled by Task 13. Until then the only figures available are roofline
predictions at the project's calibrated 0.8 efficiency: ~30 tok/s for the 27B at
UD-Q4_K_M and ~87 tok/s for the 9B at Q4_K_M, from 616 GB/s per card._
