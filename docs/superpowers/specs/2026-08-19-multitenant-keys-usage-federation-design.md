# Design: per-user API keys, per-key usage accounting, and GPU federation

> Brainstorm provenance: classified architectural. Trust model, component
> placement, key format, storage authority, login flow, provisioning ownership
> and offline-upstream behaviour were locked interactively with the operator
> (2026-08-19). Six probes ran against the live node BEFORE this document was
> finished; §1 records what they returned, because two of them changed the
> design and a third replaced a requirement's justification.

Extends [`2026-08-19-turing-dual-qwen-node-design.md`](2026-08-19-turing-dual-qwen-node-design.md)
and [`2026-08-19-queue-visibility-design.md`](2026-08-19-queue-visibility-design.md).
Measured ceilings remain authoritative in
[`llm/linux-turing-dual/docs/measured-ceilings.md`](../../../llm/linux-turing-dual/docs/measured-ceilings.md).

## 0. What exists, and what this changes

The node today serves seven model presets behind one nginx listener on `:80`,
authenticated by **a single shared API key** held by llama.cpp's own
`--api-key-file`. That key is indivisible: it cannot be attributed, cannot be
revoked for one consumer without revoking all of them, and produces no record of
who used what. The dashboard reports queue and GPU state for the node as a
whole and has no concept of a user.

This design adds three things, in this order:

| Phase | Delivers | Depends on |
|-------|----------|------------|
| **1** | Many keys instead of one; exact per-key token accounting; mint/revoke; usage panel | nothing outside this repo |
| **2** | Cognito login, group-gated minting, a shared registry on the central database server | a new Cognito app client; a provisioned database |
| **3** | Federation to other GPU hosts, starting with the RTX 4090 workstation | that host's address, and a firewall rule **on that host** |

They are phased because two of them are gated on work outside this repository
and one is not. Phase 1 alone answers the operator's headline request — *see
which key generates what usage, and register additional keys* — with zero
external dependency. It must not be blocked waiting for a database role.

This is one document rather than three because Phases 2 and 3 both extend the
same gateway and the same trust model. Splitting the spec would let those drift;
splitting the *delivery* is what the table above is for.

---

## 1. What the probes returned

Six questions could each have invalidated a branch. All six were answered
against the running node before any design was committed, in keeping with the
project's one rule: never configure a number you have not verified. Two of them
changed the design; §1.6 removed a requirement's justification and replaced it
with a better one.

### 1.1 Exact token counts need no body rewriting — the decisive finding

The risk was that OpenAI-compatible streaming only reports usage when the client
opts in via `stream_options: {"include_usage": true}`. Had that been true, the
gateway would have had to **rewrite client request bodies** to inject the option
— merging into a caller-supplied `stream_options`, recomputing `Content-Length`,
and coping with bodies that are not the JSON we expect. That is a materially
larger and more fragile component.

It is not true. The build in service emits a `timings` block on the terminal
streaming chunk **unconditionally**:

```
data: {"choices":[{"finish_reason":"length","index":0,"delta":{}}], ...
       "timings":{"cache_n":0,"prompt_n":11,"prompt_ms":86.989,
                  "predicted_n":8,"predicted_ms":103.675, ...}}
data: [DONE]
```

`prompt_n` and `predicted_n` are exact token counts. When the client *does* ask
for usage, a conventional `usage` object appears alongside it, including
`prompt_tokens_details.cached_tokens`. Non-streaming responses carry both.

**Consequence:** the gateway is a pass-through that reads the last chunk. It
never modifies a request. Requirement: read `usage` when present, fall back to
`timings.prompt_n` / `timings.predicted_n`, and treat the absence of both as
unknown rather than zero (§4.2).

### 1.2 Cognito is reachable from the node

`cognito-idp.<region>.amazonaws.com` resolved in 19 ms and completed TLS,
answering a deliberately fake pool id with a clean 404. Egress works; JWKS
verification is viable on-node.

### 1.3 The database server is reachable, needs no proxy change, and is not provisioned

The connection pooler port is open from the node, `psql` is present. Two further
findings shaped §5:

