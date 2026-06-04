---
description: Use when the user expresses an imperative intent to file an issue, write a spec, design a feature, implement/build/ship something, review code, or run autospec mid-conversation. Trigger keywords are "file an issue", "new issue", "open an issue", "create a ticket", "write a spec", "design spec", "new spec", "start a spec", "design", "new feature", "implement", "build", "ship", "review", "autospec". On an "issue" trigger, drafts a body from the last ~10 conversation turns and asks the user to approve before running gh issue create. On a "spec" trigger, hands off to /autospec-define. On a build/change verb, gates intent via the deterministic listener-match classifier and auto-routes to the mapped autospec skill with a one-line opt-out. Lives at github.com/berlinguyinca/autospec/skills/autospec-listen.
mode: primary
---

# autospec-listen (harness-neutral)

Conversation listener that watches for issue-filing or spec-writing intent
mid-conversation and dispatches to the right downstream flow without forcing
the user to remember a slash command.

## Startup self-update

```bash
#!/usr/bin/env bash
# autospec-startup-self-update — see docs/specs/2026-05-01-autospec-startup-self-update-design.md
set +e
SKILL_NAME=autospec-listen   # per-skill: autospec-define / autospec-run / autospec-listen / autospec-classify
if [ "${AUTOSPEC_NO_SELF_UPDATE:-0}" = "1" ]; then exit 0; fi
mkdir -p "$HOME/.autospec"
LOCKDIR="$HOME/.autospec/.update.lock.d"
LAST="$HOME/.autospec/last-update-check"
INSTALLED="$HOME/.autospec/installed-version"
NOW=$(date -u +%s)
if [ -f "$LAST" ]; then
    PREV=$(date -u -j -f '%Y-%m-%dT%H:%M:%SZ' "$(cat "$LAST" 2>/dev/null)" +%s 2>/dev/null \
        || date -u -d "$(cat "$LAST" 2>/dev/null)" +%s 2>/dev/null || echo 0)
    if [ "$((NOW - PREV))" -lt 86400 ]; then exit 0; fi
fi
if ! mkdir "$LOCKDIR" 2>/dev/null; then
    echo "WARN: self-update skipped (concurrent update in progress)" >&2; exit 0
fi
trap 'rmdir "$LOCKDIR" 2>/dev/null' EXIT
date -u +'%Y-%m-%dT%H:%M:%SZ' > "$LAST.tmp" && mv "$LAST.tmp" "$LAST"
REMOTE=$(curl -fsSL --max-time 5 \
    "https://api.github.com/repos/berlinguyinca/autospec/commits/main" \
    2>/dev/null | jq -r '.sha // empty' 2>/dev/null | cut -c1-7)
if [ -z "$REMOTE" ]; then
    echo "WARN: self-update skipped (network); continuing on installed version" >&2; exit 0
fi
LOCAL=$(cat "$INSTALLED" 2>/dev/null || true)
if [ "$REMOTE" = "$LOCAL" ]; then exit 0; fi
curl -fsSL --max-time 30 \
    "https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh" \
    | bash -s -- --skill all --harness all --update >/dev/null 2>&1
RC=$?
if [ "$RC" -ne 0 ]; then
    echo "WARN: self-update skipped (install rc=$RC); continuing on installed version" >&2; exit 0
fi
printf '%s\n' "$REMOTE" > "$INSTALLED.tmp" && mv "$INSTALLED.tmp" "$INSTALLED"
# Auto-init cross-tool memory (idempotent, <50ms fast-path)
bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/auto-init-memory.sh"
echo "[autospec] updated ${LOCAL:-fresh} → $REMOTE"
```

## Self-update mode

If the feature-request argument matches the regex `^\s*update\s*$` (case-insensitive, whitespace-padded), this skill enters self-update mode and does not run the normal trigger flow:

1. **Detect harness** by checking which install path exists for this skill:
   - Claude Code: `~/.claude/skills/autospec-listen/SKILL.md`
   - OpenCode:    `~/.config/opencode/agent/autospec-listen.md`
   - Codex CLI:   `~/.codex/prompts/autospec-listen.md`
2. **Re-install the full autospec suite from `main`** by piping the canonical installer:
   ```bash
   curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh | bash -s -- --skill all --harness all --update
   ```
   Run this one-liner once; it refreshes all autospec skills across all harnesses.
3. **Show the diff** between the prior installed file(s) and the freshly fetched copy (e.g. `diff <(cat <prior>) <(curl -fsSL ...SKILL.md)` or the equivalent recorded by the installer).
4. **Stop.** Do not enter any trigger flow. Print the upgrade summary and return to the user.

If no install path is detected, print `Self-update: no installed copy of autospec-listen found; run install.sh first.` and exit.

## Required capabilities & harness adapter

This skill assumes four capabilities. Map each one to your harness's actual tool. If a capability is missing, use the listed fallback.

