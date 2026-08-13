---
name: feedback_install_shared_lib_scripts_dir
description: "new skills consuming autospec-shared/scripts helpers must list them in SHARED_LIB_SCRIPT_FILES, not the repo-root SHARED_SCRIPT_FILES group"
metadata: 
  node_type: memory
  type: feedback
  originSessionId: d87fae41-0795-45c0-9afe-909bd9bc37fb
---

A skill's `install.sh` has TWO script groups with DIFFERENT source roots:

- `SHARED_SCRIPT_FILES` → resolved by `resolve_shared_scripts_dir()` = repo-root
  `scripts/` (autospec-usage-limit.sh, lint-*, ci-wait-*, gen-*-prompt.sh).
- `SHARED_LIB_SCRIPT_FILES` → resolved by `resolve_shared_lib_scripts_dir()` =
  `skills/autospec-shared/scripts/` (inject-relevant-memory.sh, emit-gaps.sh,
  and all the `growth-*` / `grow-define-*` / `validate-growth-*` helpers).

**Why:** `resolve_shared_scripts_dir` only points at repo-root `scripts/`. Listing
an autospec-shared helper in `SHARED_SCRIPT_FILES` makes `install_one` fail
`error: missing source file: .../scripts/<name>` → the standalone install dies AND
the root `install.sh --hook-mode` run reports `FAIL: <skill> (claude/opencode/codex)`
(3 pairs), which fails `test_session_start_handler.py` (asserts root install exit 0)
→ `validate.sh` exits 1. The failure surfaces far from its cause (pytest, not the skill).

**How to apply:** When authoring/decomposing a new trio skill that calls
`${AUTOSPEC_SCRIPTS_DIR}/<helper>.sh` for helpers living under
`skills/autospec-shared/scripts/`, copy the `autospec-review` install.sh pattern
verbatim: a separate `SHARED_LIB_SCRIPT_FILES` var + `resolve_shared_lib_scripts_dir`
(checkout `skills/autospec-shared/scripts` else fetched `lib-scripts/`) + its own
install loop + a `lib-scripts/` fetch branch in `fetch_source_files`. Only repo-root
`scripts/` files belong in `SHARED_SCRIPT_FILES`. Bit the grow-define skill (Plan 3,
PR #1688). Related: [[feedback_installer_excludes_runtime_libs]],
[[feedback_autospec_decomposer_gotchas]].