- The pooler carries a **wildcard route**: a connection to a database that does
  not exist is rejected by PostgreSQL itself, not by the pooler. A new database
  is therefore reachable the moment it is created — **no pooler configuration
  change is required.**
- A database named `autospec` already exists on that server, with an established
  **three-role pattern** (owner / EXECUTE-only emitter / read-only reader) and a
  `schema_migrations` convention. Its migrations live in a **separate repository
  whose tooling is not installed on this node or on the operator workstation.**
  This design therefore copies that role pattern into its own database and
  takes no dependency on that repository (§5.1).

### 1.4 The federation target is a workstation, and it is currently open

The RTX 4090 host is routable from the node over the **internal** interface
(ICMP and `:22` answer), and its inference port was **not listening** at probe
time. It also runs with no API key configured.

Two requirements follow, and they are the whole reason §6 is shaped as it is:
upstreams are **intermittent by default**, and an unauthenticated upstream
reachable by anything other than this node **bypasses this entire design**
(§6.4).

### 1.5 Dependencies are available without a virtual environment

`python3-jwt` (2.7.0) and `python3-cryptography` are already installed on the
node; `python3-psycopg2` is available from the distribution. Requirement: the
gateway installs from distribution packages only. No `pip`, no venv, no wheel
building — consistent with every other component on this node.

### 1.6 The runtime already accepts many keys — and that is not enough

Verified against the built binary, not its documentation:

```
--api-key-file FNAME   path to file containing API keys, one per line; ...
```

Two keys were appended, the router restarted, and **both authenticated (200 and
200) while a bogus key was refused (401)**. Removing a line and restarting
revoked that key (401).

So "many keys instead of one" is available with no new component at all. It is
still insufficient, for two reasons that are the entire justification for §3:

- **No attribution.** The runtime does not record which key served a request, so
  a key file can answer "who may use this node" and can never answer "who used
  it". The operator asked for the second question.
- **Revocation costs a model reload.** The key set is read at startup, so
  revoking requires a restart — which evicts the resident model and costs a
  reload. Revocation must not be a service interruption.

**Requirement:** the internal key of §3.2 is a **single-line** file that only the
gateway reads. User keys never appear in it, so no user key is ever revoked by
restarting the router.

### 1.7 A pass-through preserves streaming and prefill — measured

§3 places a Python process between nginx and the runtime. The risk is real: the
existing proxy sets `proxy_buffering off` and `proxy_request_buffering off`
precisely because a buffered body adds minutes of latency to a large prompt, and
a naive implementation that reads a request in full before forwarding it would
reintroduce exactly that.

A throwaway stdlib pass-through was measured against the runtime directly, using
**two distinct prompts** so the prefix cache could not confound the comparison
(`cache_n = 0` on both):

| | Direct | Through the pass-through |
|---|---|---|
| Prefill | 1930.8 tok/s | **1927.6 tok/s** (−0.17%) |
| Decode | 58.9 tok/s | 60.3 tok/s |
| Prompt tokens | 33,783 | 33,770 |

Response delivery was confirmed **incremental**, not buffered: 42 chunks arriving
spread over 0.77 s rather than in one block at the end.

Scope of that measurement, stated honestly: ~34k tokens on the 9B preset, not
100k on the 27B. It establishes the component's *shape* — a pass-through costs
nothing measurable — and it does not replace the acceptance test at the measured
100k ceiling (§10).

**Requirements it turns into:**

- The gateway streams **both** directions. It never reads a full request body or
  a full response into memory; both are relayed in bounded chunks, and response
  chunks are flushed as they arrive.
- Memory per in-flight request is bounded by the chunk size, not by the prompt
  size.
- The gateway sets no read timeout shorter than the proxy's, so it can never be
  the component that severs a long prefill.

---

## 2. Trust model

**Cognito authenticates humans. API keys authenticate machines.**

This is the load-bearing distinction. An agentic client — `opencode`, the OpenAI
SDK, `curl` in a script — cannot complete a browser redirect flow. So a human
authenticates once in a browser and mints long-lived keys that machines carry in
`Authorization: Bearer`. Sessions and keys are different credentials with
different lifetimes, and only one of them ever appears in a config file.

