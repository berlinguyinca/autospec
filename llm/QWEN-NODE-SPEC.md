# Portable spec: a local Qwen-class inference node

A hardware-adaptive playbook for deploying a Qwen3.8-class model (or any
similar dense/hybrid LLM) as a persistent, OpenAI-compatible local service —
plus the measurement method that decides the configuration instead of guessing
it.

This document is both a **record** of the RTX 4090 build in
[`linux-qwen38/`](linux-qwen38/) and an **executable spec**: hand it to a coding
agent on a different machine and it should reach an equivalent result on Apple
Silicon, NVIDIA consumer, NVIDIA Blackwell/DGX Spark, Intel, or AMD.

**On the reference host, one command does all of it:**

```bash
./linux-qwen38/scripts/install-node.sh --with-opencode
```

That installs llama.cpp, fetches the weights and projector, writes the router
presets, enables the boot service, verifies a completion, a long-prompt
retrieval and an image, then points OpenCode at it. On other platforms, work
through the phases below — the arithmetic and the measurement method are what
port; the commands are not.

> **The one rule.** Never configure a number you have not verified with a real
> request. Every hard-won lesson in Appendix C is a variant of that rule being
> broken.

---

## 0. What this produces

1. A pinned runtime and a **checkpoint identified by repository + revision**, not
   by a bit-width nickname.
2. Named **profiles** trading context against speed, each with a measured,
   verified ceiling.
3. A single endpoint that **switches models on demand** where the runtime
   supports it.
4. A systemd (or launchd) service that survives reboot.
5. A reproducible installer that **refuses to claim success** without a real
   completion from the served model.
6. Structural tests that need no accelerator, plus inference tests that do.
7. A written record of what was measured, what was rejected, and what was not
   tested.

---

## 1. Phase 0 — Hardware and capability audit

Record before changing anything. Every later decision keys off this.

| item | why it matters |
|---|---|
| OS + kernel | driver and runtime availability |
| CPU, cores, arch | prefill on CPU-offload paths |
| **Total RAM** | Apple/Spark: this *is* the model budget |
| **Accelerator memory** | the binding constraint on discrete GPUs |
| **Memory bandwidth (GB/s)** | sets the achievable tokens/sec ceiling |
| Compute capability / arch | decides which quant formats are legal |
| Driver + toolkit version | decides which runtime builds work |
| Existing model servers | they hold memory and ports |
| Listeners on target ports | silent collisions otherwise |

```bash
# Linux / NVIDIA
nvidia-smi --query-gpu=name,memory.total,driver_version,compute_cap --format=csv
nvidia-smi --query-compute-apps=pid,process_name,used_memory --format=csv
. /etc/os-release && echo "$PRETTY_NAME"; uname -r; free -g
ss -ltnp | grep -E ':(8000|8080|8081|1234|11434)'

# Apple Silicon
system_profiler SPHardwareDataType | grep -E "Chip|Memory"
sysctl -n hw.memsize hw.ncpu

# AMD / Intel
rocminfo 2>/dev/null | grep -E "Name|gfx" | head
vulkaninfo --summary 2>/dev/null | grep -E "deviceName|driverVersion"
clinfo 2>/dev/null | grep -E "Device Name|Global memory"
```

**Bandwidth reference** (needed for §4; verify against the vendor spec for the
exact SKU — these are the figures used in this document):

| platform | memory | bandwidth |
|---|---:|---:|
| RTX 3090 / 4090 | 24 GB GDDR6X | ~936 / ~1008 GB/s |
| RTX 5090 | 32 GB GDDR7 | ~1792 GB/s |
| DGX Spark (GB10) | 128 GB LPDDR5X unified | ~273 GB/s |
| Apple M4 Pro | up to 48 GB unified | ~273 GB/s |
| Apple M4 Max | up to 128 GB unified | ~546 GB/s |
| Apple M3 Ultra | up to 512 GB unified | ~819 GB/s |
| Intel Arc B580 | 12 GB GDDR6 | ~456 GB/s |

**Capacity classes** — this is the fork in the road:

- **Discrete GPU, weights must fit VRAM** (4090, 5090, Arc, most Radeon).
  Capacity is scarce, bandwidth is plentiful. Expect a fight over context.
- **Unified memory** (Apple Silicon, DGX Spark, Ryzen AI Max).
  Capacity is plentiful, bandwidth is scarce. Expect a fight over speed.

The whole spec bifurcates here, and most published advice silently assumes one
class or the other.

---

## 2. Phase 1 — Read the model's architecture before sizing anything

Do not assume a dense transformer. Fetch `config.json` and inspect it.

```bash
curl -s https://huggingface.co/<org>/<model>/raw/main/config.json > /tmp/cfg.json
python3 - <<'PY'
import json
c = json.load(open('/tmp/cfg.json'))
t = c.get('text_config', c)
lt = t.get('layer_types') or []
print("arch          :", c.get('architectures'))
print("layers        :", t.get('num_hidden_layers'))
print("full attention:", lt.count('full_attention') or 'all (dense)')
print("linear/hybrid :", lt.count('linear_attention'))
print("kv heads      :", t.get('num_key_value_heads'), "head_dim:", t.get('head_dim'))
print("n_ctx_train   :", t.get('max_position_embeddings'))
print("vocab         :", t.get('vocab_size'), "hidden:", t.get('hidden_size'))
PY
```

**KV cost per token** — the single most important derived number:

```
bytes/token = 2 (K+V) x N_full_attention_layers x kv_heads x head_dim x bytes_per_element
```

Note `N_full_attention_layers`, **not** total layers. For Qwen3.8-27B this is
16 of 64 (`full_attention_interval: 4`), giving **32 KiB/token at fp8/q8** where
a dense 27B of the same width would cost roughly four times as much. Hybrid
(Gated-DeltaNet / Mamba) layers hold a constant-size recurrent state instead.

Two consequences that generalise:

- **Hybrid models make six-figure context affordable.** Dense models of the same
  parameter count do not.
- **Generation speed decays far more gently with depth** on hybrid models,
  because only a minority of layers grow a cache. Measured in §6.

Also record `vocab_size x hidden_size`: for large vocabularies the embedding and
output head are a significant share of the weights, and **whether a quantiser
compresses them explains most unexplained size differences** (Appendix C.1).

---

## 3. Phase 2 — Choose the quantisation from the hardware, not the name

**Rule: nominal bit-width does not predict footprint.** Measure the file size,
and after loading, measure resident size. See Appendix C.1 for the case where a
4-bit checkpoint was *larger* than a 5-bit one.

### Format legality by architecture

| format | requires | notes |
|---|---|---|
| **NVFP4** | NVIDIA Blackwell (sm_100/110/120): RTX 50xx, B200, **DGX Spark** | ~1.5x faster where legal. **Silently unavailable on Ada/Ampere.** |
| MXFP4 | Blackwell-class | poorer linear-method support than NVFP4 |
| FP8 (W8A8) | sm_89+ | ~1 byte/param — usually too large for 24 GB at 27B |
| **W4A16** (AWQ / GPTQ / compressed-tensors) | any CUDA sm_75+ | vLLM's best-supported path; **footprint varies wildly by publisher** |
| **GGUF** Q4_K/Q5_K/Q6_K | anything llama.cpp builds for — CUDA, Metal, ROCm, SYCL, Vulkan, CPU | most portable; widest size ladder |
| **exl3** | NVIDIA, ExLlamaV3 | fine-grained bpw (2.0–6.0); best size/quality at a target footprint |
| **MLX** 4/6/8-bit | Apple Silicon | native Metal path; often fastest on Mac |

