# Self-registering GPU servers over a held-open connection — design

**Status:** design, approved in shape; awaiting spec review before planning.

**Supersedes nothing; extends** `2026-08-19-multitenant-keys-usage-federation-design.md`
§6, which assumes **the node can reach the server**. Everything here is the
inverse case: the server reaches the node, and keeps that connection open.

---

## 1. The problem with today's federation

A server joins the fleet only if two things are true, and both are the operator's
work rather than the server's:

1. Someone edits `/etc/qwen-turing/upstreams.yaml` on the node.
2. The node can open a TCP connection **to** the server — so the server must have
   a routable address and an inference port listening on it.

The second is the expensive one. It puts an unauthenticated inference port on the
GPU box's network and protects it with a firewall rule, which is exactly what the
Servers panel already flags as a configuration fault: *"reachable without a key —
restrict its port to this node"*. Today the first federated box is protected by
**one iptables chain and nothing else**. A rule that is one `iptables -F` away
from exposing a model to the LAN is not a security boundary anybody should rely
on twice.

It also excludes every machine that cannot accept inbound connections: a
workstation behind NAT, a laptop, a box in another building, anything off-campus.

**What this design adds:** a server runs a small native agent, dials **out** to
the node over 443, registers itself, and holds connections open that the node
invokes services over. The GPU box then needs **no inbound port at all** —
nothing to firewall, nothing to leave open by accident.

---

## 2. Trust model

### 2.1 Attaching is self-service; joining the default pool is not

Anyone who can sign in may attach a server, the same audience that may already
mint keys (`QT_COGNITO_USER_GROUP="*"`). This is a deliberate widening: attaching
your own GPU box should not need an operator.

But a registered server **declares its own model ids**, and the balancer routes on
those declarations. So a server that claims `qwen3.8-27b` can answer other
people's default-route requests with anything at all — wrong weights, degraded
quantisation, or a prompt logger. Attaching a box is cheap; being inserted into
*everyone's* default route is not the same act.

Therefore:

| | who | effect |
|---|---|---|
| **Attach** | any signed-in pool member | usable immediately at `/u/<id>/v1` |
| **Promote to the default pool** | `llm-admins` | `/v1` may route strangers to it |

The owner gets full use of their own hardware the moment it attaches. Nobody
silently repoints the shared default. The panel names the owner of every attached
server, and `usage_events.upstream` already attributes every request to the server
that served it, so misbehaviour is traceable rather than anonymous.

### 2.2 Three credential namespaces, never interchangeable

* `qtk_…` — a **user** key. Authenticates inference requests. Already exists.
* `qts_…` — a **server** credential. Authenticates an agent's control and pipe
  connections. Never accepted as a user key; never grants inference.
* `qte_…` — a single-use **enrolment token**, 30-minute TTL, traded once for a
  `qts_` credential.

Each is checked by its own function against its own table. A `qts_` presented as
a bearer token on `/v1/chat/completions` is refused exactly like a random string,
and the reverse holds. This is stated because the cheap implementation — one
`authenticate()` for everything — is how a server credential silently becomes an
inference key.

### 2.3 The node never tells the agent where to connect

The agent's forwarding target comes from **its own config file** on its own
machine. No message in either protocol carries a destination.

This is the invariant that keeps a compromised node from turning every attached
agent into an SSRF pivot into its owner's private network. It falls out of the
design as specified, which is precisely why it must be written down: adding a
`target` field to the handshake would look like a convenience and would hand
whoever controls the node a port scanner inside every member's LAN.

A structural test asserts neither protocol message carries a destination field.

---

## 3. Transport: a pipe pool, not a multiplexer

### 3.1 One pipe carries exactly one HTTP conversation

The obvious reading of "keep a connection open and invoke services over it" is a
multiplexed tunnel: many concurrent requests sharing one socket, each a stream of
frames. That design requires reimplementing what TCP already does — credit-based
flow control, per-stream cancellation, and a scheduler that stops a 469 KB request
body from blocking another session's tokens behind it. This node's measured
properties (a 469 KB body, 185 s of silent prefill, token-by-token SSE, 2.5 MB of
RSS growth) all live on that path, and a multiplexer puts new failure surface
directly under them.

So instead: **one WebSocket = one byte pipe = one HTTP conversation.**

```
GPU box (no inbound port)                    Node
  agent ──wss──▶ /api/agent/control          identity + liveness (one, long-lived)
  agent ──wss──▶ /api/agent/pipe   × K       idle pipes, opaque bytes
                                               │
                       gateway takes an idle pipe, speaks HTTP/1.1 over it,
                       closes it when the response ends; the agent immediately
                       opens a replacement
```