| Capability                  | Claude Code                          | OpenCode                                 | Codex CLI                                | Fallback if missing                                |
|-----------------------------|--------------------------------------|------------------------------------------|------------------------------------------|----------------------------------------------------|
| Read recent conversation    | harness-native message buffer        | harness-native message buffer            | harness-native message buffer            | Ask the user to paste the relevant turns           |
| Ask the user a question     | `AskUserQuestion`                    | inline prompt                            | inline prompt                            | Ask in the response and wait for the next turn     |
| Run `gh` CLI                | shell tool                           | shell tool                               | shell tool                               | Have the user run the command and confirm          |
| Hand off to another skill   | `Skill` invocation (`/autospec-define`) | nested skill invocation                | `/autospec-define` slash command         | Print the suggested command for the user to run    |
| Subagent model tier         | Tier B: `sonnet` + medium thinking; Tier A only if escalating to /autospec-define which honors its own tier policy | Tier B: smaller-tier `task` + medium reasoning; escalation honors target skill | Tier B: `gpt-5.1-codex-spark` + `reasoning_effort=medium`; escalation honors target | Honor AGENTS.md tier mapping; retry the same subagent UP on unavailability |
| Subagent dispatch policy   | per AGENTS.md decision matrix        | per AGENTS.md decision matrix            | per AGENTS.md decision matrix            | inline with main-session token cost                |

