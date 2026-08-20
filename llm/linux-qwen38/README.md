# Qwen3.8-27B — Linux RTX 4090 inference node

A reproducible vLLM deployment of Qwen3.8-27B on a single NVIDIA RTX 4090
(24 GiB), serving an OpenAI-compatible API for AutoSpec workers.

```
scripts/install-node.sh --with-opencode   # the whole stack: llama.cpp, weights,
                                          # projector, router, service, client
qwen38ctl status                          # what is running
qwen38ctl start vision                    # pin a single-model profile
```

`install-node.sh` is the one to run. It fetches or builds llama.cpp, downloads
the weights and the vision projector, installs the router presets and the boot
service, and then **verifies a completion, a long-prompt retrieval and an image
before reporting success**. `setup-linux-qwen38.sh` installs the optional vLLM
profiles and is not needed for the default configuration.

## Why this model is cheaper to serve than it looks

Qwen3.8-27B (`Qwen3_5ForConditionalGeneration`) is a **hybrid** model. Of its
64 layers only 16 are full attention — `full_attention_interval` is 4 — and the
other 48 are linear-attention (gated-delta) layers that carry a **constant-size
recurrent state** instead of a per-token KV cache.

That changes the arithmetic that governs everything else here:

```
fp8 KV per token = 2 × 16 full-attn layers × 4 kv_heads × 256 head_dim
                 = 32 KiB/token
```

A dense 27B would cost roughly four times that — which is what makes tens of
thousands of tokens reachable here at all. It is still not enough for the
spec's six-figure target; see below.

vLLM confirms the figure during startup: 262,144 tokens "needs 8.18 GiB",
which is 32.7 KiB/token.

**But context and concurrency come out of one pool.** It is tempting to assume
the recurrent state bounds concurrency separately — the spec's Profile B does.
Measured, it does not: the KV pool was **73,728 tokens at `seqs=1` and 75,548 at
`seqs=8`**, essentially unchanged. vLLM pads the mamba page size to equal the
attention page size and serves both from the same block pool, so what binds is
simply:

```
concurrency ≈ pool_tokens / max_model_len
```

Raising `--max-num-seqs` past that ratio buys nothing; requests queue.

## The two things that actually cost VRAM

Weights dominate, and CUDA graphs are a distant but very real second:

| consumer | cost | note |
|---|---|---|
| weights (text-only) | **18.37 GiB** | of a 24 GiB card |
| CUDA graphs | **~2.5 GiB** | disabling them took the KV pool from 25,486 → 73,728 tokens — and cost ~3x generation speed |
| FlashInfer workspace | **394 MiB** | allocated on the *first request*, not reserved by profiling — the cause of a start-healthy-then-die failure |
| desktop / other processes | ~0.7 GiB | why util 0.985 is refused outright |
| everything left for KV | 1.3–3.2 GiB | depending on the above |

That ratio is why the profiles below look the way they do, and why none of them
reaches the spec's 100–150K target. It is not a tuning failure: 18.37 GiB of
weights on a 24 GiB card leaves too little, and the fix is a smaller checkpoint,
not a smaller batch. See `config/model-artifacts.yaml` → `wanted`.

## Measured, not assumed

Every context and concurrency number in `config/profiles.d/` was measured on
this host, and every one was then **confirmed by a real completion** — not by a
successful startup. That distinction is not pedantic: a configuration that
allocates a 73,728-token pool and passes `/health` still dies on its first
request here. The measurements and the reasoning behind each value are in
**[docs/measured-ceilings.md](docs/measured-ceilings.md)**.

`tests/test_structural.sh` fails if any profile still carries an unresolved
`__PLACEHOLDER__`, so a profile can never ship with a guessed context size.

## Which model, exactly

Deployed: **`cyankiwi/Qwen3.8-27B-AWQ-INT4`** @ `63768c10` (W4A16
compressed-tensors, group size 32), served by vLLM 0.27.1. Chosen for runtime
compatibility, **not** speed — no Unsloth / NVFP4 / exl3 build is in use.
Measured alternatives, and why NVFP4 is unavailable on this card, are in
[docs/runtime-comparison.md](docs/runtime-comparison.md).

