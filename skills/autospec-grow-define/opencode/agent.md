---
name: autospec-grow-define
description: Use when the user wants /autospec-grow-define to run the planning half of the autospec-growth pipeline for a product opted into `.autospec/growth.yml` — sync/measure current metrics, fan out the 6-lens growth researcher roster (technical-seo, keyword-gap, content-opportunity, community, directory, backlink), adversarially verify + ROI/severity-rank candidates through the deterministic pipeline, then decompose survivors into linked GitHub issues (growth:artifact / growth:outbound). Stops after decomposition and hands off to /autospec-grow-run for autonomous drain.
mode: primary
---

# autospec-grow-define workflow (harness-neutral)

Run the planning half of the autospec-growth pipeline for a product/site opted
into `.autospec/growth.yml`: **measure → lens research → adversarial-verify +
rank pipeline → decompose into linked GitHub issues.** Stop after decomposition;
the implementation half is handed off to `/autospec-grow-run`.

Manage your own context — never exceed 60%. Delegate to subagents whenever your harness supports it; do not run lens research, adversarial verification, or decomposition directly in the main conversation when a subagent can do it.

<!-- autospec-block:startup-self-update SKILL_NAME=autospec-grow-define -->

## Self-update mode

If the invocation argument matches the regex `^\s*update\s*$` (case-insensitive, whitespace-padded), this skill enters self-update mode and does not run the normal pipeline:

1. **Detect harness** by checking which install path exists for this skill:
   - Claude Code: `~/.claude/skills/autospec-grow-define/SKILL.md`
   - OpenCode:    `~/.config/opencode/agent/autospec-grow-define.md`
   - Codex CLI:   `~/.codex/prompts/autospec-grow-define.md`
2. **Re-install the full autospec suite from `main`** by piping the canonical installer:
   ```bash
   curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh | bash -s -- --skill all --harness all --update
   ```
   Run this one-liner once; it refreshes all autospec skills across all harnesses.
3. **Show the diff** between the prior installed file(s) and the freshly fetched copy (e.g. `diff <(cat <prior>) <(curl -fsSL ...SKILL.md)` or the equivalent recorded by the installer).
4. **Stop.** Do not enter Phase G0 / any pipeline phase. Print the upgrade summary and return to the user.

If no install path is detected, print `Self-update: no installed copy of autospec-grow-define found; run install.sh first.` and exit.

## Required capabilities & harness adapter

This workflow assumes five capabilities. Map each one to your harness's actual tool. If a capability is missing, use the listed fallback.

| Capability                  | Claude Code                          | OpenCode                                 | Codex CLI                                | Fallback if missing                                |
|-----------------------------|--------------------------------------|------------------------------------------|------------------------------------------|----------------------------------------------------|
| Read-only codebase research | `Agent` (subagent_type=Explore)      | `task` agent in read-only mode           | `apply_patch` read-only / shell `grep`   | Do the search in-thread with `rg`/`grep`           |
| Foreground delegation       | `Agent` (subagent_type=general-purpose) | nested `task` agent, await output     | spawn nested CLI session                 | Do the work in-thread (more context cost)          |
| Background delegation       | `Agent` with `run_in_background: true` | detached `task` agent                  | nohup'd CLI session writing to a logfile | Run the monitor in a separate terminal/tmux pane   |
| Ask the user a question     | `AskUserQuestion`                    | inline prompt                            | inline prompt                            | Ask in the response and wait for the next turn     |
| Subagent model tier         | Tier A: `opus` + `ultrathink`; Tier B: `sonnet` + medium thinking | Tier A: top `task` model + high reasoning; Tier B: smaller-tier `task` + medium reasoning | Tier A: top GPT + `reasoning_effort=high`; Tier B: `gpt-5.1-codex-spark` + `reasoning_effort=medium` | Honor the per-phase tier mapping in AGENTS.md; retry the same subagent UP on unavailability |
<!-- autospec-block:harness-adapter-core -->

