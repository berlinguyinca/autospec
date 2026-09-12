# Untested public items and coverage of the change (issue #4329)

A gate that runs the existing suite cannot see that new code arrived
untested. The patch for #4238 staged two files and added **eight public
functions with zero tests** — all public, none exercised — and every
correctness gate that ran passed: the merge record said `CONVERTED+MERGED:
9065 passed, 0 failing`, a true statement that carries no signal about
whether the *new* code is among the 9065. `hold_line` shipped with a format
string carrying two `{}` placeholders against one argument — a compile
error the gate never saw, because no test called the function. `AGENTS.md`
stated TDD is non-negotiable; the agent did not follow it, and nothing in
the pipeline could tell. A rule that only a well-behaved agent enforces is
not enforced.

- **A patch adding a public item must add a test that exercises it, or
  state why not.** The gate diffs the set of public items before and after
  (`diff_public_items`) and requires every addition to be referenced by test
  code added in the *same patch*, or to carry an explicit annotated
  exemption (`linter:allow-UNTESTED_PUBLIC <item>: <reason>` — reason
  mandatory, a bare marker is rejected, as everywhere else in the
  repository). Exercised means exercised by the patch's own tests; the
  existing suite staying green is not evidence about new code.
- **Report coverage of the change, not of the repository.** The merge record
  carries `change coverage: N new public items introduced, M exercised`
  (`ChangeCoverage::line`), and a record silent about the change's coverage
  is a lie of omission, not a missing fact to default
  (`record_carries_coverage`).
- **A standard stated in AGENTS.md and unenforced by the gate will be
  violated silently.** Either mechanise it or stop claiming it: the gate
  refuses — naming every untested item — when any addition is unexercised
  and unexempted (`gate` → `GateVerdict::Refused`). The interesting failures
  are the ones where the rule was stated, believed, and had no teeth.
- **A test must be shown to fail against the defect it guards.** A test
  written after the fix, never seen red, is an assumption wearing a test's
  clothing. The regression tests here are mutation-verified: with
  `hold_line` missing from the exercised set the audit fails; restored, it
  passes.

Checkable in `autospec_core::untested_public_items` (`diff_public_items`,
`parse_exemptions`, `audit`, `ChangeCoverage`, `record_carries_coverage`,
`gate`). Tests: `crates/autospec-core/tests/untested_public_items.rs`,
including the regression that reconstructs the incident end-to-end: eight
additions, zero exercises, the bare-suite-total record, and the
mutation-verified audit.
