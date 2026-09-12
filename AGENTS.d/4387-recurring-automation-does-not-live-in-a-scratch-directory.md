# Recurring automation does not live in a scratch directory (issue #4387)

Operational logic the team depended on every day lived in a scratch
directory, and vanished.

The patch-to-PR conversion pass — selecting candidate patches, applying
them, running the gate, opening PRs, recording HELD reasons — was a set of
shell scripts in a session temp directory. It encoded real accumulated
knowledge: which failures are worth holding vs discarding, how to extract
the true failing-test set, which conflicts can be safely unioned. None of
it was in a repository. The same directory also accumulated ~60 git
worktrees and dozens of ad-hoc logs, so the real tooling was
indistinguishable from debris. When the scratch directory was cleared the
capability was simply gone, along with every lesson embedded in it.

Automation that grows inside a debugging session is never "the
deliverable", so it is never committed. Each increment is a two-line fix to
something that already exists, which never feels like the moment to start
a project. The result is a load-bearing system with no home, no tests, no
review, and no backup.

- **Automation that will run more than once does not live in a temp
  directory.** As soon as a script is invoked a second time on a
  different input, it moves into a repository with a test — or it is
  deliberately thrown away after use. "Keep fixing it in `/tmp`" is not a
  third option: the second invocation is the moment, and a scratch copy
  with no committed twin is a capability with a countdown.
- **Anything an agent writes to a scratch path has a lifetime of one
  task.** If it must outlive that task, committing it is part of finishing
  the task, not a follow-up — a "follow-up" is exactly when the temp
  directory gets cleared.
- **A spec for a recurring operational process must name where the
  implementation lives and what tests it carries.** "A script that does X"
  with no home is a design defect. The same pass settles the language: a
  process that will be maintained belongs in the project's implementation
  language, not in whatever was fastest to type — the fastest-to-type
  script is the one that dies with the session.
- **Debris does not excuse the tooling.** A scratch directory full of
  worktrees and ad-hoc logs is where real tools are *least* visible;
  indistinguishability from debris is an argument for committing, not for
  leaving things where they are.

The ratchet for invocation-side growth already exists:
`scripts/lint-scratch-promotion.sh` (RULE_ID `SCRATCH_PROMOTION`, issue
#3977) flags a helper invoked from a scratch path more than twice in the
repo's own shell and bats corpus. This invariant closes the spec side: a
spec that names a scratch tool path as where the work lives is a design
defect at review time, before the second invocation ever happens.

Checkable in `autospec_core::scratch_home` (`SCRATCH_HOME_RULE_ID`,
`SCRATCH_PREFIXES`, `TOOL_EXTENSIONS`, `is_scratch_path`,
`is_scratch_template` — an mktemp template has a one-task lifetime and is
exempt, `scratch_tool_paths`, `is_repo_home_candidate`,
`is_test_candidate`, `lint_spec_scratch_home`,
`ScratchHomeFinding::missing`). Tests:
`crates/autospec-core/tests/scratch_home.rs`, including the incident
end-to-end: a spec for the conversion pass whose scripts live under
`/tmp/conv/` names no home and no test; the same spec promoted to
`scripts/` homes with a `tests/` pin is clean; a home without a test still
names the missing test; the one-shot script carries
`linter:allow-SCRATCH_HOME <reason>`, and a bare marker is rejected.
