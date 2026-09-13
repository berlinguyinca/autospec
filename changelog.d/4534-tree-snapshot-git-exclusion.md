### Fixed

- Tree-comparison helpers in the test suite now exclude `.git` by
  construction (#4534). `snapshot_tree` in
  `crates/autospec-cli/tests/autonomous_conductor_commands.rs` and
  `crates/autospec-cli/tests/autonomous_accountability_github/contracts.rs`
  walked the whole tree including `.git`; since git rewrites `.git/index`
  as a side effect of being read, any test that snapshots a live
  repository twice with a git command in between asserts that reading a
  repository does not touch it — a claim that is false for reasons
  unrelated to the command under test. The same exclusion already existed
  in `cleanup.rs` (added when the incident's failing test was fixed); this
  closes the two copies that kept the defect reachable. A regression test
  (`tree_snapshot_excludes_version_control_metadata_by_construction`)
  snapshots a tree with a live `.git`, rewrites the index the way a git
  read does, and asserts the snapshot is unchanged while the working tree
  is still recorded.

### Documentation

- AGENTS.d note: a red test is a hypothesis about the product, not a
  verdict on it (#4534) — before reverting the change that introduced a
  failing test, read the assertion and record which side was judged wrong
  (product or assertion); where two tests assert overlapping properties
  and only one fails, the difference between them is the diagnosis.
