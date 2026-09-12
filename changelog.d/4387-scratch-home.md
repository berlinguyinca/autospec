### Added

- `autospec-core::scratch_home` — primitives for the invariant that
  recurring automation does not live in a temp directory (issue #4387: the
  patch-to-PR conversion pass — candidate selection, apply, gate, PR open,
  HELD reasons — lived as shell scripts in a session temp directory beside
  ~60 git worktrees and ad-hoc logs, and vanished with it, taking every
  embedded lesson with it). A spec that names a scratch tool path — a
  literal `/tmp/`, `/var/tmp/`, `/private/tmp/`, `${TMPDIR}/` or `$TMPDIR/`
  path with a recognized tool extension — must also name a
  repository-relative implementation home and a test under a `tests`
  segment, or `lint_spec_scratch_home` reports `SCRATCH_HOME` naming the
  scratch paths and exactly what is missing (home, test, or both). mktemp
  templates (`XXXXXX`/`$$`) are exempt: a unique-per-invocation scratch
  file is a correct one-task lifetime. A `linter:allow-SCRATCH_HOME
  <reason>` line suppresses the finding for a deliberately one-shot
  script; a bare marker is rejected. Closes the spec side of the
  scratch-promotion ratchet (`scripts/lint-scratch-promotion.sh`, #3977),
  which covers invocation-side growth.