**Persistent project notes**: write durable preferences to **`AGENTS.md`** in the repo root — recognized by Claude Code, OpenCode, and Codex. Per AGENTS.md, subagent dispatches use a **two-tier policy**: Tier A (top model + extended thinking) for research/verify/decompose work; Tier B (cheaper model + medium thinking) for mechanical steps. The orchestrator keeps the user's invoked model. Fall back UP the tier on quota/capacity or other unavailability by retrying the same subagent with the stronger tier while preserving parent context.

## Harness detection (run once at skill start, before Phase G0)

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

**Fallback rule:** If `TIER_B` is not available in your harness (model unknown, quota/capacity failure, authorization failure, or tool call returns an error for that model), silently retry the same subagent dispatch with `TIER_A`. Preserve the parent context on retry; for Codex native subagents, fork/inherit the current conversation context and use the latest top GPT model instead of moving the work into the main session. Never ask the user.

Hold `TIER_A` and `TIER_B` for the entire skill run. Every "Tier A" and "Tier B" reference below resolves to these harness-specific values.

**Model tier — what runs where:**

| Phase | Model tier | Why |
|---|---|---|
| G1 Lens research (each of the 6 lenses) | `TIER_A` | Judgment-heavy: reading real metrics/site/crawl signals and proposing candidates with defensible ROI/severity/confidence scores. |
| G2 Adversarial verify (per candidate) | `TIER_A` | Must actively try to refute a claim, not rubber-stamp it — cheap models default to agreeing. |
| G3 Decompose (delegated to `autospec-define` Phase 3) | `TIER_A` | Inherits `autospec-define`'s own spec-work tier. |
| Config read/validate, JSONL plumbing, `growth-measure.sh`/`grow-define-pipeline.sh`/`grow-define-file-issues.sh` invocations | `TIER_B` (or no subagent at all — deterministic scripts) | Mechanical: parsing, validating, and shelling out to deterministic scripts. |

## Configuration

`/autospec-grow-define` is **opt-in per repo**, mirroring `autospec-fab`'s
`.autospec/fab.yml` gate. Before any phase:

1. Check for `.autospec/growth.yml`. If absent, print:
   `"No .autospec/growth.yml found — this repo is not opted into autospec-growth. See skills/autospec-grow-define/README.md to configure a product/site and try again."`
   and **exit** (inert; do not create the file, do not proceed to any phase).
2. Render the YAML as JSON (e.g. `yq -o=json . .autospec/growth.yml`) and validate it:
   ```bash
   bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/validate-growth-config.sh" <config.json>
   ```
   If validation fails (non-zero exit), print the validator's stderr reason and **exit** — do not proceed with a partially-valid config.
3. Hold the validated `<config.json>` path for the rest of the run — Phase G0 uses it to locate metric sources, G1 uses `channels`/`targets`, G2 passes it to `grow-define-pipeline.sh`, G3 uses `approval.control_repo` and `channels` for labeling.

## Relevant memory injection (run-start, once)

Before executing the main pipeline phases, call the injector to surface relevant saved lessons:

```bash
bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/inject-relevant-memory.sh" \
  --context "autospec-grow-define growth lens research adversarial verify decompose" \
  --top-k 5
```

Prepend the output block (if non-empty) to your working context. This surfaces lessons like the growth-ethics blocklist immutability rule and prior lint/decomposition pitfalls before they recur.

## Phase G0 — Measure (optional)

Sync current metrics before research so lens candidates cite real deltas
instead of guesses. This phase is best-effort — a fresh repo with no metrics
history still proceeds.

1. Read `measurement` from `<config.json>` (GSC property, analytics
   provider/token_env, github_repo, rank_source). If the config points at
   local raw metric files (or fixture files under a configured path) that
   exist on disk, normalize each one:
   ```bash
   bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/growth-measure.sh" --normalize <provider> <raw.json>
   ```
2. If no config-pointed or fixture metric files exist, skip normalization and
   proceed with an **empty measurement envelope** (`{"provider":null,"metrics":{},"ts":<now>}`)
   — G1 lenses fall back to structural/on-site evidence (crawl, GSC query
   list if available, repo signals) rather than measured deltas.
3. **Live provider API calls (GSC, Plausible/GA4, SERP, backlink APIs) are
   out of scope for this skill — that wiring lands in Plan 4.** This phase
   only normalizes metric payloads already on disk or supplied as fixtures.