## Profiles

| profile | purpose | CUDA graphs | context | concurrency |
|---|---|---|---|---|
| `interactive` | one worker, interactive coding — **starts at boot** | on (fast) | 39,200 | 1 |
| `concurrent` | several agents in parallel | off | 8,192 | 6 |
| `extended` | repo-wide analysis, architectural review | off | **56,448** | 1 |
| `quality` | quantisation-regression control (llama.cpp Q6_K) | n/a | see qwen-local | 1 |

The `interactive` / `extended` split is the honest expression of the trade above:
you can have generation speed or you can have context, and on this card you
cannot have both. Measured: **41.5 tok/s at 39,200 context**, or **14.3 tok/s at
56,448** — disabling CUDA graphs buys context and costs ~3x speed.

Both are verified with real long prompts (33,358 and 44,238 tokens), not just
allocations — two larger figures vLLM itself reported turned out not to survive
a long request. Full numbers in [docs/measured-ceilings.md](docs/measured-ceilings.md).

Only one profile runs at a time. `qwen38ctl` stops the current one and waits for
the GPU to actually be released before starting the next — systemd reports a
unit inactive as soon as the process exits, but the driver can hold the
allocation a moment longer, which is long enough to fail the next start.

## Client configuration (OpenCode)

The endpoint moved when `longcontext` became the default, so any client pinned
to the old llama.cpp node on **:8080** silently stops working — that node is
disabled. `~/.config/opencode/opencode.json` now carries two providers:

| model id | endpoint | context | concurrent | vision | use |
|---|---|---:|---:|---|---|
| `qwen-local/qwen3.8-27b-40k` (default) | `:8080` | 40,960 | 4 | no | several agent sessions |
| `qwen-local/qwen3.8-27b-80k` | `:8080` | 81,920 | 2 | no | two large sessions |
| `qwen-local/qwen3.8-27b-160k` | `:8080` | 163,840 | 1 | no | one whole repository |
| `qwen-local/qwen3.8-27b` | `:8080` | 196,608 | 1 | no | the entire pool, solo |
| `qwen-local/qwen3.8-27b-vision*` | `:8080` | 98,304 / 49,152 / 24,576 | 1–4 | **yes** | screenshots, diagrams |
| `qwen-local/qwen3.8-27b-abliterated*` | `:8080` | 163,840 / 81,920 / 40,960 | 1–4 | **yes** | uncensored |
| `qwen-vllm/qwen3.8-27b` | `:8000` | 32,928 | 1 | no | short prompts, ~41 tok/s |

On a node that fans out, generate the client config with `--no-solo-tier`:

```
configure-opencode.py --presets /opt/qwen-vllm/etc/router-presets.ini --no-solo-tier
```

The bare pool-sized id is the one entry whose availability alone can oversubscribe
the server. A child inherits the parent's model id, so a parent sitting on the
whole pool hands every child a window the size of the pool -- and the client's
declared limit is the only admission control there is. Withhold it and the largest
selectable window becomes the `-160k` tier. Nothing else is lost: the vision
presets' solo ids are the same context as their `-96k` tiers.

**The `-NNk` entries are not separate models.** They are aliases of one loaded
model under different declared context limits, so switching between them costs
nothing — no unload, no reload. Switching between *models* (text ↔ vision ↔
abliterated) costs a swap of ~5-7 s; the presets share weights files, so the
reload comes from page cache.

`--models-max 1` is mandatory and set: the default is 4, and two 18.5 GiB models
will not co-reside on this card. That is also why concurrent sessions must all
pick tiers of the **same** model — a text session and a vision session running
together make the router reload on every request.

## Running several sessions at once

The server holds **one 196,608-token KV pool shared across 4 slots**
(`kv-unified = true`). Sessions draw from it as they need, so an 80k session can
run beside two 40k ones. Verified: 80k + 40k + 40k concurrently, zero errors,
one model load.