### Let the hardware pick the quant

`scripts/select-quant.py` automates this phase: it enumerates what a repository
publishes, derives KV cost from the base model's own `config.json`, and reports
the **highest-quality quant that still serves a target context** on the detected
hardware.

```bash
# what fits at 196k on this box, and what it would leave spare
./linux-qwen38/scripts/select-quant.py --repo unsloth/Qwen3.8-27B-GGUF \
    --target-context 196608 --kv q4_0 --emit-preset

# uncensored builds, vision required, smaller context
./linux-qwen38/scripts/select-quant.py --variant uncensored --vision \
    --target-context 98304
```

Three details that make it correct rather than approximate:

- **Rank by quant tier, not file size.** `Q4_K_L` is *larger* than `Q5_K_M` yet
  lower quality — the `_L`/`_XL` variants keep a 4-bit body and only raise the
  embedding and output tensors. Ranking on size alone picks the wrong file.
- **Group split GGUFs.** `…-00001-of-00003.gguf` is one model; treated
  separately, a single shard looks like an attractive small candidate.
- **Cap at `n_ctx_train`.** Memory is not the only ceiling.

Its context figures are **arithmetic upper bounds**. Compute buffers grow with
depth and some runtimes allocate outside their own budget, so a bound must be
confirmed with `measure-ceiling.sh` before it is configured. Validation: on the
reference host the tool independently recommended a Q5 quant at ~196k, matching
what was arrived at manually — and its KV term predicts 18.0 KiB/token for
`q4_0`, against 18.4 KiB/token measured.

### Planning a ladder for a machine you are not sitting at

`--platform` targets a named machine and `--ladder` emits a whole profile set at
once, so a context ladder can be planned before the hardware arrives.

```bash
# 48 GB MacBook: small / medium / large, best quant for each
./linux-qwen38/scripts/select-quant.py --platform mac-48 \
    --repo unsloth/Qwen3.8-27B-GGUF --kv q4_0 \
    --ladder "small=32768,medium=131072,large=262144" --emit-preset
```

**Worked example — Apple 48 GB unified.** Budget ~36.9 GiB (75% of RAM; macOS
caps GPU-wired memory near there, and raising it is a deliberate
`sysctl iogpu.wired_limit_mb` change). At `q4_0` KV:

| tier | context | best quant that fits | size | predicted t/s |
|---|---:|---|---:|---:|
| small | 32,768 | `UD-Q8_K_XL` | 29.30 GiB | 8.7 |
| medium | 131,072 | `UD-Q8_K_XL` | 29.30 GiB | 8.7 |
| large | **262,144** | `UD-Q8_K_XL` | 29.30 GiB | 8.7 |

The ladder is **degenerate on this machine, and that is the answer**: 48 GB of
unified memory holds an 8-bit 27B at the *full trained context*, so there is no
tier where you must drop quality. Compare the 24 GB discrete card, where Q5 at
196k was the ceiling.

What you pay is speed. At 273 GB/s the 8-bit weights predict **~8.7 t/s** where
Q5 predicts ~14.5. So on unified memory the interesting knob is not "what fits"
but "what is fast enough":

```bash
--min-tps 12     # -> UD-Q5_K_XL at every tier, ~14.5 t/s predicted
```

| you want | pass | you get on a 48 GB Mac |
|---|---|---|
| best quality, speed secondary | *(no floor)* | Q8 at up to 262k, ~8.7 t/s |
| usable interactive speed | `--min-tps 12` | Q5 at up to 262k, ~14.5 t/s |
| a vision tier as well | `--vision` | projector budgeted (~885 MiB) and emitted |

This is the unified-memory/discrete split from section 1 in concrete form: on the
Mac, capacity stops being the constraint and **bandwidth becomes the only real
decision.** Pick the quant from the tokens/sec you can tolerate, not from what
fits — because almost everything fits.

Two mechanics worth stating: the projector is only budgeted **and** emitted when
`--vision` is passed (emitting it otherwise spends ~885 MiB the arithmetic never
reserved, which is how a preset that "should fit" OOMs), and `--json` is the
interface for other tools — the human table changes shape and must not be
scraped.

### Uncensored / abliterated builds

Abliterated ("uncensored") community builds of the same base model are ordinary
model choices for a local node and are supported directly:

```bash
./linux-qwen38/scripts/add-gguf-model.sh \
    --repo Blackfrost-AI/Qwen3.8-27B-ABLITERATED-GGUF \
    --file Qwen3.8-27B-ABLITERATED-Q5_K_M.gguf \
    --mmproj mmproj-Qwen3.8-27B-ABLITERATED-F16.gguf
```

That downloads, writes a router preset, restarts, and verifies the model answers
— after which it is selectable from any client on the same endpoint.

Because they share the base architecture, **all the sizing arithmetic above
applies unchanged**: same layer counts, same KV cost per token, same quant
ladder. Three engineering caveats that do differ:

1. **Provenance.** These are third-party edits of the weights, not vendor
   artefacts. Pin the revision you fetched (`add-gguf-model.sh` records it in the
   preset), check file sizes, and prefer builds with real download counts.
2. **Capability is not assumed to survive the edit.** Abliteration removes
   refusal directions and can degrade instruction-following and reasoning as a
   side effect. If you rely on tool calling or long-context retrieval, re-run the
   verification suite against the new id rather than inheriting the base model's
   results.
3. **Not every build ships a projector.** Vision presets need an `mmproj`; the
   selector skips repos that publish none when `--vision` is passed.

Verified to publish usable ladders (checked, with revisions pinned at selection
time): `Blackfrost-AI/…-ABLITERATED-GGUF` and
`huihui-ai/Huihui-…-abliterated-GGUF` (both with projectors, Q2–Q8),
`JonathanColetti/…-Uncensored-GGUF` (largest download count, ships MTP draft
heads), `orcarouter/…-Uncensored-GGUF`.

### Sizing rule of thumb

```
usable_for_kv ≈ total_memory x headroom_factor − resident_weights − runtime_overhead
```

- `headroom_factor`: **0.94–0.95** on a discrete GPU with a desktop attached;
  ~0.90 on unified memory (the OS needs real room).
- `runtime_overhead`: **reserve ~1 GiB** on CUDA. Some allocations happen
  *outside* the runtime's own budget — see Appendix C.4.

Pick the largest quant that leaves the context you need. Then verify (§6).

---

## 4. Phase 3 — Predict the speed ceiling before optimising anything

Single-stream decode is **memory-bandwidth bound**. It reads the weights once
per token, so:

```
max_tokens_per_sec ≈ memory_bandwidth_bytes_per_sec / resident_weight_bytes
```

Well-configured stacks land at **60–85%** of that. If you are already there,
**stop tuning** — only smaller weights or speculative decoding move the number.

Worked examples for a ~18.5 GiB resident model:

| platform | bandwidth | roofline | observed | efficiency |
|---|---:|---:|---:|---:|
| RTX 4090 | 1008 GB/s | 51 t/s | 41.9 (llama-bench) | 82% |
| RTX 4090 | 1008 GB/s | 51 t/s | 33.0 (HTTP, 196k ctx) | 65% |
| Apple M4 Pro 48 GB | 273 GB/s | ~14 t/s | ~15 (operator) | at roofline |
| DGX Spark GB10 | 273 GB/s | ~14 t/s | not tested | — |
| RTX 5090 | 1792 GB/s | ~90 t/s | not tested | — |

**This table is the most portable thing in this document.** It says a DGX Spark
or an M4 Pro will run a 27B at roughly a quarter the speed of a 4090 *no matter
what you tune*, and in exchange will hold far more context or a far larger
model. Choose the machine for the job, then stop expecting the other machine's
numbers.

