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

## The systemd sandboxing ceiling on a CUDA host

`ProtectSystem=full`, never `strict`. Everything below looks like hardening and
produces a unit that restart-loops:

| setting | why it is not set |
|---|---|
| `ProtectSystem=strict` | blocks the NVIDIA driver's `/proc` and `/sys` access |
| `PrivateDevices=true` | hides `/dev/nvidia*`, so CUDA cannot initialise |
| `DevicePolicy=closed` | same |
| `MemoryDenyWriteExecute=` | breaks the CUDA JIT |
| `RestrictAddressFamilies=` | the driver uses netlink; the failure is opaque |

What *is* set: `NoNewPrivileges`, `PrivateTmp`, `ProtectHome`,
`ProtectControlGroups`, `ProtectKernelLogs`, `RestrictSUIDSGID`,
`LockPersonality`, `UMask=0027`, and a single `ReadWritePaths`.

Check 5 of `tests/test_structural.sh` fails the build if any forbidden setting
reappears, and it was fired in both directions to confirm it works. The reasons
also live in the unit as comments, because the next person to "tighten" this file
will read the unit, not this document.

The API key arrives via `LoadCredential`, not `Environment=`. Any local user can
read an `Environment=` value out of `systemctl show`.

## The stats surface

Two units. `qwen-turing@router` serves inference; `qwen-turing-dashboard` serves
the page. The dashboard `Wants=` but does not `Require=` the inference unit,
because showing that the node is *down* is part of its job.

```
dashboard.py --host H --port P --metrics-url URL --api-key-file FILE
collect-stats.py [--metrics-url URL] [--api-key-file FILE]     # prints JSON
```

`dashboard-run.sh` is the unit's entry point and exists only so the unit does not
need to know about `site.conf`; it resolves the bind address and both ports from
the site config and the credential from `LoadCredential`.

| shown | source |
|---|---|
| prompt tokens, generated tokens, tok/s | llama.cpp `/metrics` |
| KV-pool utilisation, requests processing/deferred | llama.cpp `/metrics` |
| per-card utilisation, VRAM, temperature, power | `nvidia-smi --query-gpu` |
| whether a model is resident at all | presence of `/metrics` output |

`GET /` is public; every number on it comes from `GET /api/stats`, which requires
the bearer key and compares it with `hmac.compare_digest`. Counters are cumulative
since the server started and reset when a model switch reloads it — which is why
the page says so in its own footer rather than looking like data loss.

## Queue observability — resolved

The design rested on a metric that had not been checked: a six-request burst
queued demonstrably (completions arrived in pairs) yet one-second sampling never
saw `requests_deferred` above zero.

Re-probed at **200 ms** with **40k-token prompts** — a short prompt starts and
finishes between two samples and makes an unusable metric look merely quiet:

| t | `requests_processing` | `requests_deferred` | slots busy |
|---:|---:|---:|---:|
| +0.03 s | 0 | 0 | 0 |
| +7.93 s | 1 | 0 | 2 |
| **+8.16 s** | **2** | **4** | **2** |
| +54.61 s | 2 | 3 | 2 |
| +67.04 s | 2 | 2 | 2 |
| +79.11 s | 2 | 1 | 2 |
| +81.21 s | 2 | 0 | 2 |

**Verdict: `requests_deferred` is usable.** It peaked at 4 — six requests against
two slots — and drained monotonically. `requests_processing` caps at the slot
count and `requests_deferred` carries the overflow, so
`outstanding = processing + queued` tracks every in-flight request and the
`/slots` fallback written into the plan is not needed.

The original miss was the *sampling*, not the metric. Recorded because the
instinct to blame the instrument would have led to building a worse design on a
fallback that was never required.

Completions observed at +53.6, +66.9, +78.8, +81.0, +81.3, +81.4 s: six
completions, which is what the rolling window counts as decreases in
`outstanding`.

---

## The stats surfaces, measured

| surface | auth | verified |
|---|---|---|
| `/` | key | 200 |
| `/api/stats` | key | **401 without, 200 with** |
| `/api/queue` | none | 200, and contains no `/` and no model name |
| `/status` | none | 200 |
| `/v1/models` | none | sanitised — no `/`, `status` dropped |
| `/api/queue-headers` | none, internal | **204 + 5 `X-Queue-*` headers** |

**The header endpoint costs 10.8 ms.** 200 sequential calls completed in 2.16 s,
which is the proof that it reads a cached snapshot and performs no I/O. nginx runs
it before every inference request, so anything that scaled with backend latency
here would add that latency to every completion.

