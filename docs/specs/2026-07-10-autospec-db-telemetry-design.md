# autospec-db telemetry — transparent per-run event emission to a central Postgres

> Brainstorm provenance: architecture, transport, optionality contract, multi-site
> constraints, and repo split were locked interactively with the operator in the
> originating session (2026-07-10). No open questions remain from the brainstorm.

## Problem

Every autospec run — interactive `/autospec-run` batches, `/autospec-autonomous`
conductor tiers, explore/growth cycles, fleet agents — produces work the operator wants
to observe centrally: what is running where, what each session is doing, which agents
have stalled, and what feature descriptions/specs the pipeline generated. Today the only
observation substrates are (a) per-host files under `~/.autospec/` (heartbeats,
run-state, ledgers) that are invisible across the operator's three physically separate,
firewalled sites, and (b) GitHub — which is rate-limited and cannot absorb
monitoring-frequency polling on top of pipeline traffic.

There is no way to answer "which of my agents, across all machines, is stalled right
now?" without ssh-ing into each host.

## Rule

Agents emit lifecycle events to a central Postgres database through ONE shared helper,
and the entire capability is 100% optional: with `AUTOSPEC_DB_DSN` unset, every emit
call is a no-op costing a single `[ -n ]` test — zero config, zero files, zero
behavioral delta, zero log noise. Emission is fire-and-forget and may NEVER block, fail,
retry inline, or slow an agent: an unreachable database costs at most
`connect_timeout=2` seconds once, then degrades to a local spool.

Observation reads move to Postgres; GitHub remains the pipeline substrate (issues, PRs,
labels) and is never polled for monitoring.

## Team personality

**Reliability/backend** — backend/bash developer, platform engineer, SRE, security
advisor, test engineer. This is telemetry-plane engineering: the risks this team must
notice are emission blocking or failing a run, credential leakage into transcripts, SQL
injection through event payloads, spool replay duplication, and clock skew across sites.
Carry into child issues: bash 3.2 compatibility, `set -eu` discipline,
subprocess-mocked bats tests (psql PATH shim, never a live database), no in-memory-only
state.

### Review counter-team

**Operator-experience & security review** — security advisor (DSN handling, EXECUTE-only
blast radius, payload injection), SRE (does a dead/slow database ever slow a run?),
operator advocate (is setup really one env file? is "optional" really zero-touch?).
Challenge: grep-audit that NO emit path can block, and that no code path prints the DSN.

## Architecture

Direct-to-Postgres, no broker, no collector service. The operator already runs a
reachable Postgres server; agents at all sites need only OUTBOUND connectivity to it
(firewall-friendly; nothing listens on agent machines).

```
autospec agents ──psql ingest() (emit-event.sh + local spool)──> Postgres
                                                                  │ read-only role
                                              Grafana/Metabase ◄──┘  (v1 UI; custom
                                                                      GUI deferred)
```

1. **Wire contract = one jsonb blob through one function.** Agents only ever execute
   `SELECT autospec.ingest(:'payload'::jsonb)`. The function (owned by autospec-db,
   SECURITY DEFINER) performs the idempotent insert internally; all typing and
   normalization happens database-side (views in the autospec-db repo). Agents never
   couple to typed columns or even the table, so the storage layer can migrate freely
   without breaking in-field emitters. (Falsified alternative, do not implement: a raw
   `INSERT ... ON CONFLICT` from the agent role — Postgres requires SELECT on the
   arbiter index for `ON CONFLICT`, which the least-privilege agent role deliberately
   lacks; verified against postgres:16.)
2. **One shared helper** `skills/autospec-shared/scripts/emit-event.sh` is the only
   integration point. Guards, in order: `AUTOSPEC_DB_DSN` unset → exit 0; `psql` not on
   PATH → exit 0; call `ingest()` with the psql `:'payload'` variable idiom (payload is
   BOUND, never spliced into SQL text — regex/quote/`$$` content in titles cannot
   inject; NB psql only interpolates `:'var'` in stdin/`-f` input, NOT inside `-c`
   strings, so the helper feeds its SQL via stdin); failure → append the event line to
   `~/.autospec/db-spool.jsonl` and exit 0.
3. **Spool drain** rides the next successful emit (or heartbeat tick — no new loop):
   each event carries a client-generated `event_uuid`; the drain replays spool lines
   through the same `ingest()` call, whose internal unique-index + `ON CONFLICT DO
   NOTHING` makes replays harmless (at-least-once, idempotent; live emit and drain
   share one code path).
