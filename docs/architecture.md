# Autospec architecture

Single-source-of-truth doc for the cross-cutting design rules every
autospec skill is expected to honor. The per-skill `SKILL.md` files
implement these rules; this file states them.

Sections:

- [Concurrency model](#concurrency-model)
- [Lock-step body rule](#lock-step-body-rule)
- [Model tier policy](#model-tier-policy)
- [Trigger keyword theory](#trigger-keyword-theory)

## Concurrency model

Per spec §5.4, autospec coordinates concurrent monitor instances using a
single per-issue **`locked-by-autospec-processor`** label as the mutex.
There is no global `flock`, no daemon heartbeat, and no GitHub-side
coordination issue.

Contract:

- **Lock-claim comment.** Before a monitor begins work on an issue, it
  posts a comment of the form
  `🔒 Auto-locked by autospec monitor at <UTC ISO-8601 timestamp>` and
  applies the `locked-by-autospec-processor` label. The comment IS the
  marker — no separate heartbeat stream is needed.
- **Yield rule (5-minute window).** Before claiming a lock, the monitor
  re-reads the issue. If a `🔒 Auto-locked` comment was posted in the
  last 5 minutes by any other actor, that is another monitor that
  just claimed it — the current monitor MUST yield (do NOT add its
  own lock label) and re-enter the ready set on the next loop
  iteration.
- **Lock release.** The monitor removes the
  `locked-by-autospec-processor` label as soon as the issue closes
  (PR merged) or the work fails (label restored to `auto-implement`
  by the monitor's recovery branch).
- **No global flock.** Each monitor relies only on the GitHub label +
  comment state. The 5-minute yield window is the only coordination
  primitive.

This intentionally tolerates short windows of concurrent inspection
(two monitors fetching the issue body) but prevents two monitors from
opening simultaneous PRs against the same issue.

## Lock-step body rule

Every multi-harness skill ships three trio files:

- `skills/<skill>/SKILL.md` — Claude Code variant (with YAML
  frontmatter including `name:` and `description:`).
- `skills/<skill>/opencode/agent.md` — OpenCode variant (frontmatter
  with `description:` + `mode: primary`).
- `skills/<skill>/codex/prompt.md` — Codex CLI variant (no
  frontmatter, body only).

The lock-step rule: **the body of all three files must be byte-identical
after frontmatter is stripped.** `scripts/validate.sh` enforces this by
diffing `awk '/^---$/{c++; next} c>=2'` (a frontmatter stripper) of
each pair. Only frontmatter differs across the trio; the body — the
actual prompt the model sees — is the same string everywhere. This is
how a single body change to a SKILL.md propagates losslessly across
Claude Code, OpenCode, and Codex CLI.

Consequences:

- Any body edit must be replicated to all three files in the same
  commit.
- The codex/prompt.md is the body of SKILL.md verbatim (no
  `---` delimiters).
- The opencode/agent.md frontmatter contains `mode: primary` and a
  description matching the SKILL.md description.

## Model tier policy

Per AGENTS.md, every subagent dispatch the orchestrator makes uses one
of two tiers:

- **Tier A — specification work** (top model + extended/maximum
  thinking). Used for design, decomposition, classification, and any
  step that produces a spec-quality artifact.
  - Claude Code: `opus` + `ultrathink`.
  - Codex CLI: top GPT + `reasoning_effort=high`.
  - OpenCode: top task tier.
- **Tier B — implementation work** (cheaper model + medium thinking).
  Used for code-edit subagents that follow a written spec.
  - Claude Code: `sonnet` + medium thinking.
  - Codex CLI: `gpt-5.1-codex-spark` + `reasoning_effort=medium`.
  - OpenCode: smaller task tier.

Fallback rule: on tier-unavailability, fall **UP** the tier (Tier B
falls back UP to Tier A — never the other direction). The orchestrator
itself stays at the user's invoked model.

Per-skill counts (validated by `scripts/validate.sh`):

| Skill | Tier-A dispatches | Tier-B dispatches |
|---|---|---|
| `autospec`         | 3 | 2 |
| `autospec-define`  | 3 | 0 |
| `autospec-run`     | 0 | 2 |
| `autospec-classify`| 1 | 0 |

(`autospec-listen` does not currently dispatch subagents directly —
it routes to other skills.)

## Trigger keyword theory

Per spec §4.4, `autospec-listen` activates on **imperative-verb-only**
trigger phrases. Bare nouns (`issue`, `spec`, `ticket`) are NOT
triggers, even when they are the dominant noun in a sentence.

Active phrases (canonical list lives in
`skills/autospec-listen/references/trigger-keywords.md`):

- Issue triggers: `file an issue`, `file this as an issue`,
  `new issue`, `open an issue`, `create a ticket`, `make an issue`.
- Spec triggers: `write a spec`, `design spec`, `new spec`,
  `start a spec`, `write a design spec`.

Matching rules:

- Case-insensitive.
- Word-boundary anchored (left and right edge of the match must be at
  start/end of the input or a non-`[A-Za-z0-9_-]` character).
- Multi-word phrases match contiguously.

This deliberately rules out false positives like "the issue here is..."
or "the spec says..." — the verb anchors the trigger, not the noun.
The classifier (`scripts/listener-match.sh`) reads the canonical
markdown reference as ground truth, so the docs and the code can never
drift.
