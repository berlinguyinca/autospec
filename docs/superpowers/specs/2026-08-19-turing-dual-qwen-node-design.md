# Design: a dual-Turing Qwen inference node

**Date:** 2026-08-19
**Status:** approved, pending implementation plan
**Target:** a 2 x RTX 2080 Ti (11 GiB, sm_75) Linux host, on-demand model
switching, API-key auth, 40k-token sessions, 1-2 concurrent seats.

> **Site coordinates are not in this document.** This repository is public.
> The real hostname, addresses and subnets live in
> `${XDG_CONFIG_HOME:-~/.config}/qwen-turing/site.conf` (mode 600, never
> committed), mirroring the pattern `llm/linux-qwen38/slurm/` already uses.
> Placeholders below read `<node-addr>`, `<node-host>`, `<uplink-if>`.

---

## 0. Why a new node rather than a new profile

`llm/linux-qwen38/` is documented as *"the RTX 4090 build, as measured"* and
every number in it was verified on 24 GiB of Ada. This node is a different
capacity class (22 GiB split across two cards) and a different compute
capability (sm_75: no bf16, no FP8, no Marlin). Overloading that directory
would make its measured claims untrue.

So: a new `llm/linux-turing-dual/`, a fourth row in `llm/README.md`, and the
shared toolkit **reused rather than forked** -- the new node invokes
`../linux-qwen38/scripts/*`, and fixes land there so both nodes benefit.

---

## 1. Hardware and host audit (Phase 0, recorded before changing anything)

| item | observed |
|---|---|
| GPUs | 2 x NVIDIA GeForce RTX 2080 Ti (TU102), 11 GiB each, **sm_75** |
| Aggregate VRAM | 22.0 GiB |
| Memory bandwidth | 616 GB/s per card (vendor figure, **not** measured) |
| CPU / RAM | 32 cores, 125 GiB |
| OS / kernel | Ubuntu 24.04.4, running 6.8.0-111, 6.8.0-138 also installed |
| Model store | `<nvme-array>`: NVMe RAID0, ~497 GiB free -- **weights live here** |
| Root filesystem | 344 GiB, 84% full before cleanup |
| Docker root | on `<bulk-array>` (15 TiB), **not** on the root filesystem |

### Faults found, all blocking

1. **Driver mismatch.** Loaded kernel module 535.288.01; DKMS module on disk
   and userspace both 580.173.02. `nvidia-smi` fails with
   `Failed to initialize NVML: Driver/library version mismatch`. Nothing is
   measurable until this is resolved, and resolving it needs a reboot.
2. **Driver sediment.** Packages for 390, 418, 430, 470, 525, 535 and 580 are
   all installed, plus `cuda-toolkit-10-0`.
3. **Xorg resident on both cards**, holding VRAM and blocking module unload.
4. **No host firewall.** `ufw` inactive; `iptables -P INPUT ACCEPT`.

### Reboot safety (verified, not assumed)

`dkms status` reports `nvidia/580.173.02` **installed for both**
`6.8.0-111-generic` and `6.8.0-138-generic`, and `nvidia.ko.zst` is present
under both module trees. `GRUB_DEFAULT=0` will select the newest kernel
(6.8.0-138), which therefore has a working module. **Without this check a
reboot could have returned a host with no GPU.**

---

## 2. Phase 1 -- cleanup and simplification

The host is an old Docker/compute server. The operator asked for it to be made
"as simple as possible". Work is tiered by reversibility.

### Tier A -- safe, reversible by reinstall (~43 GiB off root)

- Desktop stack: `ubuntu-desktop`, `gnome-shell`, `gdm3`, `xserver-xorg-core`,
  `libreoffice-*`, `thunderbird`, `cups-daemon`.
- **Swap `nvidia-driver-580` for `nvidia-headless-580` + `nvidia-utils-580`.**
  The `-driver-` metapackage pulls in X; the headless one does not. This is
  what makes "headless" a package fact rather than a disabled unit.
- Desktop snaps and every superseded revision: chromium, firefox, cups,
  dbeaver-ce, gthumb, and six stale `gnome-*` platform snaps (~18 GiB).
