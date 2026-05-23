---
name: feedback-bash-return-trap-leak
description: Bash RETURN traps set inside functions leak into caller frames; never use them for local cleanup when the calling script runs under set -u
metadata: 
  node_type: memory
  type: feedback
  wing: synthesis
  drawer_class: lesson
  originSessionId: 0a77c1fd-c243-4bf9-b3fb-4f83ae5f9830
---

`trap 'cleanup "$tmp"' RETURN` set inside a bash function does NOT scope to that function. The trap persists in the shell and fires on every subsequent function return, including frames where `$tmp` is unset. Under `set -u` (which `autospec/install.sh` uses), the trap re-evaluating an unset variable aborts with `unbound variable`.

**Why:** Concrete incident, 2026-05-17. Code reviewer (correctly) flagged that `merge_marked_block`'s `mv "$tmp" "$file"` would leak `content_file` on failure under `set -e`. The proposed fix — `trap 'rm -f "$tmp" "$content_file"' RETURN` at top of function — passed all local tests including the install-helpers unit test, but broke CI's bats smoke tests (`tests/smoke/test_install_all_skills.bats`) immediately. Local tests source the helper file and call its function in isolation; the trap had no later function returns to leak into. CI ran the full `install.sh` flow where `merge_marked_block` returns → `merge_claude_md` returns → RETURN trap fires in the latter's frame → `tmp: unbound variable` → install aborts.

**How to apply:**
- For local cleanup in bash functions that may run under `set -eu`, prefer inline cleanup with explicit error branching:
  ```bash
  if mv "$tmp" "$file"; then
      rm -f "$content_file"
      return 0
  fi
  rm -f "$tmp" "$content_file"
  return 1
  ```
- If a RETURN trap is genuinely required (e.g., many failure points), use `set -T` to enable function-local trap inheritance, and clear with `trap - RETURN` before each successful return. But the inline-cleanup pattern is almost always simpler.
- Local-only test suites that import a single helper are not a substitute for CI; the trap-leak bug was invisible to `bash tests/install/test_helpers.sh` because the test never executed any caller frame after the function returned. Re-run the broader CI surface (bats smoke tests) locally before pushing changes to helpers that install.sh sources.
