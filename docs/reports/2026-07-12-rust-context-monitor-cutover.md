# Rust Runtime Cutover Evidence

**Date:** 2026-07-12
**Decision:** do not cut over the context-monitor driver or remove any wrapper fallback.

## Context monitor comparison

The Rust implementation is a pure threshold classifier at
`crates/autospec-core/src/context/mod.rs`. It matches the valid-input Python engine
transitions: 50% requests `compact`, a later 80% reading requests `handoff`, `clear`,
and `resume`, and a reading below 30% resets state. The focused evidence is:

| Surface | Evidence | Observed result |
| --- | --- | --- |
| Rust state machine | `cargo test -p autospec-core --test context_monitor` | 4/4 passing; test runner reported under 0.01s |
| Python state machine | `python3 -m pytest packages/autospec_context_monitor/tests/engine/test_classify_sequence.py packages/autospec_context_monitor/tests/engine/test_compacted_normal_reset.py packages/autospec_context_monitor/tests/engine/test_rolled_reset.py -q` | 7/7 passing; test runner reported 0.02s |
| Installation/driver scope | Tracked Python package inventory (`git ls-files packages/autospec_context_monitor`) | 44 package files, including adapters, hook integration, injection, handoff validation, telemetry, and daemon lifecycle |
| Rust driver scope | Rust source inventory | one pure engine module; no adapter, process, injection, telemetry, or installer implementation |

The test durations are only state-machine test-runner measurements, not production latency
claims. Rust deliberately turns zero maximum tokens into 0%; the Python engine instead relies
on a valid positive maximum-token input. A future driver must either preserve that input
precondition or add a separately specified error contract.

## Decision and escape hatch

Keep the Python monitor as the production driver. The Rust engine is a verified reusable
classifier, but it cannot replace the Python package without equivalent adapters, transcript
handling, hook installation, handoff validation, telemetry, and process supervision.

No `AUTOSPEC_FORCE_PYTHON_CONTEXT_MONITOR` flag is introduced because no Rust driver is being
enabled. If a future release introduces one, it must default to the existing Python driver for
one release and provide that exact force-Python escape hatch before any default flip.

## R1 validation fallback ledger

| Fallback | Fixture/shadow proof | Current escape hatch | Removal issue | Delegation result |
| --- | --- | --- | --- | --- |
| `scripts/validate.sh` legacy executor | `legacy-fast-passed.json` and `legacy-required-failed.json`; Rust shadow aggregation is covered by `validate_shadow_results_*` CLI tests | `AUTOSPEC_FORCE_LEGACY_SHELL=1` | #1861 remains open | Rust parses and aggregates only captured results; all real validation still re-enters the shell |

For execution-capable options, the current wrapper topology is `bash scripts/validate.sh` →
`autospec validate` → `bash scripts/validate.sh` with `AUTOSPEC_FORCE_LEGACY_SHELL=1`.
`--shadow-results` is intentionally exempt from that re-entry: it aggregates a pre-captured file
without invoking the shell. The execution path deliberately has no direct-Rust baseline yet:
process count, elapsed time, and output-byte comparisons are **not delegation-ready** until the
same real validation checks have a Rust executor. Before a default delegation change, capture
those three metrics in CI for the shell and Rust executors over the same fixture corpus, retain
the one-release escape hatch, and record the removal issue here.

## Cutover gates

- Maintain fixture parity for the Python/Rust state machine, including invalid-input behavior.
- Port or explicitly retain every Python driver capability listed above.
- Capture production-shaped installation, process-count, latency, and handoff-success metrics.
- Run an observation release with the Python default and the force-Python escape hatch.
- Do not remove the validation wrapper fallback until the R1 ledger has a direct-executor
  measurement and a completed removal issue.
