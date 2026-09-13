# Line-buffered progress for the conversion pass (issue #4572)

`autospec convert --apply` wrote its per-patch decisions to stdout, and with
stdout on a pipe or a file — the normal case for anything scheduled — Rust
block-buffered them. A working pass looked dead: four consecutive "zero
decisions" status reports, two wrong hang diagnoses, and one 20-minute gate
run discarded, all read from a log whose lines were sitting in an unflushed
buffer while the HELD ledger on disk already had the answers.

Every operator-facing decision is now printed **and flushed** the moment it
is made (`convert::progress`):

- a startup banner names the held-ledger path, so the operator reads the
  durable record instead of the buffer;
- `  GATE  #<issue>: <scope>` when a patch's gate begins, naming the derived
  scope — a verdict can take 11+ minutes, and silence in that window reads
  as a hang;
- `  GATE  #<issue>: <stage>` per stage (fmt / build / clippy / test), so the
  longest stage cannot read as a stall;
- `  CONVERT  #<issue> <key>` and `  HELD  #<issue>: <reason>` at the moment
  the decision is made, not at exit.

The lines are plain `println!` + `flush` — no terminal detection, because
terminal-only flushing is backwards: it works when a human is watching and
fails when nobody is. In `--json` mode the banner moves to stderr; stdout is
reserved for the machine-readable plan.

Five git helpers (`teardown_worktree`, `run_capture_in`, `run_git`,
`run_git_in`, `run_git_capture`) move from `convert.rs` into
`convert::git`, keeping the file on its shrink side of the ratchet.

An integration test (`convert_progress_flush`) runs a real pass against a
minimal fixture crate with stdout on a pipe and a reader thread: it asserts
the GATE line arrives **while the process is still running** (a
`try_wait` at the moment of receipt, not a check on the final output), that
the banner names the ledger, and that the hold decision and summary follow
— the shape that would hold if the lines were buffered until exit.
