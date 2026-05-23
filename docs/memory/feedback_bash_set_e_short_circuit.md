---
name: feedback-bash-set-e-short-circuit
description: "Under bash set -e, `[ test ] && action` aborts the whole script when the test fails — use if/then/fi for one-sided conditionals"
metadata: 
  node_type: memory
  type: feedback
  wing: synthesis
  drawer_class: lesson
  originSessionId: 0a77c1fd-c243-4bf9-b3fb-4f83ae5f9830
---

`[ "$counter" -gt 0 ] && info "..."` under `set -e` aborts the script when `$counter` is 0. The whole `test && action` expression returns the failing exit code of the test, and `set -e` treats that as a fatal error from a top-level statement. The pattern is safe inside `if`, after `||`, or at function boundaries — but NOT as a bare top-level statement in a `set -e` script.

**Why:** 2026-05-17 smoke-test caught this in `install.sh`'s `bootstrap_turbo`:

```bash
info "bootstrap_turbo: $linked turbo skills symlinked..."
[ "$skipped_dir" -gt 0 ] && info "..."        # exit 1 here when skipped_dir==0
[ "$cleaned_nested" -gt 0 ] && info "..."     # never reached
# every step after this point silently skipped:
check_codex
merge_claude_md
offer_gitignore
```

Result: install completed "successfully" but never called the rest of the integration bootstrap. The unit tests' isolated assertions passed (bootstrap_turbo itself looked fine), but the integration test (test_gitignore_offer.sh) failed because gitignore was never touched.

**How to apply:**
- For one-sided conditionals in `set -e` scripts, ALWAYS use `if cond; then action; fi` instead of `cond && action`.
- Acceptable short-circuit forms: `cond || action` (action runs on failure — doesn't abort), `cond && action || true` (suppresses non-zero), or inside an `if`/`while`/`until` head.
- When a `set -e` script "succeeds" but later steps don't seem to run, check for stray `[ ... ] && ...` patterns where the test can legitimately be false.
- This is the second `set -e` interaction bug in `install.sh` in one day; the first was the RETURN trap leak ([[feedback_bash_return_trap_leak]]). The combination of `set -eu` + dynamically-built helper functions in `install-helpers.sh` is fragile — when in doubt, prefer explicit `if`/`then`/`fi` and inline cleanup over trap- or short-circuit-based control flow.
