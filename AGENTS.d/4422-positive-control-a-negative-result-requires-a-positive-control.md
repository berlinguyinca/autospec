# A negative search result requires a positive control: I reported 0 of 183 when the true answer was 133 (issue #4422)

The analyst reported "zero issues in this repository declare dependencies" and
filed it upstream as a process defect. **133 issues declare dependencies, with
287 edges.** They use a `## Dependencies` heading followed by a list; the
analyst searched for `Depends on #N`, `Dependencies:`, `Blocked by`, and
`Requires` as inline text, and none of those patterns matched a heading.

The consequences were real: a wrong issue filed against another team's
repository, a correct tool declared broken, and a plan to replace a working
dependency graph with a title-parsing heuristic. The frontier computation had
been returning the right answer — *nothing is ready* — the whole time.

This is not the tool-coverage defect of #4344 (GitHub code search returning
zero over a repository it does not index). Here the tool **could** see the
corpus. The failure was the **query**: a search returning zero is evidence
about two things at once — the corpus, and the query — and only one of them
had been checked.

And the confirming detail was already written down: issue #216 states it built
its DAG "from the declared `## Dependencies` blocks". The format was
discoverable from the data that had already been downloaded, **before** the
absence was asserted. Knowing the rule ("establish a positive control before
trusting a negative") did not produce applying it.

- **A negative result requires a positive control.** Before reporting "X does
  not appear", find one instance of something the query *should* match. If the
  query cannot find a known-present case, it is the query that is broken — the
  zero is *not-matched*, not *absent*.
- **Establish the actual format before asserting its absence.** Read two or
  three real examples first. Guessing the schema and then searching for the
  guess reports on the guess.
- **Scale scepticism to the strength of the claim.** "Zero out of 183" is an
  extraordinary claim about a working system, and extraordinary claims should
  trigger a second method before they are filed, not after.

For specs and for tooling: any spec for an analysis that can produce "none
found" must state how the analysis distinguishes *absent* from *not-matched*.
In practice that means asserting a known-positive fixture alongside the real
corpus — a parser that cannot find the example in its own test data must fail
rather than report zero.

This is the same family as the other "absence read as evidence" entries — an
empty failure set reading as an improvement, an empty dashboard reading as no
traffic, an unparsed API response reading as zero workers. Absence is the most
common way this system lies to itself.

Checkable in `autospec_core::positive_control` (`verdict`, `PositiveControl` —
a known-present fixture of the data, `FormatBasis` — established from real
examples versus guessed, `ClaimStrength` — ordinary versus extraordinary,
`NegativeClaim`, `Verdict::holds` — the tooling gate: a negative may be
reported as absence only when it holds, every other verdict is a failure of the
analysis, not a zero to report). The positive control is the gate:
`Verdict::QueryIsTheFinding` when the query cannot find its fixture (the query
is broken and the zero is void), `Verdict::NoPositiveControl` when no control
was run, then `Verdict::FormatNotEstablished` and `Verdict::NeedsSecondMethod`
once the query is validated. Tests:
`crates/autospec-core/tests/positive_control.rs`, including the incident
end-to-end: a zero over a working population on a guessed format with no
control is untrusted; the known-present fixture the inline-text query does not
match makes the query the finding; the fixture a format-derived query does find
validates the query and lets the format and scepticism invariants be reached.
