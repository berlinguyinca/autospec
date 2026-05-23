---
name: Autospec design preferences
description: User's recurring design choices when shaping autospec features — small-LLM target, correctness over speed, tight triggers, conservative guardrails
type: feedback
originSessionId: 7205d05b-f2fd-4cde-9ced-ecc266a9bc7b
---
When the user runs `/autospec` (or any of the family) on the autospec repo itself, these design choices repeat across sessions and should be assumed unless they say otherwise:

1. **Small local LLM is the target consumer** of generated child issues. Concrete sizing: "ensure it fits in the context of small LLM with like 60-120k context window." Combine multiple sizing caps to enforce: ≤400 words body + ≤30 lines implementation outline + ≤3 files touched. Do not propose a single soft cap.

2. **Correctness >> speed.** Verbatim: "we don't really care how long it takes as long as it gets executed right the first time." Therefore: prefer conservative guardrails (Phase 1 max 25 tool calls, Phase 4 max 40 + 3 self-reviews) over looser/tighter; no wall-clock caps; many small dependent issues over one big issue.

3. **Tight imperative-verb-only triggers** for any keyword-listener feature. Bare nouns ("issue", "spec", "ticket") are NOT triggers — too noisy. Only fire on phrases like "file an issue" / "write a spec" / "open an issue" — explicit intent.

4. **New surfaces over extending old ones.** When adding a new behavior shape, prefer a new sibling skill (e.g. `autospec-listen`) over folding it into existing `autospec` / `autospec-define`. Keeps activation surfaces clean and avoids over-triggering on every passing keyword mention.

5. **Lock-step rule is non-negotiable.** Every multi-harness skill keeps SKILL.md / opencode/agent.md / codex/prompt.md bodies identical. Tighten prompts in SKILL.md and let the existing validation enforce parity; never edit one variant without the others.

Why: These preferences emerged from the 2026-05-01 brainstorm where the user accepted recommended options on 13 of 14 design forks; the only customization was on issue sizing where they wanted multiple caps combined. Pattern indicates these design instincts are stable.

How to apply: When proposing autospec design options, lead with the choice that matches these preferences as the "(Recommended)" option. Don't ask the user to re-litigate "small vs big issues" — assume small. Don't ask "should we be permissive on triggers" — assume tight. Save the user's brainstorm budget for genuinely novel forks.
