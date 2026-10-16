# Pi-AutoSpec autonomous delivery: final report

Status: autospec-owned rows T04, T08, T09, T13, T22, T23 audited at `d1d890e` (Phase 5.5, #4766). Every other cell is `missing` or `unmeasured` until a merged PR, its merge SHA and a measured result back it. A cell is never filled from intent, and a green test is not proof of reachability.

Source: [docs/specs/2026-10-14-pi-autospec-autonomous-delivery-design.md](../specs/2026-10-14-pi-autospec-autonomous-delivery-design.md) section 6 (task DAG), section 7 (acceptance and benchmarks), section 5 (R-ROLL, R-DOC).

Status values: `missing` (nothing merged), `partial` (merged in part, remainder named under Blocker), `implemented` (merged, PR and SHA recorded, verified, and reachable or a document). A merged module that no non-test code calls is `partial` and the gap is named in the row. A status other than `missing` requires a PR link and a merge SHA in the same row.

## Status matrix

| Task | Repo | Requirement | Status | PR | Merge SHA | Measured delta | Blocker |
|---|---|---|---|---|---|---|---|
| T01 accept snapshot v3 + shared golden fixture | pi-engineering-web | R-PROTO-0 | missing | - | - | unmeasured | - |
| T02 Verifier failure baseline | pi-engineering | R-DELIVER | missing | - | - | unmeasured | - |
| T03 agent-ran proof, model policy to Pi, Pi version pin | autospec-orchestrator | R-WATCH, R-ORCH | missing | - | - | unmeasured | - |
| T04 `AGENT-NEVER-STARTED` vs `NO-OUTPUT` | autospec | R-WATCH | partial | [#4768](https://github.com/berlinguyinca/autospec/pull/4768), [#4769](https://github.com/berlinguyinca/autospec/pull/4769) | 69621d1, 48e3c09 | unmeasured. Verified at d1d890e: `fleet_dispatch::classify_run` returns `AGENT-NEVER-STARTED` for exit 0 with `session_recorded=false` or `assistant_turns=0`, and `status_triage::triage` returns `RaiseForReview` for it, both by unit test. Reachability: `classify_run` is called by `autospec dispatch runs --runs <json>` (`commands/dispatch.rs`), but nothing in this repo writes `session_recorded` or `assistant_turns` (the writer is autospec-orchestrator T03). `status_triage::triage` and `triage_report` have no non-test caller in `crates/`, and `dispatch_guard::classify_report`, the path `autospec dispatch` uses on `status.txt`, does not read the two keys. | [#4794](https://github.com/berlinguyinca/autospec/issues/4794) |
| T05 resource registry, build budgets, GC | autospec-orchestrator | R-RES, R-BUILD | missing | - | - | unmeasured | - |
| T06 local registry, cleanup, build budgets | pi-engineering | R-RES, R-BUILD | missing | - | - | unmeasured | - |
| T07 managed watchdog events (progress-based) | autospec-orchestrator | R-WATCH | missing | - | - | unmeasured | - |
| T08 shared protocol + fixtures | autospec | R-PROTO | partial | [#4770](https://github.com/berlinguyinca/autospec/pull/4770) [#4771](https://github.com/berlinguyinca/autospec/pull/4771) [#4772](https://github.com/berlinguyinca/autospec/pull/4772) [#4773](https://github.com/berlinguyinca/autospec/pull/4773) [#4774](https://github.com/berlinguyinca/autospec/pull/4774) [#4775](https://github.com/berlinguyinca/autospec/pull/4775) [#4776](https://github.com/berlinguyinca/autospec/pull/4776) [#4777](https://github.com/berlinguyinca/autospec/pull/4777) [#4778](https://github.com/berlinguyinca/autospec/pull/4778) [#4779](https://github.com/berlinguyinca/autospec/pull/4779) [#4780](https://github.com/berlinguyinca/autospec/pull/4780) | 589b22f, 45d9609, f42a504, 3e7a531, cae7fd7, 30f9ff2, 0abdf7c, 2ca6d1a, ddc6db2, 8a8edf5, 89d0bad | unmeasured. Verified at d1d890e: v1 schemas in `schemas/`, golden fixtures in `tests/fixtures/protocol/`, the Rust validator and generated types in `crates/autospec-core/src/protocol/`, and TypeScript declarations in `packages/protocol/` exist and are asserted by `tests/protocol_schemas.rs` and `tests/protocol_fixtures.rs`. Reachability: unwired in autospec. `check_envelope`, `load_dir` and the sequence/idempotency code are called only from tests and codegen; no CLI or runtime path reads a protocol message. Consumers are in other repositories and are not audited here. Open PR #4787 (closes #4782, a convergence branch) is not merged and not counted. | [#4795](https://github.com/berlinguyinca/autospec/issues/4795) |
| T09 ADR: authority, dispatcher placement; fix ADR 0002 | autospec | §4 | implemented | [#4767](https://github.com/berlinguyinca/autospec/pull/4767) | 2fd8e53 | n/a (docs). Verified at d1d890e: `docs/decisions/0004-pi-supervisor-authority-and-dispatcher-placement.md` exists with `## Context`, `## Decision`, `## Consequences` and the words `dispatcher-core`, `pi-local`, `autospec-managed`, `fencing`; `grep -c 'Dispatch judgement stays in' docs/decisions/0002-resilience-runtime-placement.md` prints 0; `grep -c 'ADR 0004'` on ADR 0002 prints 2; the PR touched no file under `crates/`. A document has no caller; it is a decision record, not a mechanism. | - |
| T10 modes, flags, recursion guard | pi-engineering | R-MODE | missing | - | - | unmeasured | - |
| T11 adapter + external entry point | pi-engineering | R-ADAPT | missing | - | - | unmeasured | - |
| T12 dispatcher-core: family independence, SoD consolidation, drop stub | autospec-dispatcher + autospec | R-DELIVER | missing | - | - | unmeasured | - |
| T13 predictive collision + placement by arch/toolchain | autospec, autospec-orchestrator | R-PAR | partial | [#4783](https://github.com/berlinguyinca/autospec/pull/4783), [#4788](https://github.com/berlinguyinca/autospec/pull/4788) | 15e1cfd, 7b03333 | unmeasured. Verified at d1d890e: `collision::predict` is called by `TaskGraph::schedule` (`initiative/dag.rs`), which `autospec initiative ready` and `initiative::mod` call, and blocks a task with `PredictedCollision` in unit tests. But `Task.affects` defaults to empty and predicts nothing; the only code that sets it is `initiative::compat_dag`, which has no non-test caller. A graph built from a plan therefore never predicts a collision unless an operator hand-writes `affects` into the stored artifact. Placement by arch/toolchain belongs to autospec-orchestrator and is not audited. | [#4796](https://github.com/berlinguyinca/autospec/issues/4796) |
| T14 review→repair→merge flow, conflict + interrupt recovery | autospec, pi-engineering | R-DELIVER | missing | - | - | unmeasured | - |
| T15 post-merge cleanup of owned resources | both | R-RES | missing | - | - | unmeasured | - |
| T16 InferWeave capability mapping + qualified failover, BLOCKED on no capacity | pi-engineering | R-MODEL | missing | - | - | unmeasured | - |
| T17 InferWeave: eligible model/slot, **estimated wait**, route reasons, reasoning flag, usage | inferweave | R-MODEL | missing | - | - | unmeasured | - |
| T18 saturation fault-injection test | pi-engineering | R-MODEL | missing | - | - | unmeasured | - |
| T19 HerdR mission/test views from snapshot+events | herdr-engineering | R-VIEW | missing | - | - | unmeasured | - |
| T20 autospec-db model/token/mission columns; GUI fix or freeze | autospec-db, autospec-gui | R-VIEW | missing | - | - | unmeasured | - |
| T21 orchestrator hardening (non-root, caps, per-worker token) | autospec-orchestrator | R-SEC | missing | - | - | unmeasured | - |
| T22 unify DAG/lease/lifecycle behind adapters + deprecation | autospec | §4.6 | partial | [#4785](https://github.com/berlinguyinca/autospec/pull/4785), [#4789](https://github.com/berlinguyinca/autospec/pull/4789), [#4790](https://github.com/berlinguyinca/autospec/pull/4790), [#4791](https://github.com/berlinguyinca/autospec/pull/4791), [#4793](https://github.com/berlinguyinca/autospec/pull/4793) | 7f5f966, f9fad84, c1df1f0, 0062cb0, d98e0c4 | unmeasured. Verified at d1d890e: the mapping doc, `compat_dag`, `compat_lease`, `compat_lifecycle` and the deprecation notes exist and their unit tests pass. Reachability: unwired by design. `grep -rn 'compat_dag\|compat_lease\|compat_lifecycle' crates` finds only `mod` declarations and doc comments outside the adapters; no caller exists. The deprecation notes are comments, not `#[deprecated]` (stated in the notes), so the old types are unchanged. #4761's acceptance 'all 9 happy WorkState values round-trip' holds only as 'never ahead': 7 of 9 return exactly, `Claimed` returns as `Delivered` and `Reviewed` as `Validated` (the test `seven_happy_states_return_exactly_and_two_share_a_task_state` asserts this). | [#4796](https://github.com/berlinguyinca/autospec/issues/4796), [#4797](https://github.com/berlinguyinca/autospec/issues/4797) |
| T23 benchmarks, chaos run (A1–A9), final report, rollback docs | autospec + pi-engineering | R-DOC | partial | [#4784](https://github.com/berlinguyinca/autospec/pull/4784), [#4786](https://github.com/berlinguyinca/autospec/pull/4786), [#4792](https://github.com/berlinguyinca/autospec/pull/4792) | c1fe715, 121036e, d1d890e | unmeasured. Verified at d1d890e: this report skeleton, `delivery_benchmark` (task set, A1-A9 catalog, baseline-relative p50/p95) and `autospec benchmark delivery --results <PATH>` exist; the CLI imports `summarize` and `SCENARIOS` from the core module and `tests/benchmark_delivery.rs` drives it. Reachable from the CLI. Not done: no benchmark was run, no results file exists, and all of A1-A9 are `not-run`; the chaos run needs pi-engineering and autospec-orchestrator. | [#4798](https://github.com/berlinguyinca/autospec/issues/4798) |
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

Identical task set against the standalone baseline. Task kinds use the wire names of `delivery_benchmark::BenchTask`. Tokens, Retries, Disk and Success each hold `baseline / pi-autospec`; Success is the share of runs that succeeded. `autospec benchmark delivery --results <PATH>` prints these rows. Time is time-to-verified in seconds. Thresholds come from measured baselines, never invented percentages; a task with no successful baseline run has no comparison.

| Task kind | Baseline p50 | Baseline p95 | pi-autospec p50 | pi-autospec p95 | Tokens | Retries | Disk | Success |
|---|---|---|---|---|---|---|---|---|
| trivial-fix | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured |
| medium-feature | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured |
| complex-rust-change | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured |
| multi-repo-contract | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured |
| failing-test | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured |
| gateway-busy | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured |
| stale-ci | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured |
| patch-collision | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured |
| crash | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured |
| long-test | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured | unmeasured |

## Rollback

Each stage rolls back independently. Standalone Pi must keep working at every step (spec R-ROLL, acceptance item 15).

1. Local adapter: disable the AutoSpec adapter in Pi; Pi falls back to its standalone mode with no AutoSpec dependency.
2. Managed backend: stop launching Pi through `harness-pi`; managed missions drain and pause, local execution continues.
3. Automatic classification: turn model-fit auto-classification off; issues keep their last recorded labels and route by those.
4. Back to standalone Pi: remove the adapter configuration and flags; no data migration is required because the protocol is additive and versioned.

## Audit test run

Run at `d1d890e` on 2026-10-16 with load average 25 to 60 from other jobs. No wall-clock timeout was added anywhere.

| Command | Result |
|---|---|
| `cargo test -p autospec-core --no-fail-fast` | exit 0; 240 test binaries, 5779 passed, 0 failed, 1 ignored. Includes the T04, T08, T13, T22 and T23 module and integration tests (`protocol_schemas`, `protocol_fixtures`, `delivery_benchmark`). |
| `cargo test --workspace --exclude autospec-core --no-fail-fast -- --skip executor_bridge` | exit 101; 64 binaries, 3034 passed, 4 failed, 11 ignored. The 4 failures (`claim_stale_heartbeat_recovery`, 3 in `autonomous_conductor_commands`) passed when re-run alone: 1 of 1 and 3 of 3. |
| `cargo test -p autospec-cli --bin autospec -- executor_bridge` | exit 101; 521 passed, 2 failed, 7 ignored. The 2 failures passed alone, 2 of 2. |
| `cargo test -p autospec-cli --bin autospec -- six_thousand_five_hundred` | A separate earlier run failed this test (sqlite `database is locked`); alone it passed, 1 of 1. |
| `cargo test --workspace --no-fail-fast` (first attempt) | Did not finish: three `executor_bridge` tests ran over 30 minutes until the run was stopped by hand. |
| `cargo test -p autospec-cli --test benchmark_delivery`, `--test dispatch_commands`, `--test initiative_commands` | 7, 44 and 16 passed, 0 failed. |

Every test passes at `d1d890e` when run alone, but no single `cargo test --workspace --no-fail-fast` invocation exited 0, so the acceptance item 'workspace tests exit 0' is not claimed. See [#4799](https://github.com/berlinguyinca/autospec/issues/4799). `autospec dispatch runs` on a record with `agent_rc=0` and `session_recorded=false` printed `never-started=1`, and `autospec benchmark delivery --results` printed `A3 pass` and `A1 not-run`.

## Blockers

Audited 2026-10-16 at `d1d890e` (Phase 5.5, #4766). Rows owned by other repositories are not listed.

| Task | Gap | Issue |
|---|---|---|
| T04 | Nothing in this repo writes `session_recorded` or `assistant_turns`, and `dispatch_guard::classify_report` ignores them; `status_triage::triage` has no non-test caller. The writer is autospec-orchestrator T03 (external dependency, not audited here). | [#4794](https://github.com/berlinguyinca/autospec/issues/4794) |
| T08 | The v1 protocol validator has no non-test consumer in autospec. | [#4795](https://github.com/berlinguyinca/autospec/issues/4795) |
| T13 | `Task.affects` is never populated outside tests, so predicted collision never fires on a generated graph. Placement by arch/toolchain is autospec-orchestrator's. | [#4796](https://github.com/berlinguyinca/autospec/issues/4796) |
| T22 | `compat_dag` has no caller (shared with T13, #4796); `compat_lease` and `compat_lifecycle` have no caller and the 9-state round trip is 7 exact. | [#4796](https://github.com/berlinguyinca/autospec/issues/4796), [#4797](https://github.com/berlinguyinca/autospec/issues/4797) |
| T23 | No benchmark was run and A1-A9 are `not-run`. External dependency: the Pi supervisor (pi-engineering) and the managed executor (autospec-orchestrator). | [#4798](https://github.com/berlinguyinca/autospec/issues/4798) |
| tests | No single `cargo test --workspace --no-fail-fast` invocation exited 0 on this loaded machine; see the audit test run. | [#4799](https://github.com/berlinguyinca/autospec/issues/4799) |

