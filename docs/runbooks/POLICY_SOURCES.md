# Autospec Policy Sources

See also [Command Index](COMMANDS.md) and [MVP Walkthrough](MVP_WALKTHROUGH.md).

Autospec consumes policy from local Constitution and Baseline repositories. Structured YAML/JSON is the primary source of truth. Markdown doctrine and playbook prose remains the human explanation and is used as a lower-confidence fallback only when structured files are absent.

No GitHub Actions, cron jobs, schedulers, or background automation are used. All commands are operator invoked and local filesystem only.

## Structured Constitution Loading

Autospec discovers:

- `manifests/constitution.yml`
- `manifests/doctrines.yml`
- `manifests/maturity-levels.yml`
- `manifests/categories.yml`
- `rules/*.yml`
- `schemas/*.json`

## Structured Baseline Loading

Autospec discovers:

- `manifests/baselines.yml`
- `manifests/profiles.yml`
- `manifests/pack-categories.yml`
- `packs/**/*.yml`
- `schemas/*.json`

Requested profiles in `.autospec/autospec.yml` expand into packs, inherited packs, required packs, capabilities, rules, quality gates, and issue templates.

## Validation

Run:

```bash
bash scripts/autospec-validate-policy-sources.sh
```

The validation report is written to:

- `.autospec/reports/policy-source-validation.json`
- `.autospec/reports/policy-source-validation.md`

Unsupported check types are reported clearly instead of crashing the audit.

## Lockfile

Run:

```bash
bash scripts/autospec-lock-policy-sources.sh
```

The lockfile is:

- `.autospec/policy-sources.lock.json`

It records manifest/rule/pack hashes and file hashes so future audits can detect policy drift.

## Extraction And Checks

Recommended local sequence:

```bash
bash scripts/autospec-load-policy-sources.sh
bash scripts/autospec-validate-policy-sources.sh
bash scripts/autospec-lock-policy-sources.sh
bash scripts/autospec-build-digital-twin.sh
bash scripts/autospec-baseline-compose.sh
bash scripts/autospec-extract-constitution-rules.sh
bash scripts/autospec-check-rules.sh
bash scripts/autospec-constitutional-gap-v1.sh
bash scripts/autospec-policy-compatibility.sh
```

## Quality Gates

Structured rule and pack `quality_gates` are extracted into:

- `.autospec/state/quality-gates.json`
- `.autospec/reports/quality-gates.json`
- `.autospec/reports/quality-gates.md`

v1 quality-gate checking maps gates to rule check results.

## Issue Plan v3

Failed structured rules generate local drafts under:

- `.autospec/backlog/issues-v3/`
- `.autospec/reports/issue-plan-v3.json`
- `.autospec/reports/issue-plan-v3.md`

Publish them only through explicit operator commands:

```bash
bash scripts/autospec-audit-to-backlog.sh --dry-run
bash scripts/autospec-publish-issues.sh --dry-run --plan v3
bash scripts/autospec-publish-issues.sh --confirm --plan v3
```

V3 issue bodies include idempotency markers for plan version, local issue ID,
rule ID(s), rule-result hash, source-gap hash, and body hash. The published
ledger stores rule IDs, quality gates, source policy files, maturity, category,
and severity so supervisor, worker, verifier, and promotion reports can explain
each autonomous action.

Older v1/v2 plans remain readable. When no `--plan` is supplied, publishing
prefers v3, then v2, then v1.

## Supervisor And Verification Traceability

Structured v3 issues are the preferred autonomy source. Supervisor dry-runs show
the selected source rule(s), baseline pack, rule severity, maturity target,
quality gates, worker eligibility, and expected validation. Worker packets carry
the same structured policy context. Verifier reports add policy traceability,
rule compliance, quality gate review, and maturity impact sections; promotion
requires those checks for v3 issues.

## Compatibility Report

Run:

```bash
bash scripts/autospec-policy-compatibility.sh
```

The report lists unsupported check types, schema mismatches, pack composition issues, rule conflicts, and recommended engine follow-up issues.