**It survives a dead backend.** With `qwen-turing@router` stopped it still returns
204 with all five headers, and `/api/queue` and `/status` still return 200
reporting `model_loaded: false`. nginx treats a non-2xx `auth_request` as a
rejection, so this property is what stops a dashboard restart from becoming an
inference outage.

---

## The proxy, measured

nginx 1.24.0 with `http_auth_request_module`. One public listener; both backends
retreated to loopback.

| | before | after |
|---|---|---|
| llama.cpp | `0.0.0.0:8080` | **`127.0.0.1:8090`** |
| dashboard | `<node-addr>:8081` | **`127.0.0.1:8081`** |
| nginx | — | **`0.0.0.0:80`** and `0.0.0.0:8080` |

Verified through the proxy:

| check | result |
|---|---|
| real completion | `'OK'` |
| `X-Queue-*` on a normal response | 5 headers, `Slots: 2` |
| **`X-Queue-*` on a STREAMED response** | **5 headers, 73 SSE chunks, `Transfer-Encoding: chunked`** |
| 40k needle | retrieved, **43.7 s** |
| one port serves both | `/` 200, `/status` 200, `/v1` 200 |
| `/api/stats` unauthenticated | 401 |
| `/models` (unsanitised twin) | **403** |
| `/v1/models` | sanitised, contains no `/` |
| `<node-addr>:8090` and `:8081` from off-host | **connection refused** |
| **inference with the dashboard STOPPED** | **200** |
| 6 concurrent via `auth_request` | all 200, 0.86–2.07 s |

Two of those matter more than they look:

**Inference survives a dead dashboard.** nginx turns a failed `auth_request` into
a client error, so without the `@inference_no_headers` fallback a dashboard
restart would have been an inference outage. The 200 above is that fallback
working.

**Headers and streaming were checked together, not separately.** `add_header`
fires when response headers are sent, which for SSE is before the body flows —
two tests each passing alone would not have proven the combination.

A cold snapshot briefly reported `X-Queue-Slots: 0` before any model was resident.
That is correct rather than a bug — slots are a property of the loaded instance —
but it is why the header set is read after a warm-up when verifying.

---

## The 100k tier and prefix caching, measured

### Prefix caching is worth ~10x, and one slot is what secures it

A controlled comparison, same ~20k shared prefix, three sequential calls:

| | call 1 | call 2 | call 3 |
|---|---|---|---|
| **one slot** (`qwen3.8-27b-100k`) | 20.0 s, cached 0 | **2.1 s, cached 19,425** | 2.0 s, cached 19,425 |
| two slots (`qwen3.8-27b`) | 27.2 s, cached 0 | 20.8 s, **cached 0** | 2.4 s, cached 19,425 |
| control, `cache_prompt: false` | 20.4 s, cached 0 | — | — |

This **confirms** the round-robin explanation rather than inferring it. With two
slots a single sequential user hits the warm slot every other turn; with one slot
every turn after the first hits it. `slot-prompt-similarity` would steer requests
to the warm slot and is exactly what commit `1cd8c1f4` disabled for crashing the
model child, so it stays at `0.0` and the preset carries the fix instead.

### `cache-reuse` was never doing anything

Removed rather than tuned. The node logged
`cache_reuse is not supported by this context, it will be disabled` at every load
— `--cache-reuse` reuses chunks across a *changed* prefix, and this hybrid model's
Gated-DeltaNet layers hold recurrent state that cannot be partially rewound. After
removal: **zero such warnings**. Ordinary prefix caching, which does work, is what
the table above measures.

### 100k verified at length

| | measured |
|---|---|
| prompt tokens retrieved | **99,710** |
| needle | **retrieved, exact** |
| wall clock, cold (includes a model reload) | **3 m 04 s** |
| `total_slots` / `n_ctx` | **1 / 102,400** |
| VRAM resident | 18,870 MiB of 22,528 (**3,658 free**) |
| predicted resident | 18,702 MiB — within 168 MiB |

**Prefill is faster than first estimated.** The router logs per-chunk progress:
**594–637 tok/s** single-session, decaying with depth as attention cost grows. The
earlier 475 tok/s figure came from the *concurrent* 40k run where two sessions
shared the cards, so it understated single-session prefill. A cold 100k prompt is
therefore ~2.8 minutes of prefill, not 3.5.

