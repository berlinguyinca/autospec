# Autonomous integration branch — self-originated work never lands directly on the parent branch

> Brainstorm provenance: architecture, interactivity, data model, error handling, and testing
> were locked interactively with the operator in the originating session (2026-07-10); this
> spec synthesizes those decisions plus the Phase 1 code-map evidence. No open questions
> remain from the brainstorm.

## Problem

When autospec runs autonomously, work it discovered FOR ITSELF — Tier 2 local discovery,
Tier 3 architecture/coverage improvement, Tier 4 internet/operator discovery, growth-tier
artifacts, explore findings promoted to implementation, QA sweep findings — is eligible for
the same admin auto-merge to the parent branch (main / master / master_ai) as
operator-approved backlog work. The agent that invented a feature also implements, approves,
and merges it: portfolio-level self-certification. A human never sees the change until it is
already on the mainline.

Phase 1 evidence: a provenance-based routing primitive ALREADY exists — explore's F3
mechanism writes `.autospec/explore-mode.json` via `scripts/explore-sandbox.sh`, the Phase 4
implementer reads it to target PRs at the sandbox branch (`scripts/lib/autospec-loop.sh:1286-1301,
1426-1431`), and main merges are refused while discovery is in flight
(`code_health:autonomous_main_merge_refused`). But it is keyed to ephemeral per-run
`autospec/explore/<date>-<slug>` branches, has NO roll-up/promotion PR (explore prints manual
`git merge` hints, `scripts/autospec-explore.sh:1245-1258`), and provenance
(`_inflight_discovery`) is in-memory only — not derivable after a crash.

## Rule

Autonomously SELF-ORIGINATED changes never merge directly to a protected parent branch. They
accumulate on a dedicated, long-lived autonomous integration branch, and reach the parent
only through a roll-up pull request that a human (or an explicit trusted-actor
control-channel `promote`) reviews and merges. Operator-originated work keeps today's
direct-to-parent auto-merge.

A separate constitution amendment (berlinguyinca/autospec-constitution, Continuous Evolution
doctrine 15 + a gate referencing doctrine 19) states this as law; it is handled outside this
repo's issues and is NOT part of this decomposition.

## Team personality

**Reliability/backend** — backend/bash developer, platform engineer, SRE, security advisor,
test engineer. This is autonomous-control-plane engineering: branch topology, gate wiring,
durable state, and resume semantics; the risks this team is expected to notice are lost
provenance across crashes, gate bypasses, branch-sync corruption, and unbounded review debt.
Carry into child issues: bash 3.2 compatibility, `set -eu` discipline, subprocess-mocked bats
tests, no in-memory-only state for anything dispatch-critical.

### Review counter-team

**Governance & operator-experience review** — security advisor (provenance-bypass attempts:
label forgery, retargeted PRs, approval spoofing), release engineer (roll-up CI correctness,
branch reset after promotion), operator advocate (is the roll-up PR actually reviewable —
manifest clarity, per-feature comments, notification ergonomics). Challenge: does every
enforcement layer fail CLOSED, and can a reviewer understand the roll-up without reading the
full diff? Stay inside issue scope while applying that lens.

## Architecture

Generalize the existing explore-sandbox mechanics; do not build a parallel scheme.

1. **`scripts/autonomous-integration-branch.sh`** (new, mirrors `explore-sandbox.sh` idioms)
   with subcommands:
   - `ensure --parent <branch> --repo <slug>` — create `autospec/autonomous-<parent>` from
     the parent tip if absent; write/update the mode file (below); idempotent.
   - `sync --parent <branch>` — merge parent INTO the integration branch (merge, not rebase,
     to keep worker-PR history stable). Merge conflict → exit 65 (park signal), never bypass.
   - `rollup-update --parent <branch> --issue <N> --pr <M>` — ensure the single open roll-up
     PR (integration → parent) exists (create on first self-originated landing, label
     `autospec:needs-human`, never auto-merge); regenerate the PR BODY as a current-state
     manifest (issues, PRs, tiers, gate-evidence links, cumulative diff stats); post ONE
     idempotent comment for the landed feature keyed by marker
     `<!-- autospec-rollup:issue-<N> -->` (issue title + link, one-paragraph summary, origin
     tier/source, gate evidence, diff stats). If the marker already exists in any comment,
     skip (crash-safe). If the roll-up's CI is red, print `rollup-red` on stdout so the
     conductor pauses further self-originated merges.
   - `reset --parent <branch>` — after the roll-up merges: recreate the integration branch
     from the new parent tip; next `rollup-update` opens a fresh roll-up PR.
   - `status --parent <branch>` — JSON: branch, rollup PR number/state, accumulated PR count,
     age days, diff lines vs parent (feeds caps).
