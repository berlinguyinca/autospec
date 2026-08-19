# A dual-Turing Qwen inference node, as measured

Two RTX 2080 Ti (11 GiB each, **sm_75**) serving Qwen3.8-27B and Qwen3.5-9B behind
one nginx port, with on-demand model switching, an API key, a queue view and a
config-health panel.

Every number here was measured on the hardware. Predictions are labelled as
predictions. **The one rule: never configure a number you have not verified with a
real request.**

```bash
# rebuild the whole node on a bare host (drivers already installed)
./scripts/install-node.sh
```

That builds llama.cpp for `sm_75`, fetches every checkpoint in
`config/model-artifacts.yaml` **by pinned revision** and verifies its byte count,
installs the config, units and nginx site, then refuses to claim success without a
real completion.

---

## What it serves

| served id | kind | context | seats | notes |
|---|---|---:|---:|---|
| `qwen3.8-27b` (+ `-50k`, `-40k`) | text | 102,400 | 2 | the default |
| `qwen3.8-27b-100k` | text | 102,400 | **1** | solo seat, guaranteed cache hits |
| `qwen3.8-27b-vision` (+ `-40k`) | vision | 81,920 | 2 | pool pays for the projector |
| `qwen3.8-27b-uncensored` (+ `-50k`, `-40k`) | uncensored | 102,400 | 2 | abliterated |
| `qwen3.8-27b-uncensored-vision` | uncensored + vision | 81,920 | 2 | Q8_0 projector, cheaper |
| `qwen3.5-9b` (+ `-80k`, `-40k`) | text | 81,920 | 2 | one card, ~2.7× faster |
| `qwen3.5-9b-vision` | vision | 81,920 | 2 | one card |

**One model is resident at a time** (`--models-max 1`). Asking for a different id
costs a reload — about 7 s for a 27B off NVMe. Every reload appears as an eviction
in the config-health panel, which is how you find out that two people are
thrashing the node between models.

---

## Using it

| URL | what |
|---|---|
| `http://<node-host>/` | dashboard (needs the API key) |
| `http://<node-host>/status` | public load page, no key |
| `http://<node-host>/api/queue` | public load JSON, no key |
| `http://<node-host>/v1` | OpenAI-compatible API base |

```bash
KEY=$(sudo cat /etc/qwen-turing.key)
curl http://<node-host>/v1/chat/completions \
  -H "Authorization: Bearer $KEY" -H 'Content-Type: application/json' \
  -d '{"model":"qwen3.8-27b","messages":[{"role":"user","content":"Hello"}],
       "max_tokens":512}'
```

OpenCode, in the shape this repository already generates:

```json
{ "provider": { "qwen-turing": {
    "npm": "@ai-sdk/openai-compatible",
    "options": { "baseURL": "http://<node-host>/v1", "apiKey": "..." },
    "models": { "qwen3.8-27b": {}, "qwen3.5-9b": {} } } } }
```

Or derive it from the server so client and server cannot drift:
`configure-opencode.py --presets /opt/qwen-turing/etc/router-presets.ini`.

### Two things that make a working node look broken

1. **Reasoning tokens run first.** `max_tokens: 16` returns **empty content** —
   all sixteen went to reasoning. Allow headroom (512+), or send
   `"chat_template_kwargs": {"enable_thinking": false}`, which answers in 2 tokens.
2. **Context tiers are a client-side contract, not a limit.** Nothing enforces
   them. A `-100k` id means *run solo*. A 100k session beside a 40k one asks for
   140k of a 102,400-token pool and **both die together** — a shared KV pool has
   no admission control.

---

## Measured

| | |
|---|---|
| 27B generation, both cards | **28.7 tok/s** (roofline predicted 29.9) |
| 9B generation, one card | **77.9 tok/s** (predicted 86.8) |
| prompt processing, single session | **594–637 tok/s**, decaying with depth |
| context verified by retrieval | **99,710 tokens** (needle at 60% depth) |
| two concurrent 40k sessions | clean, **zero** KV evictions |
| model switch (reload) | 27B **7.0 s**, 9B 4.2 s |
| context-tier switch (alias) | **538 ms** — free, same resident weights |
| prefix cache hit | **19.8 s → 2.1 s**, 19,425 cached tokens |

