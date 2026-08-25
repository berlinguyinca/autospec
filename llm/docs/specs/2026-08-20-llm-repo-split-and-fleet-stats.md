# Design: a private `autospec-llm` repository, and autospec fleet stats on the node

> Brainstorm provenance: classified architectural. History strategy, the
> private-repo leak discipline, the telemetry read credential and the shared-key
> removal ordering were locked interactively with the operator (2026-08-20). The
> shared-key removal has already shipped; this document covers the other two.

Extends the three node designs already in this directory. Measured ceilings
remain authoritative in the node's own `docs/measured-ceilings.md`.

## 0. Two changes, one document

They are together because the second lands *in* the first: the fleet panel is
code that belongs to the extracted repository, and building it in the old home
would mean moving it twice.

| | Delivers | Blocked on |
|---|---|---|
| **A. Extract to `autospec-llm`** | server-setup work lives in its own private repository, with history | nothing |
| **B. Autospec fleet stats** | the node's dashboard answers "which of my agents is stalled right now" | one read-only database password |

---

## 1. The seam is clean, and that is measured rather than assumed

Before proposing an extraction, the coupling was checked in both directions.

| | |
|---|---|
| Tracked files under `llm/` | **108** |
| Size | 1.4 MB |
| Commits touching `llm/` | **34** |
| Commits touching **only** `llm/` | **24** |
| References from `llm/` to the parent repository's tooling | **none** |

That last row is the one that matters. The node tooling sources nothing from the
parent's shared libraries, skills or helper scripts — its installer is
self-contained, which was a deliberate property of the original design and is
what makes this a move rather than a rewrite.

The traffic is entirely one-way: the parent repository refers to `llm/` from
`AGENTS.md`, one CI job, and six design documents. Nothing under `llm/` refers
out.

---

## 2. What moves, what stays, what is rehomed

**Moves:** everything under `llm/`, plus the six design documents that describe
it and currently live outside it — three specs and three plans. They are moved
**into `llm/docs/` first, as a commit in this repository**, so the history-
preserving split carries them too. Re-typing them in the new repository would
lose the review trail that says why each decision is what it is.

**Stays:** `docs/memory/`. That is the operator's cross-project memory, not this
project's documentation, and several entries are about autospec itself. Also
stays: the one test fixture whose sample diff happens to mention a nested path.

**Rehomed:** the `llm-node-checks` CI job becomes the new repository's own
workflow, and `AGENTS.md` gains a pointer where it had a subtree.

### 2.1 Layout after the move

Inside a dedicated repository the `llm/` prefix is noise. The node directories
rise to the root:

```
linux-turing-dual/     the dual-Turing node (this one)
linux-qwen38/          the RTX 4090 node
QWEN-NODE-SPEC.md      the portable, hardware-adaptive spec
docs/specs/            the design documents
docs/plans/            the implementation plans
```

That rename is a separate commit **after** the split, not a complication of it:
a split that also reorganises is a split whose history is harder to read.

---

## 3. How the split is done

`git subtree split --prefix=llm` produces a branch containing only the commits
that touched that prefix, with their messages and authorship intact. Preferred
over `filter-repo` here only because it is built into git and needs nothing
installed.

Requirements:

- The new repository is **private**, named `autospec-llm`.
- The split branch is pushed as its `main`; the working tree is verified to match
  the original `llm/` byte for byte before the old copy is deleted.
- The old `llm/` tree is removed from this repository **in the same change** that
  adds the pointer. Two copies drift the moment either is edited, and the drift
  is silent.
- The new repository's first act is to prove itself: its own CI must run the
  full test suite and both structural suites green before the old tree goes.

### 3.1 What must be verified, not assumed

The extraction is easy to get subtly wrong, so:

1. **File-for-file equality** between the split result and the original subtree.
2. **The tests pass in the new repository**, run there rather than here — a suite
   that passes only in its old location has an undiscovered path dependency.
3. **The installer still works from the new layout.** It resolves its own
   location; that resolution is exactly what a directory move breaks.
4. **No absolute path referring to the old repository** survives anywhere in the
   moved tree.

---

## 4. Private does not mean careless

The repository becomes private, and the placeholder discipline **stays exactly as
it is**: real hostnames, addresses, pool and client identifiers, database names
and interface names live in `site.conf`; committed files carry
`<angle-bracket>` placeholders; and both structural checks that enforce it stay
in force.

The reasoning, recorded so it is not relitigated as friction later: the mechanism
is already built and already tested, so keeping it costs nothing now — whereas
retrofitting it later means auditing every file. "Private" is a repository
setting that somebody can flip in one click, and a repository that was written
assuming privacy cannot be opened without a scrub. Secrets were never committed
either way; this is only about identifiers.

---

## 5. The cross-repository contract

After the split, the node reads the autospec telemetry database. That is a
dependency between two repositories, so its interface is stated rather than
implied.

**The node reads VIEWS, never raw tables.** `autospec.sessions` and
`autospec.stalled` are the contract; `events_raw` — a `jsonb` payload column
whose keys are an event schema — is not. If the event shape changes, the views
absorb it and the node needs no change. A panel built on `payload->>'step'`
would break silently on a schema revision and show an empty column rather than
an error.

**Read-only, and provably so.** The role holds `SELECT` on the five objects and
**cannot execute the ingest function** — verified, not assumed. The node can
therefore never write to, or corrupt, autospec's telemetry.

**Pull, not push.** Nothing changes in autospec. No agent learns about this node,
no emit path grows a second destination, and the optionality contract that
governs autospec's telemetry — never block, never slow a run — is untouched.

---

## 6. Fleet stats on the node

### 6.1 What is actually available

