---
name: The pre-commit lint gate shapes HOW work must be committed - plan the split before writing
description: PR_SIZE's logical_units cap binds before the line cap, every source commit needs its own doc touch, oversized files may not gain a line, and wrapping bats `run` in a helper defeats ASSERTION_DENSITY
type: feedback
wing: synthesis
drawer_class: lesson
originSessionId: e10ccb2a-958b-4bd4-b423-d8b67cdf9582
---
The `.git/hooks/pre-commit` → `lint-implementation-gates.sh` gate rejects commits
on rules that constrain commit *shape*, not code quality. Discovered the hard way
while landing ~1000 lines of model-routing work (2026-08-05): eleven rejected
attempts across two sessions before the split was right.

**Why:** each rule is individually reasonable but they interact, and the binding
constraint is usually NOT the one you expect. Retrofitting a split after writing
costs several 3–5 minute gate runs per attempt.

**How to apply** — decide the commit split *before* writing:

1. **`PR_SIZE`: `logical_units` (cap 3) binds before `changed_lines` (cap 400).**
   A 334-line, 4-file commit was rejected for `logical_units=4/3` alone. Units
   look like distinct path groups (`scripts/`, `tests/`, `install.sh`,
   `README.md` = 4). Group paths, don't just count lines. Also `raw_files` cap 8.
   A ~1000-line feature needs ≥4 commits; plan them up front.

2. **`DOC_OUT_OF_SYNC`: every commit containing a non-test source file with a
   `--long-flag` or an `UPPER_SNAKE=` assignment needs a doc file in the SAME
   commit** (`README*`, `AGENTS.md`, `docs/**`, `SKILL.md`). Test files are
   exempt (`is_test_file` → skip), and touching any one doc file satisfies the
   whole commit. Note it fires on flags in *strings too* — an `nvidia-smi
   --query-gpu=…` call inside a shell lib tripped it. Consequence: you need a
   separate small doc touch per source commit, and since `git add` is per-file
   you cannot reuse one doc file's hunks across two commits.

3. **`FILE_GROWTH`: a file already over 400 LOC may not gain a single line.**
   `install.sh` is 1995 LOC, so adding a manifest entry required appending to an
   existing line (the manifest is `for entry in $var` word-split, so two entries
   on one line is valid) and editing a stale count in place. New files over 400
   LOC fail outright — split into a CLI + a `scripts/lib/` module.

4. **`ASSERTION_DENSITY` / `VACUOUS_NO_ASSERT` match `\b(assert|expect|run|grep|
   check|verify)\b` per `@test` block.** **NEVER wrap `run` in a bats helper.**
   Any wrapper name — `run_probe`, `add`, `score`, `decide` — hides the token
   (`_` is a word char, so `run_probe` fails `\brun\b`; a helper called from the
   test body puts `run` in the *helper*, not the block). This cost four separate
   rejected commits across two sessions. Write `run <cmd>` literally at every
   call site, however verbose. `[ ... ]`/`[[ ... ]]` alone never satisfies it.
   Also `VACUOUS_OR_TRUE` flags any trailing `|| true` — `unset FOO || true` in a
   setup helper trips it.

5. **`COMPLEXITY` also flags a duplicate function name across the commit's
   changed files — including two awk programs in the SAME file** defining the
   same helper. Renaming one is the cheap fix; extracting a single parse that
   serves both callers is the better one. Functions are capped at 50 LOC too.

**Bats gotcha unrelated to the gate but hit in the same work:** `run` resets
`$output`, so capture any JSON you need to assert on twice into a local variable
*before* the next `run`, or the later assertion silently reads the wrong output.

Related: [[feedback_installer_excludes_runtime_libs]] (a new `scripts/lib/`
runtime lib must ALSO be added to install.sh's `runtime_libs` allowlist, else it
never ships), [[feedback_skill_golden_derivation_workflow]],
[[feedback_decompose_trio_prose_goldens_atomic]] (ordering: prose may not land
before the flag it documents exists, and a prose-contract test may not land
before the prose).