**Persistent project notes**: write durable preferences to **`AGENTS.md`** in the repo root — this is the de-facto standard recognized by Claude Code (also reads `CLAUDE.md`), OpenCode, and Codex. Per AGENTS.md, this listener runs at **Tier B** (it's a router, not a designer); when it hands off to `/autospec-define` for a spec trigger, the downstream skill applies its own tier policy. Fall back UP the tier on quota/capacity or other unavailability by retrying the same subagent with the stronger tier while preserving parent context.

## Trigger flow

> **Model tier:** Tier B (implementation work) — this listener is a router; subagent dispatches inside its trigger flows (drafting issue bodies, summarizing recent conversation) use cheaper models per AGENTS.md. Spec-trigger handoff to `/autospec-define` lets the downstream skill apply its own tier policy.

## Keyword auto-routing

Beyond the explicit issue/spec triggers, this listener routes common build/change verbs into the matching autospec skill so the pipeline is the default path. Routing is gated by an imperative-intent check and always offers a one-line opt-out.

**Intent gate.** Do not classify by eye. Delegate to the deterministic classifier, passing the user's latest message verbatim:

```bash
bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/listener-match.sh" --classify "<user message>"
```

It emits `{"match":bool,"skill":...,"trigger":...,"intent":"imperative|incidental|none","confidence":0-1,"autonomous":bool,"chain":<skill|null>,"gate":<string|null>}`. The gate is biased to false-negatives: descriptive ("the design is nice"), past-tense ("I reviewed it"), negated ("don't implement yet"), and interrogative ("should we redesign?") uses return `match:false` and MUST NOT route. Prefer no route over a misfire.

The two new trailing fields:
- `chain` — after the primary skill completes, invoke the chained skill (e.g. `/autospec-run` after `/autospec-refine`). Null when no chain applies.
- `gate` — before routing, the listener must satisfy the named context gate; `auto-implement-open` means run `gh issue list --repo <repo> --label auto-implement --state open --json number --jq 'length'` and route only if > 0, else stay in plain mode. `explore-confirm` means the route launches `/autospec-explore`, which starts a perpetual autonomous research + ship loop on an isolated sandbox branch whose PRs target the sandbox branch, never main; before invoking, print exactly that consequence and require ONE explicit confirmation, routing only on an affirmative reply. Null means no gate.

**Verb → skill map (D3).**

| Verb(s) in an imperative request | Routes to |
|----------------------------------|-----------|
| `design` / `new feature` / `spec` | `/autospec-define` |
| `implement` / `build` / `ship` | `/autospec-run` (or `/autospec` end-to-end when no issues exist yet) |
| `refine` / `optimize` / `polish` / `improve` / `tune` | `/autospec-refine` |
| `run` (scoped: "run it", "run the loop", "run autospec"; bare "run" only when open `auto-implement` issues exist) | `/autospec-run` |
| `refine and run` / `tune up` (combined) | `/autospec-refine` then `/autospec-run` |
| `review` | `/autospec-review` |
| `explore` / `discover` (only with an explicit build/ship/improve action connector, e.g. "explore and ship", "discover features to build") | `/autospec-explore` (gate `explore-confirm`) |
| `fix` (imperative, e.g. "fix the login flow"; trivial-fix phrasings like "fix the typo", "quick fix", "fix the formatting" stay plain) | the umbrella `/autospec` (no gate) |
| `autospec …` | the umbrella `/autospec` |

**Auto-route behavior.** When the classifier returns `match:true` with `intent:imperative`, print exactly one line then invoke the mapped skill via the harness skill-invocation primitive:

```
Routing to /<skill> — say "plain" to opt out.
```

If `gate` is non-null, perform the gate check first and skip routing (plain mode) if unmet. For `gate:explore-confirm` (skill `autospec-explore`), do NOT auto-route on the one-line opt-out alone: first print exactly that `/autospec-explore` starts a perpetual autonomous research + ship loop on an isolated sandbox branch whose PRs target the sandbox branch, never main, then require ONE explicit confirmation and invoke `/autospec-explore` only on an affirmative reply; the existing `plain`/`no` escape still cancels. If `chain` is non-null, after the mapped skill completes, invoke the chained skill (printing one opt-out line as usual).

On a non-match or `intent:incidental`/`none`, do nothing — the session proceeds normally in plain mode.

**Escape (D5).** If the user's next turn is the stop word (`plain` or `no`), cancel routing and proceed in plain mode for that request. The existing `file an issue` / `write a spec` triggers continue to fire their own flows below and are unaffected.

## Issue trigger flow

When the user utters a phrase from the **Issue triggers** list (see `references/trigger-keywords.md`), run this 7-step procedure:

1. **Read recent context.** Pull the last ~10 conversation turns from the harness's message buffer. If that capability is missing, ask the user to paste the relevant excerpt.
2. **Synthesize draft body.** Build a markdown body with three sections:
   - `## Goal` — exactly one sentence summarizing what the issue should achieve.
   - `## Context` — relevant chat excerpts that motivate the issue (verbatim quotes, ≤6 lines).
   - `## Suggested AC` — ≤3 bulleted acceptance-criteria checkboxes (`- [ ]` style).
   The full body MUST be ≤400 words.
3. **Print the draft inline** so the user can read it before approving.
4. **Confirm via the harness primitive.** Ask exactly: `File this as a new issue? [yes / edit / cancel]`. Use `AskUserQuestion` on Claude Code, an inline prompt elsewhere; if no question primitive is available, ask in the response and wait for the next turn.
5. **On `yes`**, infer the title and run:
   ```bash
   gh issue create --title "<inferred-title>" --body "<draft-body>" --label needs-classify
   ```
   Then post the resulting issue URL back to the chat. The `needs-classify` label keeps the issue out of the auto-implement queue until `/autospec-classify` sizes it.
6. **On `edit`**, accept free-text revisions from the user, re-render the draft, and re-confirm (loop back to step 4).
7. **On `cancel`**, print `OK, cancelled.` and silently exit. No persistence, no learning, no suppression list.

### Title-inference rule

Build the title from the Goal sentence using the following deterministic rule:

1. Take the `## Goal` sentence verbatim.
2. **Strip leading verbs** `Implement` / `Add` / `Fix` (case-insensitive, with any trailing whitespace) from the front of the sentence.
3. **Truncate** the remainder to 80 characters total (after the prefix in step 4 is added).
4. **Prefix with a conventional-commit tag** inferred from the original leading verb:
   - `Add` / `Implement` / new behavior → `feat: `
   - `Fix` / bug language → `fix: `
   - documentation-only language → `docs: `
   - refactoring language → `refactor: `
   If inference is ambiguous, leave the tag off entirely and let `/autospec-classify` add one later.

Example: Goal `Implement a /loop slash command that re-runs the last skill` → title `feat: a /loop slash command that re-runs the last skill`.

## Spec trigger flow

When the user utters a phrase from the **Spec triggers** list (see `references/trigger-keywords.md`), this listener acts as a **router only** — it does NOT duplicate the Phase 2 brainstorming logic that lives in `/autospec-define`.

1. **Confirm.** Ask exactly: `Start the autospec design Q&A on <inferred-topic>? [yes / cancel]`. Use `AskUserQuestion` on Claude Code, an inline prompt elsewhere; if no question primitive is available, ask in the response and wait for the next turn. The `<inferred-topic>` is a short noun phrase synthesized from the trigger phrase and the most recent user turn (≤10 words).
2. **On `yes`**, hand off to `/autospec-define` via the harness's skill-invocation primitive (Claude Code: `Skill` tool; OpenCode: nested skill invocation; Codex CLI: emit the `/autospec-define` slash command). The downstream skill applies its own tier policy. Do NOT inline any Phase 2 questions here.
3. **On `cancel`**, print `OK, cancelled.` and silently exit. No persistence, no learning, no suppression list.

## Autonomous mode

The deterministic listener-match classifier recognizes autonomous-intent
phrasing and routes the request with `--autonomous` so downstream
`/autospec`, `/autospec-define`, and `/autospec-run` skip user-facing
confirmation gates.

Trigger phrases (case-insensitive, classified imperative):

- "fix X automatically"
- "no confirmation"
- "just do it"
- "non-interactive"
- "go autonomous"
- "autonomous mode"
- "run autonomously"

On a `match:true` + `intent:imperative` classification where the message
contains one of these phrases, the listener routes to the mapped autospec
skill with `--autonomous` appended:

```
Routing to /autospec --autonomous — say "plain" to opt out.
```

If the classifier returns `intent:incidental` or `none`, do NOT route — the
session continues in plain mode. The same biased-to-false-negative rule
applies: descriptive, past-tense, negated, and interrogative uses MUST NOT
route.

The autonomous routing inherits the listener's existing `plain`/`no` escape:
the user's next-turn stop word cancels routing and proceeds in plain mode
for that request.
