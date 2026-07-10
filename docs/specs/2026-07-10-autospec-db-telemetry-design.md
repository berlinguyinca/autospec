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
subprocess-mocked bats tests (binary PATH shim, never a live database), no in-memory-only
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
autospec agents ──autospec-db binary (emit + spool, pgx)──> Postgres
                                                                  │ read-only role
                                              Grafana/Metabase ◄──┘  (v1 UI; custom
                                                                      GUI deferred)
```

1. **Transport = the `autospec-db` Go binary** (static, embedded pgx driver — NO
   `psql` or any other host dependency), shipped from the autospec-db repo via
   GitHub Releases for darwin/arm64, darwin/amd64, linux/amd64, linux/arm64.
   `autospec-db emit <kind> [key=value]...` builds the `autospec.events.v1` payload
   (event_uuid, ts, session_id from the harness env fallback chain, host) and executes
   `SELECT autospec.ingest($1::jsonb)` as a bound parameter — injection-free by
   construction. The `ingest()` function (SECURITY DEFINER, owned by autospec-db)
   performs the idempotent insert internally; agents never couple to typed columns or
   even the table. (Historical falsified alternatives, do not implement: raw
   `INSERT ... ON CONFLICT` from the agent role — requires SELECT on the arbiter which
   the role lacks; psql-based emit — rejected for the host dependency and shell JSON
   assembly fragility.)
2. **One shared shim** `skills/autospec-shared/scripts/emit-event.sh` is the only
   core-repo integration point (chokepoints source it and call
   `emit_event <kind> [key=value]...`). Guards, in order: neither `AUTOSPEC_DB_DSN`
   set nor `~/.autospec/db.env` present → return 0; `autospec-db` binary not found
   (PATH, then `~/.autospec/bin/autospec-db`) → return 0; otherwise invoke the
   binary with the caller's args. The binary is the single config authority: the
   shim's config guard exists only to avoid spawning a process on unconfigured
   machines and MUST be strictly weaker than (never stricter than) the binary's own
   config check — `install` produces db.env without exporting the DSN, so a
   DSN-only guard would silently drop all telemetry from contexts where session
   bootstrap has not run (cron, subagents, conductors). The shim adds no logic
   beyond guard + dispatch — payload assembly, timeout (2s), spool, and drain all
   live in the binary.
3. **Spool + drain are binary-internal**: on any failure the binary appends the event
   line to `~/.autospec/db-spool.jsonl` (flock-guarded — concurrent emitters from
   parallel runs are safe) and exits 0; every successful emit opportunistically drains
   the spool through the same `ingest()` path, idempotent via `event_uuid` +
   `ON CONFLICT DO NOTHING` (at-least-once; live emit and drain share one code path).
   `autospec-db drain` forces a drain manually.
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
7. **Repo split.** This repo owns: the event payload contract (below), the
   `emit-event.sh` shim, chokepoint wiring, and bats tests (which PATH-shim a stub
   `autospec-db` binary — never the real one, never a live database). The standalone
   **autospec-db** repo (Go module) owns: the `autospec-db` binary (`install`,
   `migrate`, `emit`, `drain`, `sessions [--stalled]`, `doctor` subcommands), SQL
   migrations embedded via embed.FS (`events_raw`, `ingest()`, typed views, stall
   view; the `autospec.schema_migrations` tracking table and filenames are preserved
   from the shell era so existing deployments no-op), role convergence (EXECUTE-only
   `autospec_emit`, read-only `autospec_read`), goreleaser + CI release pipeline, a
   thin `install.sh` bootstrap (platform-detect → fetch release binary →
   `autospec-db install`), Grafana dashboard JSON, and an optional
   `docker-compose.yml` for adopters without a server. A custom `autospec-gui` is
   DEFERRED — `autospec-db sessions --stalled` covers terminal monitoring, Grafana
   covers dashboards.

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

- DSN unset / binary absent / insert fails / spool unwritable → silent no-op or spool;
  exit 0 in ALL cases. Neither the `emit-event.sh` shim nor `autospec-db emit` may ever
  propagate a non-zero exit to a caller (the binary recovers panics and exits 0).
- Spool grows unbounded only while the database is unreachable; drain truncates on
  success; a size cap (default 10 MB, `AUTOSPEC_DB_SPOOL_MAX_BYTES`) drops oldest lines
  first — telemetry is lossy-by-design, runs are not.
- Malformed payload (bad JSON) → database rejects the cast; the line is dropped on next
  drain attempt after N failures (poison-message guard), never wedging the spool.

## Testing

Core repo: TDD, bats, the `autospec-db` binary mocked via a PATH-shim stub logging
argv; no live database and no real binary anywhere in core CI. Required cases:
no DSN and no db.env → zero binary invocations (the optionality proof); db.env
present with DSN unset → binary IS invoked (the bootstrap-independence proof);
binary absent → exit 0;
each chokepoint passes the mapped kind + fields; DSN never appears in any output. New
suites must be registered in a `scripts/validate.sh` check_* gate (enumerated, not
globbed). autospec-db repo: Go unit tests (spool locking, payload construction,
config parsing, env fallback chain) + integration tests against a real dockerized
Postgres in CI (migrations parity incl. shell-era `schema_migrations` rows, ingest
dedup, role blast radius, stall view).

## Out of scope

autospec-db repo content (migrations, roles, dashboards, compose) — separate repo;
custom autospec-gui; alert routing; NATS/SQS or any broker; multi-hub sync; GitHub
polling changes; retro-ingestion of historical ledgers (possible later via a one-shot
backfill script in autospec-db).

## Installer integration (autospec core install.sh)

The repo-root `install.sh` offers the database module as a first-class optional
plugin, mirroring the existing `maybe_prompt_star` prompt discipline exactly
(TTY-guard via `/dev/tty` with `[ -t 0 ] && [ -t 1 ]` fallback, quiet under
`CI`, quiet under `--dry-run`, env opt-out):

1. **Detection.** The module counts as installed when `~/.autospec/db.env`
   exists (configured) OR `~/.autospec/autospec-db/` exists (fetched). A
   config-only state (`~/.autospec/db.conf` present but no `db.env`) means a
   half-finished install: treat as installed for prompting purposes and print
   the finish hint instead of re-prompting.
2. **Fresh install (module absent).** Prompt
   `Install the optional database telemetry module (autospec-db)? [y/N]`.
   Default No; decline is remembered for the session only (no marker file — a
   later re-install may re-ask). On yes, run the autospec-db one-line
   installer (`curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec-db/main/install.sh | bash`)
   and surface its output verbatim — including the edit-db.conf-and-re-run
   message on first configuration. An installer failure warns and continues;
   it NEVER fails the autospec install.
3. **Update (`--update`) or module already installed.** No prompt. If the
   module is installed, re-run its installer to converge to the latest version
   (it is idempotent: refreshes the checkout, applies only new migrations,
   converges roles, leaves passwords untouched). If absent during `--update`,
   stay silent — updates never introduce new prompts.
4. **Env controls.** `AUTOSPEC_NO_DB_PROMPT=1` suppresses the prompt;
   `AUTOSPEC_INSTALL_DB=1` forces install/update without prompting (CI and
   scripted installs); `AUTOSPEC_INSTALL_DB=0` forces skip even when installed
   (leaves the module untouched, including no update).
5. **Testing.** bats with a PATH-shim `curl` stub (never fetches) and a fake
   `$HOME`: absent+decline → no fetch; absent+yes → installer invoked;
   installed+`--update` → installer invoked without prompt; `AUTOSPEC_INSTALL_DB=0`
   → untouched; non-TTY + no env → silent skip; installer failure → autospec
   install still exits 0. Register the suite in a `scripts/validate.sh`
   check_* gate (enumerated, not globbed).

## Decomposition hints

1. `emit-event.sh` shim (guard + dispatch to the binary) + contract doc + bats (foundation; everything depends on it — spool/payload/drain are binary-side, NOT core-side)
2. heartbeat + run-state chokepoint wiring
3. claim-guard + ledger-append chokepoint wiring
4. `feature.described` + park/stop chokepoint wiring
5. session bootstrap sourcing of `~/.autospec/db.env` + docs
6. install.sh optional-db-module prompt + auto-update (Installer integration section)
