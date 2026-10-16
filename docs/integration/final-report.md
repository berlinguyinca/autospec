# Pi-AutoSpec autonomous delivery: final report

Status: skeleton. Every cell below is `missing` or `unmeasured` until a merged PR, its merge SHA and a measured result back it. A cell is never filled from intent.

Source: [docs/specs/2026-10-14-pi-autospec-autonomous-delivery-design.md](../specs/2026-10-14-pi-autospec-autonomous-delivery-design.md) section 6 (task DAG), section 7 (acceptance and benchmarks), section 5 (R-ROLL, R-DOC).

Status values: `missing` (nothing merged), `partial` (merged in part, remainder named under Blocker), `implemented` (merged, PR and SHA recorded, verified). A status other than `missing` requires a PR link and a merge SHA in the same row.

## Status matrix

| Task | Repo | Requirement | Status | PR | Merge SHA | Measured delta | Blocker |
|---|---|---|---|---|---|---|---|
| T01 accept snapshot v3 + shared golden fixture | pi-engineering-web | R-PROTO-0 | missing | - | - | unmeasured | - |
| T02 Verifier failure baseline | pi-engineering | R-DELIVER | missing | - | - | unmeasured | - |
| T03 agent-ran proof, model policy to Pi, Pi version pin | autospec-orchestrator | R-WATCH, R-ORCH | missing | - | - | unmeasured | - |
| T04 `AGENT-NEVER-STARTED` vs `NO-OUTPUT` | autospec | R-WATCH | missing | - | - | unmeasured | - |
| T05 resource registry, build budgets, GC | autospec-orchestrator | R-RES, R-BUILD | missing | - | - | unmeasured | - |
| T06 local registry, cleanup, build budgets | pi-engineering | R-RES, R-BUILD | missing | - | - | unmeasured | - |
| T07 managed watchdog events (progress-based) | autospec-orchestrator | R-WATCH | missing | - | - | unmeasured | - |
| T08 shared protocol + fixtures | autospec | R-PROTO | missing | - | - | unmeasured | - |
| T09 ADR: authority, dispatcher placement; fix ADR 0002 | autospec | §4 | missing | - | - | unmeasured | - |
| T10 modes, flags, recursion guard | pi-engineering | R-MODE | missing | - | - | unmeasured | - |
| T11 adapter + external entry point | pi-engineering | R-ADAPT | missing | - | - | unmeasured | - |
| T12 dispatcher-core: family independence, SoD consolidation, drop stub | autospec-dispatcher + autospec | R-DELIVER | missing | - | - | unmeasured | - |
| T13 predictive collision + placement by arch/toolchain | autospec, autospec-orchestrator | R-PAR | missing | - | - | unmeasured | - |
| T14 review→repair→merge flow, conflict + interrupt recovery | autospec, pi-engineering | R-DELIVER | missing | - | - | unmeasured | - |
| T15 post-merge cleanup of owned resources | both | R-RES | missing | - | - | unmeasured | - |
| T16 InferWeave capability mapping + qualified failover, BLOCKED on no capacity | pi-engineering | R-MODEL | missing | - | - | unmeasured | - |
| T17 InferWeave: eligible model/slot, **estimated wait**, route reasons, reasoning flag, usage | inferweave | R-MODEL | missing | - | - | unmeasured | - |
| T18 saturation fault-injection test | pi-engineering | R-MODEL | missing | - | - | unmeasured | - |
| T19 HerdR mission/test views from snapshot+events | herdr-engineering | R-VIEW | missing | - | - | unmeasured | - |
| T20 autospec-db model/token/mission columns; GUI fix or freeze | autospec-db, autospec-gui | R-VIEW | missing | - | - | unmeasured | - |
| T21 orchestrator hardening (non-root, caps, per-worker token) | autospec-orchestrator | R-SEC | missing | - | - | unmeasured | - |
| T22 unify DAG/lease/lifecycle behind adapters + deprecation | autospec | §4.6 | missing | - | - | unmeasured | - |
| T23 benchmarks, chaos run (A1–A9), final report, rollback docs | autospec + pi-engineering | R-DOC | missing | - | - | unmeasured | - |
| T24 map zip lifecycle onto the existing mission state machine (additive, versioned) | pi-engineering | R-LIFE | missing | - | - | unmeasured | - |
| T25 test watchdog: per-test progress, process tree, evidence, quarantine, nextest, result cache | pi-engineering | R-WATCH | missing | - | - | unmeasured | - |
| T26 serialized SHA-fenced merge queue, post-merge smoke and revert | pi-engineering | R-MERGEQ | missing | - | - | unmeasured | - |
| T27 capability request / routing decision / event schemas + contract tests | inferweave-specs | R-MODEL, R-EVENT | missing | - | - | unmeasured | - |
| T28 gateway compatibility check (thin relay, no scheduler); fix only if broken | metabolomics-us/inferweave-gateway | R-MODEL | missing | - | - | unmeasured | - |
| T29 `AUDIT.md`, `CONTRACT_GAPS.md`, `IMPLEMENTATION_STATUS.md` deliverables | pi-engineering | R-DOC | missing | - | - | unmeasured | - |

