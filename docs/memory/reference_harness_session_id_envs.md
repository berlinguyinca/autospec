---
name: reference_harness_session_id_envs
description: Env vars that expose a stable per-session id for harness-neutral per-session locks/scoping
metadata: 
  node_type: memory
  type: reference
  originSessionId: d039abad-07c8-4595-980c-99dc83e9788b
---

For a **stable per-session identifier** that survives across many separate shell
invocations within one harness session (each `$$`/`$PPID` differs per tool call,
and `ps -o sess=` returns 0 with no controlling tty under Claude Code tool
calls), use this fallback chain — first non-empty wins:

1. `AUTOSPEC_SESSION_ID` (explicit override)
2. `CLAUDE_CODE_SESSION_ID` — **confirmed present** in Claude Code (e.g.
   `d039abad-07c8-4595-980c-99dc83e9788b`); matches the session UUID in
   `~/.claude/projects/<slug>/` and `/tmp/claude-*/.../<uuid>/` paths.
3. `CODEX_SESSION_ID` / `CODEX_THREAD_ID` (Codex; best-effort)
4. `OPENCODE_SESSION_ID` / `OPENCODE_SESSION` (OpenCode; best-effort)
5. `TERM_SESSION_ID` (Apple Terminal / multiplexer)
6. POSIX session id `ps -o sess= -p $$` (only non-zero with a controlling tty)
7. `PPID` — last resort; **unreliable**: a `env`/wrapper layer or a re-parented
   monitor changes it, so two calls in one session can yield different tokens
   (under-scope) — do NOT write a test asserting the PPID fallback collides.

Other confirmed Claude Code env: `CLAUDECODE=1`, `AI_AGENT=claude-code_<ver>_agent`,
`CLAUDE_CODE_ENTRYPOINT`, `CLAUDE_CODE_TMPDIR`.

Used by `skills/autospec-run/scripts/autospec-run-session-lock.sh` (per-session
single-instance monitor guard, PR #899). Related: [[feedback_heartbeat_cross_repo_collision]]
(path-scope shared state), [[feedback_self_consistent_test_fixtures_mask_bugs]]
(don't assert intentionally-degraded fallbacks).
