# Design: request queueing visibility for the dual-Turing node

**Date:** 2026-08-19
**Status:** approved, pending implementation
**Extends:** [`2026-08-19-turing-dual-qwen-node-design.md`](2026-08-19-turing-dual-qwen-node-design.md)

> Site coordinates stay out of this document; the repository is public. The
> campus subnet is written `<campus-cidr>`, the node's addresses
> `<node-addr>` / `<campus-addr>`.

---

## 0. What already works, and what does not

**Measured before designing anything.** Six concurrent requests against two
slots, on the running node:

| finished at | requests |
|---|---|
| +5.2 s | 2 |
| +10.4 s | 2 |
| +15.3 s | 2 |

All returned HTTP 200. llama.cpp **already queues and already processes in slot
order** — `/props` reports `total_slots: 2`, and excess requests wait rather
than failing. So this work adds *observation*, not scheduling. No scheduler, no
reordering, no admission control.

Two real gaps:

1. **The queue is invisible.** Nothing tells a caller how deep it is or how long
   a wait to expect.
2. **The queue is unbounded.** Six requests for two slots were all accepted.
   The operator chose to keep it that way: report depth, never reject. A busy
   node is slow, not broken.

---

## 1. The measurement problem, stated honestly

`/metrics?model=<id>` exposes `requests_processing`, `requests_deferred`,
`prompt_tokens_total`, `tokens_predicted_total`, `prompt_seconds_total`,
`tokens_predicted_seconds_total` and `n_decode_total`.

**There is no completed-request counter.** Per-request service time therefore
cannot be read directly, and inventing a "typical request size" to divide by
would produce a number that looks measured and is not.

Instead the collector keeps a rolling five-minute window, sampled every second,
and counts a completion as a **decrease in `processing + queued`**. From that:

```
outstanding      = processing + queued
completion_rate  = completions observed / window seconds
requests_ahead   = queued
est_wait_seconds = requests_ahead / completion_rate
```

Three properties this must have, because the alternative is a confident lie:

- **`est_wait_seconds` is `null` until completions have actually been observed.**
  The UI shows `—`, not `0`.
- **The sample count travels with the estimate**, so a figure derived from three
  observations is not presented like one derived from three hundred.
- **It is labelled an estimate.** Requests that start and finish between two
  samples are missed, which biases the rate low and the wait high.

`p50` and `p95` come from the same window: the duration of each observed
busy interval, not a modelled distribution.

**Open question to resolve during implementation, not to paper over:** the
six-request burst never showed `requests_deferred` above 0 in one-second
sampling. Either deferral is shorter-lived than the sample interval, or the
metric does not mean what its name suggests. Task 1 pins this down with a
deliberately slow burst before any arithmetic is built on it. If
`requests_deferred` proves unusable, `outstanding` is derived from
`/slots?model=<id>` (`is_processing` per slot) plus the sampler's own count of
in-flight requests instead.

---

## 2. Architecture

```
                       ONE public port
client ──▶ nginx :80 (and :8080 for compatibility; :443 when the cert lands)
             │
             ├─ /v1/*  /completion  /health  /props  /metrics  /slots
             │        ──▶ llama.cpp 127.0.0.1:8090
             │
             ├─ /v1/models ──▶ dashboard (SANITISED -- see §4.1)
             │
             ├─ /  /status  /api/*  ──▶ dashboard 127.0.0.1:8081
             │
             └─ auth_request ──▶ dashboard /api/queue-headers  (adds X-Queue-*)

dashboard 127.0.0.1:8081 ──▶ llama.cpp /metrics?model=… + nvidia-smi
```

**Only nginx listens publicly.** llama.cpp moves to `127.0.0.1:8090` and the
dashboard to `127.0.0.1:8081`, so the firewall has exactly one port to reason
about and neither backend can be reached directly even from inside the LAN.

The endpoint namespaces do not collide, which is what makes one port viable:
llama.cpp owns `/v1/*`, `/completion`, `/health`, `/props`, `/metrics`,
`/slots`; the dashboard owns `/`, `/status`, `/api/*`. So users get one URL for
everything:

| URL | what |
|---|---|
| `http://<node-host>/` | the dashboard |
| `http://<node-host>/status` | public load page, no key |
| `http://<node-host>/v1` | OpenAI-compatible API base |

`:8080` stays served by the same server block so anything already configured
against it keeps working; `:80` is the address to publish.

## 3. The proxy

nginx, not a bespoke proxy, because a correct streaming HTTP proxy is
error-prone to write and this one carries every inference request.

- `proxy_buffering off` and `proxy_request_buffering off` — streaming
  completions must pass through token by token, and prompt bodies reach 230 KB
  at 40k tokens, so neither direction may be buffered whole.
- `client_max_body_size` generous enough for a full-context prompt.
- `proxy_read_timeout` **900 s**, sized in §7.5 from measured prefill: a 100k
  prompt needs ~210 s before its first token, and a 40k pair took 114 s wall
  clock. nginx's 60 s default would sever exactly the long requests this node
  exists to serve, so the timeout is set from the slowest tier rather than from
  the fastest observation.
