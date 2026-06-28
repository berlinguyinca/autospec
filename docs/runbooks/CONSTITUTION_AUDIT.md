# Autospec Constitution Audit

The Constitution audit converts local Constitution doctrine and Baseline pack
rules into machine-checkable rules, evaluates those rules against the Digital
Twin, and produces report-only remediation plans.

This batch is local and read-only by default. It does not publish GitHub issues,
write GitHub comments, create branches, merge PRs, or run scheduled automation.

## Command

```bash
bash scripts/autospec-constitution-audit.sh
```

The unified audit v2 runs:

1. Policy source validation
2. Policy source lockfile generation
3. Digital Twin build
4. Structured Baseline composition
5. Structured-first rule extraction
6. Effective rule resolution
7. Rule checks
8. Quality gate reporting
9. Constitutional gap report v1
10. Maturity scoring
11. Issue plan v3 generation
12. Policy compatibility reporting

## Rule Extraction

Rules can come from structured YAML/JSON rule files or from Markdown doctrine
heuristics. Structured rules are the primary source. Markdown extraction is
best-effort fallback only and is lower confidence; ambiguous doctrine becomes
`manual_review`.

## Effective Rules

Effective rules combine Constitution rules, Baseline rules, selected profiles,
application type, maturity target, waivers, and opt-outs. A rule can resolve as
`active`, `waived`, `opted_out`, `manual_review`, inactive, or conflict.

## Waivers And Opt-Outs

Use `.autospec/state/rule-waivers.yml` for documented exceptions. Waivers and
opt-outs never silently remove a rule; reports keep them visible. Expired or
incomplete waivers are findings.

## Rule Checks

`scripts/autospec-check-rules.sh` evaluates active rules against the Digital
Twin, Knowledge Graph, capability registry, surfaces, settings, permissions, AI
metadata, MCP metadata, docs, tests, and repository inventory.

## Maturity Scoring

`maturity-score.{json,md}` calculates simple level scores for prototype,
production, enterprise, and autonomous maturity. v1 is percentage-based with
blocking required-rule failures.

## Issue Plan v3

`issue-plan-v3.{json,md}` and `.autospec/backlog/issues-v3/*.md` are local
drafts only. They include source rule IDs, doctrine, baseline pack, source file,
severity, maturity level, category, evidence, missing evidence, remediation
hints, suggested labels, acceptance criteria, quality gates, risk, validation
expectations, and metadata expectations.

Publish structured-rule drafts only through the explicit backlog bridge:

```bash
bash scripts/autospec-audit-to-backlog.sh --dry-run
bash scripts/autospec-audit-to-backlog.sh --confirm
```

Dry-run answers what the Constitution says is missing and what v3 backlog would
be created. Confirmed mode publishes v3 issues with idempotency markers; it does
not run a worker, create a PR, approve, or merge.

Use direct publishing when the audit has already been run:

```bash
bash scripts/autospec-publish-issues.sh --dry-run --plan v3
bash scripts/autospec-publish-issues.sh --confirm --plan v3
```

## Difference From Heuristic Gap Reports

Older gap reports use hard-coded repository heuristics. Constitution audit v1
uses extracted rules as the source of truth, then evaluates those rules against
the Digital Twin. The extraction layer is intentionally imperfect until future
Constitution/Baseline repositories provide richer structured rule sidecars.

## Recommended Sequence

```bash
bash scripts/autospec-validate-policy-sources.sh
bash scripts/autospec-lock-policy-sources.sh
bash scripts/autospec-build-digital-twin.sh
bash scripts/autospec-baseline-compose.sh
bash scripts/autospec-extract-constitution-rules.sh
bash scripts/autospec-check-rules.sh
bash scripts/autospec-constitutional-gap-v1.sh
bash scripts/autospec-policy-compatibility.sh
bash scripts/autospec-constitution-audit.sh
bash scripts/autospec-audit-to-backlog.sh --dry-run
```

See also: `docs/runbooks/POLICY_SOURCES.md`.
