---
name: Autospec mode-dispatch must not shell out user text
description: Self-update / Stop mode (and any future mode-dispatch) sections in autospec skills must be pure prose; never embed bash that consumes the user's free-form request, and never use {FEATURE_DESCRIPTION} as a heredoc body.
type: feedback
originSessionId: 1fa102a0-c695-404e-879a-47532b04d1ab
---
Mode-dispatch sections (`## Self-update mode`, `## Stop mode`, etc.) in any
autospec trio skill (autospec, autospec-run, autospec-define, autospec-stop,
autospec-listen, autospec-classify) must instruct the LLM in pure prose:
"read the request, mentally normalize, compare to literal." NEVER include a
bash code block that runs `grep`/`sed`/`[[ =~ ]]`/heredoc against the user's
free-form feature request.

**Why:** Two compounding failures historically broke `/autospec` invocations:
1. Harness permission engines evaluate Bash tool calls against allowlist
   patterns. When the bash command embeds free-form user text containing
   shell metacharacters (`!`, backticks, `|`), zsh's pattern-eval bombs with
   errors like `Shell command failed for pattern "!\` | ": (eval):1: parse
   error near |'`, aborting the skill before Phase 0.
2. The `{FEATURE_DESCRIPTION}` placeholder is NOT substituted by any harness
   (Claude Code, OpenCode, Codex). It's a convention the LLM reads, not a
   template the runtime expands. Bash heredocs of the form
   `cat <<'EOF'\n{FEATURE_DESCRIPTION}\nEOF` therefore receive the literal
   string `{FEATURE_DESCRIPTION}` — not the user's text — and silently
   normalize that placeholder.

**How to apply:** Mode-dispatch sections describe a decision the LLM makes
from the request text already in its context. Phrase as: "Read the request,
mentally collapse whitespace / trim / lowercase, and if the result is
exactly `<keyword>`, enter <mode>." Add a one-line "do NOT shell out the
user's request" warning so future maintainers don't re-introduce the bug.
The lock-step trio (SKILL.md, codex/prompt.md, opencode/agent.md) must
receive byte-identical edits — `scripts/validate.sh`'s `check_lockstep`
diffs them and fails on drift.