- Headers are injected with the `auth_request` pattern: a subrequest to
  `/api/queue-headers` on the dashboard returns `204` plus `X-Queue-*` headers,
  `auth_request_set` copies them into variables, `add_header` emits them.
  `auth_request` must **never** be able to reject the inference request — the
  endpoint always returns 204, and nginx is configured so a subrequest failure
  is non-fatal.
- Authorization passes straight through; nginx never sees or needs the key.
- Binding `:80` needs the master process to start as root; workers drop to an
  unprivileged user as nginx does by default. The backends stay unprivileged.

### TLS, prepared but not enabled

A campus certificate is being requested, so the config is written to make
enabling it a two-line change rather than a redesign:

- The `server` block for `:443 ssl` ships **commented out**, with
  `ssl_certificate` / `ssl_certificate_key` pointing at
  `/etc/ssl/qwen-turing/`, and a note that the paths are what the campus CA
  issues into.
- **No HTTP-to-HTTPS redirect and no HSTS until the certificate exists.**
  Shipping a redirect to a port that is not listening breaks the endpoint, and
  HSTS before a valid certificate makes it unreachable from browsers that have
  cached the header.
- `ufw` gains `443` only when the cert is installed, not in advance.

Until then the API key crosses the campus network in cleartext. That was already
the accepted position in the parent spec; port 80 does not change it, and the
`:443` scaffolding is what shortens the window once the cert arrives.

Headers emitted: `X-Queue-Slots`, `X-Queue-Processing`, `X-Queue-Depth`,
`X-Queue-Fullness`, `X-Queue-Est-Wait-Seconds`.

---

## 4. Public surfaces, and what they must not leak

`/v1/models` on llama.cpp is unauthenticated and exposes the child instance's
full argv — including the binary path and the API key **file location**. That is
the standard to avoid, not to copy.

`GET /api/queue` and `GET /status` are unauthenticated and return **only**:
`slots`, `processing`, `queued`, `fullness`, `est_wait_seconds`,
`completion_rate`, `p50_seconds`, `p95_seconds`, `samples`, `model_loaded`
(boolean, not the name).

Never: prompts, completions, model paths, file paths, argv, key locations, GPU
serial numbers, hostnames. A test asserts the public payload's key set matches
that allow-list exactly, so a future field cannot leak by being added to a
shared serialiser.

`/api/stats` and `/` keep requiring the key and may show everything.

### 4.1 Sanitising `/v1/models`

llama.cpp's router publishes each preset's **full child argv** on `/v1/models`
without authentication — binary path, model paths, and the API key's *file
location*. On a single public port that is the most exposed thing on the node.

Clients genuinely need `/v1/models` for discovery, so it is not blocked. nginx
routes it to the dashboard, which fetches the upstream list and returns only
`id`, `object`, `owned_by`, `created` and `aliases` — dropping `status` entirely.
A test asserts the sanitised payload contains no `/` character in any value, so a
path cannot reappear by way of a new upstream field.

llama.cpp's non-standard `/models` alias is **denied at nginx** rather than
sanitised: nothing needs it, and leaving an unsanitised twin reachable would make
the sanitising pointless.

---

## 5. Dashboard

A queue panel above the existing token panels:

- capacity bar: `processing + queued` against `slots`
- queued count, and estimated wait (or `—` with a "no samples yet" note)
- completion rate, p50 / p95 response time
- generation and prompt throughput

## 6. Connection examples, rendered from live state

A copy-paste panel built from the served model ids and the actual base URL, so
the examples cannot drift from what the node serves: `curl`, OpenAI Python SDK,
OpenAI Node SDK, and OpenCode in the `@ai-sdk/openai-compatible` shape this
repository already generates:

```json
{ "provider": { "qwen-turing": {
    "npm": "@ai-sdk/openai-compatible",
    "options": { "baseURL": "http://<campus-addr>:<QT_PORT>/v1", "apiKey": "..." },
    "models": { "qwen3.8-27b": {}, "qwen3.5-9b": {} } } } }
```

It also states the reasoning-token caveat, because it is the first thing that
will make the node look broken: both models emit reasoning before content, so
`max_tokens: 16` returns empty content. Either allow headroom or send
`chat_template_kwargs: {enable_thinking: false}`.

`configure-opencode.py` remains the supported path — it derives client config
from the router presets, so client and server cannot drift.

---

## 7. Exposure changes

| surface | before | after |
|---|---|---|
| llama.cpp | `0.0.0.0:8080` | **`127.0.0.1:8090`** |
| dashboard | `<node-addr>:8081` | **`127.0.0.1:8081`** |
| nginx | — | **`0.0.0.0:80` + `:8080`**, ufw: `<campus-cidr>` + internal |
| nginx TLS | — | `:443` prepared, enabled with the campus cert |

The dashboard becomes campus-reachable **through nginx**, not by binding a public
interface itself. That is precisely why `/status` and `/api/queue` are built to
expose load and nothing else, and why `/v1/models` is sanitised: on one public
port, every unauthenticated route is the node's attack surface.

---

## 7.5 A 100k context tier

