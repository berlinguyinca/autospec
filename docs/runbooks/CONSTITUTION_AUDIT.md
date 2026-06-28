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

The unified audit runs:

1. Constitution/baseline validation
2. Baseline composition
3. Digital Twin build
4. Rule extraction
5. Effective rule resolution
6. Rule checks
7. Constitutional gap report v1
8. Maturity scoring
9. Issue plan v2 generation

## Rule Extraction

Rules can come from structured YAML/JSON rule files or from Markdown doctrine
heuristics. Structured rules are preferred. Markdown extraction is best-effort
and evidence-scored; ambiguous doctrine becomes `manual_review`.

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

## Issue Plan v2

`issue-plan-v2.{json,md}` and `.autospec/backlog/issues-v2/*.md` are local
drafts only. They include source rule IDs, severity, category, evidence, missing
evidence, acceptance criteria, validation expectations, and worker eligibility
hints. No GitHub issues are created by this batch.

## Difference From Heuristic Gap Reports

Older gap reports use hard-coded repository heuristics. Constitution audit v1
uses extracted rules as the source of truth, then evaluates those rules against
the Digital Twin. The extraction layer is intentionally imperfect until future
Constitution/Baseline repositories provide richer structured rule sidecars.

## Recommended Sequence

```bash
bash scripts/autospec-build-digital-twin.sh
bash scripts/autospec-extract-constitution-rules.sh
bash scripts/autospec-check-rules.sh
bash scripts/autospec-constitutional-gap-v1.sh
bash scripts/autospec-constitution-audit.sh
```