Prefill is compute-bound, not bandwidth-bound, and behaves differently — it
degrades with depth (§6).

---

## 5. Phase 4 — Choose the runtime (this dominates the outcome)

On identical hardware and near-identical weights, the runtime decided the
verified context ceiling by more than 3x. **Choose it from the job.**

| runtime | best at | context behaviour | notes |
|---|---|---|---|
| **llama.cpp** | portability, long context, model swapping | quantised KV (`q4_0`/`q8_0`) makes six figures affordable | CUDA/Metal/ROCm/SYCL/Vulkan/CPU; **built-in router mode** |
| **vLLM** | throughput, concurrency, short-to-mid context | paged KV, `fp8` KV; large unbudgeted allocations | needs a supported quant; best on NVIDIA |
| **ExLlamaV3** | best quality per byte on NVIDIA | fp16 or quantised cache | needs TabbyAPI for an OpenAI endpoint |
| **MLX** | Apple Silicon | unified memory | `mlx-lm` server is OpenAI-compatible |
| **Ollama** | convenience | wraps llama.cpp | less control over KV/context |

### Decision matrix

| your situation | choose |
|---|---|
| Long context is the requirement (large repos) | **llama.cpp + GGUF, quantised KV** |
| Many concurrent agents, short prompts | **vLLM** |
| Apple Silicon | **llama.cpp (Metal)** or **MLX** |
| Unified-memory box, huge model | **llama.cpp** |
| Blackwell / DGX Spark, want the NVFP4 win | **vLLM + NVFP4** |
| Max quality at a fixed footprint, NVIDIA | **ExLlamaV3 exl3** |
| Intel Arc / Xe | **llama.cpp SYCL or Vulkan** |
| AMD | **llama.cpp HIP**, or vLLM-ROCm |

Do not assume the throughput-oriented runtime also wins on context. On the
reference build it lost badly: **56,448 vs 196,608 verified tokens.**

---

## 6. Phase 5 — Measure the ceilings (the method that matters)

Three different numbers will look like "the context ceiling". Only one is real.

| claim | what it means | trustworthy? |
|---|---|---|
| **Allocatable** | the runtime reports a KV pool of N tokens | **no** — often an aggregate, not a per-request limit |
| **Starts** | the server boots at `max_model_len = N` and answers a short prompt | **no** — can still OOM on a prompt that fills it |
| **Works at length** | a prompt filling ~85–90% of N returns a *correct* answer | **yes** |

### The procedure

1. **Free the accelerator completely, and wait for it.** Poll free memory until
   it returns, do not just stop the unit. Measuring while a previous process
   still holds memory under-reports badly (Appendix C.3).
2. **Ask for the model's full training context.** The runtime's refusal usually
   states the achievable maximum.
3. **Iterate to a fixed point.** The estimate depends on what you requested, so
   re-probe at each answer until it stops moving (Appendix C.2).
4. **Verify at length** with a needle-in-a-haystack prompt at ~85% of the
   candidate. Assert the *answer*, not just a 200.
5. **On OOM, lower the memory-utilisation fraction — not the context.** Many
   runtimes size the KV pool from the utilisation setting, so shrinking
   `max_model_len` frees nothing (Appendix C.5).
6. **Record what failed**, including the numbers that looked plausible.

Reference implementations: [`scripts/measure-ceiling.sh`](linux-qwen38/scripts/measure-ceiling.sh),
[`scripts/long-prompt-probe.py`](linux-qwen38/scripts/long-prompt-probe.py).

### Speed versus depth — always sweep it

An empty-context number is close to useless for coding work. Measure at depth:

```bash
# llama.cpp: -d pre-fills the cache to N tokens before measuring
llama-bench -m model.gguf -ngl 999 -fa on -ctk q4_0 -ctv q4_0 \
            -p 512 -n 128 -d 0,16384,32768,65536,131072,196608 -r 2 -o md
```

Measured on the reference build (Q5_K_M, q4_0 KV, RTX 4090):

| depth | prefill t/s | generation t/s | % of empty |
|---:|---:|---:|---:|
| 0 | 2,727 | 41.87 | 100% |
| 16,384 | 2,427 | 39.76 | 95% |
| 32,768 | 2,148 | 38.21 | 91% |
| 65,536 | 1,774 | 35.15 | 84% |
| 131,072 | 1,290 | 30.18 | 72% |
| 196,608 | 1,013 | 27.04 | 65% |

**Generation holds up; prefill is what degrades** (2.7x slower at 196k). For
long-context agent work prefill is the dominant cost, so budget for it — and
note that this gentle generation curve is a property of the *hybrid*
architecture, not of the runtime.

### Also measure through the API you will actually use

Synthetic benchmarks flatter. On the reference build `llama-bench` said 41.87
t/s while the same model over HTTP at full context sustained **33.0** — a 21%
gap, because the server allocates the whole KV cache up front. Quote the API
number.

---

## 7. Phase 6 — Verify functionality, not just liveness

A health endpoint proves almost nothing. Test each capability you intend to
advertise, because a client will trust the advertisement.

| capability | how to actually verify |
|---|---|
| completion | exact-match a requested token |
| **streaming** | reassemble SSE deltas before matching — the string is split across frames |
| **tool calling** | assert `finish_reason == "tool_calls"` and correct arguments |
| **reasoning** | assert `reasoning_content` is separate from `content` |
| **vision** | assert the **prompt-token delta** (~image tokens), not just the answer |
| long context | needle retrieval at ~85% of the ceiling |
| code generation | execute the produced code against test cases |

Two traps worth naming:

- **Reasoning models think by default.** "Reply with exactly X" in 16 tokens will
  spend all of them reasoning. Pass the vendor's non-thinking switch (for Qwen:
  `chat_template_kwargs: {"enable_thinking": false}`) and a generous
  `max_tokens`, or you are testing your prompt, not the server.
- **A vision test that only checks the answer proves nothing.** Ask "what board
  game uses this pattern?" over a chessboard and a text-only server still
  answers "chess". The image-token delta is the real evidence. Generate the
  image from the standard library so the test cannot pass for the wrong reason.

Reference: [`tests/test_smoke.sh`](linux-qwen38/tests/test_smoke.sh),
[`tests/test_vision.py`](linux-qwen38/tests/test_vision.py).

---

## 8. Phase 7 — Serve it, and switch models dynamically

### 8.1 Profiles

A profile is a named, verified configuration. Keep each one's settings in a file
that the launcher, the service unit, and the tests all read — one source of
truth, so a context size cannot say one thing to the server and another to the
client.

Typical set (names from the reference build):

| profile | trades | for |
|---|---|---|
| `interactive` | context | fastest single-stream generation |
| `concurrent` | context and latency | aggregate throughput |
| `extended` / `longcontext` | speed | maximum verified context |
| `vision` | context | image input |
| `router` | — | serves several of the above on one port |

Guard rails that earned their place:

- **Refuse to start if the accelerator is already occupied**, with a message
  naming the process holding it. Otherwise the failure appears minutes later as
  an unrelated OOM.
- **Declare mutual exclusion** between competing services (systemd
  `Conflicts=`), so whichever starts last wins cleanly rather than the newcomer
  dying during memory profiling.
- **Fail closed on unresolved placeholders.** A test that rejects a profile still
  containing `__CTX__` makes it impossible to ship a guessed context size.

### 8.2 Dynamic model switching

**Preferred: llama.cpp router mode.** One port, several presets, swapped on
demand from the request's `model` field. No client-side port juggling and no
operator action.

