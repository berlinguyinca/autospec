---
name: feedback_codex_exec_needs_skip_git_repo_check
description: "Any autospec script that shells out to `codex exec` MUST pass --skip-git-repo-check or real codex refuses headless ('Not inside a trusted directory'); test stubs mask it because they stub AUTOSPEC_*_BIN and never invoke real codex"
metadata:
  node_type: memory
  type: feedback
  originSessionId: 291099c4-8250-48fb-ae0e-27b24f381739
---

Real `codex exec` refuses to run in a non-trusted / non-git directory with
`Not inside a trusted directory and --skip-git-repo-check was not specified.`
So a bare `codex exec` in a shipped script fail-closes on an otherwise-working
codex install (grooming/peer-review/etc. silently do nothing). Autospec's
canonical codex invocations (peer-review + advisor dispatcher) already pass
`codex exec --skip-git-repo-check` — new integrations MUST match. With the flag,
`codex exec` stdout (captured via `2>/dev/null`) is the clean model message; the
`hook: Stop` / `tokens used` noise is on stderr and correctly discarded. The
user's interactive `codex` is aliased to `codex --yolo` (bypasses trust), which
hides the problem in manual testing — the shipped script calls plain `codex`.

**Why it slipped through:** every groom-fill test stubs `AUTOSPEC_GROOM_FILL_BIN`
and never invokes real codex, so the missing flag was invisible to the suite —
a textbook [[feedback_self_consistent_test_fixtures_mask_bugs]] case. Only live
dogfooding (running the real pipeline against real GitHub issues) caught it.

**How to apply:** (a) grep new codex shell-outs for `exec` and confirm
`--skip-git-repo-check` is present; (b) lock it with an arg-recording stub test
that asserts the exact `exec --skip-git-repo-check` argv (stubs otherwise ignore
args); (c) more broadly — dogfood LLM-backed features end-to-end against real
inputs before declaring done, because stubbed fixtures can't see real-CLI
refusals or real-content routing gaps. Fixed in autospec PR #1778 (groom-fill).
Relates to [[feedback_llm_validator_adaptive_retry]].
