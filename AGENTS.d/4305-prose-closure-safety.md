# Prose closure safety (issue #4305)

A converter marked its PR `Refs #288 (does not close it)` — the trailer
said the issue stays open — but line 11 of the same body said `closed #288`
in prose, and GitHub closed the issue on merge. GitHub's closing semantics
fire on *any* closing keyword adjacent to a reference in the body, not only
on trailers; a trailer is a statement of intent, and prose is a live
directive. The body contradicted itself, and the contradiction was
machine-detectable before publish. Four invariants, one checkable each:

- **A closing keyword never sits adjacent to the reference in prose**
  (`prose_violations`). A closing directive is a closing verb from the
  shared `CLOSING_VERBS` list (`close`, `closes`, `closed`, `fix`, `fixes`,
  `fixed`, `resolve`, `resolves`, `resolved`) followed, after optional
  whitespace, by the reference to *this* issue — `find_closure_directives`
  finds every one in the body, with a digit-suffix guard so `closed #2883`
  is not a match for `#288`, a word-boundary guard so `unclosed #288` is
  not a match, and case-insensitive verb matching. The one sanctioned spot
  is a closing trailer on the final non-empty line; everything else is a
  violation.
- **Discussing closure means escaping the reference or using a full URL**
  (`escaped_reference`, `url_reference`). `&#35;288` and
  `https://github.com/owner/repo/issues/288` render as references to
  humans and carry no closing semantics to the platform; `ReferenceForm`
  classifies each directive `Bare` / `Escaped` / `Url` and only `Bare`
  (`is_live()`) is a live directive. Escaped-entity matching requires the
  semicolon (`&#3512` is entity 3512, not entity 35 plus `12`) and
  accepts hex (`&#x23;288`).
- **A `Refs` decision asserts the issue is still open after merge**
  (`verify_after_merge`). The converter's trailer is a claim about the
  issue's post-merge state; a `Refs` body with the issue observed `Closed`
  is `PostMergeAction::ReopenIssue`, not a silent success, and a `Closes`
  body with the issue still `Open` is `ReportTrackerLag` — each renders a
  `PostMergeCheck::line()` that fails loudly instead of aging into
  "fixed" (the merged-vs-fixed discipline of the Closeout report applied
  to issue state).
- **The contradiction is linted before publish** (`lint_refs_body`,
  `pre_publish_lint`). Any live closing directive in a body the converter
  marked `Refs` is rejected by `lint_refs_body`; `pre_publish_lint` gates
  both decisions — `Closes` requires the closing trailer and no prose
  violations, `Refs` requires no live closing directives at all — so the
  check runs before the body is written, not after the merge.

Checkable in `autospec_core::prose_closure` (`find_closure_directives`,
`ClosingDirective`, `ReferenceForm`, `prose_violations`,
`escaped_reference`, `url_reference`, `has_closing_trailer`,
`is_trailer_line`, `lint_refs_body`, `pre_publish_lint`,
`verify_after_merge`, `PostMergeCheck`). Regression tests reconstruct the
incident body (`Refs #288 (does not close it)` on line 1, `closed #288`
on line 11) and assert it: `crates/autospec-core/tests/prose_closure.rs`.
