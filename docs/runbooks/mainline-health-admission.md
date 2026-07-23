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
evidence. Every record also carries `effective_policy_digest`, a canonical
binding over the resolved branch and sorted exact ignored-check names. Changing
either effective value changes the digest; YAML formatting and unrelated
configuration do not. Rust reloads the repository policy and appends this
binding on every evaluated foreground invocation, even when retained conductor
state ends the cycle. Malformed policy fails before a new record is written.
When a baseline check-name set is available, known red checks remain advisory;
only a newly red required check produces the `new-check-failed` admission halt.
Stale or unreadable baselines remain fail-closed and return a wait outcome.
If no configured, explicit, or GitHub default branch can be resolved, the
`default-branch-missing` receipt uses the reserved invalid-ref identity
`autospec:unresolved-default-branch` for digest input while retaining an empty
`branch` field as typed diagnostic evidence.

`autospec autonomous run-foreground` completes this Rust admission gate before
entering its bounded Rust conductor cycle, so missing branches or required
failed/pending checks cannot dispatch ready work.

## Premerge evidence admission

Before a claimed executor result can close a claim, run the typed Rust premerge
gate for the exact repository, issue, worker, claim, branch, and commit lane:

```
autospec autonomous premerge evaluate --repo OWNER/REPO --repo-dir DIR \
  --issue N --worker-id ID --claim-id ID --json
```

The `autospec-qa` and `autospec-secaudit` producers write schema-1 JSON only to the fixed untracked paths
`.autospec/evidence/premerge/<lane-digest>/qa.json` and `security.json`.
Each document has exactly `schema`, `kind`, `producer`, `repo`, `issue`,
`worker_id`, `claim_id`, `branch`, `commit`, `run_id`, `completed_at`,
`verdict`, `finding_codes`, and `reason`; kind is `qa` or `security-audit`, and
verdicts are `pass`, bounded-code `blocked`, or bounded-reason `failed`, with the
producer/kind pair fixed per file. Tracked
staged or unstaged changes reject admission, as do detached or non-attached
worktrees; those two untracked evidence files are intentionally permitted. The
evaluator verifies canonical lane and evidence digests, writes immutable
decisions under
`.autospec/autonomous-operator/<scope>/premerge/lanes/<lane-digest>/decisions/<evidence-digest>.json`,
plus `latest.json` and blocked-lane `quarantine.json`. Any missing, malformed,
mismatched, or unavailable evidence fails closed. Exit 0 is pass, 20 is
blocked/quarantined, and 2 is diagnostic failure. Quarantine-and-continue
orchestration remains supervised-executor follow-up; this command only records
the lane decision.

The receipt is an observability/admission artifact, not a foreground executor:
the supervised Rust executor and live QA/security producers remain follow-up
work. Claim success must include `--claim-id` and `--premerge-receipt`, and the
receipt commit must match the GitHub PR `headRefOid`; otherwise the claim remains
non-successful.