## Acceptance A1-A9

Zip acceptance A1-A9 are mandatory. Each is run as a fault-injection test or the 20-agent stress run (T23). Status is `pass`, `fail` or `not-run`.

| Scenario | Spec section 7 items | Summary | Status | Evidence |
|---|---|---|---|---|
| A1 | 10 | Hanging 8 h Rust test detected in minutes with last progress and process tree | not-run | - |
| A2 | 11 | Overnight 20-worker build storm stays inside budgets, measurable cache reuse, no Cargo lock convoy | not-run | - |
| A3 | 12 | Merge triggers idempotent, restart-safe cleanup that loses no dirty work | not-run | - |
| A4 | 9 | Model or gateway capacity failure switches to a qualified model without thrash or duplicate tool effects, else bounded BLOCKED | not-run | - |
| A5 | 7, 8 | Concurrent PRs revalidate on stale SHA, protections honored, optional CI does not hold progress | not-run | - |
| A6 | 4, 5 | Supervisor killed mid review, merge or cleanup resumes without duplicate merge or unsafe deletion | not-run | - |
| A7 | 13 | HerdR shows live state and historical replay after restart; missing telemetry labelled unknown or stale | not-run | - |
| A8 | 2, 7 | Mixed-repo mission correlated end to end; dependencies unblock only after verified gates | not-run | - |
| A9 | - | Faults in disk, permissions, test runner, GitHub, inference and network yield bounded retries, diagnostics and safe pause | not-run | - |

## Benchmarks

Identical task set against the standalone baseline. Time is time-to-verified in seconds. Thresholds come from measured baselines, never invented percentages; a task with no successful baseline run has no comparison.

| Task kind | Baseline p50 | Baseline p95 | pi-autospec p50 | pi-autospec p95 | Tokens | Retries | Disk | Success |
|---|---|---|---|---|---|---|---|---|
| trivial fix | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured |
| medium feature | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured |
| complex Rust change | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured |
| multi-repo contract | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured |
| failing test | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured |
| gateway busy | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured |
| stale CI | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured |
| patch collision | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured |
| crash | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured |
| long test | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured |

## Rollback

Each stage rolls back independently. Standalone Pi must keep working at every step (spec R-ROLL, acceptance item 15).

1. Local adapter: disable the AutoSpec adapter in Pi; Pi falls back to its standalone mode with no AutoSpec dependency.
2. Managed backend: stop launching Pi through `harness-pi`; managed missions drain and pause, local execution continues.
3. Automatic classification: turn model-fit auto-classification off; issues keep their last recorded labels and route by those.
4. Back to standalone Pi: remove the adapter configuration and flags; no data migration is required because the protocol is additive and versioned.

## Blockers

None recorded. List here every row whose Status is `partial` or `missing` because of an external dependency, with the task, the dependency and the date it was found.
