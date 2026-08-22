# Per-user login and API keys — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: use superpowers:executing-plans
> to implement this task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Replace the node's single shared API key with per-user keys that a
human mints after logging in, and record exact token usage per key.

**Architecture:** One new loopback service between nginx and llama.cpp. It
authenticates `Authorization: Bearer`, streams the exchange through untouched,
and reads exact token counts off the terminal response chunk. Humans log in
with authorization-code + PKCE against the shared identity pool; machines carry
long-lived keys. Enforcement is local (SQLite mirror) so inference never depends
on the remote registry being reachable.

**Tech Stack:** Python 3.12 stdlib + `python3-jwt`, `python3-cryptography`,
`python3-psycopg2` (all distribution packages, no venv). PostgreSQL registry,
SQLite mirror. nginx. systemd.

**Spec:** [`../specs/2026-08-19-multitenant-keys-usage-federation-design.md`](../specs/2026-08-19-multitenant-keys-usage-federation-design.md)

## Global Constraints

- **This repository is public.** No hostname, address, pool id, client id,
  database name, or group name in any committed file. Site values live in
  `site.conf` (already extended: `QT_GATEWAY_REQUIRED_VARS`).
- **Only `sub` is identity.** `email`/`cognito:username` are caller-mutable.
  Groups come from the **verified token**, never a header or query parameter.
- **A Cognito `sub` is never an API key.** Keys are independent random secrets,
  hashed at rest, shown once.
- **Fail closed on auth, fail open on telemetry.** `@inference_no_headers` must
  point at the gateway, never at llama.cpp.
- **The gateway adds no queue.** No admission control, no request queue; the
  existing queue arithmetic must stay correct.
- **Never buffer either direction.** Bounded chunks both ways, flush on arrival.
- **Revocation is never stale.** A local revoke applies before the API returns.
- **Tests touch no live service.** JWKS is a locally generated RSA key; usage
  fixtures are captured bytes; the registry is a temp SQLite file.
- Python files under `scripts/` are executable and pass `python3 -m py_compile`.
  Test files are named `test_unit_*.py` (the conftest convention).

---

## File Structure

| File | Responsibility |
|---|---|
| `scripts/keys.py` | **Pure.** Key format, generation, parse, hash, constant-time verify. No I/O. |
| `scripts/usage.py` | **Pure.** Extract exact token counts from a response body or SSE stream. No I/O. |
| `scripts/oidc.py` | **Pure.** PKCE, authorize URL, token-request body, JWT verification against a supplied JWKS. No network. |
| `scripts/keystore.py` | Storage. SQLite mirror is the enforcement point; PostgreSQL is the registry. Write-through revoke, buffered usage. |
| `scripts/gateway.py` | The HTTP service. Authn, proxy, login, key API, usage API. |
| `scripts/gateway-run.sh` | Unit wrapper; reads `site.conf`, passes credentials. |
| `systemd/qwen-turing-gateway.service` | The unit. `LoadCredential` only. |
| `sql/001-schema.sql` | Registry schema + `usage_daily` view. Runs as the **owner** role. |
| `web/login.html` | Minimal login landing + callback handling. |
| `web/index.html` | Gains "My keys" and "Usage" panels. |
| `nginx/qwen-turing.conf` | Routes inference and the auth/key API to the gateway. |
| `tests/test_unit_keys.py`, `test_unit_usage.py`, `test_unit_oidc.py`, `test_unit_keystore.py` | Unit coverage per module. |

The four pure/storage modules are separate from `gateway.py` deliberately: every
security-relevant decision (does this key verify, is this token valid, is this
user in the group) is then testable without sockets.

---

## Interfaces

Pinned here so tasks can be implemented in any order.

```python
# keys.py
PREFIX = "qtk"; KEY_ID_LEN = 12; SECRET_LEN = 32
def generate() -> tuple[str, str, str]      # (full_key, key_id, secret_hash)
def parse(presented: str) -> tuple[str, str] | None   # (key_id, secret)
def hash_secret(secret: str) -> str          # sha256 hex
def verify(secret: str, stored_hash: str) -> bool     # constant time
def public_id(full_key: str) -> str | None   # key_id only, for display

# usage.py
@dataclass
class Usage:
    prompt_tokens: int | None = None
    completion_tokens: int | None = None
    cached_tokens: int | None = None
    prompt_ms: float | None = None
    predicted_ms: float | None = None
    truncated: bool = False
def from_json_body(obj: dict) -> Usage
class StreamAccountant:
    def feed(self, chunk: bytes) -> None
    def result(self) -> Usage                # truncated=True if no terminal block

# oidc.py
def pkce_pair() -> tuple[str, str]           # (verifier, challenge)
def authorize_url(domain, client_id, redirect_uri, state, challenge) -> str
def token_form(client_id, code, redirect_uri, verifier) -> bytes
def verify_id_token(token, jwks, *, issuer, audience, now) -> dict   # raises ValueError
def groups(claims: dict) -> list[str]
def identity(claims: dict) -> tuple[str, str | None, str | None]

# keystore.py
@dataclass
class KeyRow:
    key_id: str; sub: str; label: str | None
    created_at: str; expires_at: str | None
    revoked_at: str | None; last_used_at: str | None
    secret_hash: str = ""
class KeyStore:
    def __init__(self, sqlite_path: str, dsn: str | None = None)
    def migrate_local(self) -> None
    def authenticate(self, presented: str, now: float) -> KeyRow | None
    def mint(self, sub, label=None, ttl_days=None) -> tuple[str, KeyRow]
    def revoke(self, key_id, *, sub, is_admin) -> bool
    def list_keys(self, sub, *, all_users=False) -> list[KeyRow]
    def upsert_user(self, sub, email, name) -> None
    def record_usage(self, rec: dict) -> None
    def flush(self) -> tuple[int, int]       # (flushed, remaining)
    def refresh(self) -> int                 # registry -> local mirror
    def usage(self, sub=None, days=30) -> list[dict]
    def health(self) -> dict
```

