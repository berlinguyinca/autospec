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
| `qwen3.5-9b` (+ `-80k`, `-40k`) | text | 81,920 | 2 | one card, ~2.6× faster |
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
| `http://<node-host>/v1` | OpenAI-compatible API base (balanced across the fleet) |

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
3. **`HEAD` used to answer 501 on every path.** Fixed. Both listeners now answer
   `HEAD` with the headers a `GET` would send — same status, same real
   `Content-Length` — and no body. A monitor probing `HEAD /api/queue` or
   `HEAD /api/gateway-health` is the cheap, key-free health check. `HEAD` on the
   *inference* paths answers `405` with `Allow: GET, POST` rather than relaying:
   a relayed `HEAD` would take a pipe from the tunnel pool and move the caller's
   cache affinity, to collect a refusal from a runtime that speaks only GET and
   POST.

---

## The dashboard

Sections live behind sidebar navigation rather than one long scroll, because a
reader almost always wants exactly one of them:

| Section | Answers |
|---|---|
| Overview | is it alive, is there a free seat, what are the cards doing |
| Models | what can I ask for, and what does each cost in context and seats |
| Servers | which machines are reachable, and which GPUs hold which models |
| Connect | how do I point my client at it |
| Keys | sign in, create a key, revoke one |
| Usage | what has each key actually spent |
| Health | what is misconfigured, and what should I change |

Two details worth knowing. The status block — resident model, decode rate, seats —
is **pinned beside every section**, so checking that the node is alive never costs
a click. And **Health shows only what needs acting on**, with the clean checks
behind a toggle and a count on the nav item: a panel that lists everything trains
you to skim it.

Each section is deep-linkable (`/#keys`), so a refresh keeps you where you were.

## Who may see what

Reading this node needs no credential. Signing in buys exactly two things.

| | public | needs sign-in |
|---|---|---|
| health, queue, model catalog, connection instructions | ✅ | |
| fleet: which server, what it serves, how loaded, how fast, its GPUs and seats | ✅ | |
| leaderboard, by display name | ✅ | |
| creating and revoking API keys | | ✅ |
| the chat panel | | ✅ |
| per-key usage detail, server addresses, owners | | ✅ |

Every public payload is a projection in `scripts/publicview.py`, built by
iterating an allow-list rather than by deleting private keys from a full one — so
a field added upstream is invisible until someone names it, and the failure mode
of forgetting one is a missing value rather than a disclosure. Two categories stay
out by construction: **addresses** (`base_url`, and upstream error strings, which
quote the host they failed to reach) and **identities** (emails and subjects).

### The chat panel

Signed in, `/api/chat` runs a conversation against the fleet. It is the one place
here where a cookie pays for GPU time, so `Sec-Fetch-Site` is checked before the
body is read — without that, any page on the internet could make a signed-in
browser spend this node. After validation it is the ordinary balanced path: the
same proxy, eligibility, scheduling, affinity and accounting as a key-holder's
request. Usage is attributed to the person under the sentinel key id
`dashboard-chat`, which is deliberately **not** a minted key: a real credential
its owner could neither see nor revoke would invert the point of the key surface.

The transcript lives in the page and nowhere else. Reload and it is gone.

## Accounts and API keys

Every user gets their own keys. **Sign in on the dashboard and create one** —
the key is shown once, revocable on its own, and everything it does is
attributed to you.

Signing in is also how you read the dashboard. The page used to open a prompt
asking for the shared secret, which is the very thing per-user sign-in replaces;
now an API key is an explicit fallback for anyone without an account rather than
a modal you cannot get past. One consequence worth knowing: `/api/stats` is
authorised by the gateway and accepts **either** a session or a key, so scripts
that read it keep working unchanged.

The distinction that shapes this: **sign-in authenticates people, keys
authenticate machines.** An agentic client cannot complete a browser redirect,
so a human signs in once and mints long-lived keys for the tools that need them.

| | |
|---|---|
| Sign in | the dashboard's *Your account and API keys* panel |
| Create a key | **anyone who can sign in** — the pool is the audience |
| See all users' keys and usage | needs the admin group |
| Revoke | immediate on this node; other nodes converge within the refresh interval, which the API states in its reply |

Minting is open to every member of the pool (`QT_COGNITO_USER_GROUP="*"`). Naming
a group there narrows it again, and that is a config change rather than a code
change. The **admin** group is separate and is never opened by `"*"`: it grants
seeing and revoking other people's keys, which is a different question from being
allowed to use the node.

Two things worth knowing before you lose a key:

* **The secret is shown exactly once** and stored only as a SHA-256 hash. Nobody
  can retrieve it later — not an administrator, not the database owner. Lost it?
  Revoke and make another.
* **The key id is not secret.** It identifies the key in the dashboard and on
  every usage row, which is what makes per-key attribution possible at all.

Creating and revoking keys **require HTTPS** and are refused over plain HTTP: a
key minted over cleartext is a key already disclosed. Inference itself still
works on plain `:80`, so existing clients are unaffected.

## Routing: the default base URL is the balancer