```ini
; router-presets.ini — section name IS the model id clients request
version = 1

[*]
n-gpu-layers = 999
flash-attn = auto
jinja = true
cache-type-k = q4_0
cache-type-v = q4_0

[qwen3.8-27b]
model = /path/Qwen3.8-27B-Q5_K_M.gguf
c = 196608

[qwen3.8-27b-vision]
model = /path/Qwen3.8-27B-Q5_K_M.gguf
mmproj = /path/mmproj-F16.gguf
image-min-tokens = 1024
c = 98304
```

```bash
llama-server --models-preset router-presets.ini --models-max 1 \
             --host 127.0.0.1 --port 8080
```

> **`--models-max 1` is mandatory on a capacity-constrained box.** The default is
> 4; the router will cheerfully try to hold several copies and OOM. This looks
> like a tuning knob and is actually a hard constraint.

Measured swap cost on the reference build: **5–7 s**, because both presets point
at the *same weights file* and the reload comes from page cache. Presets with
different weights cost a full load.

Verify the invariant, do not assume it — `/v1/models` should show exactly one
entry `loaded` and report `input_modalities` per preset.

**Alternatives when router mode is unavailable:**

| situation | approach |
|---|---|
| Older llama.cpp | `llama-swap` proxy (same idea, third-party) |
| vLLM | one model per process; switch by restarting the profile |
| Mixed runtimes | a thin proxy that starts the right profile and waits for health |
| Plenty of unified memory | just run two servers on two ports concurrently |

The last row is the reward for the unified-memory class: with 128 GB you can
hold a long-context text model *and* a vision model simultaneously, and skip
swapping entirely.

### 8.3 Persistence

Use a templated unit so the instance name is the profile:

```
autospec-qwen38@<profile>.service   # Linux / systemd
```

Sandbox settings that **break CUDA** — do not set them, and add a test asserting
they stay unset:

- `PrivateDevices=true`, `DevicePolicy=closed` — hide `/dev/nvidia*`
- `MemoryDenyWriteExecute=true` — breaks CUDA JIT and `torch.compile`
- `ProtectSystem=strict` — blocks driver `/proc` and `/sys` access

`ProtectSystem=full` is the strongest setting that leaves the CUDA stack
working. On macOS the equivalent is a launchd `LaunchAgent` with
`KeepAlive`/`RunAtLoad`.

### 8.5 Serving several sessions at once

One editor window is not the workload. Several agent sessions against one node
is, and a node tuned for a single enormous context serves that badly. Measure it
before assuming otherwise: a server with one slot does not refuse the second
client, it **queues** it, which looks like a healthy server and feels like a
broken one.

On the reference build, four concurrent clients against a single-slot server:

| clients | per-stream tok/s | aggregate tok/s | worst TTFT |
|---:|---:|---:|---:|
| 1 | 40.55 | 32.68 | 0.76 s |
| 2 | 40.56 | 35.55 | 4.04 s |
| 4 | 40.55 | 28.08 | 15.06 s |

Per-stream speed never moves, so a single-stream benchmark reports everything as
fine. The tell is that **aggregate throughput does not grow** while time-to-first
-token grows without bound. Raising the slot count to four turned the same
4-client case into 56.72 aggregate tok/s with a 4.78 s worst TTFT — twice the
work, a third of the wait.

**Three settings, and they are commonly confused.**

| setting | what it controls | what it costs |
|---|---|---|
| `--ctx-size` | the **total** KV pool, not a per-session limit | KV memory |
| `--parallel` | how many sessions may decode at once | compute buffers |
| `--kv-unified` | whether slots get equal fixed shares or draw on one pool | nothing |

The first is the one that surprises people. `--ctx-size 196608 --parallel 4`
logs `n_slots = 4, n_ctx_slot = 49152`: the pool was divided, not multiplied.

The second means **seats are paid for in context**. Compute buffers scale with
slot count, so adding slots at a fixed pool eventually fails to allocate — on a
24 GiB card, `--parallel 8` with `c = 196608` dies on `cudaMalloc 872.28 MiB:
out of memory`. Measure the exchange rate rather than deriving it;
`measure-slot-frontier.sh` walks it:

| slots | largest pool that loads | free VRAM |
|---:|---:|---:|
| 4 | 196,608 | 90 MiB |
| 6 | 180,224 | 158 MiB |
| 8 | 163,840 | 191 MiB |
| 12 | 131,072 | 341 MiB |

The third decides whether sessions can differ in size at all. With
`kv-unified = false` every slot is hard-capped at `c / parallel`, so a client
cannot ask for more no matter what it declares. With `kv-unified = true` all
slots see the whole pool and take what they need — which is the only way to run
an 80k session next to two 40k ones.

### 8.6 A shared pool has no admission control

This is the sharp edge, and it must be designed around rather than discovered.
Three 80k sessions against a 196k pool were **all accepted**, prefilled for 58
seconds, and then all died together:

```
decode: failed to find free space in the KV cache, retrying with smaller batch size
  ... n_batch = 16 ... 8 ... 4 ... 2 ... 1
srv  decode: Context size has been exceeded.
srv  send_error: task id = 97 / 98 / 99
```

Note *which* sessions died: all of them. A session that stayed well inside its
budget is killed by a neighbour that did not. There is no queueing, no eviction,
and no back-pressure — the server over-commits and then fails everyone.

So **the client is where the pool gets rationed.** Publish the same model under
several ids that differ only in declared context, and let the client's own
compaction keep each session inside its share:

| tier | declared context | sessions the pool funds |
|---|---:|---:|
| `-160k` | 163,840 | 1 |
| `-80k` | 81,920 | 2 |
| `-40k` | 40,960 | 4 |
| `-32k` | 32,768 | 5 |

Two properties make this work rather than merely document a convention:

- **Tiers are aliases, not presets.** `alias = id-160k,id-80k,...` gives one
  loaded model several names, so switching tiers in the client costs no reload.
  Separate presets would each be a separate process holding its own copy of the
  weights, and two 18.5 GiB copies do not fit on a 24 GiB card.
- **The invariants are enforced.** `tests/check_presets.py` fails the build when
  a tier declares more than its pool holds, when tiers are offered without
  `kv-unified`, or when a tier advertises more concurrent sessions than there
  are slots to decode them. All three are silent at configuration time and loud
  in production.

Budget to roughly 90% of the pool; the remainder absorbs generated tokens and
fragmentation. Verified at 6 slots over a 180,224 pool: a single session
retrieved a needle at 163,867 prompt tokens, four concurrent 42,996-token
sessions ran clean, and a mixed 80k + 40k + 40k round finished with zero pool
errors and exactly one model load.

**One constraint worth stating plainly:** concurrent sessions must sit on tiers
of the *same* model. With `--models-max 1`, mixing a text session and a vision
session makes the router unload and reload on every request.

**What concurrency actually buys.** Aggregate throughput rises while each
session gets slower, and at long prompts prefill dominates everything:

| workload | per-session | aggregate | worst TTFT |
|---|---:|---:|---:|
| 1 × 4k prompt | 40.07 | 23.91 | 2.16 s |
| 4 × 4k prompts | 24.29 | 56.72 | 4.78 s |
| 80k + 40k + 40k | 0.76 – 23.20 | — | 113.5 s |

The last row is the honest one for agent work: three big sessions starting
together spend roughly two minutes prefilling before anyone sees a token.
Sessions that stay resident do not re-pay that, which is the real reason to give
each one its own slot.

