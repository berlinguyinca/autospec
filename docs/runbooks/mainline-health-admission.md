# Mainline health admission

`autospec autonomous main-health --repo OWNER/REPO --repo-dir DIR [--branch BRANCH]`
is the Rust-owned health probe for autonomous Tier-1 admission.

## Repository-owned policy

On every `main-health` and `run-foreground` invocation, Rust reads
`<checkout-root>/.autospec/autonomous.yml`. When `DIR` names a subdirectory of
a Git checkout, Rust resolves the checkout root first; outside Git it uses
`DIR` literally. This file is distinct from `.autospec/autospec.yml` and
supports only this strict subset:

```yaml
main_health:
  branch: master_ai
  ignore_checks:
    - "Unit Tests"
```

The selected branch is resolved as `--branch`, then `main_health.branch`, then
GitHub default-branch metadata. Rust never silently substitutes `main`. A
configured branch that does not exist therefore produces `branch-not-found`; it
does not fall through to the default branch.

`ignore_checks` compares exact, case-sensitive check names, not regular
expressions, globs, or substrings. Matching checks remain in the persisted
evidence as `advisory`; unmatched failed or pending checks stay `required` and
block admission. The setting applies only to mainline health: it cannot relax
premerge, safety, claim, or merge policy.

Missing `autonomous.yml` keeps current default-branch behavior. Any unreadable
file or malformed relevant configuration fails closed with diagnostic exit `2`
before the foreground lease, queue selection, claim, state, or executor
dispatch. Rust does not read, export, or honor `AUTOSPEC_MAIN_HEALTH_*` for this
admission path.

## Observation and dispatch behavior

Observations are appended under repo-scoped autonomous state as
`main-health-observations.jsonl` with branch, outcome, diagnostic, and check
evidence. `autospec autonomous run-foreground` completes this Rust admission
gate before entering its bounded Rust conductor cycle, so missing branches or
required failed/pending checks cannot dispatch ready work.
