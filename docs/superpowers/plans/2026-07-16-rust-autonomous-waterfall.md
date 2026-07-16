# Rust Autonomous Waterfall Implementation Plan

**Goal:** Complete #1872 with a native, evidence-backed idle-rescan waterfall
and bounded local ideation, then make it eligible for #2076 legacy deletion.

**Global constraints:** Every tier is typed and persisted; only a completed
five-tier pass can be dry; all failure/unavailability remains visible; no shell
waterfall fallback; no automatic GitHub mutation; local ideation has at most
five scored planning/review candidates; every source file remains under 500
LOC; every task has focused tests plus formatting, clippy, and relevant CLI
integration proof.

## Task 1: Pure no-work state foundation — complete

**Committed:** `fd6d0ddd`, `44a17759`; reviewed clean.

- [x] Closed five-tier policy, contiguous/idempotent state, bounded dry history.
- [x] Strict schema-1 codec, sealed derived evidence references, threshold-two request.
- [x] Fail-closed persisted-history overflow regression.

## Task 2: Typed waterfall receipt and locked persistence

**Files:** Add `crates/autospec-core/src/autonomous/waterfall.rs` and its
private codec; add core tests; add CLI persistence module and integration tests.

- [ ] Write RED tests for exact receipt scope/tier/pass validation, digest/path
  tampering, concurrent cursor ownership, atomic replacement, and failed/
  not-run preservation.
- [ ] Implement pure `TierReceipt`, `TierStatus`, funnel counts, sealed
  evidence references, and strict schema-1 parsing.
- [ ] Implement the CLI-scoped lock and atomic `waterfall-state.json`/
  receipt persistence; lock contention must return a typed held result.
- [ ] Verify focused core and CLI tests, then commit with Lore trailers.

## Task 3: Run Tier 1 through the native coordinator

**Files:** Add waterfall coordinator module; modify foreground dispatch and
conductor tests.

- [ ] Start a pass only after the current Rust ready queue is empty and a
  conductor lease is held.
- [ ] Persist a Tier-1 receipt from typed queue evidence. Queue read errors are
  `failed`; an empty page is the appropriate exhausted observation.
- [ ] Resume deterministically after an interrupted pass; never run a second
  independent loop or change claim ownership.
- [ ] Prove foreground performs no shell waterfall invocation and never writes
  `why-no-work.json` while any later tier is `not_run`.

## Task 4: Port Tier 1.5 native promotion/grooming observation

- [ ] Implement read-only issue enumeration, closed eligibility/classification
  evidence, deduplication, and governance decisions in Rust.
- [ ] Preserve every skip/hold/quarantine/routing reason in a Tier-1.5 receipt.
- [ ] Keep promotion/body/label mutation outside this observer; a produced
  candidate returns to normal Rust queue admission.
- [ ] Test malformed/missing GitHub data as `failed`, never dry.

## Task 5: Port Tier 2 local discovery funnel

- [ ] Implement a deterministic local-signal collection adapter using existing
  Rust specialist evidence.
- [ ] Add typed proposal, deduplication, adversarial verification, ROI, and
  rank receipts; persist each input/output reference.
- [ ] Separate proposal production from any GitHub filing and prove produced
  proposals do not receive `auto-implement` labels.
- [ ] Treat every incomplete stage as `failed` or `not_run`, never dry.

## Task 6: Port Tier 3 architecture/debt producer

- [ ] Define portable architecture, test-coverage, and debt evidence adapters
  based on checked-in project metadata rather than a shell self-improvement loop.
- [ ] Persist deterministic rule versions and evidence; failures remain typed.
- [ ] Integrate output into the common TierReceipt funnel and test Rust-only
  authority boundaries.

## Task 7: Port Tier 4 explicit external-source producer

- [ ] Extend typed config with source allowlists and limits; parse it strictly.
- [ ] Implement direct, bounded source retrieval with byte/time caps and
  untrusted evidence storage; no shell or branch/issue mutation.
- [ ] Run the same typed funnel as Tier 2; disabled policy is `not_run`, a
  source error is `failed`, and an exhausted configured source is dry.

## Task 8: Complete pass recording and local ideation

- [ ] Write `why-no-work.json` only after the five verified receipt outcomes
  form a complete dry pass.
- [ ] Edge-trigger `autospec autonomous ideate` from exactly two source pass
  IDs; it is idempotent for that pair.
- [ ] Validate bounded scored model output against the sealed package, write
  local JSON and Markdown, and prohibit all GitHub/work-dispatch mutation.
- [ ] Test a malformed/unavailable producer result, deterministic sorting,
  repeat invocation, and static/PATH authority guards.

## Task 9: Final authority audit and legacy deletion handoff

- [ ] Run workspace formatting, clippy, tests, fast validation, and
  no-shell/no-legacy source scans.
- [ ] Compare every legacy waterfall tier and failure branch to a native
  receipt/producer test. File any remaining non-parity gap instead of deleting.
- [ ] Only after parity review, remove legacy waterfall ownership as part of
  #2076 and rerun its full Rust-cutover gate suite.
