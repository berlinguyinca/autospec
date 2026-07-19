# Closeout Report

**Result** - Added the hello command and smoke coverage.

**Claims**

- [verified] `src/hello.sh` prints `hello autospec`.
- [verified] `bats tests/hello.bats` passes.

**Proof type** - runtime.

**Before/after** - before: no hello command; after: one command and one smoke test.

**Artifacts** - `src/hello.sh`, `tests/hello.bats`, command: `bats tests/hello.bats`.

**Scoped git status** - `src/hello.sh`, `tests/hello.bats`.

**One likely hidden failure** - The command may not be executable on a filesystem that dropped mode bits.

