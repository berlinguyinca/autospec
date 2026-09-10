# Conversion-pipeline gate contract

Issues #3748 and #3747. Rust core: `crates/autospec-core/src/conversion_gate.rs`
(module `autospec_core::conversion_gate`); the run-level judge in
`crates/autospec-core/src/verification/mod.rs` consumes the same classifiers.

Agent-generated patches pass through a build gate before they may be emitted
as **conversion candidates**: the gate runs a build stage, a test stage, and a
fmt check, records each stage's exit code in a status file, and the status
file's classification decides whether the patch enters the **conversion queue**.

## The incident

A gate whose build stage was `cargo build` reported `build_rc=0` while the
test stage failed to compile the patch's tests (rustc errors such as
`cannot find function ... in this scope`). The classifier saw a green build
and classified the run `UNKNOWN-NO-BASELINE` — a status that asserts *no
baseline exists* — even though the actual evidence was that the tests do not
compile at all. The patch was emitted as a conversion candidate on the
strength of a build stage that had never looked at the test targets.

## The contract

### 1. A gate's scope, not just its outcome, is part of its contract

`cargo build` compiles lib and bins; it does **not** compile test targets, so
its exit code is no evidence about the tests. The gate's build stage is
therefore:

```text
cargo check --all-targets
```

`cargo check --all-targets` compiles every target the test stage will build —
lib, bins, tests, benches, examples — without linking, so a green exit code is
genuine evidence that the patch's tests compile. `gate_scope_violation`
rejects commands whose scope is narrower than that (`cargo build`, bare
`cargo check`); `cargo test ...` is accepted because it compiles all targets.

### 2. `TESTS-DO-NOT-COMPILE` is a terminal status of its own

Statuses written to the status file (`GateStatus`):

| Status | Meaning | Terminal | Admits to queue |
|---|---|---|---|
| `PASS` | Build and test stages green | no | yes |
| `TESTS-DO-NOT-COMPILE` | Test stage failed to build | **yes** | no |
| `BUILD-FAILED` | Build stage failed | **yes** | no |
| `NO-TEST-DB` | Declared test database unreachable; test stage's runtime results void | **yes** | no |
| `NEW-TEST-FAILURES` | Tests built; ran; failed; baseline attributes the failures | no | no |
| `UNKNOWN-NO-BASELINE` | Tests built; ran; failed; no baseline to attribute them | no | no |

Classification precedence (most specific evidence first):

1. `test_build_failed` → `TESTS-DO-NOT-COMPILE`. **Unconditional**: it holds
   regardless of `build_rc`, `test_rc`, or `has_baseline`. A baseline can
   justify (attribute) a test failure, but no baseline can justify a test that
   does not compile.
