# AutoSpec Growth — /autospec-grow-define (Plan 3 of 5)

**Status:** Design
**Date:** 2026-07-09
**Author:** berlinguyinca
**Depends on:** Plan 1 foundation, Plan 2 candidate pipeline
**Feeds:** `/autospec-grow-run` (Plan 4)

## Summary

`/autospec-grow-define` is the LLM-driven trio skill that turns a product's
`.autospec/growth.yml` into a batch of linked GitHub issues: it runs the 6
researcher lenses, pipes their candidates through the Plan 2 deterministic
pipeline (validate → dedup → verify → rank), and decomposes the top-N into
`growth:artifact` / `growth:outbound` issues, then stops and hands off to
`/autospec-grow-run`. It mirrors `autospec-define`'s shape (research → issues →
stop) and reuses `autospec-explore`'s embedded researcher-roster pattern.

The deterministic backbone already exists (Plans 1–2); this plan adds the
**orchestration prose** (a lock-step trio skill) that dispatches LLM subagents
around that backbone.

### Non-goals

- No live GSC/GA/GitHub API calls — G0 measurement uses the Plan 1 seam with
  manual/fixture input; live adapters are Plan 4.
- No outbound drafting/gating/publishing — `growth:outbound` issues are filed as
  stubs; Plan 4 drafts, gates, and queues them.
- No implementation of `growth:artifact` issues — that is `/autospec-grow-run`.

## Skill shape

A trio skill under `skills/autospec-grow-define/`:
`SKILL.md` + `codex/prompt.md` + `opencode/agent.md`, bodies byte-identical
(lock-step), derived via `derive-trio.sh --in-place` and fingerprinted with
`gen-skill-goldens.sh`. `codex/prompt.md` carries the leading blank line the
lock-step check requires.

**Required structural sections** (per repo conventions and prior new-skill
gotchas):

- **Self-update mode** — pure prose, no shelled-out user text.
- **Required capabilities & harness adapter** — the adapter row.
- **Harness detection + Model-tier** — resolve `TIER_A` (research, verify,
  decompose) and `TIER_B` (cheap mechanical) once at skill start.
- **Relevant memory injection** — run-start memory query.

Opt-in: the skill reads `.autospec/growth.yml`, validated by Plan 1's
`validate-growth-config.sh`. Without a valid config the skill is inert (prints a
one-line pointer to `growth.yml` and exits).

## Flow

### G-detect (once at start)

Harness detection → resolve `TIER_A`/`TIER_B`; load + validate `growth.yml`;
inject relevant memory. Resolve the growth-shared script dir once.

### G0 — Measure (optional)

If the config points at raw metric files (or a fixture is supplied), run Plan 1
`growth-measure.sh --normalize <provider> <raw>` for each available provider and
merge into a single **metrics envelope** `{gsc?, github?, ...}`. If no metrics
are available the envelope is `{}` — lenses proceed on config + crawl alone.
Live API fetching is Plan 4.

### G1 — Lens research (Tier-A fan-out)

The **Lens roster** — a section embedded in the trio body — defines 6 dispatches,
one Tier-A subagent each, budget-scaled by the lens's ledger source-weight
(Plan 1 `growth-source-weights.sh`; a down-weighted lens gets fewer proposals):

1. **technical-seo** — audit the site (crawl allowed via the harness browser
   tools) for CWV, meta/canonical, schema, sitemap, broken links.
2. **keyword-gap** — from the GSC envelope (if present) + competitor analysis,
   propose pages targeting position-5–20 quick wins.
3. **content-opportunity** — topic clusters, comparison/"vs" pages, docs gaps.
4. **community** — on-topic subreddits/HN/forums/Discords/awesome-lists, each
   annotated with its self-promo rule and a cadence-safe posting suggestion.
5. **directory** — legitimate directories the product qualifies for.
6. **backlink** — guest-post targets and integration partners.

Each subagent receives: the product/site/targets from `growth.yml`, the metrics
envelope, and its lens name; it returns an array of **candidate JSON objects**
conforming to the Plan 2 candidate schema (`lens` set to its own name).

The roster is prose in the lock-step trio (one maintenance surface), matching
how `autospec-explore` embeds its researcher roster.

### G2 — Pipeline (deterministic, Plan 2)

Collect all lens candidates into one JSONL stream, then:

1. **Validate** each with `validate-growth-candidate.sh`; drop invalid
   candidates with a logged reason (a malformed lens output must not abort the
   batch).
