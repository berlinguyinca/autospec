# Self-gating of gatekeeping automation (issue #4263)

The watchdog reaped in queue order under a cap (two agents at 4h01m and
3h51m survived three sweeps while younger offenders were killed in each —
the cap was consumed by whoever came first in the `squeue` listing, and the
worst offenders were never at the front); the conversion selector keyed
"already attempted" on the **issue**, so a new patch for an issue whose
earlier patch had been attempted was never selected, and it counted **issue
directories** instead of patches, reporting "15" where the true population
was 11 issues / 14 patches / 15 directories. The predicate for all of it
lived in a one-line `jq` in a scratch-path script that died with the
session, and the whole stack was trusted because it had been running for a
while. Five rules, each checkable:

- **Destructive scripts need a dry-run mode, and the first production run
  needs a dry-run first.** `gate_destructive_run` refuses a destructive
  script that has no dry-run flag, and refuses the first production run
  until a dry run has been performed; a dry run itself is never refused.
- **Selection predicates report the denominator and live in a reviewable
  file.** `SelectionReport::line()` renders "N of M selected" — the
  denominator is never optional, and a numerator above the denominator is a
  counting bug, not an edge case. A report that is not written to a file
  has no home and is a finding.
- **Automation that decides work is an artifact.** `AutomationArtifact`
  requires a file (an inline predicate has no home), a comment per
  condition explaining the bug it encodes, and one fixture test; each
  missing element is a separate finding.
- **Cap the blast radius, and make the cap bind visibly.**
  `select_reaps(candidates, cap)` returns a `ReapPlan` that names every
  deferred offender; a zero cap is an error, not a silent pass, because a
  destructive reaper with nothing to fill is a configuration to surface.
  Selection is by severity — most seconds over the limit first — never by
  listing order, or the cap starves exactly the offenders it was meant to
  reach (queue order starved the two worst offenders across three sweeps;
  severity order selects them first).
- **Recency is not reliability.** `trust_verdict` returns true only for
  `ReliabilityEvidence::Gated`; `RanRecently { times }` is not trust
  evidence at any count, because a script that has been running for a
  while is exactly the one nobody re-reads.
- **"Attempted" keys on the patch, not the issue** (`AttemptLedger`). An
  attempt is a fact about the patch it was made on, not a verdict on the
  issue; a new patch for an attempted issue is still a candidate.
  `selector_line` reports "attempted N of M patches (K issues)" — patch
  count primary, issue count recorded separately, so a unit mix-up is
  visible in the line itself.
- **A directory is not a candidate** (`WorkState`). Directories are created
  when an agent *starts*; patches are what an agent may never produce.
  Only `HasPatch` is a conversion candidate — `InProgress` and `Converted`
  are not.

Checkable in `autospec_core::self_gate` (`gate_destructive_run`,
`SelectionReport`, `AutomationArtifact`, `select_reaps`, `trust_verdict`,
`AttemptLedger`, `WorkState`). Tests:
`crates/autospec-core/tests/self_gate.rs`.