**Layer split does not sum bandwidth.** The 27B figure is what one card's
616 GB/s yields against 16.46 GB of weights; the second card adds capacity, not
speed. That is why the 9B — a third of the size, on one card — is 2.7× faster.

Full detail, including what was rejected and what was never tested, in
[`docs/measured-ceilings.md`](docs/measured-ceilings.md).

---

## How it is put together

```
client ─▶ nginx :80 (+:8080)          ONE public port
            ├─ /v1/*  …            ─▶ llama.cpp 127.0.0.1:8090
            ├─ /v1/models          ─▶ dashboard (sanitised)
            ├─ /  /status  /api/*  ─▶ dashboard 127.0.0.1:8081
            └─ auth_request        ─▶ dashboard /api/queue-headers  (X-Queue-*)
```

**Neither backend listens publicly.** Both are loopback-only and verified refused
from off-host, so the firewall has exactly one port to reason about.

### nginx is a single point of failure for inference

A dead proxy is a dead endpoint. That was accepted deliberately in exchange for
queue headers and for moving both backends off every public interface. What is
*not* accepted is a dead **dashboard** taking inference with it: nginx turns a
failed `auth_request` into a client error, so an `@inference_no_headers` fallback
proxies without headers instead. Verified by stopping the dashboard and getting a
200.

Three nginx settings are enforced by `tests/test_structural.sh` rather than
trusted, because each failure presents as *the model hanging* rather than as a
proxy setting:

- `proxy_buffering off` and `proxy_request_buffering off` — completions stream, and
  a 40k prompt is ~230 KB;
- `proxy_read_timeout 900s` — a 100k prompt needs ~170 s of prefill before its
  first token, and nginx's 60 s default would sever exactly the requests this node
  exists to serve.

### TLS

Scaffolded, not enabled: `nginx/qwen-turing.conf` carries a commented `:443` block
with the paths a campus CA will issue into. **Deliberately no redirect and no
HSTS** until a certificate exists — a redirect to a port that is not listening
breaks the endpoint, and HSTS cached before a valid certificate makes the node
unreachable from every browser that saw it. A structural check fails the build if
either appears early. Until then the API key crosses the network in cleartext.

---

## Operating it

```bash
systemctl status qwen-turing@router qwen-turing-dashboard nginx
journalctl -u qwen-turing@router -f
curl -s http://<node-host>/api/queue | python3 -m json.tool     # no key needed
```

Site coordinates live in `/etc/qwen-turing/site.conf` (the units read that path;
`ProtectHome=true` means they cannot see a home directory). The API key is a
root-only file injected by `LoadCredential` — never an `Environment=` value, which
any local user can read from `systemctl show`.

### The sandboxing ceiling

`ProtectSystem=full`, **never `strict`**. Each of `ProtectSystem=strict`,
`PrivateDevices=true`, `DevicePolicy=closed` and `MemoryDenyWriteExecute=` breaks
CUDA on this host, and a structural check fails the build if any reappears. The
reasons also live in the unit as comments, because whoever tightens this next will
read the unit rather than this file.

### Adding a model

1. Add it to `config/model-artifacts.yaml` with `repository`, `revision`, `file`,
   `size_bytes` — and a `local_file` if its remote name could collide. Two
   different projectors both ship as `mmproj-F16.gguf`.
2. Add a preset section to `config/router-presets.ini`. **Vision is always its own
   preset**, never an `mmproj` on a text preset.
3. `./scripts/install-node.sh --skip-build` — it fetches only what is missing.
4. Verify at length: a real completion, and for vision a real image.

---

## What a third card would change

At 33 GiB: `--models-max 2` becomes affordable, making switches instant instead of
a reload, and the 100k preset could keep two seats instead of one. A 204,800-token
pool (two concurrent 100k seats) also comes into range, though compute buffers grow
with sequence length so that wants measuring rather than configuring. Needs a PSU
check first — three cards at ~250 W plus a 32-core CPU wants a 1200 W-class supply.