> **Benchmark honestly.** Size prompts with the server's own tokenizer
> (`/tokenize`), never with an assumed tokens-per-line constant. A filler line
> estimated at 17 tokens was 21, so a "4 × 40k" run really sent 4 × 51,800 and
> failed a configuration that fits comfortably. And force a fixed decode length
> (`ignore_eos`); a model that answers in three tokens reports a fine per-stream
> rate and a meaningless aggregate.

---

### 8.6b Does a bigger quantisation actually answer better? Measure it

This project ran for a long time on the assumption that Q8 beats Q5, and that
assumption decides which build to serve. It is testable, and on this workload it
did not hold.

`compare-quants.py` asks two live endpoints the same generated, exact-match
questions — arithmetic, instruction-following, code reasoning, in-context
retrieval — and counts disagreements. Local Q5_K_M on an RTX 4090 against
UD-Q8_K_XL on a 96 GiB Blackwell, 40 items, temperature 0, reasoning enabled:

| | Q5_K_M | UD-Q8_K_XL |
|---|---:|---:|
| total | 40/40 | 40/40 |
| disagreements | — | **0** |

Three things make that number worth reading, and one limits it:

- **Reasoning has to be on.** With thinking disabled both builds scored 1/10 on
  code reasoning, and a category both fail 90% of cannot tell them apart. That
  was a defect in the instrument, not a finding about the models.
- **Infrastructure failures must be separated from wrong answers.** Two earlier
  runs showed the 8-bit build "losing" 8–11 items; every one was a `500` from a
  crashed server child, and on every item where it answered it agreed. Counting
  those as quality would have produced exactly the wrong conclusion.
- **It replicated** across three runs on different allocations and nodes.
- **It cannot resolve a small gap.** At n=40 a difference under ~7 items is
  indistinguishable from noise, so this says "no visible degradation at Q5", not
  "identical".

The practical consequence: on this hardware the 5-bit build is not the
compromise it looks like. Spend the bigger card on **context and concurrency**,
which are measurable and large, rather than on quantisation quality, which here
was not measurable at all.

### 8.7 Size the context from the work, not from the spec sheet

"How much context do I need" is an empirical question, and for anyone who has
been running agent sessions the answer is already in the logs.
`analyze-session-contexts.py` reads them and reports the three numbers that
decide a tier:

**The floor** — context present before any work is done: system prompt, project
instructions, memory, skill and MCP tool schemas. Measured on this operator's
own OpenCode sessions:

| percentile | floor |
|---|---:|
| p50 | 14,492 |
| p75 | 17,062 |
| p90 | 37,873 |
| max | 76,006 |

This is the number that kills small tiers. A 32k tier looks reasonable and
**cannot start a p90 session** — the conversation is over the limit before the
first question. The floor is also client-specific and not transferable: the same
operator's Claude Code sessions floor at 39,655 median and 70,272 max, nearly
three times higher, because the system prompt and skill set are larger. Measure
the client you will actually use.

**Growth per turn** — 764 tokens/turn median while rising, measured on the
rising segments only. A transcript that compacts is a sawtooth, and averaging
across the drops understates how fast the window fills.

**Coverage** — what a tier buys, given that floor and that growth:

| tier | turns that fit | sessions that never compact | turns before first compaction |
|---:|---:|---:|---:|
| 32,768 | 17.5% | 23.2% | 19 |
| 40,960 | 23.6% | 28.6% | 29 |
| 65,536 | 45.2% | 58.9% | 58 |
| 81,920 | 52.1% | 69.6% | 77 |
| 131,072 | 70.4% | 85.7% | 135 |
| 163,840 | 79.5% | 89.3% | 174 |

Read the last two columns together. Doubling 40k to 80k moves "never compacts"
from 29% to 70% and buys 48 more turns — a large gain. Doubling again to 160k
adds only 20 points and, on a 24 GiB card, costs the ability to run more than
one session. That is where the money is on this hardware.

**Sizing rule.** A tier must clear `p90 floor + (turns you want × growth)`. For
these sessions, wanting 30 productive turns: `37,873 + 30 × 764 ≈ 60,800`. So
40k is workable but tight, 64–80k is comfortable, and anything at or below 32k
is a trap.

### 8.8 Housekeeping: compaction is a schedule, not an accident

A local node has a hard ceiling and no graceful degradation, so compaction has
to be planned rather than hit.

- **Set the client's declared limit below the served context, not equal to it.**
  Generated tokens land on top of the prompt. Budget ~90%.
- **Compaction is not free — it is a full re-prefill.** The next request after a
  compaction cannot reuse the cached prefix, because the prefix changed. On this
  build that is 50–115 s at large contexts. Fewer, larger compactions beat many
  small ones.
- **Keep the floor small.** It is paid on every single turn and it is the one
  part of the budget that buys nothing. Trimming MCP servers and skill sets a
  session does not need moves the p90 floor down and every tier up.
- **Prefer a fresh session to a compacted one** when the task changes. Compaction
  preserves a summary of work you are done with; a new session starts at the
  floor with full fidelity.
- **Give each concurrent session its own slot.** Sessions that stay resident keep
  their prefix cached and do not re-pay prefill; sessions that queue through a
  shared slot re-prefill each time they are swapped in.

---

---

## 9. Phase 8 — Wire the clients, and treat that as part of the deployment

**Changing a port silently breaks every configured client.** On the reference
build, switching the default profile moved the endpoint and left OpenCode
pointed at a disabled service — it would have failed on every request. Re-point
clients in the same change that moves the endpoint, and prefer keeping a stable
port.

For OpenCode (`~/.config/opencode/opencode.json`), one provider per endpoint and
one entry per router preset:

```json
{
  "model": "qwen-local/qwen3.8-27b",
  "provider": {
    "qwen-local": {
      "name": "Qwen3.8-27B Q5_K_M (llama.cpp router, auto-swaps)",
      "npm": "@ai-sdk/openai-compatible",
      "options": { "baseURL": "http://127.0.0.1:8080/v1" },
      "models": {
        "qwen3.8-27b": {
          "name": "Qwen3.8-27B — 196k ctx (text)",
          "tool_call": true, "reasoning": true, "attachment": false,
          "limit": { "context": 196608, "output": 32768 }
        },
        "qwen3.8-27b-vision": {
          "name": "Qwen3.8-27B — 98k ctx (vision)",
          "tool_call": true, "reasoning": true, "attachment": true,
          "limit": { "context": 98304, "output": 32768 }
        }
      }
    }
  }
}
```

Rules:

- **Declare only capabilities you tested.** `attachment: true` on a server with
  no projector loaded produces confusing failures far from the cause.
- **Set `context` to the verified ceiling**, never the aspiration.
- **Keep the provider key stable** when re-pointing, so an existing default
  `model` string keeps resolving.
- If a profile needs an operator action, **put the command in the display name**
  so the model picker itself is the documentation.

The same JSON shape works for any OpenAI-compatible client (Codex CLI, Zed,
Continue, aider) — only the file location differs.

---

## 10. Phase 9 — Report honestly

State, with numbers: hardware and bandwidth; runtime and version; checkpoint
**repository + revision**; quantisation; resident weight size; verified context
with the evidence that verified it; generation and prefill rates including
depth; VRAM/RAM at load; what was rejected and why; **what was not measured**.

Two claims to refuse to make:

- Long-context performance you did not execute. Label extrapolations as such.
- Quality comparisons you did not run. "Q5 beats Q4" is an assumption until
  measured; publisher KL-divergence figures are evidence, vibes are not.

---

## Appendix A — Reference build: what was actually measured