What this buys, all of it for free rather than for code:

* **No head-of-line blocking and no flow control to write.** Each request owns a
  socket, so backpressure is TCP's, end to end.
* **Cancellation is closing the pipe** — which is already how a client abort
  becomes a `499` here. There is no CANCEL message to get wrong.
* **No keep-alive body-drain class of bug.** A pipe serves one request and dies,
  so there is no "next request on this connection" to corrupt. That bug has
  already shipped twice in this project (once as
  `code 501, Unsupported method ('{"model":...}POST')`, once on the routing
  refusals); this transport cannot express it.

The cost is sockets instead of frames: a box with `--parallel 2` holds roughly
four to six. That is the trade, taken deliberately.

### 3.2 The pool prepays the TLS handshake

One request per pipe would mean a TLS handshake per request — on the critical
path, which is unacceptable. It is not, because the agent keeps **K idle pipes
open at all times** (default `slots + 2`) and replaces each one the moment it is
consumed. The handshake happens *before* the request exists.

### 3.3 The protocol, in full

**Control** — `GET /api/agent/control`, `Authorization: Bearer qts_…`, WebSocket
upgrade. JSON text frames:

| direction | message | fields |
|---|---|---|
| agent → node | `hello` | `agent_version`, `server_id`, `gpus`, `slots`, `note` |
| node → agent | `ready` | `pool_member`, `pipes_wanted`, `heartbeat_seconds` |
| node → agent | `need_pipes` | `n` (nudge when the pool runs dry) |
| agent → node | `capabilities` | `gpus`, `slots` (on change) |

Note what is absent: no model list (the node probes `/v1/models` over a pipe on
the timer it already runs, so there is one source of truth and one code path), and
no destination (§2.3).

**Pipe** — `GET /api/agent/pipe`, same credential, WebSocket upgrade. After the
handshake the connection is **opaque**: binary frames the node sends are bytes the
agent writes to its local target; bytes the agent reads back become binary frames.
The agent opens its local TCP connection **lazily**, on the first byte, so an idle
pipe does not pin an idle llama.cpp connection.

**Duplicate ids.** If a live control connection already exists for a server id,
the new one is closed with `4409`. A rebooted box therefore waits for the stale
connection to be reaped — which happens within one missed heartbeat cycle
(§5) — and its reconnect backoff covers that gap.

### 3.4 Keeping idle pipes alive through nginx

`proxy_read_timeout 900s` will reap a pipe that carries no traffic, so a quiet
hour would otherwise be followed by a connection reset on the first real request.
The node sends a **WebSocket ping on each idle pipe every 240 s**. Ping and pong
are control frames, so they do not disturb the opaque payload — this works
*because* the transport is WebSocket rather than a raw byte stream.

The ping must originate at the **node**, not the agent: nginx resets
`proxy_read_timeout` on data read *from the upstream*, and the gateway is the
upstream. An agent-side keepalive would be the one direction that does not stop
nginx reaping the pipe. The agent's pong then puts traffic on the other direction
too, so both of nginx's timers stay live.

### 3.5 Verified: the gateway's data path does not change

`http.client` speaks HTTP over any object providing `sendall` and
`makefile("rb")`. Checked before writing this spec, not assumed: a request was
issued and a chunked response read through a hand-made pipe object.

The single seam this adds to the gateway is therefore one function:

```python
def connect(server) -> socket-like    # a real TCP socket, or a tunnel pipe
```

`_relay`, `_send_upstream_request`, `_pump`, the 900 s timeouts, the accounting,
the 8 KB model peek and the eligibility filter are all untouched. A tunnelled
server is not a second kind of upstream; it is the same upstream reached through a
different socket.

---

## 4. Capacity, and a refusal that has to be honest

The node cannot invent a socket. When every pipe for a server is in use the
options are to wait, to queue, or to refuse.

Queueing is out: property 2 of the gateway is that it adds no admission control
of its own, because a second queue would make the dashboard's queue arithmetic
lie. So:

* The node waits up to **5 s** for an idle pipe.
* Failing that: `503`, `type: "no_capacity"`, naming the server, counted in the
  routing tally.
* For the **balanced** route this is mostly invisible, because readiness is part
  of ranking: a tunnelled server with no idle pipe is ranked **last** among
  eligible servers rather than excluded. `/v1` routes around a busy box instead
  of refusing; only an explicit `/u/<id>/v1` pin can hit `no_capacity`.