---

## Task 1: Key format and verification

**Files:** Create `scripts/keys.py`, `tests/test_unit_keys.py`

**Interfaces:** Produces everything in the `keys.py` block above.

- [ ] **Step 1: Write the failing tests**

Cover: round-trip; a key id is stable and public; the secret never appears in
the stored hash; a tampered secret fails; a truncated/garbage/wrong-prefix key
parses to `None`; two generations never collide; verify is constant-time (called
via `hmac.compare_digest`).

```python
def test_generate_round_trips_and_hides_the_secret():
    full, key_id, h = keys.generate()
    assert full.startswith("qtk_") and full.count("_") == 2
    parsed = keys.parse(full)
    assert parsed is not None and parsed[0] == key_id
    assert keys.verify(parsed[1], h)
    assert parsed[1] not in h           # the hash is not the secret

def test_a_tampered_secret_is_refused():
    full, key_id, h = keys.generate()
    kid, secret = keys.parse(full)
    assert not keys.verify(secret[:-1] + ("a" if secret[-1] != "a" else "b"), h)

@pytest.mark.parametrize("bad", ["", "qtk", "qtk_", "qtk_short_x", "nope_aaa_bbb",
                                 "qtk_aaaaaaaaaaaa", "Bearer qtk_a_b"])
def test_malformed_keys_parse_to_none(bad):
    assert keys.parse(bad) is None
```

- [ ] **Step 2:** Run them, confirm they fail (`ModuleNotFoundError`).
- [ ] **Step 3:** Implement `keys.py`. `secrets.token_bytes` → base32 lowercased,
      padding stripped, sliced to the declared lengths. `hash_secret` is
      `hashlib.sha256(secret.encode()).hexdigest()`; document **why not argon2**
      (160 bits of CSPRNG has nothing to brute-force; a work factor would only
      add latency per request).
- [ ] **Step 4:** Run tests, all pass.
- [ ] **Step 5:** Commit.

---

## Task 2: Exact usage extraction

**Files:** Create `scripts/usage.py`, `tests/test_unit_usage.py`, `tests/fixtures/`

**Interfaces:** Consumes nothing. Produces `Usage`, `from_json_body`,
`StreamAccountant`.

- [ ] **Step 1: Capture real fixtures from the running node** — a streamed
      response with only `timings`, a streamed response with `usage` and
      `cached_tokens`, and a non-streaming body. Commit them. Hand-written
      fixtures that mirror the parser's assumptions cannot catch a bug in those
      assumptions.
- [ ] **Step 2: Write the failing tests.** Assert exact numbers from the
      fixtures — including that a **truncated** stream (fixture cut before the
      terminal chunk) yields `truncated=True` and **not** zero.
- [ ] **Step 3:** Run, confirm failure.
- [ ] **Step 4:** Implement. Prefer `usage`, fall back to `timings.prompt_n` /
      `predicted_n`, treat both absent as unknown. `StreamAccountant.feed` keeps
      only a small tail buffer — it must not accumulate the whole stream.
- [ ] **Step 5:** Run, pass. **Step 6:** Commit.

---

## Task 3: OIDC — PKCE and token verification

**Files:** Create `scripts/oidc.py`, `tests/test_unit_oidc.py`

- [ ] **Step 1: Write the failing tests** against a **locally generated RSA
      keypair serving as a fake JWKS** — never the live provider. Cases: valid;
      expired; wrong audience; wrong issuer; `alg=none`; a token signed by a key
      not in the JWKS; `token_use` mismatch; and **groups supplied in a header
      rather than the token grant nothing** (assert `groups()` reads only claims).
- [ ] **Step 2:** Run, confirm failure. **Step 3:** Implement using `jwt` with
      `algorithms=["RS256"]` explicitly (never from the token header).
- [ ] **Step 4:** Run, pass. **Step 5:** Commit.

---

## Task 4: Registry schema

**Files:** Create `sql/001-schema.sql`

- [ ] **Step 1:** Write the migration: `schema_migrations`, `users`, `api_keys`,
      `usage_events`, and `usage_daily` as a **view** (a view rather than a
      rollup table — two slots cannot generate enough rows to need a job, and a
      job is another thing to fail silently).
