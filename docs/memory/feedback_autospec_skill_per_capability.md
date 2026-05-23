---
name: Operator-facing capabilities should be top-level autospec-* skills, not sub-modes
description: Every distinct operator-facing capability in the autospec family ships as a slash command (skill), with inline sub-modes only as in-flight convenience shortcuts to the same backend
type: feedback
originSessionId: ff14de95-2db2-420a-9835-0401379148a3
---
When designing a new operator-facing capability in the autospec family
(stop, resume, profile-switch, sweep, abort, …), default to a
**top-level `/autospec-<verb>` slash command** with full skill scaffold
(SKILL.md + opencode/agent.md + codex/prompt.md + install.sh +
uninstall.sh + README.md) — not a sub-mode of an existing skill.

**Why:** verbatim from user 2026-05-01 during stop-mechanism brainstorm
Q2:

> "we also need to be able to just cancel it by callint /autospec-stop
> or so as skill"

The reason isn't aesthetic — it's discoverability and lock-step.
Operators tab-complete `/autospec-` and see the full menu of capabilities;
sub-modes hide behind regex shortcuts that operators have to remember.
The shipped family demonstrates this: `/autospec`, `/autospec-define`,
`/autospec-run`, `/autospec-classify`, `/autospec-listen` are all
discrete skills for discrete capabilities. Adding `/autospec-stop` (and
future `/autospec-resume`, `/autospec-sweep`, …) keeps the pattern.

**How to apply:**

1. **Default architecture for any new capability** — top-level skill is
   the canonical entry point. Skill body is small: parse args, dispatch
   to a `scripts/<helper>.sh` that does the actual work.
2. **Inline sub-modes are convenience-only** — when the operator is
   *already* mid-conversation with another autospec skill (e.g.
   `/autospec` running a long pipeline), an inline regex sub-mode like
   `^\s*<verb>(\s+--\w+)*\s*$` lets them invoke the capability without
   typing the new skill name. The sub-mode short-circuits to the same
   `scripts/<helper>.sh` — single source of truth.
3. **Skill body shape** for these wrapper skills mirrors
   `skills/autospec-classify` (small, focused, no Phase 0–6 pipeline).
   Just: detect harness, parse args, dispatch helper, print stdout.
4. **Lock-step trio still applies** — the new skill ships with the trio
   (SKILL.md + opencode/agent.md + codex/prompt.md) byte-identical
   bodies, validated by `scripts/validate.sh check_lockstep`. No
   exceptions.
5. **Validator addition** — every new skill should grow a corresponding
   `check_required_files` entry in `scripts/validate.sh` so the trio
   scaffold can't silently rot.
6. **Documentation pattern** — README.md gets a new row in the skill
   table; AGENTS.md gets a `## <Capability> authority` heading
   parallel to existing `## Auto-merge authority` if the capability
   carries authority semantics (admin-merge, paused-by-user, etc).

The combined cost — full skill scaffold + helper script + sub-mode
shortcut + validator hook — is meaningful (~10–15 file changes), but
amortizes well: every autospec feature added in this shape lands as a
dual entry point that operators can invoke from any starting state
(fresh terminal OR mid-pipeline), and the skill family stays
self-similar so newcomers can predict the shape of any new capability
from the existing five.