4. **Chokepoint wiring, never per-site.** Emission hooks into the shared lifecycle
   helpers that all runs already pass through — NOT into individual skills/scripts
   (the origin:self grep-audit across five sibling issues is the cautionary tale):
   - heartbeat write → `heartbeat` event (session, repo, issue, step, pr)
   - run-state transition → `session.started` / `session.step` / `session.terminal`
   - claim-guard acquire/release → `claim` event (surface, conflict)
   - explore/growth ledger append → `artifact.filed`
   - define/decompose issue+spec generation → `feature.described` (full body text)
   - stop.flag / park / quota exhaustion → `session.parked`
5. **Session identity** rides the existing harness session-id fallback chain
   (`CLAUDE_CODE_SESSION_ID` → harness-neutral fallbacks); every event also carries
   `host` (hostname, first-class column downstream) so the sessions grid groups by
   machine/site.
6. **Stalled-agent detection is server-side**: a SQL view flags any session whose last
   heartbeat is older than a threshold with no terminal event. Agents contain zero
   stall logic. Alerting (ntfy/Slack) attaches to that view in Grafana — out of scope
   for this repo.
7. **Repo split.** This repo owns: the event payload contract (below), `emit-event.sh`,
   chokepoint wiring, and tests. The standalone **autospec-db** repo owns: SQL
   migrations (`events_raw`, `ingest()`, typed views, stall view), role bootstrap
   (EXECUTE-only `autospec_emit`, read-only `autospec_read`), Grafana dashboard JSON, and an optional
   `docker-compose.yml` (postgres + grafana) for adopters without a server. A custom
   `autospec-gui` is DEFERRED until dashboards pinch (write operations like remote
   stop would revive it).

## Data model

Event payload contract `autospec.events.v1` (JSON, one object per event):

| field | type | notes |
|---|---|---|
| `schema` | string | literal `autospec.events.v1` |
| `event_uuid` | string | client-generated UUIDv4; idempotency key |
| `kind` | string | `heartbeat` \| `session.started` \| `session.step` \| `session.terminal` \| `session.parked` \| `claim` \| `artifact.filed` \| `feature.described` |
| `ts` | string | client UTC ISO-8601; server also stamps `received_at` (authoritative for stall math — clock skew safe) |
| `session_id` | string | harness session-id fallback chain |
| `host` | string | `hostname -s` |
| `repo` | string | `owner/name` |
| `tier`, `issue`, `pr`, `branch`, `step`, `outcome`, `detail` | optional | kind-specific; collector accepts unknown fields silently (agents may run ahead of the hub) |

Contract rules: additive-only within v1; unknown fields ignored; a v2 bump requires the
database to keep accepting v1. The contract doc lives in this repo (single source of
truth); autospec-db pins a version.

DSN handling: `~/.autospec/db.env` (chmod 600) exporting `AUTOSPEC_DB_DSN` with
`sslmode=require` + `connect_timeout=2`; sourced by session bootstrap. The DSN is never
echoed, never logged, never in any resolved command line that reaches a transcript.
Server-side hardening (TLS, SCRAM, EXECUTE-only agent grants, pg_hba scoping) is documented in
autospec-db, not enforceable here.

## Error handling

- DSN unset / psql absent / insert fails / spool unwritable → silent no-op or spool;
  exit 0 in ALL cases. `emit-event.sh` may never propagate a non-zero exit to a caller.
- Spool grows unbounded only while the database is unreachable; drain truncates on
  success; a size cap (default 10 MB, `AUTOSPEC_DB_SPOOL_MAX_BYTES`) drops oldest lines
  first — telemetry is lossy-by-design, runs are not.
- Malformed payload (bad JSON) → database rejects the cast; the line is dropped on next
  drain attempt after N failures (poison-message guard), never wedging the spool.

## Testing

TDD, bats, psql mocked via PATH shim logging argv; no live database anywhere in CI.
Required cases: unset-DSN emits zero psql invocations (the optionality proof); psql
absent → exit 0; insert failure → line lands in spool, exit 0; drain replays with
`ON CONFLICT` semantics (shim asserts event_uuid present); payload with quotes/`$$`/
backslashes arrives bound, not spliced; DSN never appears in any output. New suites
must be registered in a `scripts/validate.sh` check_* gate (enumerated, not globbed).

## Out of scope

autospec-db repo content (migrations, roles, dashboards, compose) — separate repo;
custom autospec-gui; alert routing; NATS/SQS or any broker; multi-hub sync; GitHub
polling changes; retro-ingestion of historical ledgers (possible later via a one-shot
backfill script in autospec-db).

## Decomposition hints

1. `emit-event.sh` + spool + contract doc + bats (foundation; everything depends on it)
2. heartbeat + run-state chokepoint wiring
3. claim-guard + ledger-append chokepoint wiring
4. `feature.described` + park/stop chokepoint wiring
5. session bootstrap sourcing of `~/.autospec/db.env` + docs
