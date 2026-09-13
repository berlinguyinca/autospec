# A priority was attached to an unmeasured number (issue #4561)

#4560 stated that one conflict shape was "the dominant conflict in this
backlog" and "the highest-yield single change available to conversion
throughput". Measured, it is worth **11%** of conflict-held patches, and
the dominant shape is something else entirely (76% `code × code`, which
no classifier can resolve).

The error was not arithmetic. Every individual number was real:

- 5 of 15 conflict *region instances* were `additive_declarations` — true
- the top three contended *files* held 62 of 96 conflicts — true
- those files were append-only indexes — true

None of them support the claim. Region-instance share is not per-patch
yield (a patch is held if **any** region is unresolvable, so a shape
appearing in many patches alongside other shapes frees none of them).
File contention is not region kind. Both were measurements of adjacent
quantities, promoted to the quantity the decision needed, on a 15-region
sample, and written into a filed issue **with a priority attached**.

The correct measurement took one scan: apply each patch to a scratch
worktree, classify both sides of every conflict region, and count
patches whose regions are *all* resolvable. 266 regions, 57 patches,
~10 minutes. It was available before the claim was made, and it was not
run.

This is the same failure as
[4446](4446-broadest-token-first-a-negative-grep-is-a-pattern-property-until-proven-a-code-property.md)
(a negative grep is a property of the pattern until proven a property of
the code) and
[4449](4449-silent-false-negatives-recall-rules-are-not-fixes.md) (a
warning on stderr with a wrong answer on stdout is a wrong answer), in
the direction of *quantities* rather than *absences*.

- **A quantity used to rank work must be the quantity that ranks it.**
  "How often does shape X appear" and "how many items would resolving X
  free" are different questions with different answers. Name the
  decision the number drives, then measure *that*.
- **Sample size is part of the claim.** "Dominant" from 15 observations
  is a hypothesis. State n, or state nothing — a share without a
  denominator reads as measured and is not.
- **Any-of blocks all-of.** Whenever an item is blocked if *any* of its
  parts fails, per-part frequency systematically overstates per-item
  yield, and the overstatement grows with the number of parts. This
  shape recurs — held patches, failing gates, unmet dependencies — and
  the per-item count is always the one that matters.
- **A priority in an issue is a claim, and carries the same
  evidentiary burden as a technical assertion.** "Highest-yield" is
  falsifiable. If it has not been measured, the issue says what is
  known and omits the ranking.
- **Prefer the cheap direct measurement to the clever indirect one.**
  The scan that settled this was a loop over patches in a scratch
  worktree. Reasoning from file-contention counts was faster to produce
  and wrong.

For specs and agent prompts: when a spec or issue asks an agent to
prioritise, rank, or call something "highest-impact", it must require the
ranking metric to be stated and measured, with n. An agent that cannot
measure it reports the ordering as unknown rather than inferring one
from whatever counts are nearest to hand.

Checkable in `autospec_core::ranking_evidence` (`Share`, `ShareStatus`,
`MIN_MEASURED_N`, `Metric`, `EvidenceKind`, `RankingTarget`, `Backlog`
with `per_part_share` / `per_item_yield` / `yield_comparison`,
`YieldComparison`, `any_of_item_yield`, `any_of_gap`, `RankingClaim`,
`ClaimVerdict`, `judge_claim`, `unknown_ordering_line`).
Tests: `crates/autospec-core/tests/ranking_evidence.rs`, including the
regression that reconstructs the incident at scale (57 held patches,
261 conflict regions: the `additive` shape is ~20% of region instances
but frees only 6 of 57 patches — 11% — while `code × code` is the 80%
dominant shape that no classifier resolves; the filed claim is a metric
mismatch, the corrected per-item claim with n=57 and direct evidence is
admissible, and the gap between per-part frequency and per-item yield
is non-decreasing in the number of parts).