This is a failure mode that does not exist for directly-reachable servers, and it
is the price of the box having no inbound port. It should be visible in the panel
rather than surprising in a log.

---

## 5. Liveness, disconnect, reconnect

| control connection | state | in the balanced pool? |
|---|---|---|
| enrolled, never connected | `unknown` | no |
| live | `online` | if promoted |
| closed or heartbeat missed | `offline`, with `last_seen` | no |

The node pings the control connection every **20 s** and reaps it after **10 s**
without a pong, which is what makes a half-open connection — a box that lost power
rather than closing cleanly — resolve in about half a minute instead of never.

The agent reconnects with exponential backoff and jitter (1 s → 60 s cap). Jitter
matters at fleet scale: a node restart would otherwise bring every agent back in
the same second.

An offline server keeps its row, its models and its `last_seen` in the panel. It
disappears from routing, not from view — the existing distinction between
*offline* and *never configured* is one the panel already makes.

---

## 6. The agent: one native executable, no dependencies

### 6.1 Go, and why the choice is really about TLS

The agent must speak `wss://` on Linux, Windows and macOS. TLS is the dependency
that cannot be avoided by discipline: hand-rolling it is out of the question, and
linking OpenSSL, SChannel or SecureTransport means a C toolchain and per-platform
build hosts. Go's standard library carries a pure-Go TLS stack and loads the
system root store on all three, so the whole agent is stdlib.

Verified on this machine rather than asserted — five targets, one build host, no C
toolchain, and `go list -m all` reporting no dependency beyond the main module:

| target | stripped size |
|---|---|
| linux/amd64 | 3412 KB |
| linux/arm64 | 3264 KB |
| windows/amd64 | 3561 KB |
| darwin/arm64 | 3299 KB |
| darwin/amd64 | 3522 KB |

Built with `CGO_ENABLED=0 go build -trimpath -ldflags="-s -w"`. Around 3.4 MB
including a TLS stack, which is the whole point: one file to copy, nothing to
install beside it.

The WebSocket client is written against RFC 6455 directly (~150 lines: the
Upgrade handshake, client-side masking, ping/pong, close) rather than pulled in as
a module, so the dependency count stays at zero. The frame codec is the one piece
that exists twice — Go on the agent, Python on the node — and both sides get
their own tests.

### 6.2 Supervision is per-OS, and shipped as data

No service-manager library, because that would end the zero-dependency claim for a
problem that is solved by three small files:

* **Linux** — a systemd unit (`Restart=always`, `RestartSec` with jitter).
* **macOS** — a launchd plist (`KeepAlive`).
* **Windows** — a Task Scheduler XML registered at boot. Deliberately not a
  native Windows service: that needs `golang.org/x/sys/windows/svc`, and a
  scheduled task restarts a crashed process just as well.

`qwen-turing-agent install` writes the right one for the platform it is running
on and prints what it wrote.

### 6.3 Config and credential

Flags or a file beside the binary; the credential is written on enrolment and
never printed again:

