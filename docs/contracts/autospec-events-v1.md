# `autospec.events.v1` — telemetry payload + binary CLI contract

Single source of truth for the autospec telemetry event contract. Every emitter
in this repo and the `autospec-db` ingest hub pin to the payload shape below.

- **Source spec:** [`docs/specs/2026-07-10-autospec-db-telemetry-design.md`](../specs/2026-07-10-autospec-db-telemetry-design.md)
- **Epic:** #1769. Foundation shim: #1770.
- **Scope of this doc:** the payload field contract, the `autospec-db` binary CLI
  contract the shim dispatches to, and the boundary that keeps all transport
  internals in the `autospec-db` repo. Chokepoint wiring, the installer, and the
  autospec-db SQL/schema are out of scope here.

## Payload fields

Event payload contract `autospec.events.v1` — one JSON object per event.

| field | required | value |
|---|---|---|
| `schema` | yes | literal `autospec.events.v1` |
| `event_uuid` | yes | client-generated UUIDv4; the idempotency key |
| `kind` | yes | one of the 8 kinds below |
| `ts` | yes | client UTC ISO-8601 (server also stamps an authoritative `received_at`, clock-skew safe for stall math) |
| `session_id` | yes | harness session-id fallback chain (`CLAUDE_CODE_SESSION_ID` → harness-neutral fallbacks) |
| `host` | yes | `AUTOSPEC_DB_HOST_LABEL` when set to a non-empty value, else `hostname -s` (see `telemetry.host_label` below) |
| `repo` | yes | `owner/name` |
| `tier` | optional | kind-specific |
| `issue` | optional | kind-specific |
| `pr` | optional | kind-specific |
| `branch` | optional | kind-specific |
| `step` | optional | kind-specific |
| `outcome` | optional | kind-specific |
| `detail` | optional | kind-specific |

The seven required fields — `schema`, `event_uuid`, `kind`, `ts`, `session_id`,
`host`, `repo` — are present on every event. The optional fields are carried
only by the kinds that populate them.

### Event kinds

| kind |
|---|
| `heartbeat` |
| `session.started` |
| `session.step` |
| `session.terminal` |
| `session.parked` |
| `claim` |
| `artifact.filed` |
| `feature.described` |

`session.parked` covers both the governor soft-park and the stop-flag path; the
`outcome` field (`soft-park` vs `stop`) disambiguates.

## Contract rules

- **Additive-only within v1.** New optional fields may be added; existing fields
  never change type or meaning.
- **Unknown fields are ignored server-side.** Agents may run ahead of the hub, so
  the collector silently accepts fields it does not recognize.
- **A v2 bump must still accept v1.** The database keeps ingesting v1 payloads
  after any future `autospec.events.v2` is introduced.
- **The DSN is never logged.** `AUTOSPEC_DB_DSN` is never echoed, never logged,
  and never appears in any resolved command line that reaches a transcript. The
  DSN lives only in `~/.autospec/db.env` (chmod 600, exports `AUTOSPEC_DB_DSN`
  with `sslmode=require` + `connect_timeout=2`) and is sourced by session
  bootstrap.

## Binary CLI contract (`autospec-db`)

The shared shim `skills/autospec-shared/scripts/emit-event.sh` is a THIN
dispatcher — guard + binary resolution + `autospec-db emit "$@"`. It performs no
payload assembly, spool, drain, or timeout. Chokepoints source the installed shim
and call `emit_event <kind> [key=value]...`.

### CLI surface

```
autospec-db emit <kind> [key=value]...   # append one event
autospec-db drain                        # force-drain the spool
```

- **Exit 0 always.** `autospec-db` recovers panics and never propagates a
  non-zero status. Emission may never block, retry inline, or alter a caller's
  exit/return value. The shim mirrors this: every path through `emit_event`
  returns exit 0.
- **Spool path:** `~/.autospec/db-spool.jsonl` — a local fire-and-forget spool,
  one JSON event per line (JSONL), flock-guarded so parallel emitters are safe.
  On any failure the binary appends the event line here and exits 0.
- **Binary resolution:** the shim resolves `autospec-db` on `PATH` first, then
  falls back to `~/.autospec/bin/autospec-db`. An absent binary is a silent
  no-op (`return 0`).

### Kill switch and no-op conditions