- [ ] **Step 2:** Apply it as the **owner** role. `ALTER DEFAULT PRIVILEGES`
      only covers objects created by the role it names, so a superuser applying
      this would leave the app role with no privileges — presenting later as a
      mystifying permission bug.
- [ ] **Step 3:** Verify from the node as the app role: INSERT into
      `usage_events` succeeds, `CREATE TABLE` is still refused.
- [ ] **Step 4:** Commit.

---

## Task 5: Store — local enforcement, remote registry

**Files:** Create `scripts/keystore.py`, `tests/test_unit_keystore.py`

- [ ] **Step 1: Write the failing tests** (SQLite only, `dsn=None`): mint then
      authenticate; a revoked key fails **immediately** after `revoke()`
      returns; an expired key fails; `list_keys` is scoped to one `sub` unless
      `all_users`; a non-admin cannot revoke another user's key; usage buffers
      and `flush()` is idempotent on `event_id`; `authenticate` on an unknown
      key id does no work beyond a lookup.
- [ ] **Step 2:** Run, confirm failure.
- [ ] **Step 3:** Implement. The SQLite mirror is the **enforcement point**;
      PostgreSQL writes are best-effort and buffered. Revoke writes local first,
      then the registry. `authenticate` must be O(1) by `key_id`.
- [ ] **Step 4:** Run, pass. **Step 5:** Commit.

---

## Task 6: The gateway service

**Files:** Create `scripts/gateway.py`, `scripts/gateway-run.sh`,
`systemd/qwen-turing-gateway.service`

**Consumes:** every interface above.

- [ ] **Step 1:** Implement the proxy path first — authenticate, stream both
      directions in bounded chunks, record usage from the terminal chunk. It
      holds the **internal** key for llama.cpp and never forwards the client's.
- [ ] **Step 2:** Verify by request at the measured 100k ceiling: incremental
      delivery, prefill within noise, bounded memory. **This is criterion 1 and
      it runs first** — a failure here changes the component's shape.
- [ ] **Step 3:** Add login: `/auth/login`, `/auth/callback`, `/auth/logout`.
      In-memory sessions keyed by a random id; cookie `HttpOnly`, `Secure`,
      `SameSite=Lax`. Never store or proxy a password.
- [ ] **Step 4:** Add the key API: `GET/POST /api/keys`,
      `DELETE /api/keys/<key_id>`, `GET /api/me`, `GET /api/usage`. Minting
      returns the secret **once**. Group membership gates minting; a user in
      neither group gets an explicit "authenticated, not authorized" naming the
      group they need.
- [ ] **Step 5:** Refuse minting and revocation over plain HTTP.
- [ ] **Step 6:** Unit + wrapper, `LoadCredential` only, never `Environment=`.
      `ProtectSystem=strict` is permitted here (no CUDA), with the site-config
      directory in `ReadOnlyPaths=` — `ProtectHome=true` alone already broke a
      sibling unit's config read on this node.
- [ ] **Step 7:** Commit.

---

## Task 7: Rewire nginx, and prove the bypass is closed

**Files:** Modify `nginx/qwen-turing.conf`, `tests/test_structural.sh`

- [ ] **Step 1:** Add a `qwen_gateway` upstream. Point the inference locations
      **and `@inference_no_headers`** at it. Route `/auth/`, `/api/keys`,
      `/api/me`, `/api/usage` to it. Keep `= /v1/models` on the dashboard (exact
      match beats regex) and `= /models` at 403.
- [ ] **Step 2: Write the failing structural check** — no client-reachable
      `location` may `proxy_pass` to the llama.cpp upstream. Confirm it fails
      first if the fallback still points there.
- [ ] **Step 3:** Give llama.cpp its own single-line **internal** key, and
      register the previous shared key in the registry as an ordinary key so
      existing clients keep working (spec §11).
- [ ] **Step 4: Prove it by request, not by reading config:** stop the gateway
      and confirm inference is **refused**, not served.
- [ ] **Step 5:** Commit.

---

## Task 8: Dashboard panels

**Files:** Modify `web/index.html`, create `web/login.html`

- [ ] **Step 1:** "My keys" — label, key id, created, last used, expiry, revoke.
      Minting shows the secret exactly once with that stated plainly.
- [ ] **Step 2:** "Usage" — per key, per model, per day; requests, tokens,
      cached-token share, truncated count **shown separately**. Admins get a
      by-user view.
- [ ] **Step 3:** Build every URL from `location.origin` (the existing panel's
      convention) so a hostname is never baked in.
- [ ] **Step 4:** Confirm no per-user field can reach a public payload — the
      forward allow-list test must still pass.
- [ ] **Step 5:** Commit.

---

## Task 9: End-to-end acceptance

- [ ] Run the spec's Phase 1 criteria 1–7 and Phase 2 criteria 8–12 against the
      live node, recording measured numbers in `docs/measured-ceilings.md`.
- [ ] Full suite + both structural suites green.
- [ ] Commit and push.