Hold the normalized envelope(s) (or the empty envelope) for G1 lens dispatches.

## Phase G1 — Lens research (delegate, one Tier-A subagent per lens)

> **Model tier:** `TIER_A` (research) — top model with extended thinking; resolved at startup.

Dispatch the full **Lens roster** below in parallel — one foreground Tier-A
subagent per lens, each bounded to a fresh context (config summary + G0
measurement envelope + that lens's evidence sources only; do not fork the full
parent conversation). Each lens subagent researches, then returns a JSON array
of candidate objects matching the Plan 2 candidate schema
(`skills/autospec-shared/schemas/growth-candidate.schema.json`):

```json
{
  "lens": "<one of the 6 lens tokens>",
  "channel": "technical_seo|content|outreach|directories",
  "kind": "artifact|outbound",
  "title": "<human-readable candidate title>",
  "norm_title": "<normalized dedup key>",
  "rationale": "<why this candidate, grounded in evidence>",
  "evidence": ["<citation 1>", "<citation 2>"],
  "roi": 1,
  "severity": 1,
  "effort": "small|medium|large",
  "confidence": 0.0
}
```

**Budget per lens.** Before dispatching, compute each lens's fan-out budget
(how many candidates to ask for / how much crawl-depth or query-breadth to
spend) by reading the ledger-derived weight for that lens:

```bash
bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/growth-source-weights.sh"
```

This prints a JSON object keyed by the 6 lens tokens with a smoothed weight in
`[0,1]`. Scale each lens's candidate budget (e.g. `round(base_budget *
weight[lens] * 2)`, floor 1) so lenses that have historically produced
merged/published/shipped candidates get more research budget this cycle, and
lenses that mostly get refuted get less — without ever starving a lens to
zero.

Append every candidate object returned by every lens (one JSON object per
line) to a single candidates file, e.g. `/tmp/grow-define-candidates.jsonl`.
This file is the G2 pipeline's first input.

## Lens roster

Dispatch exactly these 6 lenses, each a Tier-A subagent, each returning
candidate JSON per the schema above:

1. **`technical-seo`** — Technical-SEO auditor. Crawl the site (via
   `autospec-playwright` / Claude-in-Chrome if available, else static analysis
   of `site.repo_path`): Core Web Vitals signals, meta tags, canonical/hreflang,
   structured data (schema.org), sitemap/robots.txt, broken links, index
   coverage. Candidates are `kind: artifact`, `channel: technical_seo`.

2. **`keyword-gap`** — Keyword-gap researcher. Use the G0 GSC measurement
   envelope (queries ranking position 5–20 are quick wins) plus
   `targets.keyword_seeds` from config to find competitor keyword gaps.
   Candidates are typically `kind: artifact`, `channel: content` (new/updated
   pages targeting the gap).

3. **`content-opportunity`** — Content-opportunity researcher. Identify topic
   clusters, "vs"/comparison pages, and tutorial/docs gaps relative to
   `product.competitors` and `product.value_props`. Candidates are `kind:
   artifact`, `channel: content`.

4. **`community`** — Community researcher. From `targets.communities` in
   config, propose genuinely on-topic posts for subreddits, HN, forums,
   Discords, and awesome-lists — each candidate's `rationale` MUST cite the
   venue's recorded `self_promo_rule` and `cadence_cap_per_week`. Candidates
   are `kind: outbound`, `channel: outreach`.

5. **`directory`** — Directory/listing researcher. From
   `targets.directories`, propose legitimate directory/listing submissions
   (Product Hunt, AlternativeTo, awesome-list PRs) the product genuinely
   qualifies for. Candidates are `kind: outbound`, `channel: directories`
   (an awesome-list PR that lands directly in the target repo may instead be
   `kind: artifact` if it's a self-mergeable PR against a repo the product
   owns).

6. **`backlink`** — Backlink/partnership researcher. Identify guest-post
   targets and integration partners from `product.competitors`,
   `product.personas`, and any backlink data in the G0 envelope. Candidates
   are `kind: outbound`, `channel: outreach`.

Every lens candidate MUST set `norm_title` to a stable, whitespace/case
-normalized dedup key (the same on-topic idea proposed twice, even with
different phrasing, should normalize to the same key) — this is what
`grow-define-pipeline.sh`'s dedup step matches against the ledger.

## Phase G2 — Pipeline (delegate verify, then deterministic rank)

> **Model tier:** `TIER_A` (verify) — top model with extended thinking; resolved at startup.

1. **Adversarial verify (delegate, one Tier-A subagent per candidate, or
   batched per lens if your harness supports structured batch output).** For
   each candidate in the G1 candidates JSONL, dispatch a Tier-A subagent
   prompted to **actively try to refute** the candidate — check the cited
   evidence, re-derive the claim from source data where possible, and look
   for reasons it's stale, already shipped, ethically blocked, or
   unsupported. The subagent must default to `real:false` on any genuine
   doubt (fail-closed, matching the pattern used by `autospec-explore`'s
   verify stage). Collect one verdict object per candidate:
   ```json
   {"norm_title": "<candidate norm_title>", "real": true, "reason": "<1-line justification>"}
   ```
   Append every verdict (one JSON object per line) to a verdicts file, e.g.
   `/tmp/grow-define-verdicts.jsonl`.

2. **Deterministic pipeline.** Run the shared G2 script against the
   candidates, verdicts, and the validated config:
   ```bash
   bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/grow-define-pipeline.sh" \
     /tmp/grow-define-candidates.jsonl /tmp/grow-define-verdicts.jsonl <config.json>
   ```
   This deterministically validates each candidate against the schema, dedups
   against `.autospec/growth/ledger.jsonl` (both previously-shipped and
   previously-rejected items, so refused candidates don't reappear every
   cycle), drops any candidate with no matching verdict or a `real:false`
   verdict, ROI/severity-ranks the survivors, and slices to
   `grow.max_issues_per_cycle` (default 8). It prints the ranked top-N
   candidates (one JSON object per line) to stdout — capture this as the
   ranked output for Phase G3.

If the ranked output is empty, print `"Phase G2: no candidates survived
verification/ranking this cycle."` and skip Phase G3.

