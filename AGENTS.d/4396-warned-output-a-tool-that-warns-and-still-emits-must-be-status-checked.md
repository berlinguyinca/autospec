# A tool that warns and still emits output must have its exit status checked (issue #4396)

`comm -23 a b` computed "patches that have not been attempted". Its inputs
had been produced with `sort -un` — numerically sorted — and `comm` requires
**lexically** sorted input. It printed a diagnostic to stderr:

```text
comm: file 1 is not in sorted order
```

…and then emitted a result anyway. The result said **588 fresh patches**
when the true answer was **164**. Because stdout carried a plausible-looking
list, the number was nearly reported as the size of the backlog — a 3.6x
overstatement that would have driven a decision about where to spend hours.
The same class of error has now produced a wrong backlog figure twice: the
earlier "121 patches awaiting conversion" report, where the real number was
11.

A failed command announces itself. A command that warns and then emits a
result looks like a successful measurement, and piping its stdout onward
while ignoring its status converts a *detected* error into a confident wrong
answer — which is strictly worse than a crash, because nothing downstream
can tell.

- **A set operation must not depend on an ordering convention the producer
  does not guarantee.** Either do set operations in a language with real
  sets — the replacement here was four lines of Python and is
  order-independent — or sort defensively at the point of use with the exact
  collation the consumer requires, never relying on how the input was
  produced. `sort -un` guarantees numeric order; `comm` requires lexical
  order at the point of use. The producer's guarantee is not the consumer's
  contract (`check_order` — `OrderVerdict::Unordered` names both collations
  and both remedies).
- **A tool that warns on stderr and still writes a result to stdout is a
  tool whose exit status must be checked.** Piping its stdout onward while
  ignoring its status converts a detected error into a confident wrong
  answer — strictly worse than a crash, because nothing downstream can tell
  (`Step` / `StepOutcome::ConfidentWrongAnswer`). The warning *was* the
  error report; the unchecked status is what made it unheeded.
- **Any pipeline step whose output feeds a reported number needs a sanity
  assertion on the result.** Here, "fresh cannot exceed total patches" —
  588 == 588 was the giveaway: a filter that excludes anything cannot select
  everything (`CountBound`: over the total is `FAIL` — the number was not
  measured; equal to the total is `WARN` — the filter may not have run).
  The earlier 121-against-11 report would have been a straight `FAIL` had
  the bound been asserted.
- **When a spec asks for a count of outstanding work, it must name the
  filter.** "has a patch" (588), "has no branch or PR" (164), and "…and the
  issue is still open" (75) are three different numbers for the same
  question. Reporting the wrong one is not a rounding error — it is an 8x
  misstatement of the remaining work (`BacklogQuestion` — an unnamed filter
  is a finding, and a reported count prints its filter on the line:
  `164 outstanding (has no branch or PR)`).

Checkable in `autospec_core::warned_output` (`Collation`, `OrderConsumer`,
`SetOp`, `check_order`, `OrderVerdict`, `Step`, `StepOutcome`, `CountBound`,
`Filter`, `BacklogQuestion`). Tests:
`crates/autospec-core/tests/warned_output.rs`, including the regression that
reconstructs the incident end-to-end: `sort -un` output into `comm`
(`Unordered`), the warned step with the unchecked status
(`ConfidentWrongAnswer`), the 588-against-588 sanity bound (the equality that
was the giveaway), the earlier 121-against-11 report (`impossible`), and the
three filters that are three numbers for one question.