### 2.1 Only `sub` is identity

The operator's own Go services already encode this as a test: `email` and
`cognito:username` are **caller-mutable** and must never be used as an identity
key. Requirements:

- The user's identity is the verified token's `sub`, and nothing else.
- Group membership is read from the verified token's group claim, **never** from
  a request header, query parameter, or client-supplied body field.
- Display name and email may be *stored* for presentation, refreshed from the
  verified token on each login, and are never used for authorization.

### 2.2 A key is not a subject identifier

The operator's web frontend sets its `x-api-key` to the Cognito `sub`. That is
sound for a service sitting behind an API-Gateway authorizer, and it is wrong
here: a `sub` is neither secret nor revocable, and a leaked one cannot be
rotated without destroying the user.

**Requirement, recorded so that a later change does not "harmonize" it away:**
keys on this node are independently generated random secrets, hashed at rest,
displayed exactly once, and revocable individually. A Cognito `sub` must never
be accepted as an API key.

### 2.3 Who may mint

Minting requires membership in a named Cognito group, read per §2.1. The pool is
the operator's **production** pool containing the whole laboratory; unrestricted
self-service would grant every existing user a seat on a two-slot node.

Two groups, both configured site-locally:

| Group | May |
|---|---|
| user group | mint, list, revoke **their own** keys; see **their own** usage |
| admin group | all of the above, plus see all users' usage and revoke any key |

A verified token whose subject is in neither group authenticates successfully and
is authorized for nothing. That is a distinct state from "not logged in" and the
dashboard must say which.

---

## 3. Component layout

```
campus / LAN ──▶ nginx :80  ──▶ gateway :8082 ──┬─▶ llama.cpp 127.0.0.1:8090
                      :443                      └─▶ federated upstream (Phase 3)
                        │
                        └── auth_request ──▶ dashboard 127.0.0.1:8081
```

One new service, `qwen-turing-gateway`, binding `127.0.0.1:8082`. It
authenticates the caller, resolves the requested model to an upstream, proxies
the exchange, and records what it cost. nginx keeps every responsibility it has
today.

### 3.1 Division of labour by failure mode

This is the reason the gateway is a separate process rather than code added to
the dashboard: the two have **opposite** correct behaviour when they break.

| Layer | Owns | When it fails |
|---|---|---|
| nginx | TLS, timeouts, unbuffered streaming, queue headers | headers vanish; inference survives |
| gateway | **authentication**, routing, accounting | **inference stops** |
| llama.cpp | inference; internal key | — |

The dashboard's queue snapshot is telemetry: losing it must never cost service,
which is why the existing proxy deliberately fails open around it. Authentication
is the opposite: losing it must never grant service.

### 3.2 The fail-open fallback is an authentication bypass, and must be repointed

The listener today carries, inside the inference location:

```nginx
auth_request /internal-queue-headers;
error_page 500 502 503 504 = @inference_no_headers;
proxy_pass http://qwen_llama;
```

`@inference_no_headers` proxies to llama.cpp **without** the subrequest. That is
correct today, because llama.cpp itself holds the key.

The moment authentication moves into the gateway, this becomes a **bypass**:
`error_page` is scoped to the location, so it catches the *gateway's own* 502 as
readily as a dashboard timeout, and routes the request to a backend that no
longer authenticates anything. Gateway down, inference open.

Three requirements, all cheap, defending in depth:

1. **`@inference_no_headers` proxies to the gateway**, not to llama.cpp. Queue
   headers still degrade independently; authentication does not.
2. **llama.cpp retains `--api-key-file`**, holding a **single-line** *internal*
   key known only to the gateway and distinct from every user key (§1.6). Any
   path that reaches it directly fails closed with a 401 instead of serving, and
   no user key is ever revoked by restarting the router.
3. **A structural check fails the build** if any client-reachable `location` in
   the site file can `proxy_pass` to the llama.cpp upstream. The suite already
   has precedent for a check of exactly this shape guarding the HSTS trap.

