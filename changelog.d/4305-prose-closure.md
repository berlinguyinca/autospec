### Added

- `autospec_core::prose_closure`: prose closure safety for PR bodies
  (issue #4305). A PR body marked `Refs #N` that also contained
  `<closing-verb> #N` in prose closed the issue on merge even though the
  trailer said it would not. Encodes the converter's rules as checkable
  primitives — `find_closure_directives` / `ClosingDirective` (a closing
  verb adjacent to a reference, classified `Bare` / `Escaped` / `Url` via
  `ReferenceForm`), `prose_violations` (closing directives anywhere in the
  body except a closing trailer on the final non-empty line — invariant 1),
  `escaped_reference` / `url_reference` (the safe ways to discuss closure,
  invariant 2), `verify_after_merge` / `PostMergeCheck` (a `Refs` decision
  with the issue observed closed is `ReopenIssue`, never silent —
  invariant 3), `lint_refs_body` (rejects a live `Bare` closing directive
  in any body the converter marked `Refs` — invariant 4), and
  `pre_publish_lint` (the machine-detectable contradiction gate, run before
  publish, for both `Closes` and `Refs` decisions). Regression tests
  reconstruct the incident body (`Refs #288 (does not close it)` on line 1,
  `closed #288` on line 11) and assert it.