2. **Mode file / routing contract.** Reuse the EXISTING Phase 4 routing contract: the
   integration branch writes `.autospec/explore-mode.json` with the same shape
   (`{branch, slug, base, head_sha}`) plus `"kind": "integration"`. Consumers at
   `autospec-loop.sh:1286-1301,1426-1431` keep working unchanged; explore's per-run sandbox
   continues to write `"kind": "explore"` (absent kind = explore, backward compatible).
   Unification: when the conductor (not a standalone `/autospec-explore` session) drives
   discovery tiers, it points explore at the integration branch as `--base`, so explore's
   sandbox IS the integration branch under autonomous mode.
3. **Provenance resolver: `scripts/autonomous-provenance.sh resolve --issue <N> --repo <slug>`**
   prints `self` or `operator`. Rules, in order:
   - label `origin:self` present AND no approval marker → `self`;
   - approval marker present → `operator`. Approval marker = label
     `approved-by-operator` whose `labeled` timeline event actor
     (`gh api repos/{repo}/issues/{n}/timeline`, filter `event=="labeled"` and
     `label.name=="approved-by-operator"`, take the LAST such event's `actor.login`) is a
     `safety.issue_intent_gate.trusted_actors` login — a label present but last applied by
     an untrusted/bot actor does NOT count (forgery-proof); or an issue comment matching
     `^approve(d)?\b` authored by a trusted-actor login (reuse `lint-issue-safety.sh`
     parsing);
   - no `origin:self` label AND issue author is a human (not the bot identity) → `operator`;
   - anything else (unknown author, missing labels, API ambiguity) → `self` (fail closed).
   Durable by construction: labels + comments live on GitHub, so resume re-derives identical
   answers; nothing dispatch-critical stays in memory.
4. **Filing-site labeling.** Every autonomous `gh issue create` call site applies
   `origin:self` at creation (label auto-created idempotently): Tier 2/3 self-improvement
   (`scripts/autonomous-self-improvement.sh:192`), explore filing paths
   (`scripts/autospec-explore.sh:755,759`, `scripts/autospec-autonomous-explore-drain.sh`),
   QA sweep (`scripts/qa-finding-to-issue.sh:122`, `scripts/qa-brute-force-sweep.sh:70,74`),
   growth define filing. Grep-audit for any other autonomous `gh issue create` and cover it.
5. **Conductor dispatch-time resolution.** Before draining a batch, the conductor resolves
   provenance per issue and splits the batch: `self` issues dispatch with the integration
   branch active in the mode file (Phase 4 targets it); `operator` issues dispatch with the
   mode file cleared/parked (Phase 4 targets the parent as today). Lock-step rules unchanged.
   After each self-originated PR merges into the integration branch, call `rollup-update`.
6. **Premerge-gate check (defense in depth).** New `scripts/autonomous-guardrails.sh
   self-originated --pr <N> --repo <slug> --protected-branches <csv>` subcommand: if the PR's
   base is a protected parent branch and the linked issue's provenance resolves `self`, fail.
   The protected list = the repo's default branch plus
   `autonomous.self_originated.protected_branches` (config, default empty — covers
   `master_ai`-style secondary mainlines); integration branches themselves are never
   protected.
   Gate wiring mirrors diff-guard (`autonomous-premerge-gate.sh:344-350`): output
   `block self_originated_direct_merge`, apply `autospec:needs-human`, exit 1.
7. **Caps.** Config via `autospec_runtime_config_get` (`autospec-runtime-config.sh:26`):
   `autonomous.self_originated.integration_branch_prefix` (default `autospec/autonomous-`),
   `autonomous.self_originated.max_open_prs` (default 20),
   `autonomous.self_originated.max_age_days` (default 14),
   `autonomous.self_originated.max_diff_lines` (default 5000),
   `autonomous.self_originated.allow_direct_merge` (default `false`; flipping requires a
   trusted-actor marker recorded in the config commit). Cap exceeded (from `status`) → park
   self-originated tiers + notify; operator tiers unaffected.
8. **Control channel.** Add `promote` and `discard` to `RESERVED_LABELS`
   (`autonomous-control-channel.sh:54`) and `emit_decision` (`:155-210`); conductor consumer
   (`autospec-loop.sh:1123-1144`): `promote` (trusted actor only) merges the roll-up PR and
   runs `reset`; `discard` closes the roll-up, deletes the integration branch, and reopens
   its issues with a `discarded-from-rollup` comment.

## Interactivity

Operators interact through: (a) the roll-up PR — diff = exactly what would land on the
parent; body = manifest; comments = chronological per-feature changelog with notifications;
(b) control-channel `promote` / `discard` issues; (c) approval markers on individual issues
to flip provenance pre-dispatch; (d) config keys above. No new CLI surface beyond the two
scripts' subcommands.

## Data model

- Labels: `origin:self` (#8250df), `approved-by-operator` (#0e8a16),
  `discarded-from-rollup` (comment marker only, no label).
- Mode file: `.autospec/explore-mode.json` gains optional `"kind": "integration"|"explore"`.
- Roll-up comment idempotency marker: `<!-- autospec-rollup:issue-<N> -->`.
- Roll-up body manifest markers: `<!-- autospec-rollup-manifest:begin/end -->` (regenerated
  wholesale between markers).
- No schema/DB changes; all state lives in GitHub (labels, PRs, comments) + the mode file.

## Error handling

- `sync` merge conflict → exit 65; conductor parks self-originated tiers, notifies
  (`notify.sh`), records `code_health:integration_sync_conflict`; operator resolves manually.
- Red roll-up CI → `rollup-update` prints `rollup-red`; conductor pauses further
  self-originated merges (operator tiers continue) until CI is green or `discard`.
- Crash/resume: provenance from labels, branch from config prefix + parent, roll-up PR by
  `gh pr list --head <integration> --base <parent>`; the per-feature comment marker prevents
  double-posting. Nothing requires session memory.
- Fenced surfaces: blast-radius quarantine evaluates FIRST and wins (quarantine, not
  integration branch) — order preserved in the gate.
- `gh` API failures in `rollup-update` → retry once, then park with notification (the merge
  into the integration branch has already happened; the comment retries on next landing via
  the idempotency scan).

## Testing

TDD, bats, subprocess mocks for `gh`/`git` per existing idioms; mirror
`tests/autonomous/test_sandbox_routing.bats`, `test_premerge_gate.bats`,
`test_control_channel.bats`. New `tests/autonomous/test_integration_branch.bats` +
`test_provenance_resolution.bats` covering:

- provenance: origin:self → self; approval label flips to operator; trusted-actor approval
  comment flips; unknown/missing → self (fail closed); human-authored unlabeled → operator.
- target-branch resolution: self batch writes kind=integration mode file; operator batch
  clears it; mixed batch splits.
- gate: self-originated PR based on parent → `block self_originated_direct_merge`; based on
  integration branch → merge-ok path unaffected; operator PR on parent unaffected.
- roll-up lifecycle: first landing opens PR with label + manifest; second landing updates
  body + adds exactly one new comment; re-run after simulated crash adds NO duplicate
  comment; human merge + `reset` recreates branch; next landing opens a fresh PR.
- caps: exceeding max_open_prs parks self tiers (mock status), operator tier proceeds.
- sync conflict: exit 65 parks and notifies, never bypasses.
- control channel: `promote` by trusted actor merges + resets; `promote` by untrusted actor
  refused; `discard` closes + comments.
- No real GitHub calls in unit bats; one E2E-gated test may exercise a throwaway repo under
  the existing `E2E=1` convention.

Note for implementers: `scripts/autonomous-*.sh` are a FENCED surface
(`.autospec/autospec.yml` fenced_surfaces `autonomous-control-plane`), so implementation PRs
for these issues will quarantine to async human review instead of auto-merging — expected
and correct for this feature; plan review turnaround accordingly.

## Non-goals

- No change to operator-originated Tier 1 auto-merge behavior.
- No per-feature human review at the worker-PR level (the gate already covers it); the human
  reviews the roll-up.
- No engine-repo exemption: autospec dogfooding follows the same rule.
- The constitution amendment ships separately (policy repo), not from these issues.
- No rewrite of explore's standalone (non-conductor) sandbox flow; it keeps ephemeral
  branches when invoked directly.

## Acceptance criteria

- [ ] A discovery-tier issue implemented autonomously merges into
      `autospec/autonomous-<parent>`, appears in the roll-up body manifest, and has exactly
      one descriptive roll-up comment — while an operator-approved issue in the same run
      merges directly to the parent.
- [ ] Removing the approval marker from an issue flips routing to the integration branch on
      the next dispatch; adding it flips back (fail-closed provenance).
- [ ] A self-originated worker PR retargeted at the parent is blocked by the premerge gate
      with `block self_originated_direct_merge`.
- [ ] Caps park self-originated tiers with a notification instead of growing the branch.
- [ ] Roll-up comment posting is idempotent across a simulated crash/resume.
- [ ] `promote` (trusted actor) merges the roll-up and resets the branch; `discard` closes
      and comments; untrusted `promote` is refused.
- [ ] All new bats suites pass; `bash scripts/validate.sh` passes.
