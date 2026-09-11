# Conversion-pipeline gate contract

Issues #3748 and #3798. Rust core: `crates/autospec-core/src/conversion_gate.rs`
(module `autospec_core::conversion_gate`).

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
| `PASS` | Build and test stages green; a baseline existed and was compared | no | yes |
| `VERIFIED-ABSOLUTE` | Every gate ran and returned zero; no baseline to compare against | no | yes |
| `TESTS-DO-NOT-COMPILE` | Test stage failed to build | **yes** | no |
| `BUILD-FAILED` | Build stage failed | **yes** | no |
| `NO-TEST-DB` | Declared test database unreachable; test stage's runtime results void | **yes** | no |
| `NEW-TEST-FAILURES` | Tests built; ran; failed; baseline attributes the failures | no | no |
| `UNKNOWN-NO-BASELINE` | Tests built; ran; failed; no baseline to attribute them — a harness fault (missing baseline preparation), not a property of the patch | no | no |

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
   holds **only when the test stage actually built and failed**.
5. otherwise → every gate ran and returned zero: `PASS` when a baseline
   existed and was compared, `VERIFIED-ABSOLUTE` when it did not (issue
   #3798).

Only `PASS` and `VERIFIED-ABSOLUTE` admit a patch to the conversion queue;
in particular a patch whose tests do not compile never reaches it.

### The #3798 split: absolute green is not "unknown"

A baseline answers the weaker question — did the patch make things *worse*?
— and a run whose gates all ran and returned zero does not need it. Before
this split, one label covered two incompatible situations: *we could not
measure the code* and *we measured everything; we could not compare the
delta*. Collapsing the second into the first threw away a stronger result
because a weaker one was unavailable, and it stalled the serial chain behind
a fully green patch while every loop logged a healthy line.

- `VERIFIED-ABSOLUTE` records absolute green: every gate emitted positive
  evidence of having run and returned zero, with no baseline to compare
  against. It admits to the conversion queue exactly as `PASS` does, so a
  green patch with no baseline reaches the conversion pass without human
  intervention.
- `UNKNOWN-NO-BASELINE` is reserved for a test stage that genuinely failed
  at run time with no baseline to attribute the failures to. It reads as a
  report that the **harness** did not prepare a baseline — a fault in
  preparation, not a judgement about the work — so it is actionable by the
  right party.
- The absolute-green shortcut is the rare path, not a workaround: the
  remedy for a routinely missing baseline is to capture it as part of
  preparing the run (the base commit is already checked out; measuring it
  is the same command the patch is measured with).

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

### 4. A green run labelled `UNKNOWN-NO-BASELINE` is re-derived at read time

Files written by classifiers before the split (or by a mislabelled one) can
claim `status=UNKNOWN-NO-BASELINE` while recording `build_rc=0 test_rc=0
fmt_rc=0` — the exact shape of the #3798 incident, where two fully green
patches sat invisible to every loop because the one field every downstream
consumer reads asserted *unknown*. The recorded rcs are positive evidence
that every gate ran and passed; the status line is the one wrong field.

`parse_status_file` keeps the written status, and
`StatusFile::is_mislabeled_no_baseline` flags the case (status is
`UNKNOWN-NO-BASELINE` while all of `build_rc`, `test_rc`, and `fmt_rc` are
zero). `StatusFile::effective_status` returns the status the recorded
evidence actually supports — `VERIFIED-ABSOLUTE` for a mislabeled green
file, the written status for everything else — so consumers that check
`effective_status().admits_to_conversion_queue()` admit the green artifact
instead of holding it. A file with any nonzero recorded gate keeps
`UNKNOWN-NO-BASELINE`: a recorded negative is evidence, not a missing
comparison. Together the two mechanisms keep a finished artifact from being
invisible to every consumer at once: the conversion pass reads the
re-derived status, and the dispatch guard (#3764) names the artifact by
path whenever it refuses to destroy it.

### 5. The known-failing set is a file, not a tolerated number

`UNKNOWN-NO-BASELINE` names a *missing* baseline. A baseline that exists but
lives only as an operator's expectation — "this suite always fails two tests" —
is the same fault wearing a green label: the gate cannot tell a tolerated
pre-existing failure from a new one, so every failure is tolerated. Issue #4291
found `crates/autospec-cli --test autonomous_conductor_commands` in exactly that
state: two tests red on `main`, no record of which two.

The baseline is therefore a committed file,
`autospec/baseline-failures.txt` (override with `AUTOSPEC_TEST_FAILURE_BASELINE`),
one entry per known-failing test:

```text
<cargo test id>\tissue=#<N>[\tbinary=<test target>]
```

sorted with `LC_ALL=C`, no duplicate ids, `#` for comments. The gate is
`scripts/test-failures-baseline.sh`:

| Mode | Behaviour |
|---|---|
| `--check` (default) | run the targets (or parse `--run-log`), lint the file, block on any live failure absent from it |
| `--lint-baseline-only` | validate the file's syntax without running cargo |
| `--rebaseline` | rewrite the file **downward only**: drop entries proven to pass, never add one |

Three rules keep it a baseline instead of a list of excuses:

- **Every entry carries `issue=#<N>`.** A known failure with no linked issue is
  unowned: nothing schedules its removal, so it keeps tolerating whatever else
  fails under that name. Growth is refused twice — `--rebaseline` exits 1 on an
  unlisted failing id (`REBASELINE_REFUSED_UPWARD`) rather than absorbing it, and
  a hand-added entry without the column is rejected (`BASELINE_ENTRY_NO_ISSUE`).
- **An entry whose test now passes is blocking** (`STALE_BASELINE_ENTRY`), not a
  note. A baselined id suppresses the next regression of the same test, so a
  fix that leaves the entry behind re-arms the bug.
- **An entry that was merely not executed warns and stays.** Silence is not
  evidence of a fix; only an observed pass drops an entry.

A harness that never ran is `HARNESS_NEVER_RAN`, a compile error is
`TESTS_DO_NOT_COMPILE` (§2), and failures the log declares but never names are
`FAILURE_ATTRIBUTION` — all exit 1, none of them a pass, none of them
`VERIFIED-ABSOLUTE`. Exit codes: 0 = live failures are exactly the baselined
ones, 1 = at least one blocking finding, 2 = usage or environment error.

Tests: `tests/test-failures-baseline.bats`, with the `main` run that produced
the two entries kept at
`tests/fixtures/test-failures-baseline/main-baseline.log`.
