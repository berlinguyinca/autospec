### Changed

- Issue contract now names the implementation surface (issue #4439): the
  decomposer contract requires an **Implementation surface** section (crate +
  module, or an explicit script justification) alongside the existing
  **Implementation language** section, `AGENTS.md` states the default as a
  rule (new logic is the repository's implementation language; shell is for
  process supervision and harness entry points only; a PR that adds shell
  lines states why), and the Phase 4 implementer prompt binds the issue's
  named language over matching local style. The three shell-default patches
  from the #4439 table (#3833, #3832, #3820) were re-dispatched with the
  surface named; the two `gen-issue-skeleton` ones now target the Rust
  generator (`crates/autospec-core/src/issue_skeleton.rs`) that #4440 landed.
