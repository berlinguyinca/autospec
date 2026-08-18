---
name: Autospec mode-dispatch must not shell out user text
description: Self-update / Stop mode (and any future mode-dispatch) sections in autospec skills must be pure prose; never embed bash that consumes the user's free-form request, never use {FEATURE_DESCRIPTION} as a heredoc body, and never inline shell using $1/$2 — the harness DOES substitute those.
type: feedback
wing: synthesis
drawer_class: lesson
originSessionId: 1fa102a0-c695-404e-879a-47532b04d1ab
modified: 2026-08-17T02:50:58.073Z
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

**Positional parameters ARE substituted (verified 2026-08-16).** The rule
above is about `{FEATURE_DESCRIPTION}`, which no harness expands. `$1`/`$2`
are the opposite case: Claude Code DOES substitute them into the rendered
skill body at load time. Invoking `/autospec-split split docs/specs/foo.md`
rendered the injected `startup-self-update` block's `target="$1"` as
`target="docs/specs/foo.md"` — while `subcommand="$2"` and
`{FEATURE_DESCRIPTION}` both stayed literal. The substituted assignment
*overrides the caller's argument*: `heal_autonomous_operator_wrappers()`
loops over ~12 wrapper paths calling `write_autonomous_operator_wrapper
"$target" …`, and the body then writes to the substituted path every
iteration and `chmod +x`es it — i.e. a real file passed as an argument gets
overwritten with a wrapper script. It runs before the daily throttle check.
The block is expanded from `templates/skill-blocks/startup-self-update.md`
by `scripts/expand-skill-blocks.sh` into 30 installed skill files; the repo
sources hold only the `<!-- autospec-block:startup-self-update -->` marker,
so the source greps clean and only the *installed* copies show the hazard.

**Resolved** by #3177 / PR #3182 (merged 2026-08-17): the shell moved into
`scripts/autospec-startup-self-update.sh`, and a structural validator now
rejects a positional-parameter assignment in any skill-block template.

**But the fix under-generalized, twice — this is the durable lesson.**
It gated the *instance*, not the *class*:

1. **Wrong scope.** The regression scan covers `templates/skill-blocks/`
   only. Trio bodies are not scanned, and `run_pr_size_gate()` still assigns
   `PR_SIZE_PHASE="$1"` / `PR_SIZE_BASE_OID="$2"` inside a *rendered* skill
   body across 6 trio files. Tracked as #3101. Lower severity than #3177 —
   a substituted OID yields a bogus `git diff` and the gate returns 1, so it
   fails closed with no overwrite payload.
2. **Wrong pattern.** The scan greps `="\$[12]"`, but the same function
   assigns `PR_SIZE_HEAD_OID="$3"`. Any audit of this class must match
   `="\$[0-9]"`, not just `$1`/`$2`.

**How to apply:** Never inline shell that uses positional parameters into a
SKILL.md body — call a script by path instead. When auditing this class of
bug, grep the INSTALLED skills (`~/.claude/skills/*/SKILL.md`) *and* the trio
bodies (`skills/*/SKILL.md`, `codex/prompt.md`, `opencode/agent.md`), not the
repo sources alone, and match `="\$[0-9]"` rather than a two-digit subset.
When you fix an instance of a class, make the gate cover the class. Mode-dispatch sections describe a decision the LLM makes
from the request text already in its context. Phrase as: "Read the request,
mentally collapse whitespace / trim / lowercase, and if the result is
exactly `<keyword>`, enter <mode>." Add a one-line "do NOT shell out the
user's request" warning so future maintainers don't re-introduce the bug.
The lock-step trio (SKILL.md, codex/prompt.md, opencode/agent.md) must
receive byte-identical edits — `autospec validate`'s `check_lockstep`
diffs them and fails on drift.