### 3.3 No per-request network calls

Three things in this design are tempting to fetch per request and must not be:
the JWKS document, the key registry, and upstream health. Each gets a cached
snapshot refreshed by one background timer, following the pattern already
established for the queue snapshot — where a per-request subrequest would have
both added latency and corrupted the sampling window it was reading.

| Cached | Refresh | Stale behaviour |
|---|---|---|
| JWKS | on unknown key id, at most once per minute | reject the token |
| key registry | 30 s, plus write-through on local change | serve last known (§4.3) |
| upstream health | 15 s | treat as `unknown`, never as online |

### 3.4 Sandboxing, by name

The gateway touches no GPU, so it is not bound by the confinement ceiling the
router unit hit — but "stricter" is not a specification, and the directives that
broke sibling units here are known. Required, explicitly:

- `ProtectSystem=strict` is permitted (the router cannot take it; CUDA needs
  write access this process does not).
- `ProtectHome=true` **with** the site-configuration directory added to
  `ReadOnlyPaths=` — that exact combination already broke a sibling unit's
  configuration read on this node, and adding the path is the fix.
- `NoNewPrivileges=true`, `PrivateTmp=true`, `RestrictAddressFamilies=` limited
  to the families actually used.
- Secrets arrive via `LoadCredential` **only** — never `Environment=`, which any
  local user can read through `systemctl show`.

The gateway needs no journal group membership; it logs to its own unit.

---

### 3.5 The gateway adds no queue of its own

The queue panel's arithmetic reads the runtime's own counters, and its rolling
window was calibrated against today's topology. Inserting a component in front
of the runtime could create a **second** place where requests wait — and a panel
that reports one queue while requests pile up in another is a panel that lies.

**Requirement: the gateway performs no admission control and maintains no
request queue.** It accepts a connection, authenticates, and relays; concurrency
is bounded by the runtime's slots exactly as it is today. The existing queue
arithmetic therefore remains correct and unchanged.

If admission control is ever wanted (it is not in scope — see §11), the queue
panel must be extended in the same change. It may not be added silently.

## 4. Keys and accounting

### 4.1 Key format

```
qtk_<12-char key id>_<32-char secret>
```

The **key id** is not secret. It is stored in clear, printed in the dashboard,
attached to every usage row, and is how a human refers to a key. The **secret**
is 160 bits of CSPRNG output stored as a SHA-256 digest, shown once at creation
and never retrievable.

**SHA-256 rather than a password KDF, deliberately.** A work factor defends a
low-entropy secret against offline guessing. A 160-bit random secret has nothing
to guess, so argon2 or bcrypt here would buy no security and would add latency
to every single request. The prefixed key id also makes lookup O(1) rather than
a scan across every stored hash.

Requirements:

- Verification is constant-time against the stored digest.
- A key carries an optional expiry and an optional label, and records
  `created_at`, `last_used_at`, `revoked_at`.
- The plaintext secret is never written to a log, a spool file, an error message,
  or a usage row. A test asserts this against the assembled log output.

### 4.2 What a usage record contains

One row per completed request:

`ts`, `key_id`, `sub`, `model`, `upstream`, `endpoint`, `prompt_tokens`,
`completion_tokens`, `cached_tokens`, `prompt_ms`, `predicted_ms`,
`status_code`, `streamed`, `truncated`.

`cached_tokens` is genuinely useful here and not decoration: this node's prefix
cache was measured at roughly a tenfold saving on a warm slot, so the share of a
user's prompt tokens served from cache is the difference between a cheap
conversation and an expensive one.

**The honest edge case.** If a client disconnects mid-stream, the terminal chunk
carrying the counts never arrives. The count is then genuinely unknown.
Requirement: such rows record the tokens actually observed, set
`truncated = true`, and are **aggregated separately** — never estimated from
byte counts, never silently reported as zero. This is the same restraint that
removed p50/p95 latency from the queue design: a fabricated number that looks
like a measurement is worse than an absent one.

### 4.3 Availability, and why revocation is different

