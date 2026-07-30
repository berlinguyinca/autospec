# Runtime session terminal foreground implementation plan

**Goal:** Allow interactive runtime-session children to read their controlling
terminal while preserving process-group cleanup and restoring the caller afterward.

## Tasks

1. Add a PTY-backed regression to `crates/autospec-cli/tests/runtime_terminal.rs`.
   Run it alone and confirm it times out before the production change.
2. Add Unix foreground capture, child handoff, and restoration to
   `crates/autospec-cli/src/commands/runtime/env/worker.rs`.
3. Carry the restoration guard through
   `crates/autospec-cli/src/commands/runtime/env/session.rs` and combine any restore
   error with the existing session result.
4. Re-run the focused test, runtime-session tests, formatting, Clippy, the workspace
   suite, and `autospec validate`.
5. Review the diff, commit with issue evidence, open and merge the PR after CI, then
   rebuild and atomically reinstall the local Autospec binary.