Four slots rather than six, even though six load: **a seat the pool cannot pay
for is a trap, not capacity.** Six slots at the default 40,960 tier declares
245,760 against a 180,224 pool, and a shared pool has no admission control, so
filling every slot killed every session in it -- 48 `Context size has been
exceeded` errors on 2026-08-18, all of them subagent fan-out. Four slots on the
196,608 pool is 163,840 for a full house at the default tier: the failure is no
longer reachable by arithmetic.

Two things make this configuration rather than luck:

**Seats are paid for in context.** `--parallel` costs VRAM in compute buffers,
so more slots means a smaller pool. `scripts/measure-slot-frontier.sh` walks the
exchange rate on this card:

| slots | largest pool that loads |
|---:|---:|
| 4 | 196,608 |
| 6 | 180,224 |
| 8 | 163,840 |
| 12 | 131,072 |

**The pool has no admission control.** Over-subscribe it and *every* in-flight
session dies, not just the greedy one — three 80k sessions against a 196k pool
were all accepted, prefilled for 58 s, then all failed with `Context size has
been exceeded`. The `-NNk` tiers exist so OpenCode compacts each session before
that happens. Keep the running total of live sessions at or under 163,840;
`tests/check_presets.py` fails the build if a tier outgrows its pool or claims
more concurrent sessions than there are slots.

### Fan-out costs what a session costs, times the width

A subagent is a whole session. It gets its own context, and on this client it
**inherits the parent's model id** unless something pins it -- so a parent sitting
on the solo tier hands each of its children a window the size of the pool. That
is how the 08-18 failure happened: four children, each declaring 180,224,
summing to 288,970 tokens of peak against a 180,224 pool.

Two numbers make fan-out affordable, both measured on this host:

| child | floor before any work |
|---|---:|
| stock `general` | 37,113 |
| same, with the `skill` tool denied | **18,030** |
| same again, in a directory with no `AGENTS.md` | **8,474** |

The floor is additive, and every term was measured on this host rather than
estimated:

| term | tokens |
|---|---:|
| OpenCode base prompt + a narrowed tool set | 8,474 |
| project instructions (`AGENTS.md`, `CLAUDE.md`, `rules/*.md`) | 9,556 |
| the skill roster, when the `skill` tool is enabled | 16,252 |
| the tool schemas a general-purpose child keeps (edit, write, webfetch, task) | ~2,300 |

Those four sum to the 37,113 that was actually observed, which is the check that
the model is right rather than merely plausible.

The skill tool injects the whole skill roster, ~16.3k tokens, into any session
that can call it -- 40% of a 40,960 tier spent on skills a child should not be
loading in the first place. So the client is configured (`~/.config/opencode/opencode.json`)
to pin every spawnable child to `qwen-local/qwen3.8-27b-40k`, deny it `skill`,
deny it `task` so fan-out cannot nest, and retire the operator-facing skills from
`mode: all` to `mode: primary` so an 11k-21k skill body is never spawnable as a
child.

`scripts/context-budget-check.py` is the gate. It reads the **loaded** server
rather than these presets, resolves agents through `opencode agent list` and
`opencode debug agent` so it sees built-ins and config overrides, and prices two
bounds -- the caller's fan-out width, and a full house of every slot:

```
context-budget-check.py --width 3
context-budget-check.py --width 3 --pool 196608 --slots 4   # price a preset first
```

It reads the newest top-level session out of `~/.local/share/opencode/opencode.db`
as well, because the tier a session picked in the TUI overrides the configured
default -- that override is exactly what went wrong on 08-18, and a guard that
trusted the config would have called it healthy. `tests/test_context_budget.py`
covers that case, the sum that fits, and an unpinned child.

Measure raw concurrency with `scripts/bench-concurrency.py`:

```
bench-concurrency.py --model qwen3.8-27b-40k --concurrency 1,2,4
bench-concurrency.py --mix "qwen3.8-27b-80k:81920,qwen3.8-27b-40k:40960"
```