**Inference must never depend on the shared database or on Cognito being
reachable.** Both are remote, and neither is on this node's failure budget.

- The shared registry is the record of truth; the node keeps a **write-through
  local mirror** which is the actual **enforcement point**.
- If the registry is unreachable, existing keys keep working and usage records
  buffer locally, draining when it returns. Degradation is "usage is stale", not
  "the node is down".
- **Revocation must not be stale.** A revoked key that still works because a
  cache has not refreshed is the failure that matters here. A revoke issued on
  this node applies to the local mirror synchronously, before the API call
  returns success. A revoke issued elsewhere converges within the refresh
  interval, and the dashboard **states that bound** rather than implying
  instantaneous propagation.
- Local usage buffering is idempotent on a per-record identifier, so a drain
  that partially succeeded cannot double-count.

---

## 5. Storage

### 5.1 A separate database, owned by this project

A new database on the existing server, with three non-superuser roles mirroring
the pattern already established there: an owner that holds the schema, an
application role with data privileges only, and a read-only role for reporting.
No pooler change is needed (§1.3).

It is a separate database rather than a schema inside the production
metabolomics database — that would share backup, migration and blast radius with
production science data — and rather than a schema inside the existing
`autospec` database, whose migrations are owned by a repository not available
here (§1.3).

Tables: `users`, `api_keys`, `usage_events`, `usage_daily` (a rollup, so the
dashboard never scans raw events), `schema_migrations`.

### 5.2 Provisioning is the operator's to run

The DDL requires a superuser. During probing, the credential found in the
operator's test resources proved to be a **full superuser** on the production
server, despite the `readOnly=true` hint in its JDBC URL — that flag is a
client-side hint and not a server restriction.

**Requirement: this project runs no DDL with that credential.** The spec ships
the exact provisioning script; the operator or their DBA reviews and runs it.
The gateway connects as the application role, which has no DDL privilege, and
fails with a clear diagnostic — not a silent fallback — if the schema version it
requires is absent.

Separately, and outside the scope of this work: that superuser credential in
plaintext test resources is worth rotating into a genuinely read-only role.

### 5.3 Nothing site-identifying is committed

This repository is public. New site-local values — the identity pool's region,
pool id and client id, its group names, the hosted login domain, the database
host, port, name and user, and every federated upstream's address — live in the
site configuration file and the credential store, never in a committed file.

Requirements: extend the required-variable list so a missing value fails fast
with a named variable rather than a confusing runtime error; extend the existing
literal-address scan to cover the new files; keep the database password and any
client secret in the credential store.

---

## 6. Federation

### 6.1 Route by model name

An upstream registry — example committed, real file site-local — declares for
each host: an id, a base URL, a credential name, a namespace prefix, and either a
static model list or an instruction to poll the host's own model list.

Routing is by **model name**, deterministically, not by load-balancing a pool.
Pooling the same model across hosts would need capacity and queue-depth
awareness on both sides and would make a request's destination
unpredictable — which on hosts with different cards means unpredictable
throughput too.

### 6.2 Namespacing preserves every existing client

Local models keep their bare names. Remote models are prefixed with their
upstream's id. Both hosts run Qwen builds, so unprefixed remote names would
collide; and every client configured today keeps working untouched.

### 6.3 Intermittent by default

The first federation target is a **workstation** — it reboots, sleeps, and was
not serving when probed. Requirements:

- Health is sampled by one background timer (§3.3) into
  `online | offline | unknown` with a `last_seen`. Never probed per request.
- A request for a model on an offline upstream returns an immediate `503` whose
  body names the upstream and its `last_seen`. **It must never hang** — an agent
  waiting out a 900 s proxy timeout on a workstation that is simply switched off
  is the failure mode this exists to prevent.
- `/v1/models` lists **only reachable** models, so a client's model picker never
  offers something that immediately fails. The dashboard shows the full
  configured roster including offline entries with their `last_seen`, which is
  where a human wants that distinction. OpenAI-compatible clients have no field
  in which to express availability, so honesty there means omission.
