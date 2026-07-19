# Test-quality workstream runbook

`scripts/test-quality-workstream.sh` is the deterministic control surface for the
continuous test-quality workstream introduced by issue #1534. It complements the
existing mutation runner by recording metrics over time and converting concrete
quality gaps into autospec implementation work.

## Metric ledger and gate

Record per-crate coverage, mutation score, and flake counts as JSONL:

```bash
bash scripts/test-quality-workstream.sh record-metric \
  --ledger .autospec/test-quality/metrics.jsonl \
  --crate autospec-core \
  --coverage 91 \
  --mutation 86 \
  --flakes 0
```

Gate the latest reading for each crate against floors and previous readings:

```bash
bash scripts/test-quality-workstream.sh gate \
  --ledger .autospec/test-quality/metrics.jsonl \
  --min-coverage 90 \
  --min-mutation 80 \
  --max-flake-rate 0
```

The gate fails on coverage regressions, mutation-score regressions, or flake-rate
regressions, and it emits stable `RULE:crate:detail` lines for automation.

## Surviving mutants

Convert survivor JSONL from a mutation run into ready-to-file `auto-implement`
issue bodies:

```bash
bash scripts/test-quality-workstream.sh propose-mutant-issue \
  --mutants .autospec/test-quality/survivors.jsonl \
  --out .autospec/test-quality/issues
```

Each generated issue includes the survivor description plus a red/green command:
the new test must fail while the mutant is present and pass after the intended
implementation is restored.

## Flake quarantine

Quarantine a nondeterministic test and emit a hardening issue:

```bash
bash scripts/test-quality-workstream.sh quarantine-flake \
  --ledger .autospec/test-quality/metrics.jsonl \
  --quarantine .autospec/test-quality/quarantine.jsonl \
  --issues-dir .autospec/test-quality/issues \
  --crate autospec-cli \
  --test tests::sometimes_times_out \
  --reason 'failed 2/5 retries'
```

The generated issue requires reproducing the flake and fixing nondeterminism
without weakening assertions.

## Read-only test-file enforcement

Before dispatching an implementer into a protected tree, lock test paths:

```bash
bash scripts/test-quality-workstream.sh lock-tests --repo-root . --paths tests
bash scripts/test-quality-workstream.sh check-readonly --repo-root . --paths tests
```

`check-readonly` fails with `TEST_FILE_WRITABLE:<path>` for any writable test
file, making assertion-gutting visible before implementation proceeds.
