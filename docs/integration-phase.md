# Integration phase (issue #3565)

Gate conflict resolution on **symbol preservation**, not on a green build.

A conflict resolution that takes only one side compiles, passes the test
suite, and ships — while silently dropping the other side's work. A
build-and-test gate cannot see that: the dropped side's feature simply never
gets exercised. The integration phase therefore gates every landing on a
property a build can never check:

> **The union of the symbols added by both parents of a conflict must be
> present in the tree after resolution.** A resolution that drops one is
> rejected with the dropped symbol named.

## Command

```
autospec integrate --repo <path> --base <trunk> --branch <b> [--branch <b> ...]
                   [--verify-cmd '<POSIX sh command>'] [--json]
```

- Rebase and land a batch of branches onto `--base` (the trunk) in the order
  the `--branch` flags are given (dependency order: dependencies first).
- After each branch is rebased onto the current trunk, the verify command is
  run in the repository via `/bin/sh -c`. It must exit 0 before the branch is
  landed (fast-forward merge). A failure names the branch in the report and
  blocks that branch's landing; the rest of the batch stays landable.
- Exit codes: `0` every branch landed; `2` usage error, or at least one
  branch halted or verification failed (the report names which and why).

```
$ autospec integrate --repo /tmp/repo --base main --branch feat/a --branch feat/b \
    --verify-cmd 'cargo test --workspace'
  feat/a: landed
  feat/b: halted: semantic conflict in src/s.rs (both sides rewrote the hunk at line 2); the rest of the batch remains landable

integration: 1 of 2 branch(es) landed
```

## How a conflict is handled

1. `git rebase --no-ff --no-edit` (behind the `Vcs` trait, so the phase is
   testable without a git binary). The invocation **never** passes
   `--skip`, `--edit`, or `--abort`; there is no skip path in the
   trait — a conflict cannot be skipped by any caller.
2. The conflicted files are read with `git diff --diff-filter=U --diff3`, so
   each hunk carries the **ancestor**, the **trunk (ours)** side, and the
   **branch (theirs)** side.
3. Classification:
   - **Additive** — neither side deleted an ancestor line (both sides only
     added). The built-in union resolution keeps the trunk file and splices
     in the branch side's added lines, then the resolution is checked by the
     preservation gate before it is accepted.
   - **Semantic** — a side deleted or rewrote an ancestor line, or the
     gate rejects the resolution. The branch is **halted** with a report of
     every conflicted file and hunk; the rebase is aborted and the batch
     continues with the next branch (the trunk has only ever advanced with
     verified, landed branches, so the rest stays landable).
4. The preserved tree is written, `git add -A`-ed, and the rebase continues
   (`git rebase --continue`) until it finishes clean.
5. The verify command runs (re-verification after every rebase; a failure
   names the branch). On success the branch is fast-forward landed.

## The preservation gate

`autospec_core::integration::symbols`:

- `extract_symbols(text)` — identifier tokenizer (ASCII runs, length ≥ 2,
  language-keyword stoplist, case-sensitive). Deliberately cheap: it catches
  a dropped struct field, function, or guard — exactly what a green build
  cannot.
- `side_additions(ancestor, side)` — multiset diff: the lines `side` added
  relative to `ancestor`.
- `check_preservation(conflicted, resolution)` — for every symbol added by
  either side of any hunk, assert it is present in the resolved content.
  Missing symbols are reported as `MissingSymbol { file, symbol }`.
- `classify_and_gate` — additive conflicts get the union resolution and are
  then gated; a gate failure or a semantic classification is a
  `GateFailure` (the halt).

When the batch is fully processed, the phase calls `Vcs::settle`: no rebase
is left in progress and the repository is checked out on the trunk — even
when the last branch halted. A halt never leaves the working tree sitting
on the halted branch.

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | every branch in the batch landed |
| `2` | usage error, or at least one branch halted (semantic conflict, gate rejection, verify failure, or rebase error); the report names each |

## Tests

- `crates/autospec-core/tests/integration_phase.rs` — fake-VCS coverage:
  dependency-order landing, additive union, drop-side rejection (both
  directions), semantic halt with the rest of the batch landing, verify
  failure naming the branch, rebase-error reporting, and the
  no-`--skip` invocation guarantee.
- `crates/autospec-core/tests/integration_phase_git.rs` — real-git e2e:
  additive conflict lands with both sides' content on trunk; semantic
  conflict halts and reports the hunk while the rest lands; verify failure
  names the branch and blocks the land.
- `crates/autospec-core/src/integration/` — unit tests for marker parsing
  (diff3, including unterminated-marker rejection), symbol extraction,
  multiset diff, preservation gate, and classification.
