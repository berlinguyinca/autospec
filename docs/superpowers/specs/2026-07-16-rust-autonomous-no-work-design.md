# Rust Autonomous No-Work and Ideation Design

> **Superseded after the state foundation:** The full native waterfall and
> local ideation contract is defined in
> [`rust-autonomous-waterfall-design.md`](rust-autonomous-waterfall-design.md).

**Issue:** #1872
**Parent:** #2076
**Status:** approved by the standing Rust-cutover execution directive

## Goal

Make Rust autonomous mode record why deterministic discovery produced no safe
candidate, preserve that evidence across full-waterfall passes, and request a
bounded planning-only ideation backlog after repeated verified dry passes.

## Decision

The initial migration is an explicit Rust
`autospec autonomous no-work record|status` adapter. It accepts only closed,
typed tier observations; it does not scrape shell output or claim that the
current Tier-1 foreground queue is a complete waterfall. Future typed Tier
1.5–4 adapters call `record` with their actual results. The adapter writes one
atomic schema-1 `why-no-work.json` in the repo-scoped operator directory.

This is preferred over a passive Tier-1-only foreground report because Tier
1.5–4 are not yet Rust-owned. It is preferred over porting all discovery
executors in this child because those are independent migrations with distinct
process, model, and external-reference contracts.

## Architecture

`autospec_core::autonomous::no_work` is a pure dependency-free policy module.
It has no file, process, GitHub, shell, model, or agent authority. It owns the
ordered tiers `tier1`, `tier1_5`, `tier2`, `tier3`, and `tier4`; each carries
one exact outcome:

- `produced { count }`
- `dry { reason }`, where reason is exactly `no_proposals_generated`,
  `deduplicated`, `verification_rejected`, `roi_filtered`, or
  `already_implemented`
- `not_run { reason }`
- `failed { reason }`

Only exactly five explicit `dry` outcomes form a full dry waterfall. `not_run`,
`failed`, and `produced` never count as dry. A positive `pass_id` is
idempotent: recording the same valid pass again leaves its count unchanged;
recording an older or conflicting duplicate is rejected. A non-dry pass resets
the consecutive full-dry count to zero. The fixed Rust threshold is two
consecutive full-dry passes; there is no environment or shell configuration
authority.

The CLI validates `--repo`, `--pass`, and one repeated `--tier` observation per
ordered tier, reads the previous same-repo artifact, invokes the pure policy,
and atomically writes the replacement artifact. `status --json` reads and
projects the same validated artifact. It performs no network, process, queue,
claim, label, issue, PR, or implementation mutation.

## Data Contract

`why-no-work.json` contains schema, repo, timestamp, full pass ID,
consecutive dry full-pass count, threshold, ordered tier outcomes, decision,
and a per-reason count projection. Its decisions are `idle_rescan` and
`ideation_backlog_refresh_required`.

The latter embeds a planning-only request with `candidate_limit: 5`, required
candidate scores `impact`, `importance`, `risk`, and `effort`, and these six
exact questions:

1. What features are missing?
2. What can we do here?
3. Find 5 new features.
4. Rank them by impact and importance.
5. Which are safe to implement autonomously now?
6. Which need a planning/spec issue first?

The request has `disposition: "planning_only"` and `remote_mutation: "none"`.
It never creates an issue. A later ideation producer must keep its ranked
top-five proposals in this contract and receive separate approval for any
GitHub mutation.

## Boundaries

- No code in `scripts/autonomous-waterfall.sh` or
  `scripts/lib/autospec-loop.sh` is reused or modified.
- Do not treat unported tiers as dry.
- Do not accept arbitrary filesystem evidence paths, raw logs, child output,
  leases, or secrets in the record interface.
- No no-work command may execute `sh`, `bash`, `omx`, `gh`, an agent, or a
  model; it is an operator-artifact adapter only.
- This child does not delete shell waterfall code. #2076 may do so only after
  every tier has an equivalent typed Rust producer.

## Tests and Completion

Core tests cover all five dry reasons, full-pass completeness, not-run/failed
exclusion, duplicate pass idempotency, conflicting/older pass rejection,
threshold two, and request bounds. CLI tests cover record/status JSON,
atomic persistence, malformed/foreign prior artifact failure before write,
and static no-shell/no-GitHub/no-issue-authority guards. Full formatting,
clippy, workspace tests, and fast validation must pass.
