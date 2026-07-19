# Performance and benchmark-regression workstream runbook

`scripts/performance-workstream.sh` is the deterministic control surface for the
continuous performance workstream introduced by issue #1536. It records
per-commit benchmark baselines, gates statistically significant regressions,
emits `auto-implement` issue bodies for offending metrics, and writes the
before/after benchmark report required for optimization PRs.

## Record benchmark baselines

```bash
bash scripts/performance-workstream.sh record-benchmark \
  --ledger .autospec/benchmarks/performance.jsonl \
  --benchmark execution_fast_path \
  --commit "$(git rev-parse --short HEAD)" \
  --p50-ms 40 --p99-ms 45 --allocations 100 --samples 30 --stddev-ms 1
```

Rows are JSONL so CI can store one p50/p99/allocation record per benchmark and
commit without requiring a database.

## Gate a candidate commit

```bash
bash scripts/performance-workstream.sh gate \
  --ledger .autospec/benchmarks/performance.jsonl \
  --baseline-commit BASE_SHA \
  --candidate-commit CANDIDATE_SHA \
  --max-regression-pct 10 \
  --min-z-score 2 \
  --regressions-out .autospec/benchmarks/regressions.jsonl
```

The gate compares candidate p50, p99, and allocation counts against the baseline.
p99 regressions must exceed both the percentage threshold and the z-score
threshold before they block, which avoids filing noise for statistically weak
wall-clock drift. The `execution_fast_path` benchmark also carries the shared
`<50ms` fast-path guard.

## File regression issues

```bash
bash scripts/performance-workstream.sh propose-regression-issue \
  --regressions .autospec/benchmarks/regressions.jsonl \
  --out .autospec/benchmarks/issues
```

Generated issues include `auto-implement`, the offending metric, baseline and
candidate values, and a primary smoke-test command that must fail before the fix
and pass after the optimization.

## Attach an optimization report to PRs

```bash
bash scripts/performance-workstream.sh optimization-report \
  --before /tmp/bench-before.jsonl \
  --after /tmp/bench-after.jsonl \
  --out reports/performance-before-after.md \
  --max-regression-pct 10
```

The report is refused if any other benchmark regresses beyond the threshold, so
optimization PRs carry a reproducible before/after delta without collateral
fitness-function regressions.