## Phase G3 — Decompose

Split the ranked output from Phase G2 by `kind`.

**Routing (explicit):**

- **`kind: outbound`, and any `kind: artifact` candidate that is a simple,
  single-issue artifact** (one file or one small self-contained change, no
  natural multi-issue split) → file directly with the shared script:
  ```bash
  bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/grow-define-file-issues.sh" \
    <ranked.jsonl> <config.json>
  ```
  This creates one GitHub issue per ranked candidate with the correct
  `growth:artifact`/`growth:outbound` + `growth:<channel>` labels, appends a
  `pending` ledger line per created issue, and prints the created issue
  numbers.

- **`kind: artifact` candidates that warrant decomposition into multiple
  linked issues** (e.g. a comparison-page campaign spanning several pages, or
  a technical-SEO fix touching more than ~3 files/logical units) → do **not**
  file directly. Instead delegate to `autospec-define`'s Phase 3 decomposition
  (same sizing caps, lint loop, and `## Model fit` classification it already
  runs) to create the umbrella + linked children, then add the
  `growth:artifact` label and the appropriate `growth:<channel>` label to
  every created issue (`gh issue edit <N> --add-label
  "growth:artifact,growth:<channel>" --repo {repo}`).

Both paths append the resulting issues to the growth ledger (directly via
`grow-define-file-issues.sh`, or via an explicit
`growth-ledger.sh --append` call after the `autospec-define` Phase 3 path
creates its issues) so future G2 dedup passes see them.

## Stop

Print a run summary and hand off — do not launch `/autospec-grow-run`
automatically:

```
Phase G3 complete on {repo}.
- measured: <provider(s) normalized, or "none (empty envelope)">
- lens candidates: <total across all 6 lenses>
- verified real: <count> / refuted: <count>
- filed: <N> issues (<A> growth:artifact, <B> growth:outbound)
- ledger: <path to .autospec/growth/ledger.jsonl>

Run /autospec-grow-run to drain the growth backlog now, or let your external
conductor (/autospec-grow) pick it up.
```
