# autospec-explore — discovery precision refinement (gap-confirmation + fail-closed verify)

## Summary

A live local-tier `/autospec-explore` discovery pass on the autospec repo itself
produced **183 deduped candidate proposals that collapsed to 0 confidently
file-worthy** after adversarial verification. Even the highest-signal source
(`source-analysis`) was **4/4 factually wrong** on direct evidence-checking
(claimed README/tests/install-guard were missing when all three already exist).

Root cause — a single missing stage: **researchers emit a "gap exists" claim and
nothing ever confirms the gap is real against the current files.** Three
compounding researcher bugs amplify it, and the aggregator's verify stage fails
**open** (everything survives) when no LLM skeptic is wired, which is exactly the
autonomous `--once` subprocess path.

This spec **extends** the existing `dedup → verify → ROI → synthesis → rank`
pipeline — it does not replace it — with one new deterministic stage, one
inverted failure mode, and four targeted researcher patches. Scope is
deliberately bounded: no researcher rewrite, no new researchers, internet
researcher untouched this pass.

## Problem detail (empirical, from the live run)

| Source | Raw | Defect |
|---|---|---|
| `quality-resilience` | 100 (cap) | `assert_*`-only detector misses native bats `[ … ]`/`run`; self-declares every hit `silent-wrong`/0.85, flooding the severity-first rank; saturates its cap |
| `self-leverage` | 50 (cap) | greps `skills/**/*.md` + `docs/specs/*.md` — matches **prose describing** human-in-loop steps (incl. deliberate safety gates) and proposes "auto-resolving" them |
| `codebase-signals` | 24 | known TODO/FIXME-grep prose-noise class |
| `source-analysis` | 5 | LLM fed only `head -c 8000 README.md`; declares things "missing" that sit below the 8KB cut; no re-check |
| `spec-vs-code` | 1 | matched the template placeholder `{{ac_bullets_from_taxonomy}}` as an unimplemented AC |

## Design

### 1. New stage — deterministic gap-confirmation (the core fix)

Inserted in `explore-research-cycle.sh` **between dedup and verify**. Proposals
may carry a new optional `gap_check` object:

```json
"gap_check": { "kind": "absent", "needle": "autospec-autonomous", "haystack": "README.md" }
```

Confirmation rules (all deterministic, repo-relative, no LLM):

- `kind: "absent"` — the proposal claims `needle` is **missing** from `haystack`.
  Confirm by fixed-string searching `haystack` for `needle`. **If found → drop**
  (the gap does not exist). `haystack` may be a file, a glob, or `"<repo>"` for a
  repo-wide `git grep -F`.
- `kind: "present"` — the proposal claims a real call-site/pattern **exists** at
  `needle` in `haystack`. Confirm it is actually there; **if absent → drop**.
- **Refute-by-default for gap-claiming sources.** Any proposal carrying a
  `gap_check` is verified regardless of source. Additionally, sources in
  `GAP_CLAIMING_SOURCES` — **default `source-analysis`, `self-leverage`** (the two
  researchers converted to emit `gap_check` this pass; configurable via
  `AUTOSPEC_EXPLORE_GAP_CLAIMING_SOURCES`) — whose proposal carries **no**
  `gap_check` are dropped, forcing a falsifiable claim. Sources NOT in the set
  (`spec-vs-code`, `codebase-signals`, `quality-resilience`, `dogfooding`,
  `dependency-health`, …) keep their existing behavior — they are not killed for
  omitting `gap_check`. Converting more sources is a deliberate later edit; this
  keeps the valuable `spec-vs-code` source alive and is backward compatible with
  existing aggregator tests.
- `needle`/`haystack` are validated: `haystack` must resolve inside the repo
  (no absolute paths, no `..` escape); a malformed/zero-match-impossible
  `gap_check` drops the proposal (fail-closed) and is counted.

New counter `proposals_after_gap_confirm` is emitted in the cycle JSON and the
per-iteration log. Each drop is recordable to the outcome ledger as
`outcome=gap_unconfirmed`.

### 2. Fail-closed verify in the autonomous path

Today `verify_mode` degrades to `no-op-unverified` and **all proposals survive**
when no skeptic verdict map is supplied. That is correct for an interactive run
(the operator is the skeptic) but wrong for autonomous auto-shipping.

New behavior, gated on an explicit `--autonomous` flag / `AUTOSPEC_EXPLORE_AUTONOMOUS=1`
(set by the `--once` path and the conductor):

- autonomous **and** `verify_mode == no-op-unverified` → **cap final output to 0**,
  emit `code_health:explore_verify_unavailable_failclosed`, set
  `failclosed: true` in the cycle JSON. Nothing is filed.
- interactive (default) → unchanged (current `no-op-unverified` survive behavior).