- A remote host's queue depth is reported **as the remote reports it**, or shown
  as `unknown`. It is never inferred, and this node's own queue arithmetic is
  never presented as covering remote capacity.

### 6.4 The upstream must not be an open bypass

An unauthenticated inference port reachable by anything other than this node
defeats the whole design: a client that can reach it directly needs no key, and
its usage is attributed to nobody.

**Requirement, and it is work on the *other* host, not this one:** each
federated upstream either restricts its inference port to this node's address,
or requires its own key which this node holds in its credential store.
Preferably both. Phase 3 does not ship until the first upstream satisfies this,
and the dashboard marks an upstream reachable-and-unauthenticated as a
**configuration fault**, in the same panel that already reports silently
discarded options.

---

## 7. Dashboard

Three additions to the existing keyed dashboard.

**My keys** — list with label, key id, created, last used, expiry, and a revoke
control. Minting shows the secret exactly once, with the copy-once expectation
stated plainly, and a warning that it will never be shown again.

**Usage** — per key, per model, per day: requests, prompt and completion tokens,
cached-token share, and truncated-record count shown separately (§4.2). Admins
additionally get a by-user view. A user in neither group sees an explicit
"authenticated, not authorized" state naming the group they need.

**Upstreams** (Phase 3) — each configured host with its state, `last_seen`,
model count, and the configuration-fault marker from §6.4.

### 7.1 The public surfaces must not grow a user field

Per-key usage is user-identifying: real names, real email addresses. The public
load page and the public queue endpoint exist for colleagues and carry no
credential.

Requirement: every public payload continues to be assembled by **iterating a
forward allow-list of field names**, so a field added upstream is absent from the
public payload rather than leaked into it. That mechanism already protects the
queue payload; this design extends it to cover every new endpoint, and a test
asserts that a newly added internal field does not appear publicly.

### 7.2 Login

The identity pool has no hosted login domain configured; the operator's frontend
authenticates against it directly. Phase 2 therefore requires, as operator
action on the pool:

1. A **separate app client** for this dashboard — public, no client secret, PKCE
   required — leaving the existing frontend's app client untouched.
2. A hosted login domain on the pool.
3. A callback URL pointing at this node's `:443` dashboard path. **Ordering
   constraint: the certificate and the `:443` listener must exist before this
   app client is created**, or the redirect target does not resolve and login
   fails in a way that looks like a pool misconfiguration.
4. The two groups from §2.3, and membership.

The gateway performs the authorization-code exchange with PKCE, verifies the
returned token against cached JWKS, and holds a short server-side session. It
never stores a user password and never proxies one.

### 7.3 Connection examples stay honest

The existing examples panel renders from live state. It gains: the user's own key
id (never the secret), the current model roster including any federated entries,
and the per-key usage endpoint. Examples must render the same values the node
would actually accept — the panel's existing contract.

---

## 8. Exposure change, and the one thing that got worse

The operator chose plain HTTP twice, and both times the only secret on the wire
was one shared inference key. **Phase 2 changes what is on the wire**: session
tokens for the identity pool behind the operator's production application. A
captured inference key means someone else's tokens on a two-slot GPU; a captured
pool session is lateral movement into production. Those are not the same risk,
and the earlier decision was not made about the second one.

Resolution, chosen by the operator: **inference stays on plain `:80` exactly as
today**, and login plus key management move to `:443` with a self-signed
certificate, swapped for the campus certificate when it is issued. API clients
are entirely unaffected; operators accept a browser warning once.

Requirements:

- The `:443` server carries the dashboard and gateway administration paths.
- **No HTTP-to-HTTPS redirect and no HSTS header**, for the reason the existing
  design already records: a redirect to a port that is not listening breaks the
  endpoint, and an HSTS header cached before a valid certificate exists makes the
  node unreachable from every browser that saw it. The existing structural check
  that fails the build if either appears must be extended to cover the new
  server block, not bypassed by it.
- Key minting and revocation are refused over plain HTTP with an explanatory
  error, not silently downgraded.
- The firewall opens `:443` on the same interfaces as `:80`.

---

## 9. Testing