Measured against the live database before designing against it: **10,698 events**
across eight kinds, and `sessions` already aggregates per session:

`session_id`, `host`, `repo`, `started_at`, `last_seen_at`, `last_heartbeat_at`,
`last_step`, `last_issue`, `last_pr`, `is_terminal`, `is_parked`,
`terminal_outcome`, `event_count`.

Event kinds, by volume: `heartbeat`, `artifact.filed`, `session.terminal`,
`claim`, `session.parked`, `session.step`, `feature.described`,
`session.started`.

So the question the autospec-db design set out to answer — *which of my agents,
across all machines, is stalled right now* — is answerable from one view. This
panel is a reader, not a new aggregation layer.

### 6.2 Where the code lives

The **dashboard** collects it. It is already the process that polls, caches and
serves numbers on a background timer, and this is the same shape of work. The
**gateway** authorises it, exactly as it now does for `/api/stats`.

```
browser → nginx → gateway (session or key) → dashboard → autospec views
```

Requirements:

- A **separate refresher thread and a separate cache entry**, so a slow or
  unreachable telemetry database cannot delay the node's own stats. The two have
  nothing to do with each other and must not share a failure.
- The refresh interval is minutes, not seconds. This is fleet state, not queue
  state; polling it at the queue's rate would be pointless load on a shared
  production database.
- **Inference never depends on it**, and neither does the rest of the dashboard.
  Unreachable degrades to "unavailable" with the reason, in the manner the
  config-health panel already uses.

### 6.3 It must never be public

Session identifiers, repository names, issue and PR numbers are internal. The
public load page and the public queue endpoint carry no credential and are read
by colleagues.

Requirement: `/api/fleet` is reached only through the gateway, and the
forward-iterating public allow-list — which already guarantees that a field
added upstream is absent from public payloads rather than leaked into them —
must not grow a fleet field. A test asserts it.

### 6.4 "Stalled" does not mean what the panel would imply

Measured against the live database before designing the panel, and it changed the
design:

| | |
|---|---|
| Sessions | 7,109 |
| Flagged by the `stalled` view | **5,645 — 79%** |
| Of those, last seen **within 24 h** | **0** |
| Of those, last seen over 7 days ago | 4,331 |
| Sessions that ever emitted a terminal event | **655 of 7,109 (9%)** |

So the view is behaving correctly and would still make a useless panel. It flags
"no terminal event and no recent heartbeat", and **91% of all sessions never
emitted a terminal event at all** — they were killed, parked, or simply ended
without one. A panel headlining *5,645 stalled agents* would be noise, and noise
teaches an operator to stop reading the panel.

Requirements that follow:

- **The headline is windowed: stalled AND last seen within 24 hours.** Right now
  that number is **zero**, which is the true and useful answer — nothing is stuck.
- The historical backlog is shown as history, clearly separated, or not at all.
  It is not an alert.
- **The window is the panel's; the threshold stays autospec's.** Stall detection
  lives in `autospec.stalled_sessions()`, and reimplementing it here would create
  two definitions that disagree at the boundary. The panel filters the view's
  output by recency; it never decides what stalled means.

### 6.5 An unreporting host is invisible, and invisible looks healthy

Every session in that database comes from **two hosts, both Macs** — 7,053 from
one, 56 from the other. Nothing from any Linux host, because the emitter binary
is not installed on them; verified absent on both the inference node and the
workstation.

That is the failure mode this panel must not have. "Which of my agents, across
all machines, is stalled?" cannot be answered by a view that only sees the
machines that happen to be reporting, and a host that stopped emitting looks
exactly like a host with nothing to do.

Requirement: the panel lists **hosts and when each last reported**, and says
plainly that it can only see hosts running the emitter. Coverage is part of the
reading, not a footnote.

### 6.6 The rest of the honesty

**Leads with recency.** At the time of writing the newest event is two days old.
An idle fleet and a broken panel look identical unless the panel says which, so
it opens with *"last event N ago"* and labels quiet as quiet — the same restraint
that made the queue panel show an em dash rather than a fabricated zero.

Beyond the headline: sessions by host with what each was last doing, features
filed, artifacts recorded, events per day.

## 7. Acceptance criteria

**A. Extraction**

1. The new private repository's tree is **byte-identical** to the original
   `llm/` subtree, verified by comparing file lists and hashes.
2. Its history contains the commits that touched `llm/`, with messages intact —
   spot-checked against a commit whose message carries a measurement.
3. The full test suite and both structural suites pass **in the new repository**.
4. The installer runs from the new layout and still refuses to claim success
   without a real request.
5. No absolute path referring to the old repository survives in the moved tree.
6. The old tree is deleted and `AGENTS.md` points at the new repository, in one
   change.

**B. Fleet stats**

7. The panel shows the real session count, host breakdown and stalled list from
   the live database.
8. With the telemetry database unreachable, the panel says so **and the rest of
   the dashboard is unaffected** — verified by blocking the connection, not by
   reading the code.
9. Inference latency is unchanged with the fleet refresher running.
10. `/api/fleet` without a session or key is refused.
11. No fleet field appears in any public payload.
12. An empty result renders as "quiet since <time>", never as an empty panel.

---

## 8. Out of scope

- Changing anything in autospec's telemetry emission. This is a reader.
- Alerting on stalled agents. The panel makes them visible; deciding who gets
  woken up is a policy question with no obvious owner yet.
- Migrating `docs/memory/` anywhere. It is the operator's, and it spans projects.
- Any write path to the telemetry database, now or later. The credential cannot
  do it and should stay that way.
- Retiring the production superuser credential recorded in an earlier design.
  Still the operator's, still outstanding, still not this work.