2. `build_rc != 0` → `BUILD-FAILED`.
3. `!test_db_reachable` → `NO-TEST-DB`. The declared test-database dependency
   was unreachable, so the test stage never ran against a live database and
   its runtime exit code is void. A **non-result**, not a regression: the
   failures are a property of the node, not of the patch (issue #3725). It is
   terminal — re-running the test stage on the same node or consulting a
   baseline cannot demote it.
4. `test_rc != 0` → `NEW-TEST-FAILURES` when a baseline exists,
   `UNKNOWN-NO-BASELINE` when it does not. `UNKNOWN-NO-BASELINE` therefore
   holds **only when the test stage actually built**.
5. otherwise → `PASS`.

Only `PASS` admits a patch to the conversion queue; in particular a patch
whose tests do not compile never reaches it.

### 3. Contradictory signals are flagged at write time

`build_rc=0` claims the build stage verified the code it was scoped to;
`test_build_failed` records that the test stage could not build the same
code. Both are true only if the build stage never saw the test targets — the
signature of the incident above. `render_status_file` / `write_status_file`
mark such files with a `contradiction=build_ok_but_tests_do_not_compile`
token **at write time**, and `parse_status_file` re-derives the flag from the
recorded signals for files written before the token existed.

Status file format:

```text
status=<STATUS> node=<NODE> build_rc=<n> test_rc=<n> fmt_rc=<n>[ contradiction=<reason>]
```

`fmt_rc` is recorded as evidence; fmt is gated separately from build/test
admission. `node` records the scheduler-assigned node the run happened on, so
a reader comparing two runs' failures can see whether they ran in the same
world; it is always written, but optional when parsing files written before it
existed (they parse with an empty node).

## #3747: a hold must say which stage failed, and which tests

The status *classification* above was correct about the incident. The **hold
reason** emitted for the same class of runs was not, and a hold reason is what
the next session reads. Four defects, four rules.

### 4. Exit codes decide the stage; the output only names it

The gate classified a run by grepping `^error:` and calling any match a build
failure. Cargo prints `error:` for compile failures **and** for test failures
(`error: test failed, to rerun pass …`, `error: N targets failed`), so a patch
that compiled cleanly and failed tests was held as a *build error*. A bare
`error:` prefix is not a classifier: it matches three different events and
names none of them.

The stage comes from the exit codes, which cargo does not confuse:

```text
failed_stage(build_rc, test_rc) ->  Some(Build) | Some(Tests) | None
```

`build_rc != 0` is a build failure; `build_rc == 0 && test_rc != 0` is a test
failure; both zero is no failure. Output text may **name** what the exit codes
established and may never flip a stage they established: a line matching
`^error[E…` seen with `build_rc=0` produces no build hold.

The two output shapes are matched separately, by the parts that distinguish
them (`compile_failure_line` / `test_failure_line`):

| Shape | Matches | Does not match |
|---|---|---|
| compile failure | `error[E0425]: …`, `error: could not compile …` | `error: test failed`, a test's own `error:` log line |
| test failure | `error: test failed …`, `error: 2 targets failed` | `error[E0425]`, `error: could not compile` |

`output_reports_compile_failure` / `output_reports_test_failure` apply those
patterns per line, and `verification::reports_hard_error` — the check that
keeps a target which never built from reading as "no tests in scope" — is now
their disjunction instead of the bare prefix. A passing test that prints
`error: connection refused` about a socket it closed no longer fails the run.

### 5. Every test hold names its failing tests

One code path recorded failing test *names*; the other recorded a count
(`HELD: tests failed -- 2`). A count cannot be compared against a baseline,
cannot be re-run, and cannot be read by the next session, so both paths now
carry names, collected by one parser from the two places cargo prints them
(the `test <path> ... FAILED` progress lines and the `failures:` block that
follows them):

```text
failing_test_names(output) -> Vec<String>   // deduplicated, sorted
```

`classify_hold(build_rc, test_rc, output)` returns a `GateHold` whose rendered
line is the hold record:

```text
HELD: build error -- error[E0425]: cannot find function `x` in this scope
HELD: test failure -- 2 failing tests: a::one, b::two
HELD: tests do not compile -- error: could not compile `crate` (test "it")
```

A test hold line never contains the word *build*, and a test hold with no
names in the output says so explicitly —
`HELD: test failure -- failing test names unavailable (test_rc=101)` — rather
than degrading to a count or borrowing the build wording. `hold_for_run` takes
the `GateRun` itself, where `test_build_failed` (#3748) outranks the exit
codes.

The other hold path, `autonomous::test_gate`'s `GateVerdict::message()`, named
only its *attribution* (`HELD: tests failed -- caused`). An attribution is not
a name either, so it now appends the persistent failures:

```text
HELD: tests failed -- caused -- 2 failing: a::one, b::two
```

Tests that failed in the suite and passed on their isolated re-run are flaky,
are reported by `GateVerdict::flaky`, and are deliberately **not** named in
the hold line — naming them would assert a failure the gate just retracted.

### 6. The test gate compares names against the baseline, not a count against zero

The test gate asked "is the failing count greater than zero?". The validate
gate (#3715, #3727) asks "which failures were not in the baseline?", and only
that question distinguishes a regression from a known-broken trunk:

```text
compare_tests(failing, baseline) -> Passed | PreExisting | NewFailures | NoBaseline
```

| Baseline | Failing | Result | Holds |
|---|---|---|---|
| any | none | `Passed` | no |
| `Some(set)`, superset of failing | subset | `PreExisting` | no |
| `Some(set)`, any name not in it | overlap allowed | `NewFailures` | **yes** |
| `None` (no comparison performed) | any | `NoBaseline` | **yes** (fail-closed) |

Two consequences the count comparison got wrong, both pinned by tests:

- **Equal count, different set is a regression.** Baseline `[a, b]`, failing
  `[b, c]`: the count says "nothing changed", the set difference names `c`
  and holds.
- **An empty baseline is not the absence of one.** `Some(&[])` is a baseline
  that was measured and found nothing failing, so any failure is new and
  holds. Only `None` means no comparison was performed.

### 7. The base sha a hold records is the one the tests ran against

A base sha read *before* the per-patch `git fetch origin/main` is one merge
behind by construction: the hold names a commit the patch was never tested
on, and the reader cannot tell. `BaseRevision` records the fetch as part of
the capture instead of hoping the caller ordered it right:

```text
BaseRevision::capture(sha, fetch_completed) -> Current | PreFetch | Unknown
```

- `Current` — captured after the fetch; `tested_base()` returns the sha.
- `PreFetch` — captured before it; `tested_base()` returns `None` and the log
  field reads `base=unrecorded pre_fetch_sha=<sha>`, so the value survives as
  evidence without being asserted as the base.
- `Unknown` — nothing captured; the field reads `base=unknown`.

`hold_log_line(&hold, &base)` composes the record, e.g.

```text
HELD: test failure -- 2 failing tests: a::one, b::two base=3f2a1c9
```

The same discipline already governs the schedule itself
(`execution::patch_pipeline::plan_pass`, #3698: a verdict computed against a
base that is no longer the trunk tip is a hypothesis, never a memo hit); this
closes the reporting side of it.