2. **Dedup** against the ledger with `growth-candidate-dedup.sh` (full
   seen-set).
3. **Verify** each survivor: dispatch a Tier-A adversarial-verify subagent
   prompted to *refute* the candidate (default to `real:false` on doubt); it
   returns `{real, reason}`. Pass candidate + verdict to
   `growth-candidate-verify.sh`, which emits survivors and records refutations
   to the ledger (feeding per-lens down-weighting).
4. **Rank** the verified survivors with `growth-candidate-rank.sh`.

### G3 — Decompose top-N

Slice the ranked list to `grow.max_issues_per_cycle` (config, default 8), then
by `kind`:

- **`growth:artifact`** → reuse `autospec-define` Phase 3 decomposition: create
  linked, small-LLM-friendly auto-implement issues with model-fit labels, plus
  `growth:artifact` and a channel label. These drain through `/autospec-grow-run`
  → `/autospec-run` like any autospec issue.
- **`growth:outbound`** → a lightweight outbound issue template: the drafted-
  content placeholder, target venue, the self-promo rule it satisfies, rationale,
  and evidence; labeled `growth:outbound` + `growth/needs-draft`. NOT
  auto-implement — Plan 4 drafts, gates, and queues these.

Every filed issue also appends a `pending` ledger line (Plan 1 `--append`) with
its issue number, `lens` as `source`, `kind`, `channel`, and `norm_title`, so
the next cycle dedups against it and G5 (Plan 5) can attribute outcomes.

### Stop

After issues are filed, print a summary (candidates per lens; validated /
deduped / verified / refuted counts; issues filed by kind with numbers) and hand
off to `/autospec-grow-run`. The skill does not implement or publish anything.

## Reuse

- **Plan 1** — `validate-growth-config.sh`, `growth-measure.sh`,
  `growth-source-weights.sh`, `growth-ledger.sh`.
- **Plan 2** — `validate-growth-candidate.sh`, `growth-candidate-dedup.sh`,
  `growth-candidate-verify.sh`, `growth-candidate-rank.sh`.
- **`autospec-define`** — Phase 3 artifact decomposition (invoked, not forked).
- **`autospec-explore`** — embedded researcher-roster pattern.
- **`autospec-shared`** — harness detection + memory injection helpers.

## Error handling

- No/invalid `growth.yml` → inert exit with a pointer.
- A lens subagent that dies or returns malformed output → its candidates are
  skipped (logged); the batch continues (no single lens can abort the run).
- Every candidate is validated before the pipeline; invalid ones are dropped,
  not filed.
- Verify defaults to refuted on ambiguity (Plan 2 fail-closed); an unparseable
  verdict never files an issue.
- `gh` issue-creation failure for one candidate is logged and skipped; the ledger
  line is only appended after the issue is confirmed created (no orphan ledger
  entries).

## Testing

This is a trio skill, so testing differs from Plans 1–2:

- **Lock-step:** `check_lockstep` + `check_derive_trio_consistency` — the three
  bodies stay identical; goldens regenerated with `gen-skill-goldens.sh`.
- **Frontmatter** parse for all three files.
- **Structural-section check** in `validate.sh`: named-content assertions that
  the skill contains Self-update mode, the harness-adapter row, the Model-tier
  table, and the Lens roster (6 lenses).
- **Smoke test (bats)** with **subprocess mocks** for subagent dispatch and `gh`
  (mocks prevent the monitor-stall failure mode): drive the orchestration
  deterministically with a fixed set of mock lens candidates and mock verdicts,
  and assert — invalid candidates dropped; dedup applied against a real
  `growth-ledger.sh --append` ledger; refuted candidate recorded and not filed;
  top-N slice honored; `growth:artifact` vs `growth:outbound` routed to the
  correct template/labels; one `pending` ledger line appended per filed issue;
  no ledger line on a mocked `gh` failure.
- **Install/uninstall** `bash -n` clean; skill registered in the installer.

## Open questions

- Whether the technical-seo lens's site crawl should be mandatory or best-effort
  when browser tools are unavailable in the harness (default: best-effort —
  degrade to config-only proposals, never block).
- `grow.max_issues_per_cycle` default (8) — revisit once real cycles show batch
  sizes that keep `/autospec-grow-run` healthy.
- Whether verify should run per-candidate (simple, more subagents) or batched
  (fewer calls, coarser verdicts). Default: per-candidate, matching the Plan 2
  verify-harness's one-verdict-per-candidate contract.
