# Conversion-pipeline gate contract

Issue #3748. Rust core: `crates/autospec-core/src/conversion_gate.rs`
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
