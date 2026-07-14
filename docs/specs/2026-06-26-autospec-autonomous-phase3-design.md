# /autospec-autonomous Phase 3 — operator persona & priority intelligence (design spec)

**Date:** 2026-06-26
**Builds on:** `docs/specs/2026-06-25-autospec-autonomous-design.md` (Phase 1 —
shipped) and `docs/specs/2026-06-26-autospec-autonomous-phase2-design.md`
(Phase 2 — shipped). Read both first; this spec *implements* the Phase-3 scope
those documents deferred and **resolves the three "Phase-3 contract corrections"**
the Phase-1 spec recorded (lines 126–138 there).
**Phase 1 + 2 status:** complete and Phase-5.5-audited (#1380, #1402); deps below
verified against repo source, not assumed.
**Review status:** revised after two independent peer reviews (codex + Opus
critic). Both flagged the self-brainstorm panel as the riskiest (two P0s) and
lowest-ROI piece; per operator decision (2026-06-26) the **panel is deferred** to a
gated follow-up (see "Deferred"). All other review findings are folded in below.

## Phase-4 supersession note (2026-07-06)

`docs/specs/2026-07-06-autospec-autonomous-platform-design.md` supersedes the Phase-3 startup `AskUserQuestion` gate. Persona and priority intake remain inputs, but the conductor must infer or read them from persisted files and Tier-0 labels without blocking on operator input.

## Goal

Make the conductor decide *what to build next the way the operator would* by
building an inspectable, hand-editable **operator persona** and turning operator
**priorities** into ranking pressure — **without adding a second ranker**. The
persona influences *what ships next* through exactly one seam: a matcher that marks
matching discovery proposals `priority:true`, consumed by the *already-shipped*
bounded priority boost. This makes "simulate what I would do" concrete and
auditable while keeping the safe Phase-1/2 ranking path unchanged.

Scope this phase: persona sources (F1), persona synthesis (F2), transcript-mining
refiner (F3), priority + persona-aware matcher/intake (F4), the `/autospec-persona`
calibration interview skill (F5), and digest/control surfacing (F6). The advisory
self-brainstorm panel is **deferred** (see "Deferred").

## Locked decisions (operator, 2026-06-26)

- **Persona source = interview + transcript mining.** `/autospec-persona` is the
  supervised ground-truth seed; transcript mining refines on a cadence.
- **Persona scope = global + per-repo overlay.** A global, operator-level base
  (`~/.autospec/operator-persona.md`) plus a repo-local overlay
  (`.autospec/persona-overlay.md`). **Source-class routing (resolves review P2):**
  repo-local inputs (`docs/memory/`, `AGENTS.md`, `docs/AUTONOMY-CHARTER.md`) feed
  the **overlay**; the **global base** is built only from operator-level inputs —
  the interview answers (operator-level) and cross-repo mined decision patterns.
  This keeps one repo's memory from silently steering another repo.
- **No second ranker (resolves review P0 #1).** The persona/priorities affect
  ranking solely by setting `priority:true` on matching proposals, which the
  existing `explore-research-cycle.sh` boost consumes. The conductor does not rank
  or file issues on a parallel path.
- **Panel deferred** (see "Deferred"); ROI-gated follow-up.

## ROI justification (resolves Phase-1 correction §3)

The remaining cut clears the Phase-1 "cut it if the delta is only rank slightly
differently" bar without relying on the panel:

- **Captures decision *rules*, not just current wants.** `--priorities`/steer say
  *what* to favor now; the interview + persona capture *how* this operator decides
  (quality-vs-speed tolerance, when to defer/reject, fork-vs-invoke, destructive
  comfort), reusable across cycles — steering text cannot encode this.
- **Makes priorities actually bind.** Today `AUTOSPEC_PRIORITY_BOOST` fires only on
  proposals already flagged `priority:true`/`source==steer`; there is **no shipped
  path** from free-text `--priorities` to that flag (review P1 #3). F4 builds that
  missing wire — without it, the operator's stated priorities are inert.
- **Audit trust for weeks-unattended runs.** The persona is a readable, editable
  file; the digest surfaces refresh status, calibration agreement, and which
  priorities biased the day's work. Over weeks this is the difference between
  trusting and re-deriving the conductor's choices.
- **Honest baseline.** The shipped `explore-source-weights.sh` Bayesian weighting
  already biases toward what ships clean. This phase does **not** claim to beat
  that; it adds the *operator-intent* axis (priorities + decision rules) that
  source-weights cannot represent, and routes it through the existing bounded boost
  so the two compose rather than compete.

## Prerequisites (verified 2026-06-26 against repo source)

- **Bounded priority boost exists** (`explore-research-cycle.sh`): a proposal with
  `priority:true` or `source=="steer"` gets `base × AUTOSPEC_PRIORITY_BOOST`,
  **clamped twice** — a multiplier floor at 1.0 *and* a score cap at
  `max(non_boosted_score) × multiplier` so a boosted item can reorder but never
  swamp the field. F4 feeds this exact mechanism; no new ranker.
- **No `--priorities` consumer exists** anywhere in `scripts/` or the skill
  (verified by grep) — F4 must build the free-text→`priority:true` matcher and its
  invocation point. The Phase-1 spec's "feed the existing multiplier" described a
  wire that is not yet there; F4 lays it.
- **Self-generated → sandbox** routing exists (Phase 2 F3) via
  `.autospec/explore-mode.json` + the Phase-4 implementer; discovery issues remain
  routed there. (This phase files no issues on a parallel path.)
- **Trio derivation tooling** exists (`derive-trio.sh --in-place`,
  `gen-skill-goldens.sh`, `validate.sh::check_derive_trio_consistency`).
- **Per-repo memory corpus convention** exists and is repo-slug-scoped:
  `~/.claude/projects/-Users-<user>-IdeaProjects-<repo>/memory` (used by
  `apply-memory-tags.sh`, `assemble-impl-prompt.sh`). F3 pins to this, not an
  unscoped `~/.claude/projects/` glob.
- **Conductor lock is per-repo** (`autospec-loop.sh`); the **global** persona file
  has no lock — F2 must add one (review "Concurrency").

## Features

### F1 — Persona source-gathering helper (deterministic, unit-testable)

`scripts/autonomous-persona-sources.sh` — gathers and **orders** persona inputs in
strict precedence and emits a structured bundle (JSON) for the F2 synthesizer. Pure
shell, no LLM, fully unit-asserted. **Class routing (review P2):**
- **Global-base inputs** (operator-level): interview answers
  `~/.autospec/operator-persona.answers.json` (precedence 0, supervised); cross-repo
  mined patterns from F3 (precedence 3).
- **Overlay inputs** (repo-local): `docs/memory/` `feedback_*`/`project_*` +
  `AGENTS.md` (precedence 1); `docs/AUTONOMY-CHARTER.md` (precedence 2).

Emits two sub-bundles (`global`, `overlay`) so F2 can synthesize the base and apply
the overlay distinctly. Missing sources skipped, never fatal. `set -eu`, if/then/fi,
jq `capture()`/`==`. `+` bats. **This is the deterministic half of the Phase-1
"persona merge is not a pure shell transform" correction.**

### F2 — Persona synthesis (Tier-A LLM) + cache/cadence/lock

The conductor invokes a **Tier-A** agent that reads the F1 bundle and synthesizes
the global persona to `~/.autospec/operator-persona.md`, then applies the per-repo
overlay to produce the effective persona, recording a **confidence note per
dimension**. Refresh governed by `AUTOSPEC_PERSONA_REFRESH_DAYS` (default 7) +
staleness check. **Operator edits preserved:** blocks wrapped in
`<!-- operator-edited -->` … `<!-- /operator-edited -->` are read as input and never
overwritten. **Global-file lock (review concurrency):** F2 acquires a
`mkdir`-based lock on `~/.autospec/.persona.lock.d` before writing the global file,
so two repos' conductors refreshing on cadence cannot corrupt it; lock is advisory
and self-healing (stale lock older than N min is reclaimed). **Tested as an LLM
step, not asserted deterministic** (`feedback_self_consistent_test_fixtures_mask_bugs`):
bats covers cache/cadence/staleness/edit-preservation/lock logic with the synthesis
boundary mocked; a structural presence check validates real output shape without
asserting exact text. **LLM half of the correction.**

### F3 — Transcript-mining refiner (continuous, cadence-bound, scoped)

`scripts/autonomous-persona-mine.sh` — on the persona refresh cadence (NOT every
cycle), mines the session corpus for recurring decision patterns (greenlight vs.
reject, recurring steering phrases, push-back points) → a compact mined-decision
digest consumed as F1 precedence-3. **Corpus pinned (review P1 #5):** defaults to
the repo-slug-scoped path above; cross-repo mining for the *global* base is
explicitly **opt-in** via `AUTOSPEC_PERSONA_MINE_GLOBAL=1` (default off → only this
repo's corpus). **Privacy:** output is *decision patterns*, never raw transcript
text; a redaction pass strips obvious secrets/paths; the persona file lives in
uncommitted `~/.autospec`. **Opt-out:** `AUTOSPEC_PERSONA_MINE=0` disables mining
entirely. Bounded to top-N patterns; logs if truncated
(`feedback_explore_codebase_signals_false_positives`). Fail-soft no-op when the
corpus is absent. `+` bats with a fixture transcript dir (real temp files, not
process-sub — `feedback_bash32_process_sub_test_file`).

### F4 — Priority intake + persona-aware proposal matcher (the binding seam)

Two parts, one child:
1. **Intake.** `--priorities "a; b; c"` flag (and, first run with none supplied,
   no `AskUserQuestion`; infer from persisted priorities, operator persona, and
   Tier-0 control labels, then proceed). Persists to `~/.autospec/autonomous-priorities.md` (operator-editable
   mid-run; appendable via an `autospec:steer` issue). **Wires the control payload
   (review P1 #4):** the conductor's control-channel handling currently discards
   `DIRECTIVE:`/`PRIORITY_ISSUE:` lines after `printf`; F4 captures them into the
   priorities file so steer directives actually persist.
2. **Matcher (resolves review P1 #3).** `scripts/autonomous-priority-match.sh` — a
   **Tier-C** step that, given the priorities file + the effective persona + a
   proposal, decides whether the proposal advances a stated/inferred priority and,
   if so, sets `priority:true` on it **before** the existing boost runs. This is the
   *only* way the persona affects ranking — through the existing clamped boost, **no
   second ranker, no parallel filing** (resolves P0 #1). Because the boost is
   clamped (floor 1.0 + score cap), a wrong match can reorder but cannot starve or
   drop a candidate (resolves P0 #2). `+` bats (parse, persist, steer-capture,
   match sets the flag, boost-then-clamp respected, never bypasses the safety gate).

### F5 — `/autospec-persona` calibration interview skill (full trio)

Standalone top-level skill (`feedback_autospec_skill_per_capability`),
trio-authored (SKILL.md → `derive-trio.sh --in-place` → `gen-skill-goldens.sh`,
**atomic in one issue** — `feedback_decompose_trio_prose_goldens_atomic`). A Tier-A
agent reads the repo (`docs/specs/`, `docs/memory/`, `AGENTS.md`, charter, recent
commits, open issues, code areas) and **generates ≤50 repo-grounded questions**
(hard cap) from real tensions, asked in themed `AskUserQuestion` batches
(multiple-choice preferred). **Resume format defined (review):** answers persist to
`~/.autospec/operator-persona.answers.json` as
`{"version":1,"batches":[{"id","status":"done|pending","questions":[{"q","choice","free"}]}],"next_batch"}`;
re-running resumes at `next_batch`, never re-asking `done` batches.
**Calibration metric defined (review):** interleaved questions whose answer is
inferable from memory/charter measure agreement as *% of multiple-choice
calibration questions where the operator's choice matched the inferred prediction*
(free-text excluded from the %, optionally LLM-graded as a note); disagreements
correct the model, not just append. Output updates the source-0 answers file (global
base) with per-dimension confidence. **First-issue structural sections required**
(`feedback_autospec_decomposer_gotchas`): Startup self-update, Model-tier adapter
row, harness-detection block; codex/prompt.md leading blank line. Lifecycle:
`/autospec-autonomous` checks for the persona on startup; if absent, offers to run
`/autospec-persona` (operator may skip → fall back to inferred sources). **New
validate gate (review P3):** `tests/persona/*.bats` is a net-new top-level dir with
no existing glob — this child adds a `check_persona_suite` function to `validate.sh`
in the same commit (`tests/autonomous/*` is auto-globbed; `tests/persona/*` is not).
`+` bats + trio/golden gates.

### F6 — Digest + control-channel surfacing

Extend the daily digest (Tier-C render) and control channel: digest gains a
**persona block** (last refresh, per-dimension confidence, calibration-agreement %)
and a **priorities block** (active priorities + which filed work each biased);
control channel gains `autospec:recalibrate-persona` to trigger a persona refresh /
interview re-run on the next cycle. Smallest child; depends on F2/F4/F5.
`+` bats (blocks render; recalibrate label triggers refresh).

## Deferred — advisory self-brainstorm panel (ROI-gated follow-up)

Both reviews recommend deferring the panel; recorded here so the design intent
survives. A future phase MAY add an operator-proxy panel **only after** evidence
that persona-fed ranking (F4) + Bayesian source-weights underperform. If built, it
must: (a) emit a **per-candidate multiplier file** that `explore-research-cycle.sh`
reads (mirroring `explore-source-weights.sh`) rather than ranking/filing on a
parallel path — preserving "no second ranker"; (b) include a **starvation guard**
(force a never-selected high-confidence candidate to surface after N cycles;
escalate a starved category); (c) define ledger-reject suppression precisely —
suppress only `refuted`/`abandoned` (never transient `qa_failed`/`stalled`) with an
operator-priority re-eligibility override. Gated by `AUTOSPEC_PANEL` (default off).

## Decomposition preview (6 children + 1 epic + Phase 5.5 audit)

1. **EPIC** — /autospec-autonomous Phase 3 (operator persona & priority intelligence).
2. **F1** persona source-gathering helper (deterministic, class-routed) + bats. **Blocks F2/F4.**
3. **F2** persona synthesis (Tier-A) + cache/cadence/edit-preservation/global-lock + bats. Depends F1.
4. **F3** transcript-mining refiner (scoped, opt-in global, redacted) + bats. Depends F1.
5. **F4** priority intake + control-payload capture + persona-aware matcher + bats. Depends F2.
6. **F5** `/autospec-persona` interview skill (trio + goldens atomic, adds `check_persona_suite`) + bats. ctx:120k reasoning:deep, Tier A.
7. **F6** digest + control-channel surfacing + bats. Depends F2/F4/F5.
8. **Phase 5.5 audit + remediation** — never skip (`feedback_per_pr_lgtm_misses_integration`); verify the deterministic/LLM persona split, class routing (repo memory → overlay, not global base), **no-second-ranker** (persona affects ranking ONLY via `priority:true` → clamped boost), boost clamp holds (no starvation/drop), mining corpus is repo-scoped + opt-in-global + redacted + opt-out honored, global-persona lock, and `tests/persona/*` is gated.

Each child ≤400 words, ≤3 logical units; trio prose + goldens atomic; edit SKILL.md
→ `derive-trio.sh --in-place` + `gen-skill-goldens.sh`. New test dirs/files gated by
`validate.sh` in the same child (anchor: #1402 added `check_autonomous_phase2_suite`
after `tests/autonomous/*.bats` rotted ungated — `validate.sh` autonomous glob).

## Tests

External boundaries (LLM synthesis, `gh`, transcript reads, `AskUserQuestion`,
matcher LLM) mocked as subprocesses. No gitignored fixtures — note
`.autospec/persona-overlay.md` and `.autospec/operator-persona.answers.json` are
**intentionally operator-local** (gitignored); tests materialize fixtures at
runtime (`feedback_gitignored_fixtures_pass_in_authoring_worktree`). bats `[ -f ]`
writes a real temp file first (`feedback_bash32_process_sub_test_file`); `set -eu` +
if/then/fi; jq `capture()`/`==` not `test()`; no RETURN traps.

- `tests/autonomous/test_persona_sources.bats` — precedence + class routing (repo
  memory → overlay; interview/mined → global base); per-repo overlay overrides;
  missing source skipped not fatal.
- `tests/autonomous/test_persona_synthesis.bats` — cadence/staleness; operator-edit
  block preserved; global-lock prevents concurrent clobber; synthesis mocked;
  structural presence check.
- `tests/autonomous/test_persona_mine.bats` — patterns from fixture transcripts;
  repo-scoped by default, global only under `AUTOSPEC_PERSONA_MINE_GLOBAL=1`;
  redaction strips secrets; `AUTOSPEC_PERSONA_MINE=0` no-op; absent corpus no-op.
- `tests/autonomous/test_priority_intake.bats` — parse/persist; control-payload
  `DIRECTIVE:`/`PRIORITY_ISSUE:` captured (not discarded); matcher sets
  `priority:true` on a matching proposal; boost applies then clamps; never bypasses
  the gate; no parallel filing path created.
- `tests/persona/test_persona_skill.bats` — ≤50-question cap; resume from
  `next_batch`; calibration-agreement % computed over multiple-choice only; answers
  file shape.
- `autospec validate` — new `tests/autonomous/*` auto-globbed; **new
  `check_persona_suite` gates `tests/persona/*`**; `/autospec-persona` trio goldens
  regenerated; `check_derive_trio_consistency` passes.

## Self-review

- **Placeholders:** none.
- **Consistency:** persona build split per the Phase-1 correction (F1 deterministic
  unit-asserted; F2 LLM evaluated). Persona affects ranking through exactly one seam
  (F4 → existing clamped boost) — no second ranker, no parallel filer (resolves
  review P0 #1). Class routing keeps repo memory out of the global base (review P2).
- **Scope:** one multi-issue pipeline gated on F1; panel deferred (review consensus
  + operator decision); ROI justified honestly against the Bayesian baseline, not a
  flat-ranking strawman.
- **Critical risks:** (a) wrong persona/priority match mis-ranks → bounded by the
  existing floor+cap clamp; cannot starve or drop (resolves P0 #2); (b)
  LLM-determinism fallacy → F1/F2 split + evaluated-not-asserted tests; (c) mining
  contamination/PII → repo-scoped default, opt-in global, redaction, opt-out,
  uncommitted output (resolves P1 #5); (d) cost → mining + synthesis on cadence not
  per-cycle, Tier-C matcher/digest, `AUTOSPEC_PERSONA_*` levers; (e) global-file
  races → F2 lock; (f) integration drift → Phase 5.5.
- **On merge:** flip the persona/intake items from "roadmap" to shipped in the
  Phase-1 spec's Phasing section (leave the panel as deferred); note the persona in
  `docs/AUTONOMY-CHARTER.md`.