The operator wants ~100k context for light coding work and accepts fewer
concurrent seats for it. On this model that is cheap, because only 16 of 64
layers grow a KV cache — 18.0 KiB/token at `q4_0`.

Non-KV footprint is **16,902 MiB**, derived from the measured 18,342 MiB with the
27B resident minus the 1.41 GiB the 81,920-token pool costs. So:

| pool | KV | resident total | free | tiers it funds (max 2 slots) |
|---:|---:|---:|---:|---|
| 81,920 (today) | 1.41 GiB | 18,342 MiB | 4,186 MiB | 40k x2 |
| **102,400** | **1.76 GiB** | **18,702 MiB** | **3,826 MiB** | **100k x1, 50k x2, 40k x2** |
| 163,840 | 2.81 GiB | 19,782 MiB | 2,746 MiB | 100k x1 |
| 204,800 | 3.52 GiB | 20,502 MiB | 2,026 MiB | 100k x2 |

**Decision: `c = 102400`.** Raising the pool from 81,920 costs **360 MiB** and
buys the 100k tier outright, leaving 3.8 GiB spare. 204,800 would fund two
concurrent 100k seats but leaves only ~2 GiB, and compute buffers grow with
sequence length — so that is a measurement to attempt later, not a number to
configure now.

This needs no new preset and no reload to reach. The pool is shared
(`kv-unified = true`), so it is expressed as **tier aliases** on the existing
model, which is how this node already rations context:

| alias | seats it advertises | pool used |
|---|---:|---:|
| `qwen3.8-27b-100k` | 1 (solo) | 102,400 |
| `qwen3.8-27b-50k` | 2 | 102,400 |
| `qwen3.8-27b-40k` | 2 | 81,920 |

"Dropping a slot" is exactly what the `-100k` alias means: it declares a solo
session. There is still no admission control, so the aliases are a **client-side
contract** — a 100k session running beside a 40k one asks for 140k of a 102,400
pool and both die together. `check_presets.py` must fail the build if any tier
advertises more sessions than there are slots or more pool than exists.

### The cost is latency, not memory

At the **measured** prompt rate of 475 tok/s (39,909 tokens prefilled in 84 s —
not the ~50 tok/s the short-prompt benchmark reports, which is dominated by
overhead):

| prompt | time before the first token |
|---:|---:|
| 40,000 | ~84 s (1.4 min) |
| 100,000 | **~210 s (3.5 min)** |

Three consequences that must be designed for, not discovered:

1. **`proxy_read_timeout` must exceed prefill plus generation**, which is why
   §3 sets it to 900 s rather than to a small multiple of the 114 s that two
   concurrent 40k sessions took. Sizing a timeout from the fastest tier is how a
   proxy ends up severing the slowest one.
2. **The queue's wait estimate must tolerate multi-minute requests.** A five-
   minute rolling window can contain a single 100k request and nothing else, so
   the sample count matters more than ever and `p95` will be dominated by tier.
3. **`cache-reuse` is what makes the 100k tier usable for coding.** A follow-up
   turn that shares a prefix prefills only the delta rather than paying 3.5
   minutes again. This is the within-slot reuse that is already on; cross-slot
   reuse stays off for the reasons in the parent spec.

The tier is verified the same way as the others: a needle retrieved from a real
~100k prompt, not an advertised limit.

---

## 8. Acceptance criteria

1. A deliberately slow burst drives `queued` above zero, and the dashboard shows
   it. **This is the test that proves the queue is observable at all.**
2. `est_wait_seconds` is `null` with zero samples and becomes a number after
   completions are observed.
3. Inference still works through nginx: a real completion, and **a streamed
   completion arriving incrementally rather than in one buffered blob**.
4. A 40k-token prompt still succeeds through the proxy — the case a default
   `proxy_read_timeout` and request buffering would break.
5. `X-Queue-*` headers present on inference responses.
6. `/api/queue` and `/status` answer without a key; their payload matches the
   allow-list exactly.
7. `/api/stats` still 401s without a key.
8. Killing the dashboard does **not** break inference — headers go missing,
   requests still succeed.
9. Concurrency unchanged: two 40k sessions still complete with zero KV evictions.
10. **A needle is retrieved from a real ~100k prompt** through the proxy, and the
    request is not severed by a timeout.
11. `check_presets.py` passes with the 102,400 pool and all three tier aliases.
12. `http://<node-host>/` serves the dashboard and `http://<node-host>/v1`
    serves the API — one port, both.
13. **Neither backend is reachable directly**: `<node-addr>:8090` and
    `<node-addr>:8081` refuse connections from off-host.
14. `/v1/models` through nginx contains **no filesystem paths**, and `/models`
    is refused.
15. Both units and nginx survive a reboot.

---

## 9. Out of scope

Per-request tickets or queue positions, hard capacity limits and 503-when-full,
request prioritisation, authentication for the public load surfaces, a second
concurrent 100k seat (the 204,800 pool, which needs measuring first), and any
change to the slot count itself — the 100k tier reduces seats by declaring a
solo alias, not by reconfiguring `parallel`.
