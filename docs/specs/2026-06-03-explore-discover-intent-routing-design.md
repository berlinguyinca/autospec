# explore/discover intent routing → /autospec-explore (via autospec-listen)

- **Date:** 2026-06-03
- **Status:** Design (Phase 2)
- **Author:** berlinguyinca (refined via /autospec-refine)
- **Tracker target:** `berlinguyinca/autospec`

## Problem statement

Operators want `/autospec-explore` to fire when they express **intent to
explore/discover** features to build — without typing the slash command. But
`autospec-explore` launches a **perpetual autonomous research + ship loop on an
isolated sandbox branch** (PRs target the sandbox, never main). "explore" and
"discover" are extremely common words ("explore the codebase", "let me explore
options", "exploratory test"), so a naive keyword auto-trigger would
catastrophically auto-launch a PR-shipping loop on incidental mentions — the
exact misfire class recorded in `feedback_omc_autopilot_misfire`.

The repo already has the correct mechanism: **autospec-listen** with
`scripts/listener-match.sh`, a deterministic classifier whose intent-gate is
**biased to false-negatives** ("prefer no route over a misfire"), plus an
opt-out and a `gate` field for pre-route checks.

## Goals / non-goals

**Goals**
- Route **explore/discover imperative research-and-ship intent** to
  `/autospec-explore` through the autospec-listen classifier.
- A **confirm-before-launch** gate, because the action is heavyweight/perpetual.
- Never fire on read/understand/conversational/incidental mentions.

**Non-goals**
- Making `autospec-explore` a bare-keyword auto-trigger (rejected: bypasses the
  intent-gate). Its frontmatter only gets a consistency note.
- Adding explore/discover to `trigger-keywords.md` — that file is the SoT only
  for the issue/spec back-compat triggers, NOT the verb→skill map (Phase-1
  verified: `parse_trigger_md` feeds `classify_phrase` lines 258-271 only).
- Changing native `/autospec-explore` behavior.

## Design

### Mechanism (autospec-listen classifier only)

1. **D3 verb branch** — add an explore/discover branch to `classify_phrase()`
   in `scripts/listener-match.sh` (the if/elif chain, ~lines 362-393, alongside
   the `implement/build/ship` and `design` branches). When the phrase carries
   explore/discover intent AND passes the imperative gate, it MUST emit
   **explicitly** (like the bare-run gate exemplar at line 353), so it can carry
   the gate value rather than falling through to the generic emit (which passes
   an empty gate):

   ```
   emit_classify_json true autospec-explore explore imperative 0.7 "$is_auto" "" explore-confirm
   ```

   `discover` is an alias verb routing to the same skill/trigger.

2. **D4 intent suppression** — in `is_imperative()` (~lines 190-241), strengthen
   the descriptive/read suppressor group (231-238) so read/understand explore
   phrasings return suppress. Combined with the existing question / negation /
   past-tense groups, this enforces the negative table below. The explore branch
   routes ONLY when the phrase co-occurs with build/ship/feature intent (e.g.
   "explore **and ship**", "discover features **to build**", "explore the repo
   **for improvements**"); a bare "explore <noun>" without build intent does not
   route.

3. **`explore-confirm` gate (emit + honor)** —
   - **Emit:** the D3 branch sets `gate=explore-confirm`.
   - **Honor:** define + honor it in the lock-step trio
     (`skills/autospec-listen/{SKILL.md,codex/prompt.md,opencode/agent.md}`),
     byte-parallel, next to the existing `auto-implement-open` gate
     (SKILL.md:108 define, :128 honor). Semantics: on a match with
     `gate=explore-confirm`, the listener prints exactly what `/autospec-explore`
     will do — *"starts a perpetual autonomous research + ship loop on an
     isolated sandbox branch; PRs target the sandbox branch, never main"* — and
     requires **one explicit confirmation** before invoking the skill. The
     existing `plain`/`no` escape (D5) still cancels.

4. **Consistency note** — add one line to `autospec-explore`'s frontmatter
   `description` noting it can be reached via explore/discover intent through
   autospec-listen (mirroring autospec-loop's disambiguation wording), WITHOUT
   making it a bare-keyword trigger.

### The routing contract (the heart of this spec — drives the bats tables)

**CLAIM → `{match:true, skill:"autospec-explore", trigger:"explore", intent:"imperative", gate:"explore-confirm"}`:**

- "explore and ship new features"
- "discover features to build"
- "autonomously explore the repo for improvements"
- "go explore and build improvements"
- "discover and implement enhancements"
- "start exploring features to ship"

**SUPPRESS → `{match:false}` (stay in plain mode, never route):**

- "explore the codebase"
- "explore this file" / "explore the data"
- "explore options" / "let me explore" / "I'll explore later"
- "exploratory test" / "let's do exploratory testing"
- "the exploration is done" (past/descriptive)
- "should we explore X?" (interrogative)
- "don't explore yet" (negated)
- any incidental / system-reminder occurrence of the word

## Testing

- `tests/unit/test_listener_classify.bats` (mirror its existing positive blocks +
  negative-via-gate block): a **positive table** asserting each CLAIM phrase
  yields `.skill=="autospec-explore"` and `.gate=="explore-confirm"`, and a
  **negative table** asserting every SUPPRESS phrase yields `.match=="false"`.
  The negative table is the **critical regression guard**.
- Real invocation of the actual script (no mocks).

## Validation

- Extend `check_keyword_routing_section()` in `autospec validate` (lines
  139-147) to also assert the trio contains the explore→`autospec-explore`
  verb-map row and the `explore-confirm` gate semantics (grep across all three
  trio files).
- Lock-step: the verb-map table row + gate define/honor land **byte-identically**
  in SKILL.md / codex/prompt.md / opencode/agent.md (the existing lock-step
  check enforces this).

## Risks & mitigations

| Risk | Mitigation |
|------|------------|
| "explore the codebase" auto-launches the perpetual ship loop | D4 suppression + require build/ship co-intent + the negative bats table as a hard guard. |
| Confirmation bypassed → silent launch | `explore-confirm` gate honored in all 3 trio files; route only on explicit yes. |
| Verb added in the wrong place (trigger-keywords.md) | Phase-1-verified: verbs live in the D3 code branch + trio table, not trigger-keywords.md. |
| Lock-step drift across the trio | Single edit mirrored to all three; validate.sh lock-step + keyword-routing checks. |

## Decomposition hint for /autospec-define

This is a small, cohesive feature. Prefer **one issue** if the body fits the
caps; otherwise split into exactly two with a `Depends on` edge:

1. **Classifier** — `scripts/listener-match.sh` explore/discover D3 branch
   (emits `gate=explore-confirm` on imperative build-intent) + D4 read/understand
   suppression + `tests/unit/test_listener_classify.bats` positive + negative
   tables.
2. **Listener prose + gate honor + validation** (depends on #1) — verb-map table
   row + `explore-confirm` gate define/honor in the lock-step autospec-listen
   trio + `autospec-explore` description note + extend `validate.sh`
   keyword-routing. Consumes the `explore-confirm` value emitted by #1.
