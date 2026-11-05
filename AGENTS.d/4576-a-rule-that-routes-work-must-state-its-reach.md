# A rule that routes work must state the reach it assumes (issue #4576)

Two standing rules are each correct in isolation and contradict each other
when the defect is outside the repository. "Fix any error you find first —
errors corrupt everything downstream" and "fix via autospec, not by hand:
observed errors become issues that agents fix". The second is right about
*this* repo — a hand fix here competes with the product code that should
carry the behaviour. But it silently assumes **every defect is reachable by
an agent dispatched against this repo.** A large share are not: the
dispatcher, the worker launcher, the reconciler and the base-refresh logic
live as scripts on the cluster, in a path no agent checks out.

For those, the rule says "file an issue and an agent will fix it" — and a
defect that stalled agents for hours (#4571) sat unfixed across several
sessions while its issue was dutifully filed, well evidenced, and completely
inert, because no agent could ever be dispatched against the file it
described. Following the rule where it has no reach is indistinguishable from
doing nothing, and it *feels* like diligence, which is why it persisted for
hours across sessions of otherwise careful work.

This is the same shape as two findings already in the repository: a label
with no consumer (#4565), and a readiness predicate gated on a permanent
property (InferWeave #326). In all three, a mechanism looked like it was
working because the producer side succeeded and nothing checked the consumer.

- **A process rule that assumes a capability must state that assumption.**
  When a rule routes work to a destination, the rule states the reach it
  assumes — where the destination can and cannot act — and a rule that routes
  work without stating that reach is a finding (`UNSTATED_CAPABILITY`).
- **A rule must say what to do when the destination cannot perform the work
  it routes.** A rule that states a reach but has no escape hatch for the
  defects outside it is a finding (`NO_ESCAPE_HATCH`): routing them is
  indistinguishable from doing nothing.
- **The no-hand-patch rule has a scope.** An in-reach defect is never
  hand-patched (`HAND_PATCH_IN_REACH` — a hand fix competes with the product
  code that should carry the behaviour). An out-of-reach defect routed to an
  agent that cannot act on it is the incident (`UNREACHABLE_DISPATCHED`): it
  should have been labelled unreachable so it is never queued, and stopped
  with a guarded hand fix.
- **A hand stopgap for an out-of-reach defect is permitted only in a specific
  shape.** Minimal, guarded, reverted-on-failure, and paired with a
  companion issue that carries the invariant into the eventual Rust
  implementation. Each missing element is a finding
  (`STOPGAP_NOT_MINIMAL`, `STOPGAP_NOT_GUARDED`,
  `STOPGAP_NOT_REVERTED_ON_FAILURE`, `STOPGAP_WITHOUT_ISSUE`).
- **The issue must say which of the two it is.** The companion issue for an
  unreachable component is labelled so it is never queued for an agent that
  cannot act on it (`STOPGAP_ISSUE_UNLABELLED`).
- **The stopgap is not the fix.** It is the bleeding stopped. The companion
  issue closes only when the defect is fixed for real, in the eventual
  implementation — never when the stopgap lands
  (`STOPGAP_READ_AS_FIX`).

Checkable in `autospec_core::rule_reach` (`Reach`, `Rule`,
`assumption_findings`, `Stopgap`, `stopgap_findings`, `Disposition`,
`handling_findings`, `audit` — pure in-memory). Tests:
`crates/autospec-core/tests/rule_reach.rs`, including the regression that
reconstructs the incident end-to-end (a rule that stated no reach, an
out-of-reach defect, `Dispatched` → `UNSTATED_CAPABILITY` and
`UNREACHABLE_DISPATCHED` together) and the clean cases (a scoped rule that
handled an out-of-reach defect with a well-formed stopgap, and an in-reach
defect dispatched to the agent that can act on it).