`/v1` does not mean "this machine". It means **whichever machine should serve
this** — so every client already pointed at this node is load-balanced without
being reconfigured, which is the only way a default can mean anything.

| Target | Base URL |
|---|---|
| **balanced (the default)** | `/v1` |
| the balancer, named | `/u/auto/v1` |
| this node, insisted on | `/u/local/v1` |
| a named server | `/u/<id>/v1` |

Registration is one entry in `/etc/qwen-turing/upstreams.yaml` — see
[`config/upstreams.yaml.example`](config/upstreams.yaml.example).

Two more ways to join, both from the dashboard rather than from a file. A **tunnelled** server runs [the agent](../agent/README.md), dials **out** to
`wss://<node-host>/api/agent/…` and holds connections open that this node invokes
inference over — so the box needs no inbound port at all. A **static** server is
one the node dials, registered through the same flow instead of by editing the
registry. Any OpenAI-compatible server qualifies; llama.cpp is the tested one.

The first tunnelled server was measured at **2748.9 tok/s prefill through the
tunnel against 2750.3 direct on the box** — 0.05% apart — and its inference port
is now bound to loopback with its firewall exception deleted, because nothing
needs to reach it any more.
See [the design](../../docs/superpowers/specs/2026-08-19-agent-tunnel-self-registration-design.md).

A **dialled** server may carry the credential it demands, given at attach time
and stored on the node — reported as *held*, never returned. A **tunnelled** one
cannot, and that is not a gap: a pipe carries bytes verbatim, so the agent would
have to rewrite the request head to add a header. Bind that target to loopback
instead; nothing can reach it, which is stronger than a key.

Attaching is self-service for anyone who can sign in. Joining the **balanced**
pool is an admin action, because a registered server declares its own model ids —
attaching your own box is cheap, being inserted into everyone's default route is
not.

Balancing applies **only** to the endpoints that name a model — `chat/completions`,
`completions`, `embeddings`, `rerank`. `/health`, `/slots`, `/metrics` and
`/props` describe *this* box, so answering them from another machine would report
someone else's GPUs as ours. That list is an allow-list rather than an
exclude-list, so the next endpoint llama.cpp grows stays local until someone
decides otherwise.

**A server is only ever sent a model it advertises.** This is the rule that
matters most, because the failure it prevents is silent: llama.cpp does **not**
refuse a request for a model it has not got — it answers with whatever is loaded.
Measured here, a request naming `qwen3.5-9b-vision` came back served by
`qwen3.8-27b` with no error at all. A vision request answered by a text model, or
an uncensored one by the aligned model, is far worse than a refusal. So each
server's own `/v1/models` is a routing input, and a model nothing here serves is a
clean `404` rather than a substitution.

Among the servers that *can* serve it, the choice is **whichever will answer
soonest**, from numbers this node measured rather than specifications anyone
declared:

```
estimate = queued_ahead x mean_service  +  prompt_tokens / prefill_rate
```

`prefill_rate` and `mean_service` are rolling means from this node's own
accounting for that server and that model — `prompt_ms` and `predicted_ms` come
from the model's own response, so a box that claims a 4090 and delivers 40 tok/s
is ranked by the 40. A server with no history yet gets a conservative default, so
it is tried and thereby measured; scoring it zero would rank it first forever and
scoring it infinity would mean it never ran again.

Two things sit outside the estimate. An admin-set **priority** tier is absolute —
only the highest tier with a candidate is considered, because an override whose
effect depends on load is not an override. And **prefix-cache affinity** is a
factor on the prefill term, worth about the tenfold it was measured at: the
machine already holding your conversation is strongly preferred, but a warm
server that is ten times slower still loses, which a plain "stick with the last
one" rule gets wrong.

Every reply carries `X-Routed-To`, `X-Routed-Why` and `X-Routed-Est` (the
predicted seconds), and the Servers panel tallies the reasons and shows each
server's measured rate. A scheduler that cannot be second-guessed from outside is
one nobody can debug — and without the tally, "the balancer quietly became
local-always" looks exactly like a balancer that is working.

A tunnelled server's capacity is the pipes it holds open, so it can genuinely run
out — unlike a directly reachable one. A balanced request **ranks a saturated
server last** and goes around it; a pin waits up to 5 s and then gets `503
no_capacity`, naming the server. That is honest: this node cannot invent a
socket, and queueing the request here would make the dashboard's queue arithmetic
a lie.

A refusal distinguishes three situations a single message would blur — the model
is on a server that is **not in the balanced pool** (a `404` that tells you to
pin `/u/<id>/v1`, since you may well be its owner), on one that is **not
answering** (`503`, where waiting may help), or **nowhere** (`404`, where it will
not).

Pinning `/u/<id>/v1` names the *machine*, not the model — so the same check
applies there, and a pin for a model that server has not got is refused rather
than silently answered by the wrong weights. `X-Route-Force: 1` sends it anyway
for the cases where you mean it.

