# Making the implementation-quality gate satisfiable

**Date:** 2026-08-05
**Status:** design — awaiting approval
**Scope:** three defects in `scripts/lint-implementation.sh` that make the
pre-commit gate reject refactors it simultaneously demands, plus a bootstrap
order that fixes them without a single bypass.

## 1. Problem

`COMPLEXITY` tells you to split oversized files. `PR_SIZE` caps a commit at 400
changed lines. Moving a line counts twice — once as a deletion, once as an
addition — so the two rules constrain each other. Above a certain file size they
become **mutually unsatisfiable**, and the file can never be brought into
compliance by any sequence of commits.

This was found by attempting the split the gate asked for. The refactor was
completed and verified byte-identical, and could not be committed.

### 1.1 The satisfiability bound

`COMPLEXITY` evaluates **changed files**, so every intermediate commit is judged
on the file's absolute size. A staged split therefore fails at each step until
the final one, which means the file must cross under the limit in a *single*
commit.

For a file of `L` lines, with `D` deleted and `A` added in that commit:

```
file-LOC:  L - D + A <= 400      =>  D - A >= L - 400
PR_SIZE:   D + A <= 400
```

Substituting the first into the second:

```
(L - 400 + A) + A <= 400   =>   A <= (800 - L) / 2
```

Consequences:

- **`L >= 800`: no solution exists.** No commit, and no sequence of commits, can
  bring the file under 400.
- `L = 739` (`autospec-autonomy-v2-lib.py`): `A <= 30`. But the `<=50` LOC
  per-function rule *forces* additions — splitting `worker_recipe` (58 LOC) and
  `autonomy_status` (67 LOC) requires new `def` lines. Measured actual: `A = 46`,
  `D = 394`, 440 changed. No compliant arrangement exists for this file either.

### 1.2 Measured blast radius

| Population | Count |
|---|---|
| Gate-checked files over 400 LOC | 202 |
| Of those, over 800 LOC — permanently unfixable | 51 |
| Largest | `crates/autospec-cli/src/commands/autonomous/executor_bridge.rs`, 49,481 LOC |

Verified directly: a one-line diff against `crates/autospec-cli/src/commands/claim.rs`
(11,153 LOC) produces
`COMPLEXITY:…: file is 11153 LOC (AUTOSPEC_MAX_FILE_LOC=400); split into smaller modules`.

### 1.3 The gate is a growth-only ratchet

The gate runs **only** as a local pre-commit hook (`.git/hooks/pre-commit`,
shared across worktrees). No workflow in `.github/workflows/` invokes
`lint-implementation.sh`, so the GitHub merge path never encounters it.

