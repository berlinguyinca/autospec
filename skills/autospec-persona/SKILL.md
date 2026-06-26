---
name: autospec-persona
description: Use when the operator wants to calibrate autospec to their judgment — runs a repo-grounded calibration interview (≤50 questions in themed AskUserQuestion batches), interleaves calibration questions whose answers are inferable from memory/charter to measure agreement, and writes ~/.autospec/operator-persona.answers.json. Resumable — re-running continues at the next pending batch and never re-asks done batches.
---

# autospec-persona (harness-neutral)

Standalone calibration-interview skill. A Tier-A agent reads the current repo
(`docs/specs/`, `docs/memory/`, `AGENTS.md`, the autonomy charter, recent
commits, open issues, and the main code areas), surfaces the real tensions and
recurring decisions, and turns them into **≤50 repo-grounded questions** (a hard
cap) asked in themed `AskUserQuestion` batches (multiple-choice preferred). The
operator's answers persist to `~/.autospec/operator-persona.answers.json` and the
run is fully **resumable**: re-invoking continues at the next pending batch and
never re-asks a `done` batch.

Manage your own context — never exceed 60%. Delegate to subagents whenever your harness supports it.

<!-- autospec-block:startup-self-update SKILL_NAME=autospec-persona -->

## Self-update mode

If the feature-request argument matches the regex `^\s*update\s*$` (case-insensitive, whitespace-padded), this skill enters self-update mode and does not run the normal interview:

1. **Detect harness** by checking which install path exists for this skill:
   - Claude Code: `~/.claude/skills/autospec-persona/SKILL.md`
   - OpenCode:    `~/.config/opencode/agent/autospec-persona.md`
   - Codex CLI:   `~/.codex/prompts/autospec-persona.md`
2. **Re-install the full autospec suite from `main`** by piping the canonical installer:
   ```bash
   curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh | bash -s -- --skill all --harness all --update
   ```
   Run this one-liner once; it refreshes all autospec skills across all harnesses.
3. **Show the diff** between the prior installed file(s) and the freshly fetched copy (e.g. `diff <(cat <prior>) <(curl -fsSL ...SKILL.md)` or the equivalent recorded by the installer).
4. **Stop.** Do not read the repo, generate questions, or run the interview. Print the upgrade summary and return to the user.

If no install path is detected, print `Self-update: no installed copy of autospec-persona found; run install.sh first.` and exit.

## Invocation

```
/autospec-persona [--reset] [--max-questions N] [--dry-run]
```

- `--reset` — discard any existing `~/.autospec/operator-persona.answers.json` and
  start a fresh interview from `next_batch=0`. Without it, the skill **resumes**.
- `--max-questions N` — lower the question budget below the hard cap of 50
  (never above; a value > 50 is clamped to 50).
- `--dry-run` — generate and print the themed batches and the calibration
  predictions without calling `AskUserQuestion` or writing the answers file.

## Required capabilities & harness adapter

This workflow assumes a small set of capabilities. Map each one to your harness's actual tool. If a capability is missing, use the listed fallback.

| Capability                  | Claude Code                          | OpenCode                                 | Codex CLI                                | Fallback if missing                                |
|-----------------------------|--------------------------------------|------------------------------------------|------------------------------------------|----------------------------------------------------|
| Read-only codebase research | `Agent` (subagent_type=Explore)      | `task` agent in read-only mode           | `apply_patch` read-only / shell `grep`   | Do the search in-thread with `rg`/`grep`           |
| Foreground delegation       | `Agent` (subagent_type=general-purpose) | nested `task` agent, await output     | spawn nested CLI session                 | Do the work in-thread (more context cost)          |
| Ask the user a question     | `AskUserQuestion`                    | inline prompt                            | inline prompt                            | Ask in the response and wait for the next turn     |
| Subagent model tier         | Tier A: `opus` + `ultrathink`; Tier B: `sonnet` + medium thinking | Tier A: top `task` model + high reasoning; Tier B: smaller-tier `task` + medium reasoning | Tier A: top GPT + `reasoning_effort=high`; Tier B: `gpt-5.1-codex-spark` + `reasoning_effort=medium` | Honor the per-phase tier mapping in AGENTS.md; retry the same subagent UP on unavailability |
<!-- autospec-block:harness-adapter-core -->