**Host:** Ubuntu 24.04.4, kernel 6.8.0-137 · RTX 4090 24,564 MiB, driver
580.173.02 · 503 GiB RAM · model family Qwen3.8-27B
(`Qwen3_5ForConditionalGeneration`, 64 layers, 16 full-attention, 4 KV heads,
head_dim 256, n_ctx_train 262,144).

| runtime | checkpoint | quant | disk | resident | gen t/s | verified context |
|---|---|---|---:|---:|---:|---:|
| llama.cpp b10434 **(shipped)** | `unsloth/Qwen3.8-27B-GGUF` | Q5_K_M + q4_0 KV | 18.46 GiB | ~18.9 GiB | **33.0** (API) | **196,608** — 167,148-token retrieval verified by the installer |
| llama.cpp b10434 (shipped) | same + `mmproj-F16` | Q5_K_M | 19.3 GiB | ~22.3 GiB | ~33 | **98,304** — 83,593-token retrieval; image = +1,026 tokens |
| vLLM 0.27.1 | `cyankiwi/Qwen3.8-27B-AWQ-INT4` @`63768c10` | W4A16 g32 | 19.57 GiB | 18.37 GiB | **41.3** | 32,928 — 28,020-token retrieval |
| vLLM 0.27.1 | same, eager | W4A16 | 19.57 GiB | 18.37 GiB | 14.3 | 56,448 — 44,238-token retrieval |
| ExLlamaV3 1.4.2 | `turboderp/Qwen3.8-27B-exl3` @`4.00bpw` | exl3 4.0 bpw | 15.70 GiB | 18.5 GiB w/ 32k fp16 cache | 27.3 | 98,304 — 83,581-token retrieval |

**Rejected, with reasons:**

| checkpoint | quant | size | why |
|---|---|---:|---|
| `unsloth/…-NVFP4`, `RadixArk/…-NVFP4` | NVFP4 | 20.4–21.8 GiB | **Blackwell only**; this is Ada (sm_89) |
| `goldhub/…-INT4-W4A16-AutoRound` | W4A16 | 26.37 GiB | larger than VRAM before any KV |
| `lued/…-INT8-W8A16-MTP` | W8A16 | 29.44 GiB | far too large |
| `Qwen/Qwen3.8-27B-FP8` | FP8 | ~27 GiB | too large |
| `…-GGUF` Q6_K | Q6_K | 21.3 GiB | leaves ~2 GiB — below the safe headroom |
| MTP speculative decoding | — | +2.37 GiB | does not fit; see C.6 |

**Not measured** (stated so silence is not mistaken for a result): any quality
or regression comparison between quants; concurrency above 8; exl3 3.5/5.0 bpw;
reboot recovery; sustained multi-hour load.

---

## Appendix B — Platform playbooks

### B.0 Getting llama.cpp: what is prebuilt and what is not

Checked against release `b10434`. **There is no prebuilt Linux CUDA binary** —
this surprises people, and it is the one platform where you must compile.

| target | how | asset / note |
|---|---|---|
| **Linux + NVIDIA** | **build from source** | `cmake -DGGML_CUDA=ON`; needs `nvcc` |
| Linux + NVIDIA (quick) | prebuilt | `…-bin-ubuntu-vulkan-x64.tar.gz` — works, usually slower than CUDA |
| **macOS Apple Silicon** | prebuilt | `…-bin-macos-arm64.tar.gz`, Metal included, ~11 MB |
| Linux + Intel | prebuilt | `…-bin-ubuntu-sycl-fp16-x64.tar.gz` |
| Linux + AMD | prebuilt Vulkan, or build HIP | no Linux ROCm release asset |
| Linux CPU / arm64 | prebuilt | `…-bin-ubuntu-x64` / `-arm64` |
| Windows | prebuilt | CPU, CUDA 12.4/13.3, ROCm, SYCL, Vulkan all shipped |

`scripts/install-node.sh` encodes this table: it reuses an existing install,
otherwise fetches for Metal/SYCL/Vulkan/CPU and compiles for CUDA. It then
asserts the build actually has `--models-preset`, because router mode is what
the dynamic switching depends on.


### B.1 NVIDIA consumer, Ampere/Ada (RTX 3090/4090, 24 GB) — the reference case

Capacity-constrained, bandwidth-rich. NVFP4 unavailable.

- **Long context:** llama.cpp + GGUF Q5_K_M, `q4_0` KV → ~196k verified.
- **Throughput:** vLLM + W4A16, but expect a much lower context ceiling.
- Reserve ~1 GiB headroom; utilisation above ~0.95 becomes fragile (C.7).
- CUDA graphs are worth ~3x generation speed and cost ~2.5 GiB — keep them
  unless you are buying context, and split that trade into two profiles.

### B.2 NVIDIA Blackwell (RTX 50xx) and DGX Spark / GB10

- **NVFP4 is legal here and is the main win** (~1.5x). This is the one platform
  where the vendor "optimised" build genuinely applies.
- Upstream guidance for single 24–32 GB Blackwell cards: `--enforce-eager` to
  avoid CUDA-graph OOM, and a modest `--max-model-len`.
- **DGX Spark is a different machine class:** 128 GB unified at ~273 GB/s. Do
  **not** expect 4090 speeds — the roofline is ~14 t/s for a 27B at ~18.5 GiB.
  What you get instead is capacity: run a much larger model, or hold several
  models resident at once and stop swapping (§8.2).
- On Spark, prefer capacity-hungry configurations: higher-precision quants
  (Q6/Q8/FP8) and unquantised KV, since bandwidth is the constraint and
  precision is nearly free in memory terms.

### B.3 Apple Silicon (M-series, unified memory)

- Unified memory: weights, KV, and the OS share one pool. Budget ~90%, not 95%.
- **Two good runtimes:** llama.cpp with Metal (most portable, best context
  control, `q4_0`/`q8_0` KV) and **MLX** (`mlx-lm`, often fastest, OpenAI-
  compatible server, native 4/6/8-bit).
- Bandwidth is the whole story: M4 Pro ~273 GB/s → ~14 t/s for a 27B;
  M4 Max ~546 → ~28; M3 Ultra ~819 → ~41. Pick the chip for the target rate.
- Persistence is launchd, not systemd. No CUDA-graph or FlashInfer concerns —
  and correspondingly none of the unbudgeted-allocation traps in C.4.
- The reference operator measured ~15 t/s on an M4 Pro 48 GB with Q4_K_M, which
  is essentially at roofline — a useful sanity check that the model in §4 holds
  across architectures.

### B.4 Intel (Arc / Xe / Core Ultra)

- **llama.cpp with SYCL** (oneAPI) or **Vulkan**; Vulkan is the easier build and
  usually close in speed.
- IPEX-LLM is the vendor path and supports more formats but tracks upstream
  loosely; check hybrid-architecture support before committing.
- vLLM's Intel support is thinner than its CUDA support — treat GGUF as the
  default, not the fallback.
- Arc B580 has 12 GB: a 27B at 4-bit will not fit. Drop to a smaller model or
  accept partial CPU offload and a large speed penalty.

### B.5 AMD (ROCm)

- **llama.cpp with HIP** is the reliable path; Vulkan is the fallback when ROCm
  does not support the SKU.
- vLLM-ROCm works on supported datacentre parts (MI200/MI300); consumer RDNA
  support is inconsistent.
- Add the service account to `video` and `render` groups.

### B.6 CPU only

Viable for correctness testing, not for interactive use. Bandwidth is ~50–100
GB/s at best, so a 27B at 4-bit runs at low single digits. Use a much smaller
model instead.

---

### B.7 NVIDIA datacentre (A100 / H100) — when the constraints disappear