- Driver sediment from item 2 above, plus `cuda-toolkit-10-0`.
- Old kernels 5.3.0-26, 5.15.0-153, 6.8.0-84, 6.8.0-101. **Keep 111 and 138** --
  111 is running and 138 is the next boot target; both have the nvidia module.
- `/opt` relics: DaVinci Resolve, four 2017-18 JetBrains installs, a stale
  `ideaIU` tarball, monero-gui.
- `jenkins`, `davinci-resolve`, TeamViewer and its logs.
- `journalctl --vacuum-size=200M` (3.1 GiB now), apt cache, obsolete `mlocate`
  (`plocate` is already installed).

### Tier B -- reversible by construction: moved, not deleted (~86 GiB off root)

- `/var/lib/libvirt/images` -- two qcow2 images from 2019 (21 GiB + 16 GiB);
  all eight domains are `shut off`. **Moved to `<bulk-array>`; domains left
  defined.** Nothing is destroyed.
- `~/.lmstudio` (30 GiB) moved to `<nvme-array>`. Inspect first: it may
  already hold GGUF weights that make a download unnecessary.
- Stale JetBrains caches in `$HOME` (~20 GiB).

Net effect on root: **84% -> ~44%**.

### Tier C -- explicitly authorised by the operator

- **Docker prune**: 97 images (~4.8 TB) and 2 stopped containers, including
  `binbase.steinlee` whose 522 GB lives in the container's *writable layer*.
  Three zero-replica swarm services are removed first. Frees `<bulk-array>`,
  where the Docker root lives -- **not** the root filesystem.
- **Account removal**: `leon` (uid 1004), `jvogel` (1003), and the orphaned
  `diego` (1001) and `sajjan` (1002), which have no home directories. Verified
  first: no running processes, no crontabs, no files outside `$HOME`, never
  logged in. `leon` is removed from the `docker` group.
  `/home/leon` (19 GiB) is already mirrored at `<bulk-array>/backup/leon`
  (18 GiB),
  **which is preserved** -- it contains lab data.

### Explicitly NOT touched

- Docker **volumes** (64, ~70 GiB). Volumes are where databases live and one is
  Postgres-shaped. Never blanket-pruned; a per-volume review is a separate task.
- Blockchain chain data in the operator's home (~77 GiB). Chain data re-syncs;
  wallet keys in the same directories do not.
- The 2.5 TB of 2022 archival tars on `<bulk-array>`.

---

## 2.0 Execution order and safety barriers

Cleanup is not a flat list. Three barriers order it, because several Tier A
items are coupled to the reboot and several Tier C items are irreversible.

**The reboot is an explicit barrier.** Everything destructive that can wait
until after a confirmed-good boot, does.

### Barrier 1 -- immediately before the reboot: re-verify DKMS

`dkms status` was checked *before* the package work and reported
`580.173.02` built for both kernels. That is a **pre-purge fact and does not
survive the purge on its own**: swapping `nvidia-driver-580` for
`nvidia-headless-580` in the same transaction that removes `xserver-xorg-core`
can change how `libnvidia-compute-580` resolves, or let apt autoremove
something the DKMS build needs.

So `dkms status` and `ls /lib/modules/*/updates/dkms/nvidia.ko*` are re-run
**after all package changes and before the reboot**. If the module for the
next boot target is missing, the reboot does not happen.

### Barrier 2 -- kernel removal waits for a successful boot

Keep `6.8.0-101` in addition to 111 (running) and 138 (next boot target).
`6.8.0-138` has **never been booted on this host**. Removing 101 before 138
proves itself shrinks the fallback set at exactly the wrong moment. 101 is
removed only after 138 has booted cleanly with working GPUs.

### Barrier 3 -- firewall comes after the reboot, never during

`ufw` is **not** enabled in the cleanup phase. The host default is
`INPUT ACCEPT`, three operator SSH sessions are live, and the box carries
Docker and VLAN bridges. Enabling a default-deny firewall on a machine that is
minutes from a reboot is how a host becomes unreachable.

Sequence: reboot first, confirm GPUs, then add rules with the SSH allow
verified **from a second, independent connection before the first is dropped**,
and a scheduled `ufw disable` timer armed as a dead-man switch until the rules
are confirmed good.

### Barrier 4 -- array health re-checked inside the delete step