| | path | protection |
|---|---|---|
| Linux / macOS | `/etc/qwen-turing-agent/` or `~/.config/…` | mode `0600` |
| Windows | `%LOCALAPPDATA%\qwen-turing-agent\` | inherited ACL, `icacls` line printed |

Stated plainly: Windows gets no DPAPI, because that is a dependency. The
credential is a file readable by that user's processes, and the `install` command
prints the `icacls` command that tightens it. A `qts_` credential grants only the
ability to offer that server's inference capacity to the node, and is revocable
from the panel in one click.

### 6.4 What the agent must never do

* Forward to anything but its own configured target (§2.3).
* Accept an inbound connection. It is a client on every socket it holds, which is
  what makes the GPU box's firewall irrelevant to it.
* Log prompt or completion bodies. It moves bytes; it does not read them.

---

## 7. Node-side changes

| file | change |
|---|---|
| `scripts/wsframe.py` | **new** — RFC 6455 server codec: handshake, mask, fragmentation, control frames, close codes |
| `scripts/tunnel.py` | **new** — per-server idle-pipe pool, the socket-like pipe adapter, readiness accounting |
| `scripts/gateway.py` | `connect(server)` seam; three endpoints; ranking by readiness; `no_capacity` |
| `scripts/keystore.py` | `llm.servers` + enrolment tokens, mirrored to SQLite like `api_keys` |
| `scripts/upstreams.py` | tunnel-registered servers merge with file-registered ones; `via: tunnel \| direct` |
| `sql/002-servers.sql` | **new** — the table, owned by the same role |
| `nginx/qwen-turing.conf` | `location ^~ /api/agent/` to the gateway with `Upgrade` |
| `web/index.html` | Attach-a-server flow; `tunnelled` badge; owner; idle-pipe count |
| `agent/` | **new** — the Go agent, its three supervision files, and a build script |

The mirror pattern is deliberate and matches `api_keys`: the SQLite copy is the
enforcement point, so a Postgres outage degrades enrolment (no new servers) rather
than inference (existing ones keep working).

nginx uses a literal `proxy_set_header Connection "upgrade"` in that one location
rather than the usual `map $http_upgrade` at `http` level, because the location
serves nothing but WebSocket and the committed file is a `server` block.

---

## 8. Dashboard

The Servers panel gains, per server: `tunnelled` or `direct`, the owner's name,
`in the default pool` or `reachable at /u/<id>/v1 only`, and the idle-pipe count —
which is the honest way to show remote capacity, since the node's own queue
arithmetic has never covered another machine.

A tunnelled server is **not** shown the "reachable without a key" warning, because
it is not reachable at all. That warning stays for file-registered servers, where
it remains true.

Attach flow: **Attach a server** → name and note → the page shows the one-time
token, the platform picker, and the exact command line, with the token shown once.

---

## 9. Testing

**Frame codec (both languages).** Masking both directions; a payload split across
TCP reads; fragmented messages; a control frame interleaved between fragments;
close codes; oversized and malformed lengths refused. The Python side gets the
node's cases, the Go side the agent's; neither is trusted because the other
passes.

**Pipe adapter.** `http.client` over a pipe: a 469 KB body written through, a
chunked response read incrementally, a mid-stream close surfacing as a
`BrokenPipeError` rather than a hang.

**Integration, against a fake agent** implemented in a test thread — the real
protocol, not a mock of it:

* a 469 KB request body arrives at the fake target **byte-identical**;
* response chunks reach the client **incrementally** (asserted by timing the
  first chunk against the last, not by reading the whole body);
* a client abort closes the pipe and records `499`;
* pipe starvation returns `503 no_capacity` on a pin, and is **ranked around** on
  the balanced route;
* a control disconnect marks the server offline within one heartbeat cycle;
* a second control connection for a live id is refused `4409`;
* a `qts_` credential is refused on `/v1/chat/completions`, and a `qtk_` on
  `/api/agent/control`;
* an enrolment token works once and never twice.

**Structural.** Neither protocol message carries a destination field; the agent
binary cross-builds for all five targets in CI; every module the gateway imports
is installed (the check that already exists).

---

## 10. Acceptance criteria

1. A signed-in pool member attaches a server from the dashboard, runs one command
   on the box, and sees it `online` — with no operator edit on the node.
2. The first tunnelled server is **bender, with its iptables exposure removed and
   its inference port bound back to loopback**. This is the criterion that
   matters: the design's whole justification is that the GPU box needs no inbound
   port, so the acceptance test is deleting the firewall rule that used to be the
   only protection.
3. A ~100k-token request through the tunnel, measured: prefill, wall clock, and
   gateway RSS growth, recorded in `docs/measured-ceilings.md` next to the direct
   figures — including the honest comparison, whichever way it falls.
4. `/v1` routes to the tunnelled server under the existing rules; the model
   eligibility filter still applies; `X-Routed-To` names it.
5. Killing the agent marks it offline within 30 s and `/v1` routes around it.
   Restarting the agent restores it without operator action.
6. A newly attached server is reachable at `/u/<id>/v1` and **absent** from the
   balanced pool until promoted.

---

## 11. Out of scope

* **Multiplexing.** Revisit only if socket count becomes a real limit, which at
  this fleet's size it is not.
* **Queueing on the node.** Ruled out in §4, permanently, for the same reason it
  was ruled out for local inference.
* **This node offering itself upward to another hub.** The inverse direction is a
  separate design with its own trust questions; nothing here forecloses it.
* **Agent auto-update.** The binary is 3.4 MB and copying it is the update.
  Self-updating code that holds a credential is a much larger security surface
  than it looks.
* **Non-llama.cpp targets.** The agent forwards to an OpenAI-compatible endpoint;
  whether that is llama.cpp, vLLM or MLX is the target's business, but only
  llama.cpp is tested.