At one slot, four clients do not fail — they **queue**. Per-stream speed stays
at 40.55 tok/s while worst-case time-to-first-token goes 0.76 s → 15.06 s and
aggregate throughput does not grow. Six slots turn the same case into 56.72
aggregate tok/s at a 4.78 s worst TTFT.


The vLLM provider is still a manual switch (`qwen38ctl start interactive`),
because it is a different runtime and the router only fronts llama.cpp.

Capabilities are declared from what was tested, not assumed:

- `tool_call: true` — verified, `finish_reason: tool_calls` with correct arguments
- `reasoning: true` — verified, `reasoning_content` returned separately
- `attachment` — **true only for `qwen-vision`**, verified by a generated 256x256
  chessboard: the prompt grew by 1,026 tokens (matching `--image-min-tokens
  1024`) and the model answered `chess`. The token delta is the real check; a
  server ignoring `--mmproj` would still answer plausibly from the text alone.

The vision profile costs context, not quality: same Q5_K_M weights, but the
885 MiB projector displaces ~98k tokens of KV. It still verified an
83,593-token retrieval in 40 s.

## Sizing the tiers (measured, not guessed)

`scripts/analyze-session-contexts.py` reads real agent sessions — Claude Code
transcripts and the OpenCode database — and reports what context the work
actually carried. Across this operator's OpenCode sessions:

| | p50 | p90 | max |
|---|---:|---:|---:|
| floor, before any work | 14,492 | 37,873 | 76,006 |
| session peak | 62,421 | 170,783 | 756,987 |

Growth while rising is 764 tokens/turn, so a tier buys a predictable number of
turns before the client must compact:

| tier | sessions that never compact | turns before first compaction |
|---:|---:|---:|
| 40,960 | 28.6% | 29 |
| 49,152 | ~40% | 39 |
| 81,920 | 69.6% | 77 |
| 163,840 | 89.3% | 174 |

**There is no tier below 40k, and that is deliberate.** The p90 floor is 37,873,
so a 32k tier cannot start a heavy session at all — the system prompt, project
instructions, memory and MCP tool schemas are already over the limit. The floor
is client-specific: the same operator's Claude Code sessions floor at 39,655
median, nearly three times OpenCode's, so measure the client you will use.

Housekeeping that follows from this:

- Compaction is a **full re-prefill** — 50–115 s at large contexts on this card,
  because the cached prefix changes. Fewer, larger compactions beat many small.
- The floor is paid on every turn and buys nothing. Trimming unused MCP servers
  and skills moves every tier up.
- Prefer a fresh session over a compacted one when the task changes.

## Coexistence with the llama.cpp node

This host also runs `qwen-local.service`, the llama.cpp deployment of the same
model. It holds ~23 GiB of the 24 GiB card when resident. **The two cannot run
at once.**

- vLLM listens on **8000**, llama.cpp on **8080**, so neither unit needs editing
  to run the other.
- The unit declares `Conflicts=qwen-local.service`, which systemd applies in
  both directions: starting either runtime stops the other, instead of the
  newcomer OOMing several minutes into vLLM's memory-profiling pass.

**Rolling back to llama.cpp is one command:**

```
qwen-localctl resume        # stops vLLM, restores the llama.cpp node on :8080
```

That path is the known-good configuration on this machine and is worth keeping
in reach — the vLLM profiles here are newer and less proven.

## Profile versioning

Weights and quantisation are pinned by immutable revision in
`config/model-artifacts.yaml`. Changing either is a **new profile version**
(`…-v1` → `…-v2`), never an edit in place, so a benchmark result always names a
configuration that can be reconstructed. `tests/test_structural.sh` enforces
that the pin is a full commit sha rather than a branch name.

## Layout

