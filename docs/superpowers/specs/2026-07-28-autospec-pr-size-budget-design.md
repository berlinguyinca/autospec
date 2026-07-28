# Autospec PR Size Budget Design

## Objective

Keep every Autospec implementation pull request small enough to review and
validate quickly without stopping autonomous work or discarding completed work.

## Size contract

An implementation PR may change at most:

- 400 total diff lines, counted as additions plus deletions;
- 8 raw files; and
- 3 logical units.

Logical units use the existing issue-sizing rules: a multi-harness skill trio
and its derived goldens count as one unit. Binary changes are oversized because
their line count cannot be proved.

The proactive continuation threshold is 320 changed lines or 7 raw files.
Autospec checks the budget after each implementation checkpoint and immediately
before draft creation and merge.

## Continuation behavior

When an implementation reaches the proactive threshold with acceptance criteria
still unfinished, Autospec freezes the completed slice and opens it as a capped
PR. It creates ordered continuation issues for the unmet criteria, each with an
exact `Depends on` link, implementation outline, tests, and remaining acceptance
criteria. Continuations start from clean isolated worktrees after their
dependencies merge.

The original issue becomes an umbrella. Part PRs reference it with `Part of`
rather than `Closes`. Each continuation PR closes only its child issue. Autospec
closes the umbrella only after every recorded continuation PR is observed
merged. The queue therefore keeps moving without operator confirmation.

If the hard limit is discovered before a draft exists, Autospec must not create
the draft. It preserves the oversized branch as a non-mergeable checkpoint,
records an oversize receipt, and decomposes the remaining work. Commits may be
reused only when their boundaries independently satisfy the cap; otherwise the
children reapply the work from a clean base. No branch or unpushed work is
deleted during splitting.

## Enforcement

One deterministic `PR_SIZE` policy supplies the same verdict to:

1. issue/decomposition sizing;
2. the implementation checkpoint;
3. draft-PR admission;
4. independent review; and
5. the final merge gate.

A hard-limit finding blocks remote draft creation and merge. It emits measured
line, raw-file, and logical-unit counts plus a deterministic split directive.
The shell and Rust paths must have parity tests for the same boundaries.

## Exceptions

The only exception syntax is:

```text
Guardian: skip-PR_SIZE # <reason>
```

The reason must identify one of:

- a generated migration;
- dependency-solver lockfile output; or
- mandatory lock-step artifacts such as harness mirrors and derived goldens.

Manual implementation or test code is never exempt. A syntactically valid
exception produces an auditable `INFO:PR_SIZE` record and the independent
reviewer must explicitly accept the stated category. Bare, unknown, or
over-broad reasons remain blocking.

## Recovery and notifications

Continuation state is durable and idempotent. Restarting after issue creation,
draft creation, or a merged child must reuse the recorded child instead of
filing duplicates. The session receives notifications when Autospec approaches
the threshold, splits work, blocks an invalid exception, creates a continuation,
or completes the umbrella.

## Verification

Tests must prove:

- 400 lines, 8 files, and 3 logical units pass;
- 401 lines, 9 files, and 4 logical units fail;
- 320 lines or 7 files with unfinished criteria creates continuation state;
- open/draft and merge gates share the same verdict;
- only the three exception categories are accepted;
- restart does not duplicate continuation issues or PRs; and
- the umbrella closes only after every child PR is observed merged.

The repository-wide `autospec validate` suite must pass with no skipped required
checks before the final child merges.