The most recent commit on `main`, `cba8d65e` (#2950), added **218 lines to the
49,481-line file**. That commit would be rejected outright by the hook, and it
landed through the merge path.

So the gate:

- does **not** prevent oversized files from growing (merges bypass it), and
- does **prevent** them from shrinking (local refactors are rejected).

It permits only the direction that makes the problem worse. This is why the debt
reached 49k lines in one file while the tool nominally forbids 400.

### 1.4 Defect: cyclomatic proxy counts per file, compares per function

`check_cyclomatic` prefers `radon` and falls back to a keyword count. The
fallback's own comment reads *"count decision points per function"*. The
implementation counts **the whole file**:

```bash
count="$(grep -cE '^\s+(if |elif |else:|for |while |case |except |catch )' "$diff_file")"
[ "$count" -gt "$_COMPLEXITY_MAX_CYCLOMATIC" ]   # threshold 10, meant per function
```

The threshold is a per-function value applied to a file-wide total, so the rule
**punishes decomposition** — precisely the remedy `COMPLEXITY` prescribes. A
module of eleven single-branch functions fails; one function with ten nested
branches passes.

Observed: a newly extracted module with 13 branches across 6 small functions was
rejected, while the original 739-line lib scores **46** and passes only because
it is never in a diff.

`radon` would make this accurate. It is referenced **only** inside
`lint-implementation.sh` and is not declared as a dependency anywhere in the
repo, so the inaccurate fallback is the de-facto path.

### 1.5 Defect: `PR_SIZE` has no refactor exception

`PR_SIZE` supports `Guardian: skip-PR_SIZE # <category>: <detail>`, validated
against a fixed set: generated migrations, dependency-solver lockfiles, and
mandatory lock-step artifacts. A file split — the action `COMPLEXITY` instructs
the author to perform — has no category, so there is no in-band way to declare
one. The remaining routes are an env threshold override or the git flag that skips
hooks entirely, both out-of-band bypasses.

### 1.6 Defect: `VACUOUS_TAUTOLOGY` matches `sys.exit(`

The tautology pattern includes the Jest/Mocha skip function `xit(` **unanchored**,
so it also matches any identifier ending in `xit` before a paren. `sys.exit(1)`
and `raise SystemExit(1)` both contain the substring and are reported as
"Tautological assertion — always passes regardless of code under test."

Verified: `printf 'sys.exit(1)\n' | grep -cE 'xit\('` returns 1.

Every changed Python or JavaScript file with an exit call is blocked by a finding
that cannot be acted on, and the documented `# linter:allow-` escape hatch does
not reach this rule — `check_file_loc`, `check_function_loc`, `check_cyclomatic`,
and the tautology detector all call `emit_capped` without consulting it.

## 2. Fixes

### Fix 1 — file-LOC becomes a ratchet, not an absolute

For a file already over the limit, fail only when the change makes it **worse**.

- File at or under 400 after the change: pass.
- File over 400 both before and after: pass if `after <= before`, fail otherwise,
  with the message naming both numbers.
- New file over 400: fail as today.

This is the load-bearing fix, and it is **self-unblocking**: shrinking an
oversized file becomes permitted, which is what allows the remaining fixes to
land inside the large files later. It also closes the ratchet — growth is now
what gets blocked, in the one path where the gate actually runs.

`AUTOSPEC_MAX_FILE_LOC` keeps its meaning for files under the limit.

### Fix 2 — cyclomatic complexity is measured per function

Match the implementation to its comment and its threshold: attribute each
decision keyword to the enclosing function and compare each function's count to
`AUTOSPEC_MAX_CYCLOMATIC`. Report the function name and its own count.

Declare `radon` an optional dev dependency and keep preferring it. The corrected
fallback must agree with `radon` in direction on a small fixture set, so a
machine with `radon` and one without do not disagree about whether a commit may
land.

### Fix 3 — `PR_SIZE` gains a validated refactor category

Add `refactor: file split` to the validated exception categories. Validation, so
it cannot be used to wave through unrelated work:

- every changed file is either deleted-from or newly added;
- total additions across **new** files are within 10% of total deletions from
  **existing** files (a move, not a rewrite);
- no file gains net lines.

A commit that fails these checks is not a split and is rejected as today.

### Fix 4 — anchor `xit(`

Match `xit(` only at line start or after a non-identifier character:
`(^|[^A-Za-z0-9_.])xit\(`. Verified: the existing detection test for a real
`xit(` skip still passes, and `sys.exit(` no longer matches.

Separately, route `check_file_loc`, `check_function_loc`, `check_cyclomatic`, and
the tautology detector through the same `linter:allow` consultation every other
rule uses, so the documented escape hatch is real for them.

## 3. Bootstrap order — no bypass required

The fixes live in `scripts/lint-implementation.sh` (1,796 LOC) and
`tests/unit/test_lint_implementation.bats` (878 LOC). Both are over 800, so by
§1.1 neither can be edited under the current rules. The way out does not need an
exception:

1. **Add `scripts/lint-implementation-gates.sh`** — a new, small file
   implementing the corrected file-LOC, per-function cyclomatic, and anchored
   tautology checks, with its own tests. A new file under 400 LOC passes the gate
   as it stands today.
2. **Re-point the hook.** `scripts/install-implementer-precommit.sh` is 94 LOC
   and edits cleanly. The generated hook is untracked, so no rule applies to it.
   The hook runs the new gates file first, then delegates the untouched rules to
   `lint-implementation.sh`.
3. **Now the ratchet is active**, so edits that *shrink* an oversized file are
   permitted. Remove the superseded checks from `lint-implementation.sh` — a
   shrinking change, therefore allowed — and trim its test file the same way.
4. **Finish the autonomy-lib split** that this work was blocking. Until then,
   `tests/unit/test_autonomy_module_parity.bats` keeps the extracted modules from
   drifting away from the lib's still-present copies. The recipe is in §3.1.

### 3.1 Reproducing the autonomy-lib slimming

This was completed and verified once — 391 LOC, every function under 50, artifacts
byte-identical to `origin/main` across ten commands — and could not be committed.
The diff itself cannot be stored in the repo either: at 513 lines it exceeds
`PR_SIZE`, the same rule this spec fixes. It is mechanical to regenerate:

```python
# Delete the functions now living in the extracted modules.
import ast
p = 'scripts/autospec-autonomy-v2-lib.py'
lines = open(p).read().split('\n')
MOVED = ['default_capabilities', 'capability_yaml', 'load_capability_statuses',
         'ensure_capabilities', 'built_in_recipes', 'write_recipe_files',
         'recipe_index', 'detect_stack', 'stack_confidence', 'recipes',
         'rule_results', 'find_recipe', 'rule_to_recipe']
spans = sorted(((n.lineno, n.end_lineno) for n in ast.parse('\n'.join(lines)).body
                if isinstance(n, ast.FunctionDef) and n.name in MOVED), reverse=True)
for start, end in spans:                      # swallow trailing blank separators
    e = end
    while e < len(lines) and lines[e].strip() == '':
        e += 1
    del lines[start - 1:e]
open(p, 'w').write('\n'.join(lines))
```

Then, by hand:

1. Add the four module imports (`capabilities`, `recipes`, `rulemap`, `stack`),
   importing only the names still referenced — `load_capability_statuses`,
   `recipe_index`, `find_recipe`, `recipes`, `rule_results`, `rule_to_recipe`,
   `detect_stack`, `stack_confidence`.
2. Delete the lib's now-unused `yaml_scalar`; `slug` is still used by `decompose`.
3. Split `worker_recipe` (58 LOC) into `_worker_refusal_reason` and
   `_worker_execution_markdown` plus a short orchestrator.
4. Split `autonomy_status` (67 LOC) into `_status_data` and `_status_markdown`.
5. Delete `tests/unit/test_autonomy_module_parity.bats` — it would then compare a
   module to itself.

Verify by running both versions over `recipe_index`, `detect_stack`,
`rule_to_recipe`, `decompose`, `patch_plan`, two `worker_recipe` calls,
`evidence_tests`, `rule_recheck`, and `autonomy_status`, and diffing the two
`.autospec` trees.

Each step is independently committable and green. This is the strangler pattern:
a new small module takes over the broken rules, and the oversized original is
reduced once reduction is legal.

## 4. Verification

- Unit tests per corrected rule, including the cases that motivated it: a
  well-decomposed module that today fails cyclomatic, an oversized file shrinking
  by one line, an oversized file growing by one line, `sys.exit(`, and a genuine
  `xit(`.
- A satisfiability regression test: for a synthetic 900-LOC file, assert a
  shrinking commit is accepted. Under today's rules no such commit exists, so
  this test fails before the fix and passes after — the direct encoding of §1.1.
- Re-run the 202 over-400 files through the corrected gate and confirm no
  no-op-change commit is rejected.
- Confirm `radon`-present and `radon`-absent runs agree in direction on a fixture
  set.

## 5. Risk

- **Ratchet semantics accept existing debt.** A 49,481-line file stays legal at
  49,481 lines. That is the point: the gate stops rewarding the merge path for
  growth, and stops punishing the local path for cleanup. Reducing the debt is
  separate, planned work — this only makes it possible.
- **Per-function cyclomatic will surface real findings** currently hidden by
  file-level counting in files that are never in a diff. Expect new findings on
  first touch of large files; they are genuine.
- **The refactor exception is a new hole.** Its validation is mechanical and
  narrow, but it is the first category that is not tied to generated artifacts.
  It should be reviewed as a security-relevant change to a merge gate.
- **CI does not run this gate at all.** Every fix here improves a hook that only
  the local path executes. Whether `lint-implementation.sh` belongs in CI is a
  larger decision and is deliberately out of scope, but until it is answered the
  gate's guarantees apply to agent commits only, not to `main`.

## 6. Out of scope

- Actually reducing the 51 over-800 files.
- Adding `lint-implementation.sh` to CI.
- The `ASSERTION_DENSITY` and `VACUOUS_NO_ASSERT` rules not recognising helper
  functions that wrap `run` — a related false positive, worth its own issue.
- `SECURITY` scanning prose in Markdown. This document was itself rejected for
  naming the hook-skipping git flag inside a sentence explaining not to use it,
  and had to be reworded. Documentation cannot currently discuss the flags the
  gate forbids, which is the same false-positive family as §1.6: a pattern
  matched without regard to whether the file is code.
