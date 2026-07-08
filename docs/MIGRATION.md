# Autospec MVP Migration Notes

## Heuristic gaps vs structured rule reports

Older constitutional and baseline reports used heuristic Markdown gap extraction. The MVP path prefers structured Constitution/Baseline YAML rules, then falls back to heuristic extraction when structured policy is unavailable.

## issue-plan-v1/v2/v3 compatibility

`issue-plan-v1/v2/v3` remain readable. New structured-rule work should prefer `issue-plan-v3` because it carries rule IDs, quality gates, maturity target, severity, and structured remediation metadata.

## Published issue ledger compatibility

Existing `.autospec/state/published-issues.json` entries remain readable. New v3 entries include `plan_version`, `rule_ids`, `quality_gate_ids`, `source_policy_files`, `maturity_level`, `category`, and `severity`.

## Policy lockfile introduction

`.autospec/policy-sources.lock.json` records local Constitution/Baseline policy inputs for reproducibility and drift detection.

## Digital Twin state files

Digital Twin v1 adds metadata under `.autospec/state/`, including repository inventory, technology registry, capability registry, surface maps, workflow/domain maps, and knowledge graph files.

## Command changes

Release-candidate hardening adds local diagnostics:

- `scripts/autospec-preflight.sh`
- `scripts/autospec-mvp-smoke.sh`
- `scripts/autospec-command-audit.sh`
- `scripts/autospec-report-index.sh`
- `scripts/autospec-validate-state.sh`
- `scripts/autospec-sensitive-output-audit.sh`
- `scripts/autospec-recovery-status.sh`
- `scripts/autospec-clean-generated-reports.sh`

## Deprecated or legacy commands

No legacy commands are removed in this batch. Prefer structured v3 commands when available, but keep v1/v2 fallbacks for older repositories.
