# Harness-Aware Model Tier Resolution for autospec Skills

**Date:** 2026-05-07  
**Status:** Draft  
**Scope:** `skills/autospec/SKILL.md`, `skills/autospec-run/SKILL.md`, `skills/autospec-define/SKILL.md`, `skills/autospec-classify/SKILL.md`, `skills/autospec-review/SKILL.md`, `AGENTS.md`, `scripts/validate.sh`

---

## 1. Problem

Every autospec skill embeds the same verbose multi-harness tier string in each subagent dispatch brief:

```
Claude Code: `sonnet`; Codex: `gpt-5.1-codex-spark`; OpenCode: smaller task tier. Fall back UP on unavailability.
```

When autospec runs in Claude Code, the brief still carries GPT model names that Claude cannot use. The orchestrator must silently discard the non-applicable harness lines. This is fragile, wastes tokens, and has caused model-selection confusion in practice. Worse, the Tier A counterpart repeats the same cross-harness listing in eight additional locations. There is no explicit fallback-up rule that says "do not ask the user".

## 2. Solution

### 2.1 Harness detection block

Add a `## Harness detection` section to each SKILL.md, positioned **immediately before Phase 0** (or before the first `## Phase` heading for skills that lack a Phase 0). This block runs once at skill load and resolves two variables:

- `TIER_A` — top model, extended thinking, spec/review work  
- `TIER_B` — cheaper model, medium thinking, implementation work

Detection logic (prose instruction to the orchestrator LLM):

```
Detect your harness by checking available tools before any phase:

1. **Claude Code** — the `Agent` tool with a `subagent_type` parameter is available.
   TIER_A = `opus` + `ultrathink`  (model ID: claude-opus-4-7)
   TIER_B = `sonnet`               (model ID: claude-sonnet-4-6)

2. **OpenCode** — a `task` tool with model/tier configuration is available (no `subagent_type`).
   TIER_A = top-tier task model + high reasoning
   TIER_B = smaller-tier task model + medium reasoning

3. **Codex CLI** — neither `Agent` nor a configurable `task` tool is available; `apply_patch` is the primary edit tool.
   TIER_A = current top GPT model + `reasoning_effort=high`
   TIER_B = `gpt-5.1-codex-spark` + `reasoning_effort=medium`

**Fallback rule:** If TIER_B is not available in your harness (model unknown, tool absent, or
tool call returns an error for that model), silently use TIER_A instead. Never ask the user.

Hold TIER_A and TIER_B for the entire skill run. Every "Tier A" and "Tier B" reference below
resolves to these harness-specific values.
```

### 2.2 Simplified tier briefs

Replace every verbose multi-harness tier string in subagent dispatch `> **Model tier:**` blockquotes with a short reference to the resolved variable.

**Tier A brief (before):**
```
> **Model tier:** Tier A (spec work) — top model with extended/maximum thinking per AGENTS.md.
> Claude Code: `opus` + `ultrathink`; Codex: current top GPT + `reasoning_effort=high`;
> OpenCode: top task tier. Fall back UP on unavailability.
```

**Tier A brief (after):**
```
> **Model tier:** `TIER_A` (spec work) — top model with extended thinking; resolved at startup.
```

**Tier B brief (before):**
```
> **Model tier:** Tier B (implementation work) — cheaper model with medium thinking per AGENTS.md.
> Claude Code: `sonnet`; Codex: `gpt-5.1-codex-spark`; OpenCode: smaller task tier.
> Fall back UP on unavailability.
```

**Tier B brief (after):**
```
> **Model tier:** `TIER_B` (implementation work) — cheaper model with medium thinking; resolved
> at startup. Silently fall back to `TIER_A` if unavailable.
```

The guardian audit brief (Tier A variant inside process(ISSUE)) follows the same pattern.

### 2.3 AGENTS.md update

Add a `### Harness detection protocol` subsection to the existing `## Subagent model selection (two-tier, cost-aware)` section that:

1. Documents the three-step detection logic (mirrors §2.1 above).
2. States the fallback rule explicitly: "If TIER_B is unavailable in your harness, silently use TIER_A. Do not ask the user."
3. Cross-references the per-harness model names already listed in the tier table.

The existing `## Subagent model selection` heading and tier table are preserved unchanged so `validate.sh` keeps passing.

### 2.4 validate.sh update

Add a new `check_harness_detection_block()` function that verifies each SKILL.md contains:

1. A `## Harness detection` heading.
2. The literal strings `TIER_A` and `TIER_B` in the body of that section.
3. The fallback rule string `Silently` (or `silently`) in the same section.

Also update the existing `check_subagent_model_tier()` tier-brief checker so it accepts **either** the old verbose multi-harness format OR the new `TIER_A`/`TIER_B` reference format. This allows incremental migration without breaking CI for partially-migrated states.

Call `check_harness_detection_block` from the main validation loop alongside the existing checks.

---

## 3. Affected files and change summary

| File | Change |
|---|---|
| `AGENTS.md` | Add `### Harness detection protocol` subsection; add explicit fallback-up-silently rule |
| `skills/autospec/SKILL.md` | Add `## Harness detection` block; replace 7 tier briefs (4× Tier A, 2× Tier B, 1× guardian) |
| `skills/autospec-run/SKILL.md` | Add `## Harness detection` block; replace 6 tier briefs (3× Tier A, 3× Tier B) |
| `skills/autospec-define/SKILL.md` | Add `## Harness detection` block; replace 4 tier briefs |
| `skills/autospec-classify/SKILL.md` | Add `## Harness detection` block; replace 1 tier brief |
| `skills/autospec-review/SKILL.md` | Add `## Harness detection` block; replace 1 tier brief |
| `scripts/validate.sh` | Add `check_harness_detection_block()`; update `check_subagent_model_tier()` to accept both formats |

---

## 4. Issue decomposition plan

All child issues carry `auto-implement`. Issues are ordered by dependency.

1. **AGENTS.md — add harness detection protocol subsection** (no deps)
2. **validate.sh — add `check_harness_detection_block()` + update tier-brief acceptor** (depends on #1)
3. **autospec/SKILL.md — add harness detection block + simplify tier briefs** (depends on #2)
4. **autospec-run/SKILL.md — add harness detection block + simplify tier briefs** (depends on #2)
5. **autospec-define/SKILL.md — add harness detection block + simplify tier briefs** (depends on #2)
6. **autospec-classify/SKILL.md + autospec-review/SKILL.md — add harness detection block + simplify tier briefs** (depends on #2)

---

## 5. Out of scope

- `model-profiles.yml` runtime profile system (`ctx:*` × `reasoning:*` ordinals) — orthogonal to tier policy; not changed.
- Self-update mode detection logic in `autospec-run/SKILL.md` — separate concern; not changed.
- Any install-time stamping or script-based resolver — rejected in favour of the simpler in-skill detection block.
- New user-facing commands or flags — no user-facing API change; detection is fully transparent.

---

## 6. Acceptance criteria

- [ ] Each of the 5 SKILL.md files contains a `## Harness detection` section with `TIER_A`, `TIER_B`, and the fallback rule.
- [ ] No SKILL.md file contains the legacy string `Claude Code: \`sonnet\`; Codex:` in a `> **Model tier:**` blockquote.
- [ ] `scripts/validate.sh` run passes with exit 0 against all 5 updated SKILL.md files.
- [ ] `scripts/validate.sh` run passes with exit 0 against a SKILL.md that still uses the old verbose format (backward-compatible acceptance in `check_subagent_model_tier()`).
- [ ] AGENTS.md contains a `### Harness detection protocol` subsection under `## Subagent model selection`.
- [ ] The existing `check_subagent_model_tier()` validation in `validate.sh` still passes (heading-level check unchanged).
