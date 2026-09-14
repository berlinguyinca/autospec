# A red test is a hypothesis about the product, not a verdict on it (issue #4534)

A converted patch landed a test that failed on main:

```
dry_run_tree_stays_byte_identical ... FAILED
  "a dry-run must leave the tree byte-identical (path, size, mtime)"
```

The obvious response — the one announced — was to revert. The patch had
been held in an earlier pass for exactly this failure, so reverting looked
like restoring a decision. Reading the test first changed the answer.
`snapshot_tree` walked the whole tree **including `.git`**. The dry-run
reads repository state by invoking git, and git rewrites `.git/index` as a
side effect of being read. The assertion said *reading a repository does
not touch the repository* — false, and false for reasons that have nothing
to do with the command under test. The command was correct; the test was
over-strict by one exclusion.

Two tests were added together and only one failed.
`dry_run_leaves_git_status_unchanged` passed throughout, covering the
tracked-content half of the same property. One failing and one passing,
asserting nearly the same thing, points at what differs between them —
repository internals versus tracked content. That took one read; the
revert would have discarded 719 lines of working feature.

- **A failing test is a hypothesis about the product, not a verdict on
  it.** Before reverting the change that introduced it, read the assertion
  and decide which side is wrong. A revert is correct when the *product*
  is wrong; when the *test* is wrong, a revert deletes working code and
  leaves the bad assertion pattern free to recur in the next patch that
  copies it.
- **Two tests asserting overlapping properties, one failing: the
  difference between them is the diagnosis, usually one read away.**
- **A test that snapshots a directory tree excludes version-control
  metadata by default.** Reading a repository mutates it; asserting
  otherwise is a defect in the assertion, and it recurs wherever a test
  compares trees.
- **A hold or revert decision on a failing test records which side was
  judged wrong — product or assertion** — so the reasoning is auditable.
- **Reverting is not the default response to a red test introduced by a
  merge.** The default is a one-read diagnosis of which side is at fault.

For specs and agent prompts: when a gate holds a patch on a failing test,
the recorded reason must state the failing assertion and the judgment
(product wrong / assertion wrong / environment wrong). An agent instructed
to "revert what broke" without that judgment step will revert working
feature on an over-strict test.

Checkable in the tree-snapshot helpers themselves, which now exclude
`.git` by construction: `snapshot_tree` in
`crates/autospec-cli/src/commands/cleanup.rs` (the original fix),
`crates/autospec-cli/tests/autonomous_conductor_commands.rs`, and
`crates/autospec-cli/tests/autonomous_accountability_github/contracts.rs`.
Regression test:
`tree_snapshot_excludes_version_control_metadata_by_construction` in
`crates/autospec-cli/tests/support/autonomous_recovery_accountability.rs`
— it snapshots a tree with a live `.git`, rewrites `.git/index` the way a
git read does, and asserts the snapshot is unchanged while the working
tree is still recorded.