```
config/
  common.conf              shared settings — the single source of truth
  profiles.d/*.conf        per-profile overrides
  model-artifacts.yaml     provenance, and what was rejected and why
scripts/
  setup-linux-qwen38.sh    idempotent installer; refuses to claim success
                           without a real completion from the served model
  serve-profile.sh         launcher (ExecStart)
  qwen38ctl                control CLI
  measure-ceiling.sh       find and prove the real context ceiling
  benchmark.py             concurrency sweep → JSON + Markdown
systemd/
  autospec-qwen38@.service template unit, instance name = profile
tests/
  test_structural.sh       no GPU needed; safe in CI
  test_smoke.sh            the gate the installer will not skip
  test_context_budget.py   fan-out arithmetic; no GPU, no server needed
  test_configure_opencode.py  client tier generation, incl. --no-solo-tier
docs/
  measured-ceilings.md     what this hardware actually does
runtime-descriptor.json    machine-readable, for the AutoSpec model router
benchmark-profile.yaml     what the qualification benchmark must run
```

## Simplifications

Recorded so they are visible rather than discovered later.

1. **One template unit instead of two named units.** The spec sketched
   `autospec-qwen38-interactive.service` and `-concurrent.service`.
   `autospec-qwen38@.service` covers all three vLLM profiles from one file, and
   makes mutual exclusion a property of the unit rather than of a script.
   `autospec-qwen38@interactive.service` is the instance the spec named.
2. **INT8 `lm_head` / INT8 `embed_tokens` are not implemented.** No public
   checkpoint quantises them, so this would require self-quantisation with
   llm-compressor. It is the single highest-value follow-up: those two tensors
   cost ~4.8 GiB at fp16, and returning that VRAM buys roughly 75k tokens of
   additional context. See `config/model-artifacts.yaml` → `wanted`.
3. **Prometheus metrics are not wired up.** vLLM exposes `/metrics` natively;
   nothing scrapes it here yet. The spec called this preferred, not mandatory.
4. **MTP is off in every profile.** The `qwen3_5_mtp` method string is verified
   valid in the installed vLLM, but its benefit and — more importantly on this
   card — its VRAM cost are unmeasured. The A/B is specified in
   `benchmark-profile.yaml` and has not been run.
5. **Two upstream workarounds are configuration, not patches.** The FlashInfer
   sampler is disabled (CUDA 13 CUB incompatibility) and the venv `bin` is forced
   onto `PATH`. Both are documented with removal conditions in
   [patches/README.md](patches/README.md) and asserted by the structural tests.

## Sandbox settings that break CUDA

The unit deliberately does **not** set `PrivateDevices=true`, `DevicePolicy=closed`,
`MemoryDenyWriteExecute=true`, or `ProtectSystem=strict` — each breaks the CUDA
stack, the first two by hiding `/dev/nvidia*`. `ProtectSystem=full` is the
strongest setting that leaves CUDA working. `tests/test_structural.sh` asserts
none of the four come back.

## Federating this node to another host

This node serves inference **without an API key**, which is safe only while it is
loopback-bound (`QWEN38_HOST=127.0.0.1`, the default). The moment another node
proxies to it, the port must be bound to a routable address — and an
unauthenticated inference port reachable by anything other than that node lets
any client bypass the proxying node's authentication entirely, with its usage
attributed to nobody.

[`ops/federation-firewall.sh`](ops/federation-firewall.sh) is the guard, and it
belongs on **this** host — the proxying node cannot apply it:

```bash
QWEN38_FED_ALLOW="<peer-addr>" sudo -E ./ops/federation-firewall.sh --persist
sudo ./ops/federation-firewall.sh --status
```

It restricts every OpenAI-compatible port on this host (both runtimes, since
guarding only the one you happen to be thinking about is how the other gets
exposed), keeps loopback open, and installs a boot unit — this host has neither
`netfilter-persistent` nor `/etc/iptables`, so rules do not otherwise survive a
reboot.

Two behaviours worth knowing. Every peer is **validated before any rule is
touched**, because an earlier version flushed first and validated as it went: a
bad peer aborted part way through and left a chain with no DROP, silently
*opening* the port it exists to close. And rules land in `INPUT`, which is right
for a systemd unit on the host — containerise the server and traffic arrives via
`DOCKER-USER` instead, reopening the port without a single rule changing.