- **`AUTOSPEC_DB_DISABLE=1` — hard off.** When set, telemetry is fully disabled.
  The operator-facing surface is the `.autospec/autospec.yml` `telemetry:` block:
  `telemetry.enabled: false` makes the config loader export `AUTOSPEC_DB_DISABLE=1`.
  **Enforcement point:** the config loader (`telemetry-config.sh`, session
  bootstrap) is what honors this flag by never exporting `AUTOSPEC_DB_DSN` and
  never writing `~/.autospec/db.env` in the first place — the shim itself (this
  issue's scope stops at documenting the shim, not modifying it) has no
  special-case branch for `AUTOSPEC_DB_DISABLE`; it only ever checks
  `AUTOSPEC_DB_DSN` / `~/.autospec/db.env` presence. A disabled config that still
  leaves a stale `db.env` on disk is out of scope here and tracked with the
  loader work.
- **Total no-op** when neither `AUTOSPEC_DB_DSN` is set NOR `~/.autospec/db.env`
  exists (`db.env` alone configures the binary — session bootstrap need not have
  exported the DSN), or when the `autospec-db` binary cannot be resolved.

## Configuration surface

`.autospec/autospec.yml` `telemetry:` is the ONLY operator-facing flag surface.
Every environment variable below is derived plumbing or a CI/test override —
resolved by `telemetry-config.sh` (mirrors `advisor-config.sh`; precedence
env > yaml > default).

| yaml key | derives | meaning |
|---|---|---|
| `telemetry.enabled` | `AUTOSPEC_DB_DISABLE` | `false` → exports `AUTOSPEC_DB_DISABLE=1` (hard off); default enabled |
| `telemetry.host_label` | `AUTOSPEC_DB_HOST_LABEL` | `''` = short hostname |
| `telemetry.spool_max_bytes` | `AUTOSPEC_DB_SPOOL_MAX_BYTES` | spool size cap, default `10485760` (10 MB) |
| `telemetry.install.db_module` | installer behavior | `prompt` \| `always` \| `never` |

| env var | meaning |
|---|---|
| `AUTOSPEC_DB_DSN` | Postgres DSN (`sslmode=require`, `connect_timeout=2`); unset AND `~/.autospec/db.env` absent ⇒ total no-op |
| `AUTOSPEC_DB_SPOOL_MAX_BYTES` | spool size cap, default `10485760` (10 MB); drops oldest lines first (binary-side) |
| `AUTOSPEC_DB_DISABLE` | `=1` hard off — telemetry fully disabled |
| `AUTOSPEC_SCRIPTS_DIR` | installed-scripts dir (default `$HOME/.autospec/scripts`) |

### DSN setup

The DSN itself is never yaml-configured — it lives only in `~/.autospec/db.env`,
a `chmod 600` file that exports `AUTOSPEC_DB_DSN` (with `sslmode=require` and
`connect_timeout=2` in the connection string), e.g.:

```bash
# ~/.autospec/db.env (chmod 600; never committed, never echoed)
export AUTOSPEC_DB_DSN='postgres://user:pass@host:5432/db?sslmode=require&connect_timeout=2'
```

Session bootstrap (`~/.autospec/env`, installed by `install.sh`
`ensure_autospec_bin_path`) sources `db.env` guarded by `[ -f ]` — an absent
`db.env` is a total no-op, no directory or file is created implicitly. The same
bootstrap step calls `telemetry-config.sh` to derive `AUTOSPEC_DB_DISABLE`,
`AUTOSPEC_DB_HOST_LABEL`, and `AUTOSPEC_DB_SPOOL_MAX_BYTES` from the
`telemetry:` yaml block, honoring env > yaml > default precedence so a
pre-set env var (CI override, ad-hoc shell export) always wins. Because
`telemetry.enabled: false` is enforced by this config loader — it exports
`AUTOSPEC_DB_DISABLE=1` before any chokepoint runs — the shim never even
attempts to resolve the binary when telemetry is disabled via yaml.

## Transport internals live in the `autospec-db` repo

Everything below is owned by the standalone **autospec-db** Go module — NOT this
repo. The core repo ships only the shim, the chokepoint call-sites, and bats
tests that PATH-shim a stub `autospec-db` binary (never a real binary, never a
live database). No `psql` anywhere in the core repo.

- **Payload assembly** — the binary builds the `autospec.events.v1` object and
  stamps the auto-fields (`schema`, `event_uuid`, `ts`, `session_id`, `host`).
- **Wire call** — `SELECT autospec.ingest($1::jsonb)` with the payload as a bound
  `$1` parameter (injection-free by construction). Agents only ever execute
  `ingest()`; they never touch typed columns or the events table.
- **Timeout** — a 2s cap on the DB call.
- **Spool** — on any failure, append the event line to `~/.autospec/db-spool.jsonl`
  (flock-guarded) and exit 0.
- **Drain** — every successful emit opportunistically drains the spool through the
  same `ingest()` path; idempotent via `event_uuid` + `ON CONFLICT DO NOTHING`;
  `autospec-db drain` forces it. The `AUTOSPEC_DB_SPOOL_MAX_BYTES` cap drops the
  oldest lines first.
- **Server-side hardening** — TLS, SCRAM, EXECUTE-only agent grants, and pg_hba
  scoping are documented in autospec-db; they are not enforceable from this repo.