The suite is unit-first and touches no live external service, following the
existing convention that tests stub every subprocess and network peer.

- **Key lifecycle** — format, constant-time verification, single-display,
  revocation, expiry. A test asserts a plaintext secret never reaches assembled
  log output.
- **Token verification** — against a **locally generated RSA keypair serving as
  a fake JWKS**, never the live identity provider. Cases: valid; expired; wrong
  audience; wrong issuer; correct signature but unknown key id; a token whose
  mutable claims contradict `sub`; and a request carrying group claims in a
  **header** rather than a token, which must be ignored (§2.1).
- **Usage parsing** — from **captured real response fixtures**: a streamed
  response with only `timings`, a streamed response with `usage` and
  `cached_tokens`, a non-streaming response, and a **truncated stream** that must
  yield `truncated = true` rather than zero.
- **Routing** — local bare names, prefixed remote names, unknown model, and a
  model on an offline upstream returning 503 promptly rather than hanging.
- **Upstream health state machine** — including that an unreachable upstream
  becomes `unknown` and never `online`, and that health is never probed on the
  request path.
- **Public payload allow-list** — a newly added internal field must be absent
  from every public payload.
- **Store behaviour** — write-through revocation applies locally before
  returning; buffered usage drains idempotently; an unreachable registry does not
  block authentication.
- **Structural** — no client-reachable nginx location may reach llama.cpp
  directly; new required site variables are declared; the literal-address scan
  covers the new files; no redirect or HSTS in the TLS block.

Fixtures are captured from the running node and committed, rather than
hand-written to match the parser — a self-consistent fixture built from the
parser's own assumptions cannot catch a bug in those assumptions.

---

## 10. Acceptance criteria

**Phase 1** — criterion 1 runs **first**, because a failure there changes the
component's shape rather than its details.

1. A prompt at the measured 100k ceiling completes through the gateway, with
   prefill throughput within measurement noise of the recorded figure, and
   response chunks arriving incrementally rather than in one block. The
   pass-through shape was measured good at ~34k (§1.7); this is the same check
   at the ceiling that matters.
2. Peak gateway memory during that request stays bounded well below the prompt
   size, proving neither direction is buffered.
3. Two distinct keys both authenticate; a request with no key, a malformed key,
   and a revoked key are each rejected with the correct status.
4. Revoking one key leaves the other working, verified by request, and **without
   restarting the router** (§1.6).
5. Usage attributed per key matches the token counts the model itself reported,
   for a streamed and a non-streaming request.
6. A client disconnected mid-stream produces a `truncated` record, not a zero.
7. The gateway stopped leaves inference **refused**, not open — verified by
   request, not by reading the configuration.

**Phase 2**

8. A user in the user group can mint, list and revoke only their own keys.
9. A user in neither group authenticates and is authorized for nothing, and is
   told which group they need.
10. Group membership supplied in a header rather than a verified token grants
    nothing.
11. With the registry unreachable, existing keys still authenticate and usage
    buffers; on restoration it drains without double-counting.
12. Key minting over plain HTTP is refused.

**Phase 3**

13. A model on a reachable upstream answers through the node, with usage
    attributed to the calling key and the upstream recorded.
14. A model on an unreachable upstream returns 503 naming the upstream in under
    a second, and does not appear in the model list.
15. An upstream that is reachable **and** unauthenticated is reported as a
    configuration fault.
16. Every existing client configuration keeps working unchanged.

---

## 11. Out of scope

- Quotas, rate limits and billing. This design **measures** usage; it does not
  enforce against it. Enforcement needs an agreed policy first, and the
  measurement has to exist before the policy can be sane.
- Pooled load-balancing of one model across hosts (§6.1).
- Per-user model permissions. Any key may use any reachable model.
- Migrating the existing shared key's consumers automatically; the shared key
  stays valid through Phase 1 as one ordinary key, and is retired by hand once
  named keys have replaced it.
- Federating anything that is not OpenAI-compatible.
- The campus certificate, and any dependence on its arrival.
- Rotating the production superuser credential found during probing (§5.2),
  which is the operator's, and urgent independently of this work.