This guarantees the autonomous loop never files an unverified proposal. The
`--once` mode reports `dry=true, reason="verify-unavailable-failclosed"` in that
case, so the conductor counts it as a dry cycle and parks rather than shipping
noise.

### 3. Four researcher patches

- **`source-analysis.sh`** — read the **full** `README.md`/`AGENTS.md` (drop the
  `head -c 8000` truncation; cap at a generous 64KB and note truncation in the
  prompt if exceeded). Instruct the LLM that every "X is missing/undocumented"
  proposal **must** include a `gap_check{kind:"absent", needle, haystack}` so
  stage 1 re-verifies it; proposals without it are dropped downstream.
- **`self-leverage.sh`** — restrict the grep to **executable call sites** in
  `scripts/**/*.sh` (real `read -r`/`read -p`/`AskUserQuestion` invocations), and
  **stop scanning `*.md` prose entirely**. Each proposal emits
  `gap_check{kind:"present", needle:"<callsite token>", haystack:"<script:line>"}`.
- **`quality-resilience.sh`** — fix the assertion detector to recognize native
  bats assertions (`[ … ]`, `[[ … ]]`, `run` + status checks, `assert*`); when
  more than `COLLAPSE_THRESHOLD` (default 8) files match the same lens, emit a
  **single structural proposal** ("add an assertion-density floor lint") that
  lists the instances, instead of one issue per file; lower the per-round cap to
  25. Assertion-free proposals carry
  `gap_check{kind:"present", needle:"@test", haystack:"<bats file>"}`.
- **Anti-saturation (aggregator)** — after dedup, any single source contributing
  `> SATURATION_FRACTION` (default 0.40) of candidates is flagged `saturated`:
  its candidates are sampled down to the fraction cap and its effective
  `source_weight` is penalized for that round. Emits a `saturated_sources` list
  in the cycle JSON.

### 4. Components & interfaces

- `schemas/autospec-explore-proposal.schema.json` — add optional `gap_check`
  (`kind` enum `absent|present`, `needle` string, `haystack` string). Backward
  compatible: `additionalProperties` is already `true`; legacy proposals without
  it are unaffected except for the refute-by-default rule on gap-claiming sources.
- `explore-research-cycle.sh` — new gap-confirm pass (own Python block, mirrors
  the existing dedup/verify blocks); fail-closed branch in the finalize stage;
  saturation flagging in the aggregate step. New env knobs:
  `AUTOSPEC_EXPLORE_AUTONOMOUS`, `AUTOSPEC_EXPLORE_SATURATION_FRACTION`,
  `AUTOSPEC_EXPLORE_GAP_CLAIMING_SOURCES` (override the default set).
- `autospec-explore.sh` — `--once` exports `AUTOSPEC_EXPLORE_AUTONOMOUS=1` and
  surfaces `failclosed`/`gap_unconfirmed` counts in the 6-key yield JSON's
  `reason`.
- SKILL.md trio (`SKILL.md` + `codex/prompt.md` + `opencode/agent.md`) — document
  the new stage order `dedup → gap-confirm → verify → ROI → synthesis → rank`,
  the fail-closed autonomous rule, and the `gap_check` contract. Derived via
  `derive-trio.sh --in-place`; goldens regenerated with `gen-skill-goldens.sh`
  in the same change.

### 5. Error handling

- Malformed `gap_check` (missing field, path escape, unreadable haystack) →
  drop the proposal (fail-closed), increment `gap_check_malformed`, never crash
  the cycle.
- A `git grep`/read failure inside gap-confirm → treat that single proposal as
  unconfirmed (drop), continue with the rest.
- Empty roster after gap-confirm + verify → cycle returns 0 proposals cleanly
  (a dry round is a valid, honest outcome, not an error).

### 6. Testing

- `tests/explore/test_explore_gap_confirm.bats` — absent+found→drop;
  absent+missing→keep; present+missing→drop; present+found→keep;
  gap-claiming-source + no gap_check→drop; non-gap-claiming-source + no
  gap_check→keep; malformed gap_check→drop; path-escape `haystack`→drop.
- `tests/explore/test_explore_research_cycle.bats` (extend) — fail-closed caps to
  0 under `AUTOSPEC_EXPLORE_AUTONOMOUS=1` + no verdict map; interactive unchanged;
  saturation flag + down-sample.
- `tests/explore/test_explore_researchers.bats` (extend) — source-analysis reads
  full file & emits gap_check; self-leverage ignores `*.md`; quality-resilience
  recognizes native bats assertions & collapses.
- `bash scripts/validate.sh` green (lockstep trio + goldens + schema checks).

### 7. Scope guard (YAGNI)

Not in scope: rewriting all 11 researchers, adding new researchers, changing the
internet researcher, changing ranking weights beyond the saturation penalty, or
touching `/autospec-run`. Those are separate efforts if ever justified.
