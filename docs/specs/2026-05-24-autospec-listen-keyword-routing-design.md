# autospec-listen keyword auto-routing — design

- **Date:** 2026-05-24
- **Status:** Approved in brainstorming; pending implementation plan
- **Author:** berlinguyinca (via Claude Code)
- **Extends:** `skills/autospec-listen/` (trio) + `scripts/listener-match.sh`

## Goal

Make autospec the default path for build/change work: when a user expresses an imperative intent using common verbs (`design`, `new feature`/`spec`, `implement`/`build`/`ship`, `review`, or `autospec …`), `/autospec-listen` auto-routes the session into the matching autospec skill, printing a one-line opt-out. This maximizes pipeline usage and keeps work flowing consistently, without forcing the user to remember which slash command to type.

## Motivation

`/autospec-listen` already auto-triggers on a narrow set of phrases ("file an issue", "write a spec") and hands off to `/autospec-define`. Users still hand-craft design/implement/review work ad-hoc instead of running it through the pipeline. Broadening the trigger surface (with an intent gate) routes the common verbs into the pipeline by default.

## Decisions (resolved in brainstorming)

| # | Decision | Choice |
|---|----------|--------|
| D1 | Aggressiveness | **Auto-route + 1-line escape.** On a gated match, route immediately to the mapped skill and print e.g. `Routing to /autospec-define — say "plain" to opt out.` |
| D2 | Mechanism | **Extend `/autospec-listen`** (trigger phrases in the skill `description` + routing logic in `listener-match.sh` + skill body). Model-driven/advisory, consistent with the existing listen behavior. |
| D3 | Verb → skill map | `design`/`new feature`/`spec` → `/autospec-define`; `implement`/`build`/`ship` → `/autospec-run` (or `/autospec` end-to-end when no issues exist yet); `review` → `/autospec-review`; `autospec …` → the umbrella `/autospec`. |
| D4 | Intent gate | Fire **only** on a first-person/direct imperative request to build or change something. Suppress incidental mentions: past tense ("I reviewed it"), negation ("don't implement yet"), questions ("should we redesign?"), and descriptive use ("the design is nice"). Prefer a false-negative (no route) over a misfire — per `feedback_autospec_design_prefs` (tight triggers) and `feedback_omc_autopilot_misfire` (keyword auto-activation hazard). |
| D5 | Escape | The one-line notice names a stop word (`plain` / `no`); if the user's next turn is that word, cancel routing and proceed in plain mode for that request. |

## Architecture

### Components

1. **`scripts/listener-match.sh`** (extend): given the user's message text, classify → emit JSON `{"match":bool,"skill":"autospec-define|autospec-run|autospec-review|autospec","trigger":"<verb>","intent":"imperative|incidental","confidence":0-1}`. Encodes the D4 intent-gate heuristics (imperative-mood detection; past-tense/negation/question/descriptive suppressors). Pure bash, `set +e`, deterministic, exit 0 always (a non-match is `{"match":false}`).
2. **`/autospec-listen` trio** (`SKILL.md` + `codex/prompt.md` + `opencode/agent.md`): broaden the `description` trigger phrases to include the D3 verbs; add a `## Keyword auto-routing` section describing the gate + verb→skill map + auto-route-with-escape behavior; preserve the existing "file an issue / write a spec" triggers. Lockstep across all three files.
3. **Routing action:** once `/autospec-listen` is active and `listener-match.sh` returns `match:true` with `intent:imperative`, the skill prints the one-line escape notice and invokes the mapped skill (or hands off, as it already does for the spec trigger). On a non-match or `intent:incidental`, it does nothing and the session proceeds normally.

### Data flow

```
user message → listener-match.sh (gate + classify) → {match, skill, intent}
  match:true & imperative → print "Routing to /<skill> — say \"plain\" to opt out." → invoke /<skill>
  else → no-op (plain session)
```

### Guardrails / anti-misfire

- Intent gate biased to false-negatives; descriptive/past/negated/interrogative uses do NOT route.
- One-line escape on every route (D5) makes any misfire a single-word recovery.
- No new autonomous-loop auto-start beyond what the routed skill itself does (e.g. routing to `/autospec-run` still obeys its own queue/skip rules).

## Testing

`scripts/listener-match.sh` bats:
- Positive: each verb in an imperative request routes to the correct skill (`"implement the cache layer"` → autospec-run; `"design a webhook system"` → autospec-define; `"review the PR"` → autospec-review; `"autospec the billing module"` → autospec).
- Negative (must NOT route): `"I already reviewed it"`, `"the design looks nice"`, `"don't implement that yet"`, `"should we redesign this?"`.
- Escape: stop word recognized.
- Back-compat: existing "file an issue" / "write a spec" triggers still match.

## Out of scope

- Changing the downstream skills' own behavior.
- Non-English triggers.
- A deterministic UserPromptSubmit hook (considered and declined in favor of extending the skill; can revisit if advisory routing proves unreliable).

## Named consumer / ROI

Consumer: the operator/team — directly serves the "use autospec as much as possible / keep it moving" goal. Reuses the existing `/autospec-listen` skill and `listener-match.sh`; introduces no new top-level skill (passes `feedback_roi_check_new_components`).