**Persistent project notes**: write durable preferences to **`AGENTS.md`** in the repo root — this is the de-facto standard recognized by Claude Code (also reads `CLAUDE.md`), OpenCode, and Codex. If your harness has its own private memory (e.g. Claude Code's `~/.claude/.../memory/`), mirror the same content there. Per AGENTS.md, subagent dispatches use a **two-tier policy**: Tier A (top model + extended thinking) for spec work and Tier B (cheaper model + medium thinking) for implementation work. Calibration interviewing is spec-tier judgment work: the answers file this skill writes is precedence-0 for every downstream persona-aware decision, so the repo-reading + question-generation + calibration-prediction agent runs on Tier A. The orchestrator keeps the user's invoked model. Fall back UP the tier on quota/capacity or other unavailability by retrying the same subagent with the stronger tier while preserving parent context.

---

## Harness detection (run once at skill start, before reading the repo)

Detect your harness by checking available tools before any phase:

1. **Claude Code** — the `Agent` tool with a `subagent_type` parameter is available.
   - `TIER_A` = `opus` + `ultrathink`  (model ID: claude-opus-4-7)
   - `TIER_B` = `sonnet`               (model ID: claude-sonnet-4-6)

2. **OpenCode** — a `task` tool with model/tier configuration is available (no `subagent_type`).
   - `TIER_A` = top-tier task model + high reasoning
   - `TIER_B` = smaller-tier task model + medium reasoning

3. **Codex CLI** — neither `Agent` nor a configurable `task` tool is available; `apply_patch` is the primary edit tool.
   - `TIER_A` = current top GPT model + `reasoning_effort=high`
   - `TIER_B` = `gpt-5.1-codex-spark` + `reasoning_effort=medium`

**Fallback rule:** If `TIER_A` is not available in your harness (model unknown, quota/capacity failure, authorization failure, or tool call returns an error for that model), silently retry the same subagent dispatch with the next available stronger model. Preserve the parent context on retry; for Codex native subagents, fork/inherit the current conversation context and use the latest top GPT model instead of moving the work into the main session. Never ask the user.

Hold `TIER_A` and `TIER_B` for the entire skill run. Every "Tier A" and "Tier B" reference below resolves to these harness-specific values.

## Answers-file contract

The interview reads and writes a single global file:

```
~/.autospec/operator-persona.answers.json
```

Shape (version 1):

```json
{
  "version": 1,
  "batches": [
    {
      "id": "<theme-slug>",
      "status": "done",
      "questions": [
        { "q": "<question text>", "choice": "<selected option or null>", "free": "<free-text or null>" }
      ]
    }
  ],
  "next_batch": 1
}
```

- `version` — always `1`; a file with a different `version` is treated as
  incompatible and (with operator confirmation) reset.
- `batches[]` — one entry per themed batch, in ask order. `status` is `done`
  once every question in the batch is answered, otherwise `pending`.
- `questions[]` — each carries the prompt `q`, the selected multiple-choice
  `choice` (or `null` for free-text-only), and the `free` free-text note (or
  `null`). A multiple-choice question has a non-null `choice`.
- `next_batch` — the index into `batches[]` of the first batch whose `status` is
  not `done`. When every batch is `done`, `next_batch` equals the batch count
  (the terminal "complete" state).

Writes are atomic (temp file + `mv`) so an interrupted batch never corrupts the
file.

## Phase 1 — read the repo (Tier A)

> **Model tier:** **TIER_A** (spec-tier judgment). Dispatch ONE read-only
> research subagent to survey the repo and extract decision tensions; never
> escalate to a second Tier-A pass for a single interview.

Dispatch a read-only research subagent to survey, in priority order:

1. `docs/specs/*.md` — design tensions, deferred trade-offs, "review"-tagged
   decisions.
2. `docs/memory/` and the harness auto-memory — recurring operator preferences
   and prior feedback lessons.
3. `AGENTS.md` and any autonomy charter — standing rules and default-locks.
4. Recent commits (`git log`) and open issues (`gh issue list`) — what the
   operator actually prioritizes and rejects.
5. The main code areas — the abstractions where judgment calls recur.

The subagent returns a ranked list of **decision tensions**: each a real,
recurring choice the operator makes (e.g. "speed vs. correctness on flaky
tests", "auto-merge vs. hold for review"), with the option space for each.

## Phase 2 — generate ≤50 questions in themed batches (hard cap)

Turn the ranked tensions into questions, grouped into themed batches (e.g.
"merge autonomy", "test discipline", "spec rigor"). Rules:

- **Hard cap:** the total number of questions across all batches is **≤50**
  (or `--max-questions N` if lower). If more than 50 candidate questions exist,
  keep the 50 highest-ranked by recurrence × stakes; drop the rest. Never exceed
  the cap — this is a closed invariant the test suite enforces.
- **Multiple-choice preferred:** phrase each question as 2–4 concrete options
  drawn from the repo's real option space. Use free-text only when the answer is
  genuinely open-ended.
- **Themed batches:** ask one `AskUserQuestion` call per theme (batch), so the
  operator answers related questions together. Each batch becomes one
  `batches[]` entry.

## Phase 3 — interleave calibration questions

Within the batches, mark a subset of multiple-choice questions as **calibration
questions**: ones whose answer the agent can *infer* from memory/charter/commit
history. For each, record the agent's **inferred prediction** before asking. The
operator never sees the prediction; they just answer normally.

After the interview, compute the **calibration-agreement %**:

> agreement % = (number of multiple-choice calibration questions where the
> operator's `choice` matched the inferred prediction) ÷ (total multiple-choice
> calibration questions) × 100

Free-text questions are **excluded** from the percentage (a free-text
calibration answer may optionally be LLM-graded as a side note, but never counts
toward the %). When the operator's choice **disagrees** with the prediction, the
disagreement **corrects the model** — update the per-dimension confidence and the
inferred default for that tension, do not merely append the answer.

## Phase 4 — resume logic

On every invocation (unless `--reset`):

1. If `~/.autospec/operator-persona.answers.json` is absent, start fresh with
   `next_batch = 0`.
2. If present and `version == 1`, read `next_batch` and **skip every batch whose
   `status` is `done`**. Resume asking at `batches[next_batch]`. A `done` batch is
   never re-asked.
3. After each batch is fully answered, set its `status = done`, recompute
   `next_batch` as the index of the first non-`done` batch (or the batch count if
   none remain), and write the file atomically.
4. When `next_batch` equals the batch count, the interview is complete; print the
   summary and the calibration-agreement %.

## Phase 5 — output

On completion, the answers file is the source-0 (global base) persona input
consumed downstream (F1/#1417 reads it as precedence-0). Print:

```
autospec-persona summary
- batches: <done>/<total>
- questions asked: <N> (cap: <cap>)
- calibration (multiple-choice): <matched>/<total> = <agreement>%
- answers file: ~/.autospec/operator-persona.answers.json
```

Per-dimension confidence (raised on agreement, corrected on disagreement) is
recorded alongside each tension so downstream synthesis (F2) can weight it.

## Hard rules

- **≤50 questions, always.** The cap is a closed invariant; never ask more.
- **Never re-ask a `done` batch.** Resume strictly at `next_batch`.
- **Calibration % is multiple-choice only.** Free-text answers never count toward
  the percentage.
- **Disagreements correct the model**, they do not merely append a new answer.
- **Atomic writes only** (temp + `mv`); an interrupted batch must not corrupt the
  answers file.
- This skill writes only `~/.autospec/operator-persona.answers.json`; it never
  edits repo files.
