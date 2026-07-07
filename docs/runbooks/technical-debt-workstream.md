# Technical-debt, dead-code, and CVE workstream runbook

`scripts/technical-debt-workstream.sh` is the deterministic control surface for
the continuous debt/dead-code/dependency-freshness workstream introduced by issue
#1535. It turns analyzer output into ranked, lint-clean `auto-implement` issue
bodies; it does not delete code or upgrade dependencies by itself.

## Hotspot ranking

Rank files by churn×complexity, with duplicate-line evidence retained as a
secondary signal:

```bash
bash scripts/technical-debt-workstream.sh rank-hotspots \
  --churn .autospec/debt/churn.jsonl \
  --complexity .autospec/debt/complexity.jsonl \
  --duplicates .autospec/debt/duplicates.jsonl \
  --out .autospec/debt/hotspots.jsonl
```

Then file the highest-ROI refactor issue:

```bash
bash scripts/technical-debt-workstream.sh propose-refactor-issue \
  --hotspots .autospec/debt/hotspots.jsonl \
  --out .autospec/debt/issues \
  --test-cmd 'bash scripts/validate.sh'
```

## Dead-code removal proposals

Feed unreferenced-symbol output from `/find-dead-code`, LSP find-references,
coverage-guided analysis, or `cargo-udeps` into the helper:

```bash
bash scripts/technical-debt-workstream.sh propose-dead-code-removal \
  --symbols .autospec/debt/dead-code.jsonl \
  --out .autospec/debt/issues \
  --test-cmd 'bash scripts/validate.sh'
```

Symbols referenced only from `tests/` are treated as dead. The generated issue
requires a verified removal PR and a green suite before merge.

## Dependency freshness and CVE cadence

Normalize `cargo audit`, `cargo-deny`, OSV, or Trivy results into JSONL and
record each scan in a cadence ledger:

```bash
bash scripts/technical-debt-workstream.sh scan-advisories \
  --advisories .autospec/debt/advisories.jsonl \
  --ledger .autospec/debt/advisory-scans.jsonl \
  --out .autospec/debt/issues
```

Active, fixable CVEs with the highest CVSS score are emitted first as `p1-*`
patch issues. Each generated issue requires the full validation suite or an
explicitly supplied fitness command.