Everything hard about the 24 GiB reference build is a capacity problem. On an
A100 the capacity problem is simply gone, and the configuration changes shape.

Predictions below are the bandwidth roofline scaled by the efficiency this
project measured — and that efficiency has now been confirmed on a second,
very different card:

| card | quant | roofline | measured | % of roofline |
|---|---|---:|---:|---:|
| RTX 4090, 24 GiB, Ada | Q5_K_M | 50.9 | 40.55 | **79.7%** |
| RTX PRO 6000, 96 GiB, Blackwell | UD-Q8_K_XL | 57.0 | 45.57 | **80.0%** |

Two architectures, four times the memory, different quantisations — both at 80%
of the memory-bandwidth ceiling. **That is the finding that makes this table
usable**: the constant belongs to the runtime and the model, not to the card, so
`bandwidth ÷ resident weight bytes × 0.8` predicts a machine you have never
touched to within about 10%. Everything else here is still arithmetic; confirm
on the machine.

| card | HBM GB/s | Q5_K_M | Q8_0 | BF16 | context at Q8 |
|---|---:|---:|---:|---:|---|
| RTX 4090 24 GB | 1008 | **40.6** (measured) | does not fit | no | — |
| A100 40 GB SXM | 1555 | ~62 | ~43 | no | full 262,144 |
| A100 80 GB PCIe | 1935 | ~78 | ~53 | ~28 | full 262,144 |
| A100 80 GB SXM | 2039 | ~82 | ~56 | ~30 | full 262,144 |
| H100 80 GB SXM | 3350 | ~135 | ~92 | ~49 | full 262,144 |

**What this means in practice.**

- **A100 40 GB** already reaches the full trained context at Q5_K_M with room
  left over. The 24 GiB build's central compromise — pool versus seats versus
  quantisation — does not arise; pick Q5, take 262,144, take 8+ slots.
- **A100 80 GB** runs **Q8_0 at the full 262,144 context** with roughly 47 GiB
  still free, or **BF16 unquantised** with ~23 GiB free. At that point KV
  quantisation is also unnecessary: f16 KV costs 32 KiB/token, so 262,144 tokens
  is 8 GiB, which fits trivially. Every accuracy compromise in this document can
  be dropped at once.
- **Ampere is not Hopper.** The A100 is sm_80: **no FP8** (that starts at Ada
  sm_89) and no NVFP4 (Blackwell). vLLM recipes written around `fp8` weights or
  `--kv-cache-dtype fp8` do not apply. W4A16 Marlin does, and GGUF is unaffected
  because `q8_0`/`q4_0` KV are integer formats.
- **Concurrency should scale better than on the 4090**, which reached 2.37×
  aggregate at 4 slots. More compute per unit of bandwidth means batched decode
  has more headroom. This is **not measured** — treat it as a reason to run
  `bench-concurrency.py` on arrival, not as a number to plan against.

### B.9 Serving from a machine you do not control

B.8 covers getting a job to run. This covers keeping a client usefully connected
to one, which turns out to be a different problem: the node is borrowed, the
allocation expires, and the only route in is a tunnel. Four things had to be
true before a session survived that, and each was learned by watching it fail.

**Put an always-listening proxy in front of the tunnel.** If the listening
socket *is* the SSH forward, then every reconnect, node change, and expired
allocation reaches the client as `Connection refused` — an error a human must
notice and act on, not a slow request. A small local byte relay that owns the
port and simply *holds* a client until the upstream returns converts an outage
into latency, which HTTP clients already know how to wait through. Keep it a
byte relay, not an HTTP proxy: it must carry streamed SSE and long keep-alives
without acquiring opinions about framing. It cannot replay a connection lost
mid-response — only ones not yet started — and it should answer 503 with a
reason when its hold window finally expires, because a silent close is
indistinguishable from a crash or a client bug.

**Do not confuse liveness with capability.** A router process answers `/health`
with 200 while its model child is dead, every request returning
`500 proxy error: Could not establish connection`. An entire benchmark run
failed that way while the supervisor reported the endpoint healthy throughout.
Check liveness cheaply and often; check capability — ask for one token, require
a real answer — once per connection, which is when "the job started but the
model cannot load" actually happens. On a timer it would force a model load on
an idle node and cost minutes of GPU.

**Generate the configuration from the hardware you got, and publish it.** If the
GPU is chosen by whichever can start soonest, the card is unknown until the job
runs — 96 GiB one time, 46 GiB the next. A preset written for the larger one
loads the weights on the smaller and dies allocating state, which reads like a
corrupt model rather than a budget nobody checked. Size pool, slots and KV type
from the VRAM actually found, refuse outright when the weights do not fit, and
have the job **publish what it is serving** so the client is configured from
that rather than from a template. A client configured from a template advertises
models the server does not have and context the pool cannot fund — and
over-committing a shared pool fails *every* live session, not just the greedy
one.

**Treat an unanswered question as unanswered, not as an answer.** A supervisor
that exits when one `squeue` returns empty cannot tell "the job ended" from "the
login node throttled me", and a shared login node does throttle. Read the
transport's own exit status, require several *consecutive authoritative*
negatives before acting, bound every remote call with a timeout, and multiplex
the connections — one SSH per poll gets rate-limited into a phantom bug where a
plainly-RUNNING job looks like it never started.

---

### B.8 Shared HPC clusters (Slurm) — a node you do not own

Time-boxed access to a cluster is a different deployment from a workstation:
no root, no systemd, no persistent service, and a job that disappears at
walltime. The stack still works, but four assumptions in the main text break.

**No root, no `/opt`, no service.** Install everything under `$HOME` or scratch
and run the server as a foreground process inside the job. `install-node.sh`
assumes systemd and a service account; on a cluster, use its pieces (weights
fetch, presets, verification) and start `llama-server` directly.

**Put weights on scratch, not `$HOME`.** Home directories are small, often
NFS-mounted, and slow to page a 20 GiB file from. Set `HF_HOME` and the model
directory to the fast parallel filesystem, and expect the first load of a job to
be slow and subsequent ones to be fast only if the node's page cache survives.

**There is no prebuilt Linux CUDA llama.cpp** (see B.0), so the binary must be
built — and the build needs `nvcc` from the cluster's toolchain, which usually
means a `module load` and often a compute node. Build once into scratch and
reuse it across jobs; do not rebuild per job.

**The server is on a compute node, your editor is not.** Reach it with an SSH
tunnel through the login node, then point the client at `127.0.0.1` as usual:

```bash
# in the job: bind to the node's address, print where it landed
llama-server --models-preset presets.ini --models-max 1 \
             --host 0.0.0.0 --port 8080

# from the workstation: forward through the login node to that compute node
ssh -N -L 8080:${COMPUTE_NODE}:8080 ${USER}@${LOGIN_HOST}
```

Binding `0.0.0.0` on a shared cluster exposes an unauthenticated model server to
every other user on the network. Set an API key, or bind to the node's private
interface only, and check the site's policy before doing either.

**Sizing for a time-boxed loan.** With days rather than minutes, spend the first
hour measuring rather than assuming: `measure-ceiling.sh` for the real ceiling,
`measure-slot-frontier.sh` for the seat/pool exchange rate on that card, and
`bench-concurrency.py` for how batching scales. Those three numbers are what
make the rest of the configuration derivable instead of guessed.

**Confirm at your site before writing the job script**, and expect the answers
to be surprising. A worked example with every value measured rather than assumed
is in `linux-qwen38/hive/README.md`; on that cluster the findings that mattered
were not in any documentation:

