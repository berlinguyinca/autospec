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

## Host cleanup — Tier B and C

_Filled by Task 6 Step 7._

## Reboot: what it proved

_Filled by Task 7 Step 5 — kernel, driver, both cards, topology, NVLink presence._

## Model switch cost

_Filled by Task 12 Step 7._

## Measured throughput and verified ceilings

_Filled by Task 13. Until then the only figures available are roofline
predictions at the project's calibrated 0.8 efficiency: ~30 tok/s for the 27B at
UD-Q4_K_M and ~87 tok/s for the 9B at Q4_K_M, from 616 GB/s per card._