`userdel -r leon` destroys `/home/leon`, whose only other copy is
`<bulk-array>/backup/leon`. Array state from an earlier phase is not
sufficient: `cat /proc/mdstat` must show the array clean **and** the backup
path must be readable **in the same step, immediately before** the delete. If
either check fails, the account is left alone.

Order within Tier C: prune Docker (frees `<bulk-array>`, reversible only via
re-pull) *after* the accounts are handled, so an array problem surfaces on the
cheaper operation first.

---

## 3. Phase 2 -- runtime

**llama.cpp**, built from a pinned release tag, `-DGGML_CUDA=ON`
`-DCMAKE_CUDA_ARCHITECTURES=75`, and a portable CPU baseline rather than the
build host's native march.

**Verified before choosing:** `ggml/src/ggml-cuda/fattn.cu` dispatches
flash-attention for `cc >= GGML_CUDA_CC_TURING` and handles `head_dim 256`
(both models here use 256). FA availability is what makes the KV figures in
section 4 real; without it they would need redoing.

### vLLM is rejected, and here is why

Recorded so it is not re-litigated. sm_75 has no bf16 and no FP8. The Qwen3.8
family is hybrid -- 48 of 64 layers are Gated-DeltaNet -- and the GDN/Mamba
kernel path plus 4-bit weight-kernel architecture floors make vLLM on Turing a
research project, not a deployment. The 4090 node keeps its vLLM profiles;
this node does not get them.

---

## 4. Phase 3 -- multi-GPU layout and the VRAM budget

Both cards are symmetric once headless: `--split-mode layer --tensor-split 1,1`.

### KV cost, derived from `config.json` rather than guessed

Both models are `Qwen3_5ForConditionalGeneration`. For the 27B: 64 layers, of
which **16 are `full_attention`**, `num_key_value_heads 4`, `head_dim 256`.

```
bytes/token = 2 (K+V) x 16 full-attn layers x 4 kv_heads x 256 head_dim
```

| KV type | per token | 40,960 tok | 81,920 tok |
|---|---:|---:|---:|
| q8 / fp8 | 32.0 KiB | 1.25 GiB | 2.50 GiB |
| q5_1 | 24.0 KiB | 0.94 GiB | 1.88 GiB |
| **q4_0** | **18.0 KiB** | **0.70 GiB** | **1.41 GiB** |

Only a quarter of the layers grow a cache, which is precisely why two 40k
seats are comfortable here rather than marginal.

### Budget at the chosen configuration

| item | GiB |
|---|---:|
| Total (2 x 11.0) | 22.00 |
| CUDA context, x2 cards | -0.60 |
| Weights, 27B UD-Q4_K_M (16.46 GB) | -15.33 |
| KV pool, 81,920 tokens @ q4_0 | -1.41 |
| GDN recurrent state, 2 slots | -0.30 |
| Compute buffers -- **paid per card** | -1.50 |
| **Margin** | **~2.9** |

`--split-mode row` is not adopted unless `nvidia-smi nvlink -s` shows a bridge
is fitted. Over bare PCIe, row split typically hurts decode.

### Speed, labelled as prediction

Roofline at the project's calibrated 0.8 efficiency:

| model | weights | predicted | note |
|---|---:|---:|---|
| Qwen3.8-27B UD-Q4_K_M | 16.46 GB | **~30 tok/s** | layer split is sequential; cards do **not** sum bandwidth |
| Qwen3.5-9B Q4_K_M | 5.68 GB | **~85 tok/s** | fits one card -- pinned to a single GPU, no split overhead |

Both are replaced by measurements before anything claims them.

---

## 5. Phase 4 -- model roster and transparent switching

llama.cpp router mode: one endpoint, `--models-max 1`.

| served id | weights | layout | pool | seats |
|---|---|---|---|---|
| `qwen3.8-27b` (+ `-40k`, `-80k`) | UD-Q4_K_M, 16.46 GB | both cards | 81,920 | 2 |
| `qwen3.5-9b` (+ `-40k`) | Q4_K_M, 5.68 GB | one card | 81,920 | 2 |

