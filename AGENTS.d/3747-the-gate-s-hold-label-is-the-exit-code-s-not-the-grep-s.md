# The gate's hold label is the exit code's, not the grep's (issue #3747)

The conversion pass held a patch that compiled fine as
`HELD: build error -- error: test failed, to rerun pass
-p autospec-core --test validation_runner`, because the gate
classified by grepping the combined output:

```
if printf '%s' "$out" | grep -qE '^error(\[|:)'; then
  log "HELD: build error -- ..."
```

`cargo` prints `error:` for several unrelated conditions — `error[E0308]`
and `error: could not compile` for a real compile failure, and
`error: test failed, to rerun pass ...` / `error: N targets failed` for
perfectly-compiled code whose tests did not pass. The build check ran
before the test check, so it won. Measured across a full session: 49
"build error" holds, 38 of them (78%) unambiguous test failures — the
largest hold category, mostly wrong. The HELD reason is the only
artefact a human reads when deciding what to do with a held patch, and
the two labels call for opposite responses: broken code is re-dispatch
material, a test disagreement may be the patch being right. When two
conditions share a prefix, a prefix match picks the one you tested
first, silently, forever.

- **Classify by structured outcome, not by scraping.** The exit codes
  decide: `cargo build`'s status for "does it compile", `cargo test`'s
  status for "do the tests pass" (`failure_kind_from_exit_codes`
  delegates to the pipeline's own `classify_gate` — one classification
  for one fact, two labelings). The exit code also covers the case a
  text match cannot: a genuine compile failure like an unclosed
  delimiter matches neither discriminating pattern, and is still a
  compile failure.
- **Any remaining text match anchors on the discriminator, and
  unclassified is recorded, not guessed.**
  `^error(\[[A-Z]|: could not compile)` says compile;
  `^error: (test failed|[0-9]+ targets? failed)` says tests ran and
  failed (`is_compile_error_line`, `is_test_failure_line`,
  `classify_output`). The two patterns are disjoint, so a match is a
  match; output matching neither is `None`.
- **The evidence recorded must not depend on which route produced the
  verdict.** The second instance recorded `HELD: 1 failing
  (-p autospec-cli) -- 2720 passed; 1 failed across 43 targets` — one
  failure in 2721, and the record does not say which. Every test hold
  names the failing tests parsed from the output
  (`failing_test_names`, `HoldReason`): a hold reached by the aggregate
  count cannot be unnamed, and a test hold with no names to parse says
  `unnamed` rather than staying silent about it.
- **Compare failing-test names against the recorded baseline, not a
  count against zero.** A single pre-existing failure on `main` holds
  every patch in the backlog under a count compared to zero, and the
  hold says only "1 failing" — indistinguishable from a real
  regression. By name, a pre-existing failure is reported as
  pre-existing and a new failure is held by name
  (`TestBaseline`, `compare_to_baseline`), the way the validate gate
  compares against its baseline.
- **The recorded base sha is the sha the branch was created from.** The
  branch is cut from `origin/main` after the per-patch fetch; a sha
  captured before the fetch is one merge behind and reads as fact
  (every line read `[base=bfc62571]` while `origin/main` had advanced
  to `1d159336`). Capture it after the fetch, or drop it — a stale
  recorded sha is a finding (`BaseSha::is_stale`,
  `BaseSha::stale_finding`).

Checkable in `autospec_core::gate_hold` (`failure_kind_from_exit_codes`,
`is_compile_error_line`, `is_test_failure_line`, `classify_output`,
`first_compile_error`, `failing_test_names`, `HoldReason`,
`hold_reason_from_output`, `TestBaseline`, `compare_to_baseline`,
`BaselineVerdict`, `BaseSha` — pure in-memory, so the shell
`convpass.sh` gate can adopt them as the single source of truth).
Tests: `crates/autospec-core/tests/gate_hold.rs`, including the
regression that reconstructs the incident end-to-end: the 38 mislabeled
rerun-hint lines classified as tests, the 7 genuine compile failures
keeping their label by exit code, the unnamed hold that now names
`managed_project::it_works`, and the pre-fetch sha one merge behind.
