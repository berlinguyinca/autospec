feat(evaluation): hash-chained idempotent event journal

Closes #3554

Adds `crates/autospec-core/src/evaluation/store/journal.rs` — the hash-chained,
checkpointed event log keyed for idempotent appends — and declares the module in
`evaluation/store/mod.rs`. Includes the cherry-picked #3553 dependency
(`store/io.rs`, `store/layout.rs`, store error plumbing) rebased onto current `main`.

Chain rule: `chain(i) = sha256(chain(i-1) || 0x00 || line(i))`, seeded with
`sha256("autospec-evaluation-journal-v1")`. `Journal::open` replays from the seed and
verifies against `events.checkpoint.json`, failing closed (`EvaluationErrorKind::Integrity`)
on tampering, sequence gaps, duplicate keys, torn trailing lines, checkpoint/schema
mismatches, and journal/checkpoint divergence. `append` dedupes by idempotency key
(`false` on identical reuse, `Integrity` on conflicting reuse), appends the line
durably (`append_synced_line`), then atomically rewrites the checkpoint.
`append_with_fault` exercises torn-write rollback (rollback via `set_len`, no state change).

## Closeout report

**Result** — `evaluation::store::journal` shipped: `Journal` / `JournalEvent` /
`Checkpoint` with seed-verified chain replay, idempotent append, crash-consistent
checkpoint, 7/7 unit tests green; module declared in `store/mod.rs`.

**Claims**
- [verified] (runtime) `cargo test -p autospec-core --lib evaluation::store::journal` → 7 passed, 0 failed, incl. all four issue-required tests (idempotent dup → false; conflicting key → Integrity; edited committed line → open Integrity; torn `fail_after=5` → rollback, hwm 1, key absent).
- [verified] (runtime) `cargo test -p autospec-core --lib` → 561 passed, 0 failed.
- [verified] (runtime) `bash scripts/lint-implementation.sh --pre-commit --staged` → exit 0 (advisory INFOs only: file LOC and nesting on `format!` blocks).
- [verified] (static) `grep -c '\bf64\b' journal.rs` → 0; `wc -l` → 514 (< 600).
- [verified] (runtime) `cargo fmt --check` clean; `cargo clippy -p autospec-core --all-targets` exit 0 (no journal findings); `bash scripts/validate-agentic-rag.sh --no-tests` → OK.
- [verified] (runtime) The 6 failing workspace test targets (`runtime_resources`, `validation_runner`, `issue_commands`, `managed_project`, cli lib sandbox tests) fail identically on a clean `cd308647` worktree — pre-existing environment failures, not caused by this change.
- [assumed] `autospec evaluation doctor --json` acceptance line targets the post-Task-15 (CLI wiring) state; the `evaluation` subcommand does not exist on `main` yet, so it exits 2 ("unknown autospec command").
- [couldnt-verify] `scripts/check-generated-artifact-drift.sh` and `scripts/check-doc-links.sh` (issue Validation list) do not exist in this repo; `validate-agentic-rag.sh` has no `--changed-scope`/`--brief` flags (template boilerplate in the issue).
- [verified] (runtime) `gh` has no GitHub auth in this environment → PR could not be opened by the agent; branch pushed to `fix/issue-3554-retry` instead of force-pushing over the diverged stale remote `fix/issue-3554` (yesterday's attempt, based ~250 commits behind current `main`).

**Before/after** — journal tests 0 → 7; `evaluation::store` journal coverage: absent →
full idempotency + chain-integrity + crash-rollback matrix.

**Artifacts**
- `crates/autospec-core/src/evaluation/store/journal.rs` (514 lines, 7 tests)
- `crates/autospec-core/src/evaluation/store/mod.rs` (+`pub mod journal;`)
- Re-run: `cargo test -p autospec-core --lib evaluation::store::journal`

**Scoped git status** — added `crates/autospec-core/src/evaluation/store/journal.rs`;
modified `crates/autospec-core/src/evaluation/store/mod.rs` (branch `fix/issue-3554`,
pushed as `origin/fix/issue-3554-retry`; PR base `main`).

**One likely hidden failure** — a crash *after* the journal line is synced but *before*
the checkpoint rewrite leaves the journal ahead of the checkpoint; next `open` fails
closed (Integrity) as designed, but there is no automatic repair path yet — recovery
is manual until Task 11's transactional layer (per design §Data Model, intended
fail-closed behavior).