The 9B is the one figure above that is derived rather than measured, so its
budget is shown explicitly. Its `config.json` gives 32 layers of which **8 are
`full_attention`**, `kv_heads 4`, `head_dim 256` -> `2 x 8 x 4 x 256` =
16 KiB/token at q8, **9.0 KiB/token at q4_0**. On a **single 11.0 GiB card**:

| item | GiB |
|---|---:|
| One card | 11.00 |
| CUDA context | -0.30 |
| Weights, Q4_K_M (5.68 GB) | -5.29 |
| KV pool, 81,920 tokens @ q4_0 | -0.70 |
| Compute buffers (one card only) | -0.80 |
| **Margin** | **~3.9** |

`check_presets.py` prices this against the **loaded** server and is the gate
that must pass before the preset is trusted; until it does, the 9B pool figure
is a prediction like any other.

Switching **tiers** of one model is free -- an alias resolves to the same
resident weights. Switching **models** costs one reload, seconds from NVMe.
Weights are pinned by immutable revision in `model-artifacts.yaml`; changing
quantisation is a new profile version, never an edit in place.

### "Shared contexts" -- the honest split

The request was to share context where possible. That splits in two, and the
halves have opposite risk:

- **Within-slot prefix reuse (`cache-reuse`): ON.** Safe, and it is what
  repeated calls carrying the same large preamble actually benefit from.
- **Cross-slot reuse (`slot-prompt-similarity`): 0.0, stays off.** This is the
  exact path disabled in commit `1cd8c1f4` because the model child kept dying
  on it. It is also, unhelpfully, the half that a two-session fan-out would
  benefit from most. It is not silently re-enabled to satisfy a word.

A shared pool has **no admission control**: over-subscribe it and every live
session dies together, not just the greedy one. Rationing is therefore
client-side, which is what the `-40k` / `-80k` aliases exist to declare.

---

## 6. Phase 5 -- security posture

The operator was advised that the chosen uplink is campus-facing (VRRP from a
university range observed on it, tagged VLANs 199/200) and reaffirmed that
choice, and separately asked for no TLS for now. Both are recorded decisions.

**Residual risk, stated once:** on a campus-reachable port without TLS the API
key crosses the wire in cleartext and is capturable on-path. The threat class
is credential replay, prompt extraction, and resource exhaustion -- not the
LAN-only class. Adding TLS later is an increment, not a redesign. Many
institutions additionally require registration of network-exposed services.

Controls, kept deliberately minimal:

| control | detail |
|---|---|
| Auth | `--api-key`, 32 random bytes, root-only file, injected via systemd `LoadCredential`. Never in a unit file, never in git |
| Surface | `--no-webui`; the slots endpoint disabled (`/slots` leaks other sessions' prompts) |
| Firewall | `ufw` default-deny inbound, explicit allowlist, SSH restricted to internal ranges |
| Privilege | dedicated unprivileged system user; `NoNewPrivileges`, **`ProtectSystem=full`**, `ProtectHome`, `UMask=0027` |

### Sandboxing has a hard ceiling on a CUDA host

`ProtectSystem=full`, **not `strict`**. The existing node's unit documents why,
and it was learned the hard way:

| setting | why it is NOT set |
|---|---|
| `ProtectSystem=strict` | blocks the NVIDIA driver's `/proc` and `/sys` access |
| `PrivateDevices=true` | hides `/dev/nvidia*`, so CUDA cannot initialise |
| `DevicePolicy=closed` | same |
| `MemoryDenyWriteExecute=` | breaks the CUDA JIT |

`full` is the strongest setting that leaves the CUDA stack working, and it
still makes `/usr` and `/boot` read-only. A hardening pass that "tightens"
these will produce a unit that restart-loops, so the reasons travel with the
unit as comments.

**`ufw` on this host is not risk-free.** It carries seven VLAN bridges, Docker
swarm bridges and libvirt networks; Docker manipulates `FORWARD` directly and
`ufw` can silently break container networking. Rules must leave the Docker
chains alone, and must be tested before being persisted.

---

## 7. Phase 6 -- statistics and web UI

One systemd service and a static page. **No Prometheus, no Grafana, no
containers** -- the operator asked for simplicity, and llama.cpp already
publishes what is needed.

- From llama.cpp `--metrics`: prompt tokens, generated tokens, tokens/sec,
  KV-pool utilisation, slots busy/idle, request counts, resident model.
- From `nvidia-smi --query-gpu`: utilisation, VRAM, temperature, power per card.

The page shows tokens processed, throughput, per-card utilisation and VRAM, KV
pool pressure, which model is resident, and model-switch events. It is served
behind the same API key and is not a second unauthenticated listener.

---

## 8. Phase 7 -- network

The second interface is addressed from `site.conf`, and NIC ring buffers,
offloads and `irqbalance` are set.

**Honest caveat, recorded so it is not mistaken for a win:** at ~30 tok/s the
token stream is a few hundred bytes per second. 1 GbE is nowhere near the
bottleneck, and network tuning buys essentially zero inference performance
here. The real deliverables on this axis are correct addressing and the
firewall.

---

## 9. Tooling changes (code, not configuration)

1. **`select-quant.py` multi-GPU bug.** It runs
   `nvidia-smi --query-gpu=memory.total` and takes `.splitlines()[0]` -- the
   **first GPU only**. On this host it would report 11 GiB and conclude nothing
   fits. Fix: enumerate all devices, add a device-count notion, and account for
   compute buffers being paid per card.
2. **`serve-profile.sh` multi-GPU VRAM guard.** Line ~47 reads
   `nvidia-smi --query-gpu=memory.free --format=... | head -1` -- the **first
   card only** -- and refuses to start when it is below
   `QWEN38_MIN_FREE_VRAM_MIB`. That default is 20000, tuned for a 24 GiB card.
   On 2 x 11 GiB, card 0 reports ~11000 and **the launcher refuses to start a
   node that would have fitted**. Fix: aggregate across the devices the profile
   will actually use, and make the threshold a per-node value.
3. **`gpu-registry.json`**: new entry `NVIDIA GeForce RTX 2080 Ti` --
   `vram_mib 11264`, `compute_cap "7.5"`, `bandwidth_gbs 616`, `fp8 false`,
   `nvfp4 false`; `measured_tps` filled from benchmarks, not predictions.
4. **`check_presets.py`** must pass against the multi-GPU pool figure.
5. New config keys: `split-mode`, `tensor-split`, `main-gpu`.
6. **Leak guard**: `tests/test_structural.sh` check 22 is case-insensitive over
   `llm/` and `docs/memory/`. A committed hostname or address fails the build.
   A `site.conf.example` must cover every value the loader demands.
7. **`llm/` is in no CI workflow.** Twelve workflows exist and none reference
   it, so the leak guard and preset checks run only when someone remembers.
   The structural tests need no accelerator, so the new node wires them into CI
   -- otherwise a committed hostname reaches a public repository unnoticed.

---

## 10. Acceptance criteria

The project rule is: **never configure a number you have not verified with a
real request.** "Allocates", "starts" and "works at length" are three different
claims.

1. `nvidia-smi` clean after reboot; both cards visible; topology recorded.
2. A real completion from **both** served models.
3. A needle retrieved at ~40k **actual** prompt tokens, not an advertised limit.
4. Two concurrent 40k sessions run clean to completion.
5. `measure-slot-frontier.sh` records what a seat costs; `benchmark.py` records
   tok/s into `gpu-registry.json`.
6. Service survives a reboot unattended.
7. A request with no API key gets 401; a request from outside the allowlist
   does not connect at all.
8. Root filesystem below 50% used.
9. `check_presets.py` and `test_structural.sh` pass.

---

## 11. Three-card path (documented, not built)

At 33 GiB: `16.46 + 5.68` GB of weights plus KV and per-card buffers fits
inside ~27 GiB, so **`--models-max 2` becomes possible and model switching
becomes instant** rather than a reload. Q5_K_M (19.77 GB) also becomes viable
for the 27B. Requires a PSU check first -- three cards at ~250 W plus a
32-core CPU wants a 1200 W-class supply. Adding the card should then be a
config change plus a re-measure, not a redesign.

---

## 12. Out of scope

Vision / mmproj projector (0.93 GB spent on sessions that send no images), an
uncensored variant, `--split-mode row` absent an NVLink bridge, TLS, a
Prometheus/Grafana stack, a `toolkit/` refactor of the 4090 node, and any
per-volume Docker review.