`/v1/models` answers with the **fleet's** models, not this node's: every model
served by an online server in the balanced pool, which is exactly the set `/v1`
can route to. Unpromoted and offline servers are left out, because advertising a
model that the next request would refuse is worse than not advertising it. Remote
entries are ids only — which server holds what is fleet composition, and that
lives in the authenticated `/api/servers`. The endpoint stays public, because a
client needs discovery before it has a key.

**Reading the model does not mean parsing the body.** At most 8 KB of the request
is buffered to find the top-level `model` field, then forwarded unchanged; a 100k
prompt is ~400 KB and still streams straight through. If the field is not in that
prefix the request is kept **here** and labelled `blind`, never sent to a server
that might substitute. `model` is accepted only as a key of the top-level object,
so a `"model"` inside a user's own message text cannot steer routing.

Two behaviours to rely on. Servers are probed on a timer and **never on the
request path**, so a request for a server that is switched off is refused in
milliseconds rather than waiting out a proxy timeout — and this node is probed
the same way as any other, because a live gateway with a dead runtime is exactly
the case a hard-coded "local is always up" cannot express. And a registered
server that answers **without a key** is flagged in the panel as a configuration
fault: its port must be restricted to this node, or a client can reach it
directly and bypass authentication entirely.

### What "usage" means, exactly

Token counts are read from the model's own response, so they are exact rather
than estimated. Each row carries prompt, completion and **cached** tokens — the
last one matters here, because the prefix cache is worth roughly tenfold on a
warm slot, so the cached share is the difference between a cheap conversation
and an expensive one.

Usage is a **scoreboard**: everyone signed in sees everyone's totals, ranked by
tokens, because a leaderboard showing only your own row would not be one. It
stays behind authentication — a lab scoreboard, never a public one.

Requests whose client disconnected mid-stream are counted **separately** as
unknown. Their token counts genuinely are not available — the terminal response
chunk never arrived — and they are never estimated from byte counts nor reported
as zero.

### If you cannot sign in

There is no shared key any more: the only credential this node accepts is a
per-user key, and the only self-service way to get one is signing in. That is the
point — but it means a provider outage would otherwise leave you with no way in.

The break-glass runs on the node, as root, and goes through the same store the
API does — so it is not a backdoor, just the same operation without a browser:

```bash
head -1 /etc/qwen-turing/llmgw_app.pw \
  | sudo -u qwen-turing --preserve-env=QT_DB_HOST,QT_DB_PORT,QT_DB_NAME,QT_DB_USER \
      python3 - "<your-subject-id>" "label" <<'EOF'
import os, sys
sys.path.insert(0, "/opt/qwen-turing/bin")
from keystore import KeyStore
pw = sys.stdin.readline().strip()
dsn = (f"host={os.environ['QT_DB_HOST']} port={os.environ['QT_DB_PORT']} "
       f"dbname={os.environ['QT_DB_NAME']} user={os.environ['QT_DB_USER']} "
       f"password={pw} connect_timeout=3")
s = KeyStore("/var/lib/qwen-turing/keys.sqlite3", dsn=dsn)
s.migrate_local(); s.upsert_user(sys.argv[1], None, None)
print(s.mint(sys.argv[1], label=sys.argv[2])[0])
EOF
```

Two details that matter. The registry password is passed on **stdin**, never in
argv or the environment, because both are readable by other processes. And the
store is written as the **service user**, so the mirror does not end up owned by
root and unwritable by the gateway that has to use it.

## Measured

| | |
|---|---|
| 27B generation, both cards | **28.7 tok/s** (roofline predicted 29.9) |
| 9B generation, one card | **75.8 tok/s** on `UD-Q4_K_XL` (77.9 on the plain quant it replaced) |
| prompt processing, single session | **594–637 tok/s**, decaying with depth |
| context verified by retrieval | **99,710 tokens** (needle at 60% depth) |
| two concurrent 40k sessions | clean, **zero** KV evictions |
| model switch (reload) | 27B **7.0 s**, 9B 4.2 s |
| context-tier switch (alias) | **538 ms** — free, same resident weights |
| prefix cache hit | **19.8 s → 2.1 s**, 19,425 cached tokens |

Both models use **Unsloth Dynamic** quants: the 27B is `UD-Q4_K_M` at Dynamic
**v3.0** (the pinned revision is repository HEAD and its card says so), and the 9B
moved from a plain `Q4_K_M` to `UD-Q4_K_XL` — 2.8% slower because the file is
larger, with the quality gain being Unsloth's claim rather than this node's
measurement.

**Layer split does not sum bandwidth.** The 27B figure is what one card's
616 GB/s yields against 16.46 GB of weights; the second card adds capacity, not
speed. That is why the 9B — a third of the size, on one card — is 2.7× faster.

Full detail, including what was rejected and what was never tested, in
[`docs/measured-ceilings.md`](docs/measured-ceilings.md).

---

## How it is put together

```
client ─▶ nginx :80 (+:8080)          ONE public port
            ├─ /v1/*  /u/*  …      ─▶ gateway 127.0.0.1:8082
            │                           ├─ llama.cpp 127.0.0.1:8090
            │                           └─ another GPU server
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
curl -sI https://<node-host>/api/gateway-health                 # HEAD: is it up?
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
