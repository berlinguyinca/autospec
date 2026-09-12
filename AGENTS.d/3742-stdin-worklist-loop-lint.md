# Stdin-worklist loop lint (issue #3742)

A `while IFS= read -r x; do …; done < worklist` loop runs its entire body with
stdin pointed at the worklist file. Any command in the body that *reads stdin*
(`gh`, `ssh`, `cargo`, a `$(…)`, or `git`'s stdin-reading subcommands
`commit`/`apply`/`fast-import`/`hash-object`/`rebase`/`am`) consumes the file,
so the loop silently processes one candidate and exits 0. The fix is a
dedicated file descriptor: `while IFS= read -r x <&3; do …; done 3< worklist`.

`scripts/lint-stdin-worklist-loops.sh` is the deterministic ratchet (RULE_ID
`STDIN_WORKLIST_LOOP`). It is a read-only, single-pass, token-level AWK scanner
over shell files. Default corpus is `scripts/` and `skills/`; pass explicit
`PATH…` args, or `--root DIR`, to sweep another tree. For each `while`/`for`/
`until`/`select` loop whose `done` reads its worklist from FD 0 (`done <`, but
not `done 3<`/`done <&3`/`done < <(…)`), it scans the *direct* body for a
stdin-consuming command at command position — including inside `$(…)` (a command
substitution inherits the loop's stdin, verified empirically) and behind a
`GH_*`/`GIT_*`/`CARGO_*`/`SSH_*` variable alias or a `sudo`/`time`/`nice`/
`nohup` launcher. A nested loop with its own input is not charged to the outer
loop. A loop whose `git` subcommand does not read stdin (`log`, `ls-files`,
`rev-parse`, `ls-remote`, `grep`, …) is not a finding. Finding: `STDIN_WORKLIST_LOOP:<path>:<line>: <kind> loop (started line <N>) runs '<cmd>' at line <M> while reading its worklist from stdin; read the worklist on a dedicated FD (done 3< worklist, read <&3)`. `--list` emits an `FD0_LOOP:<path>:<line>: …` audit line per FD-0 loop and always exits 0; blocking exit code = finding count (capped at 64).

The counting half of the fix lives in the Rust conversion policy:
`autospec_core::execution::patch_pipeline::reconcile_phase_counts(candidates_produced, candidates_processed)` (rule #17 in the module) is the pure, testable primitive that the patch-to-PR conversion pass calls after its expensive-gating phase; it returns an Ok summary on equality and an `Err` (warn + non-zero exit at the caller) when processed ≠ candidates. Bats: `tests/lint-stdin-worklist-loops.bats`, fixtures under `tests/fixtures/lint-stdin-worklist-loops/`.