- **Partitions you can see are not partitions you can use.** Six `gpu-*`
  partitions were `AllowAccounts=ALL` and every submission to them failed with
  `Invalid account or account/partition combination`, because the user's
  associations covered only two general partitions. GPUs were reachable from
  those instead, via `--gres`.
- **The obvious account had `gres/gpu=0`.** The group account rejected every GPU
  request with `QOSGrpGRES`; a second, less obvious association carried the GPU
  entitlement.
- **GRES names are inconsistent across nodes** of the same model
  (`a100`, `nvidia_a100-sxm4-80gb`, `nvidia_a100_80gb_pcie`). A wrong name is an
  allocation failure, not a fallback.
- **The long-walltime partition was unusable.** It held 12 GPUs against the
  short one's 86, and `--gres=gpu:1` there estimated a start date **a month
  out**. The usable partition was preemptible: `PreemptMode=REQUEUE` with a
  130-second grace time.
- **Short jobs backfill, long ones do not.** A 3-hour, 32-CPU request queued for
  hours while GPUs sat idle; the same work at 1 hour and 16 CPUs started
  immediately.

Use `sbatch --test-only`, which allocates nothing, to establish all of the above
in minutes rather than discovering it job by job.

Two more that are cheap to check and expensive to assume: whether the login
node's `/tmp` is shared with compute nodes (it usually is not — a script staged
there fails with `No such file or directory` inside the job), and whether
compute nodes have outbound internet. Both go the "convenient" way often enough
that guessing is tempting, and the failure modes look nothing like the cause.

## Appendix C — Failure catalogue

Every entry cost real time on the reference build.

**C.1 — Nominal bit-width does not predict size.** A 4-bit W4A16 checkpoint was
**18.37 GiB resident** while a 5-bit GGUF was **18.46 GiB** — because the W4A16
build left all 48 hybrid layers, `embed_tokens`, `lm_head`, and the vision tower
at fp16, quantising only the full-attention projections and MLPs. With a 248k
vocabulary those tensors are ~2.4 GiB each. **Always compare measured footprints
across publishers**, and check which tensors a quantiser actually touches.

**C.2 — The reported maximum depends on what you asked for.** Requesting 262,144
reported 3.51 GiB available and an estimated maximum of 109,760; requesting
109,760 reported 1.36 GiB and an estimate of 39,200. A single probe returns a
ceiling that is invalid at its own answer — **iterate to a fixed point.**

**C.3 — Measuring on a not-yet-released accelerator under-reports.** A probe run
3 s after stopping the previous server read a 25,486-token pool where the true
figure was 66,446. **Poll free memory until it actually returns.**

**C.4 — Some allocations sit outside the runtime's budget.** Two on the reference
build: FlashInfer's **394 MiB workspace**, allocated lazily on the *first
request*; and the gated-delta-rule prefill kernel. Both mean a server can pass
`/health` and die on first real use. **Reserve ~1 GiB and verify with traffic.**

**C.5 — Lowering `max_model_len` frees no VRAM.** The KV pool is sized from the
memory-utilisation fraction, so the identical OOM reproduced at 109,760, 76,832,
53,312 and 36,064. **Utilisation is the lever.**

**C.6 — Speculative decoding may not fit, and may not be worth it.** The MTP
draft head was present in the checkpoint and the method string was valid, but it
needs **2.37 GiB of unquantised fp16**, loaded *after* the KV pool is claimed —
so it failed identically at utilisations 0.97, 0.95 and 0.85. Even had it fit,
that 2.37 GiB competes with the ~2.5 GiB CUDA graphs occupy, and graphs are
worth ~3x. **Check the draft head's size and precision before planning on it.**

**C.7 — High utilisation is fragile on a machine with a display.** `0.97` worked
only when the desktop happened to hold little memory; otherwise the service
refused to start (`Free memory 22.7 < desired 22.78 GiB`). Unacceptable for a
boot-time service. **Use ~0.94–0.95 on any box with a GUI.**

**C.8 — `set -e` plus a one-sided `&&` silently kills scripts.**
`[ test ] && action` returns non-zero when the test is false, aborting the
script. This made a measurement tool print its banner and vanish. **Use
`if/then/fi`.** Likewise a `grep` that matches nothing fails a `pipefail`
pipeline — append `|| true` where no-match is a valid outcome.

**C.9 — Do not delete the diagnostic on the way out.** An `EXIT` trap removing a
temp log destroyed the evidence exactly when a probe failed unexpectedly.
**Write logs to a durable path.**

**C.10 — Service accounts cannot read your home directory or write your state
tree.** A tool run under the service account died with `PermissionError` on the
caller's cwd; run under your own user it could not write the service-owned log
and compile-cache directories. **`cd` to a neutral directory and fall back to a
user-writable cache**, with a message saying so.

**C.11 — Running a venv binary by absolute path does not put its `bin` on
`PATH`.** A JIT compiler shelling out to `ninja` by name failed with
`FileNotFoundError` while `ninja` sat beside the binary being executed.
**Export the venv `bin` explicitly.**

**C.12 — `systemctl list-units` prefixes failed units with `●`.** Column-based
parsing then yields the bullet instead of the unit name — and only once
something has failed, which is exactly when the cleanup path runs. **Use
`--plain` and match the unit name by pattern.**

**C.13 — Restarting races the accelerator release.** `systemctl restart` starts
the replacement as soon as the old process exits, before the driver frees the
allocation. **Stop, wait for memory, then start.**

**C.14 — Streaming assertions must reassemble the stream.** The expected token
arrives split across SSE frames (`"STREAM"` + `"_OK"`), so grepping raw frames
can never match a correct response.

**C.15 — A tiny request does not verify a large context.** A configuration
verified only by "reply with OK" claimed 109,760 tokens and then OOMed on a 70k
prompt. **Verify at ~85–90% of the claimed ceiling.**

**C.16 — Moving the endpoint breaks clients silently.** Changing the default
profile left a configured editor pointed at a now-disabled port. **Re-point
clients in the same change**, and prefer a stable port.

---

## Appendix D — Reusable artefacts

From [`linux-qwen38/`](linux-qwen38/); the shapes port, the values do not.

| file | role |
|---|---|
| **`scripts/install-node.sh`** | **provisions the whole shipped stack**: llama.cpp (fetch or build), model + projector, router presets, unit, client — and verifies with real inference before claiming success |
| `scripts/configure-opencode.py` | derives the client config from the router presets, so client and server cannot drift |
| **`scripts/select-quant.py`** | picks the highest-quality quant that fits; knows the quant tier ladder, split GGUFs and `n_ctx_train` |
| **`scripts/add-gguf-model.sh`** | adds any GGUF (including abliterated builds) to the router and verifies it answers |
| `scripts/setup-linux-qwen38.sh` | optional vLLM profiles (throughput/short-context) |
| `scripts/serve-profile.sh` | one launcher, dispatches on runtime (vLLM / llama.cpp / router) |
| `scripts/qwen38ctl` | status / start / switch / stop, waits on health and memory release |
| `scripts/measure-ceiling.sh` | fixed-point probe + long-prompt verification |
| `scripts/long-prompt-probe.py` | needle-in-haystack at a target context |
| `scripts/bench-context-sweep.sh` | speed versus depth |
| `scripts/benchmark.py` | concurrency sweep against any OpenAI endpoint |
| `scripts/gen-runtime-descriptor.py` | machine-readable descriptor generated from the configs |
| `tests/test_structural.sh` | no accelerator needed; safe in CI |
| `tests/test_smoke.sh` | the gate the installer will not skip |
| `tests/test_vision.py` | generated image, asserts the token delta |
| `config/router-presets.ini` | dynamic model switching |
