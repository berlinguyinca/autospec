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

**Measured end-to-end, 2026-08-18** (Q8_K_XL, through the tunnel):
45.57 tok/s single stream, 53.22 aggregate at four concurrent clients.
That is **80.0% of the memory-bandwidth roofline** — against the RTX
4090's 79.7% on a different architecture, a quarter of the memory and a
5-bit quant. The 80% constant belongs to the runtime and the model, not
the card, so `bandwidth / resident weight bytes x 0.8` predicts an
untouched machine to within ~10%. Note the 4090 reached a *higher*
aggregate (56.72): the cluster card buys 8-bit weights at the full
262,144 context, not more throughput.

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

**`set -e` + `set -o pipefail` kills a polling script silently.** Any
command substitution containing a pipe aborts the whole script when the
remote side fails — `awk` exiting 2 because a file does not exist yet,
`ssh` exiting 255 because the login node throttled us. The symptom is a
bare `exit 2` with no message, mid-poll. Guard every such substitution
with `|| true`.

**Put an always-listening proxy in front of an SSH tunnel.** If the
listening socket IS the forward, every reconnect or node change reaches
the client as `Connection refused`. A local byte relay that owns the
port and *holds* clients until the upstream returns converts an outage
into latency (measured: 6s round trip across a killed tunnel). Keep it a
byte relay so it carries SSE untouched; it cannot replay a connection
lost mid-response, only ones not yet started.

**A tunnel supervisor must not treat an unanswered query as an answer.**
`[ -z "$jid" ] && exit` ended supervision on the first empty `squeue` —
indistinguishable from a throttled ssh, so one blip killed the tunnel
for the rest of the session. Read ssh's own exit status: a transport
failure is not evidence about the job, and only N *consecutive
authoritative* "no job" answers should end it. Also: each `up` spawning
another supervisor makes them fight for the local port ("Address already
in use" → backoff → 24s reconnect instead of 4s), and killing a
supervisor orphans its `ssh` child, which keeps holding that port.
Replace rather than duplicate, and reap the orphan.

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

**llama.cpp's LCP slot selection crashed the model child.** Twice, a
silent `exit(1)` — no signal, assert or CUDA error, serving at 45 tok/s
one line earlier — immediately after
`selected slot by LCP similarity, f_sim_best = ... (> 0.100 thold)`.
Setting `slot-prompt-similarity = 0.0` in the preset fixed it: 40/40
with zero child exits and zero LCP selections, against 11 and 8 failures
on the two prior runs. A mitigation, not a diagnosis; the cost is losing
cross-slot prompt-cache reuse.

**Never build with `-march=native` on a heterogeneous cluster.** hive
mixes zen3, zen4 and zen5, and the setup job and the serving job are
scheduled independently — so a build landing on zen5 produces a binary
that dies on an older node with `Illegal instruction (core dumped)`.
SIGILL can also arrive *mid-run*, when a kernel using the newer
instructions is first reached, which presents as a random crash rather
than a portability bug and is a strong candidate for the "instance ...
exited with status 1" seen mid-benchmark. Build with
`-DGGML_NATIVE=OFF -march=x86-64-v3` — v3 is the cluster's own declared
baseline, published in its module path
`linux-ubuntu22.04-x86_64_v3`. Record the baseline in the build stamp so
changing it forces a rebuild.

**Build a pinned llama.cpp release, not master.** `git clone --depth 1`
of master is unrecorded and unreproducible, and post-release master is
the likeliest source of the mid-session child crash below. `v0.1.2`
carries every flag this deployment needs (`--models-preset`,
`--models-max`, `--kv-unified`, `--image-min-tokens`). Record the ref
next to the binary — and make *reuse* require the recorded ref to match,
otherwise bumping the pin silently keeps serving the old build and the
pin is decoration.

**llama.cpp's router answers /health while its model child is dead.**
Observed twice: a benchmark failed 11 of 40 items with `500 proxy error:
Could not establish connection` while the supervisor reported health
throughout. The cluster log said `instance name=... exited with status
1` — healthy at 44.7 tok/s one line earlier, no OOM, no diagnostic
(built from llama.cpp master; pinning a release is worth trying). From
outside, a dead child looks exactly like an idle one: `/health` is 200
and every model reports `unloaded`. So probe **capability** (ask for one
token) on a schedule, not just once per connection — the probe also
heals it, because the request makes the router reload the model.

**If the server sizes itself dynamically, the client must be
configured from what it PUBLISHES, not from a template.** Once
`opencode_hive` chose the GPU by earliest start, the preset became
per-card too — but the client was still built from the 96 GiB template.
Against the 46 GiB card actually allocated, OpenCode was told about a
model the server did not have (`400 model 'qwen3.8-27b-256k' not found`)
and about 262,144 tokens against a 131,072 pool. The second is the
dangerous half: a client that fills to its declared limit over-commits
the shared pool, which fails *every* live session. The serving job now
copies its generated preset to `logs/router-presets.active.ini` and the
driver configures from that.

**f16 KV is 64 KiB/token for Qwen3.8-27B, not 32.** The formula is
`2 x 16 full-attention layers x 4 kv_heads x 256 head_dim x bytes/elem`
— 32 KiB at fp8, so double that at f16. Getting this wrong put a 16 GiB
KV cache into a 16.4 GiB hole on an L40S; the model loaded and then died
with `failed to allocate buffer for rs cache`, which looks like a broken
model rather than a budget error. If the GPU is chosen dynamically, the
preset must be generated from the VRAM actually found.

**`socket.create_connection(timeout=N)` leaves the timeout on the
socket.** It does not merely bound the handshake, so every later `recv`
inherits it — a relay built this way kills any request with a quiet
stretch longer than N (a model load, a long prefill). Call
`settimeout(None)` once connected.

**Host RAM, not GPUs, is the scarce resource — and you need almost
none.** `llama-server`'s measured RSS is **154 MB** with the weights on
the GPU and the GGUF mmap'd. The 67 GB that `sacct` reports is page
cache from copying 57 GB of weights to local NVMe, charged to a job that
died in 68 seconds without loading a model. Meanwhile a `--mem=64G`
request queued with a four-hour estimate while sixteen Blackwell GPUs
sat idle, because the allocatable Blackwell nodes had ~5 GB of RAM free
between them. Ask for 8 GB.

**A `--test-only` start estimate is a guess, and the queue moves under
it.** One job submitted on a 15-minute estimate drifted to "tomorrow
00:50" while a fresh probe showed another GPU type could start eight
hours sooner. Re-probe after ~10 minutes of queueing and switch if
another type is materially better, bounded to a couple of switches —
cancelling forfeits queue position, so thrashing is worse than waiting.
Automatic recovery must re-probe too, or an expired allocation is
re-requested on a card that has since become congested.

**`tail -f logs/serve-*.out` follows the OLDEST job.** The glob matches
every job the account ever ran and `tail` takes the first. Resolve the
current job id and follow that file.

**Let the scheduler choose the GPU type.** Probing with `sbatch
--test-only` (allocates nothing) inside one minute gave earliest starts
from 15 minutes (a6000) to four hours (6000_blackwell) to the next day
(nvidia_l40s). Pinning a type is how a session waits half a day while
five other GPUs are free. Restrict candidates to >=48 GiB for a Q8 27B
at full context.

**`ssh -N -L` does not exit when the far end dies.** The login-node
connection stays healthy; only the per-request channel fails, logging
`channel N: open failed: connect failed: Connection refused`. So the
forward's liveness is not the endpoint's liveness — hold the forward in
the background and probe the service (`/health` needs no API key)
instead of waiting for ssh to return.

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
