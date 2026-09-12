# A negative grep is a property of the pattern until proven a property of the code (issue #4446)

Investigating autospec-cli #4389, the investigator asked whether anything
creates the executor worktree before validating it:

```text
grep -n 'worktree add\|worktree_add\|create_worktree\|add_worktree' executor_bridge.rs
   -> no matches
```

Four alternates looks like coverage. The conclusion was "nothing creates the
executor worktree," and the investigation began designing a fixture fix on
that basis. The file in fact creates it at four separate call sites. The code
spells it:

```rust
git_with_path(repo, &["worktree", "add", "--quiet"], ...)
```

An argv slice. `worktree add` as adjacent words never appears in the source
and never could. Every alternate in the pattern encoded the same wrong guess
about form — prose, snake_case, or a function name — and none encoded the one
form the codebase actually uses. The empty result was manufactured by the
search, not observed in the repository. And the diagnosis built on it was
wrong in the expensive direction: it pointed at the test fixture, which was
innocent.

The existing rule — "validate a negative with a positive control"
([4344](4344-negative-evidence-a-zero-over-an-unindexed-source-is-not-absence.md))
— did not fire because the search *felt* thorough. Breadth across spellings of
the same wrong assumption is not coverage: all four alternates shared the
assumption that the two tokens are adjacent in the source text, and that
single assumption is what failed.

- **Search the broadest single token the concept must contain first** — here
  just `worktree`. It cannot be over-narrowed by a guess about form. Only
  then narrow.
- **The step order is the discharge condition.** If step 1 returns nothing,
  the negative is real. If step 1 returns hits and step 2 returns none, the
  narrowing is what removed them — that is a finding about the pattern, not
  about the code, and the negative may not be reported as a property of the
  code until the search is shown capable of finding the thing.
- **Alternates that share one structural assumption provide one test, not
  several.** Adjacency, word order, casing, "it is a function name": when
  every alternate encodes the same assumption, the pattern has one degree of
  freedom, not four. Four spellings over one failed assumption is one
  observation — the same defect as four consistent zeros over an unindexed
  source.

Checkable in `autospec_core::negative_evidence` (`Alternates::effective_tests`
and `Alternates::smell` — alternates sharing one structural assumption are
one test, not several; `DischargeSteps`, `DischargeVerdict`, `discharge` —
step 1 empty is `NegativeIsReal`, step 1 hits with step 2 empty is
`PatternRemovedThem`). Tests: `crates/autospec-core/tests/negative_evidence.rs`,
including the incident end to end: the four alternates that never encoded the
argv-slice form are one failed assumption, and the broadest token that
returns four hits makes the empty second step a finding about the pattern.