Two concurrent 40k sessions on the enlarged 102,400 pool: both HTTP 200 with the
needle, **zero** KV-cache failures, 117 s wall clock.

---

## Config health, measured

The panel reports what the operator asked for — when models are offloading —
from the router's own log:

| signal | observed on this node |
|---|---|
| evictions in the last 2 h | **8**, e.g. `qwen3.5-9b -> qwen3.8-27b`, `qwen3.8-27b-100k -> qwen3.8-27b` |
| unloads | 11 |
| silently disabled options | `cache_reuse` — historical, from loads before its removal; ages out of the 2 h window |
| prompt-cache hit rate | `null` until the current instance has been prompted |

Eight evictions came from this session's own testing, which is the point: with
`--models-max 1` every model switch is a reload, and the counter makes thrashing
visible instead of merely slow.

**The journal needs a group.** The unprivileged service user cannot read it by
default, so the panel degraded — correctly — to `journal_readable: false` and
`events: null`, rendering "event feed unavailable" rather than "0 evictions". The
dashboard unit now carries `SupplementaryGroups=systemd-journal`, which is wider
than needed (systemd has no per-unit journal ACL) and is recorded as a tradeoff in
the unit itself: the dashboard greps three patterns and exposes counts only on the
key-gated page, never on `/status` or `/api/queue`.

---

## Model switch cost, measured

`--models-max 1`, so a model change is a reload. Weights come off the NVMe RAID0.

| action | measured |
|---|---:|
| 9B cold (includes reload) | **4.2 s** |
| 9B warm | 183 ms |
| 27B cold (includes reload) | **7.0 s** |
| 27B warm | 520 ms |
| `qwen3.8-27b-40k` **alias** | 538 ms |

The alias figure is the point: it matches the warm 27B, so switching context tier
costs nothing because it resolves to the same resident weights. Switching *model*
costs one reload, which is seconds rather than minutes.

VRAM confirms the layouts are doing what the presets claim:

| resident | GPU0 | GPU1 |
|---|---:|---:|
| 27B (layer-split, `tensor-split 1,1`) | 8628 MiB | 9714 MiB |
| 9B (pinned, `tensor-split 1,0`) | 6600 MiB | **160 MiB** |

## Measured throughput and verified ceilings

Predictions replaced by measurements. The roofline was `616 GB/s / weight_bytes
x 0.8`.

| model | predicted | **measured** | % of prediction | % of roofline |
|---|---:|---:|---:|---:|
| 27B UD-Q4_K_M, both cards | 29.9 | **28.70 tok/s** | 96% | 76.7% |
| 9B Q4_K_M, one card | 86.8 | **77.92 tok/s** | 90% | 71.8% |

Prompt processing was 50-64 tok/s for the 27B and 157-195 tok/s for the 9B.

Turing lands a little under the project's 0.8 calibration, where a 4090 and an
RTX PRO 6000 both hit ~80%. Recorded rather than explained away: the roofline
constant is a calibration across cards, and this is a data point that it is
slightly optimistic for sm_75.

**Layer split does not sum bandwidth.** The 27B figure is what one card's
616 GB/s yields against 16.46 GB of weights; the second card adds capacity, not
speed. That is why the 9B — which fits one card and is a third of the size — is
2.7x faster.

### Context, verified at length

| claim | result |
|---|---|
| needle at ~14k tokens (control) | retrieved |
| needle at **39,910** tokens | **retrieved**, answer exact |
| **two concurrent** 39,909-token sessions | **both retrieved**, HTTP 200 |
| KV-cache evictions during the concurrent run | **zero** |

The concurrent pair used 79,818 of the 81,920-token pool. The failure mode this
design guarded against — a shared pool with no admission control killing every
live session at once — did not occur, because two 40k seats is exactly one full
pool by arithmetic rather than by luck. Wall clock was 114 s for both.

VRAM at 40k depth was 18,342 of 22,528 MiB, leaving ~4.1 GiB free — more headroom
than the 2.9 GiB the budget predicted, because a headless host returns ~444 MiB
per card.

### Hybrid reasoning changes what a client must send

Both models emit reasoning before content. A request with `max_tokens: 16`
returned **empty content** having spent all 16 tokens reasoning; the same request
with `max_tokens: 400` returned `OK` after 91 characters of reasoning, and with
`chat_template_kwargs: {enable_thinking: false}` returned `OK` in **2 tokens**.

This is not a defect, but a client with a tight token cap will see empty replies
and conclude the node is broken. Either allow headroom or disable thinking.
