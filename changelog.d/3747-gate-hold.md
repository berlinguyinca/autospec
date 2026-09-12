### Added

- `autospec-core::gate_hold` — primitives for the invariant that the
  conversion pass's hold label is the exit code's, not the grep's: the
  classification is the structured outcome (`failure_kind_from_exit_codes`
  delegating to the pipeline's `classify_gate`), any remaining text match
  anchors on what distinguishes the cases — `^error(\[[A-Z]|: could not
  compile)` for a compile failure, `^error: (test failed|[0-9]+ targets?
  failed)` for tests ran and failed (`is_compile_error_line`,
  `is_test_failure_line`, `classify_output` — disjoint patterns, output
  matching neither records unclassified, never guesses) — every route to a
  test hold records the same evidence, the failing test names parsed from
  cargo's `failures:` summary (`failing_test_names`, `HoldReason`, an
  unparseable hold says `unnamed` rather than staying silent), the test
  gate compares failing-test names against the recorded baseline instead
  of a count against zero (`TestBaseline`, `compare_to_baseline`), and the
  base sha a log line records is the sha the branch was actually created
  from, captured after the per-patch fetch or dropped (`BaseSha`) (#3747,
  2026-09-11).
