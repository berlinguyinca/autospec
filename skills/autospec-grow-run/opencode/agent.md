---
name: autospec-grow-run
description: Drain the growth backlog — implement growth:artifact issues via /autospec-run with a content-quality gate, run the growth:outbound draft→ethics→approval-queue pipeline (package-only, never auto-post), and measure/attribute via live adapters that re-weight the next define cycle. Use after /autospec-grow-define has filed issues.
mode: primary
---

# autospec-grow-run workflow (harness-neutral)

Run the implementation half of the autospec-growth pipeline for a
product/site opted into `.autospec/growth.yml`: **drain
`growth:artifact` issues → draft/gate/queue `growth:outbound` issues →
handle human approvals → measure and attribute.** This skill picks up
where `/autospec-grow-define` stops.

Manage your own context — never exceed 60%. Delegate to subagents whenever
your harness supports it; do not draft outbound copy or run the ethics gate
directly in the main conversation when a subagent can do it.

<!-- autospec-block:startup-self-update SKILL_NAME=autospec-grow-run -->

## Self-update mode

If the invocation argument matches the regex `^\s*update\s*$` (case-insensitive,
whitespace-padded), this skill enters self-update mode and does not run the
normal pipeline:

1. **Detect harness** by checking which install path exists for this skill:
   - Claude Code: `~/.claude/skills/autospec-grow-run/SKILL.md`
   - OpenCode:    `~/.config/opencode/agent/autospec-grow-run.md`
   - Codex CLI:   `~/.codex/prompts/autospec-grow-run.md`
2. **Re-install the full autospec suite from `main`** by piping the canonical
   installer:
   ```bash
   curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh | bash -s -- --skill all --harness all --update
   ```
   Run this one-liner once; it refreshes all autospec skills across all
   harnesses. When you author this skill's own `SKILL.md` by hand, mirror the
   change into the trio by editing `skills/autospec-grow-run/SKILL.md` and
   then re-deriving `codex/prompt.md` and `opencode/agent.md` with the repo's
   trio-derivation helper (`derive-trio.sh`, run `--in-place`), then
   regenerating the skill goldens with the repo's golden-generation helper
   (`gen-skill-goldens.sh`) for `autospec-grow-run` — never hand-edit the
   derived copies.
3. **Show the diff** between the prior installed file(s) and the freshly
   fetched copy (e.g. `diff <(cat <prior>) <(curl -fsSL ...SKILL.md)` or the
   equivalent recorded by the installer).
4. **Stop.** Do not enter Phase R0 / any pipeline phase. Print the upgrade
   summary and return to the user.

If no install path is detected, print `Self-update: no installed copy of
autospec-grow-run found; run install.sh first.` and exit.

## Run modes

The invocation argument (after the `update` check above) selects which phases
run. Match the argument case-insensitively, ignoring surrounding whitespace:

- **`outbound`** (argument matches `^\s*outbound\s*$`) — **outbound-only
  mode**: run R0 (detect) then R2 (draft → gate → queue) and R3 (handle
  approvals). **Skip R1 and R4.** The artifact drain (R1) is the autonomous
  conductor's Tier 1 job, and measurement (R4) is its own cadence-gated tier;
  re-running either here would duplicate that work.
- **`measure`** (argument matches `^\s*measure\s*$`) — **measure-only
  mode**: run R0 (detect) then R4 (measure & attribute). **Skip R1, R2, and
  R3.** This is what the conductor's cadence-gated measure tier invokes.
- **anything else / no argument** — **full pipeline**: R0 → R1 → R2 → R3 → R4,
  exactly as described below.

Resolve the mode once in R0 and hold it for the run. Each phase below names
the modes it runs in; when a phase is skipped for the active mode, do not
emit its ledger lines or notifications — proceed to the next phase it applies
to. The conductor's outbound and measure tiers rely on these narrow modes so
each tier does exactly its own slice of work.

## Required capabilities & harness adapter

This workflow assumes seven capabilities. Map each one to your harness's
actual tool. If a capability is missing, use the listed fallback.

| Capability                  | Claude Code                          | OpenCode                                 | Codex CLI                                | Fallback if missing                                |
|-----------------------------|---------------------------------------|-------------------------------------------|---------------------------------------------|-----------------------------------------------------|
| Subagent dispatch (draft/review/attribute) | `Agent` (subagent_type=general-purpose) | nested `task` agent, await output | spawn nested CLI session | Do the work in-thread (more context cost) |
| GitHub issue/label/comment ops | `gh` CLI via Bash | `gh` CLI via shell tool | `gh` CLI via `apply_patch`/shell | Abort the affected step and log a reason; never fake a `gh` result |
| JSON/JSONL manipulation | `jq` via Bash | `jq` via shell tool | `jq` via shell | Do the parsing inline in the subagent prompt |
| HTTP calls for measurement adapters | `curl` via Bash | `curl` via shell tool | `curl` via shell | Degrade that adapter to `{}` and continue |
| Human-visible run notifications | `PushNotification` | inline message / OS notify equivalent | inline message | Print the notice to the transcript and continue |
| Browser (site/UI verification, best-effort) | `autospec-playwright` / Claude-in-Chrome if available | equivalent browser tool if available | not typically available | Skip browser verification; note it as unverified |
| Subagent model tier         | Tier A: `opus` + `ultrathink`; Tier B: `sonnet` + medium thinking | Tier A: top `task` model + high reasoning; Tier B: smaller-tier `task` + medium reasoning | Tier A: top GPT + `reasoning_effort=high`; Tier B: `gpt-5.1-codex-spark` + `reasoning_effort=medium` | Honor the per-phase tier mapping in AGENTS.md; retry the same subagent UP on unavailability |
<!-- autospec-block:harness-adapter-core -->

**Persistent project notes**: write durable preferences to **`AGENTS.md`** in
the repo root — recognized by Claude Code, OpenCode, and Codex. Per
AGENTS.md, subagent dispatches use a **two-tier policy**: Tier A (top model +
extended thinking) for drafting, gate-review, and attribution work; Tier B
(cheaper model + medium thinking) for mechanical steps. The orchestrator
keeps the user's invoked model. Fall back UP the tier on quota/capacity or
other unavailability by retrying the same subagent with the stronger tier
while preserving parent context.

## Harness detection + Model tier

Detect your harness by checking available tools before Phase R0:

1. **Claude Code** — the `Agent` tool with a `subagent_type` parameter is
   available.
   - `TIER_A` = `opus` + `ultrathink`  (model ID: claude-opus-4-7)
   - `TIER_B` = `sonnet`               (model ID: claude-sonnet-4-6)

2. **OpenCode** — a `task` tool with model/tier configuration is available
   (no `subagent_type`).
   - `TIER_A` = top-tier task model + high reasoning
   - `TIER_B` = smaller-tier task model + medium reasoning

3. **Codex CLI** — neither `Agent` nor a configurable `task` tool is
   available; `apply_patch` is the primary edit tool.
   - `TIER_A` = current top GPT model + `reasoning_effort=high`
   - `TIER_B` = `gpt-5.1-codex-spark` + `reasoning_effort=medium`

**Fallback rule:** If `TIER_B` is not available in your harness (model
unknown, quota/capacity failure, authorization failure, or tool call returns
an error for that model), silently retry the same subagent dispatch with
`TIER_A`. Preserve the parent context on retry; for Codex native subagents,
fork/inherit the current conversation context and use the latest top GPT
model instead of moving the work into the main session. Never ask the user.

Hold `TIER_A` and `TIER_B` for the entire skill run. Resolve both once, at
skill start, before Phase R0.

> **Model tier:** `TIER_A` (draft, gate-review, attribute); `TIER_B` (cheap mechanical).

| Phase | Model tier | Why |
|---|---|---|
| R2 outbound draft | `TIER_A` | Drafting on-topic, non-spammy outreach copy that respects each venue's self-promo rule requires judgment. |
| R2/R3 ethics gate reviewer | `TIER_A` | Must actively look for disclosure gaps, spam framing, and policy violations — cheap models default to agreeing. |
| R4 attribution synthesis / learnings memo | `TIER_A` | Turning raw ledger + metric deltas into a defensible per-lens attribution narrative is judgment-heavy. |
| Config read/validate, JSONL plumbing, `growth-*.sh`/`grow-define-*.sh` invocations, ledger writes | `TIER_B` (or no subagent at all — deterministic scripts) | Mechanical: parsing, validating, and shelling out to deterministic scripts. |

## Relevant memory injection

Before executing the main pipeline phases, call the injector to surface
relevant saved lessons:

```bash
bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/inject-relevant-memory.sh" \
  --context "autospec-grow-run drain content-quality outbound ethics approval attribution" \
  --top-k 5
```

Prepend the output block (if non-empty) to your working context. This
surfaces lessons like the growth-ethics blocklist immutability rule, the
"never auto-post" invariant, and prior adapter fail-closed pitfalls before
they recur.

## R0 — Detect

*(runs in every mode)*

0. Resolve the **run mode** from the invocation argument (see **Run modes**
   above): `outbound`, `measure`, or full pipeline. Hold it for the run — it
   decides which of R1–R4 execute.
1. Detect your harness (see above) and resolve `TIER_A`/`TIER_B`.
2. Check for `.autospec/growth.yml`. If absent, print:
   `"No .autospec/growth.yml found — this repo is not opted into autospec-growth. See skills/autospec-grow-define/README.md to configure a product/site and try again."`
   and **exit** (inert; do not create the file, do not proceed to any phase).
3. Render the YAML as JSON (e.g. `yq -o=json . .autospec/growth.yml`) and
   validate it:
   ```bash
   bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/validate-growth-config.sh" <config.json>
   ```
   If validation fails (non-zero exit), print the validator's stderr reason
   and **exit** — do not proceed with a partially-valid config.
4. Resolve the growth-shared script directory once for the rest of the run:
   `AUTOSPEC_SCRIPTS_DIR="${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}"`
   (this is where every `growth-*.sh`, `grow-define-*.sh`, and
   `validate-*.sh` script referenced below lives). Hold the validated
   `<config.json>` path for the rest of the run.

## R1 — Artifact drain

*(full pipeline only — skipped in `outbound` and `measure` modes)*

1. Enumerate open `growth:artifact` issues in the target repo:
   ```bash
   gh issue list --repo <owner/name> --label growth:artifact --state open --json number,title,labels
   ```
2. For each issue, in turn:
   - Invoke `/autospec-run` against that single issue (invoked as the
     existing skill, not forked/reimplemented — this skill does not
     duplicate `/autospec-run`'s own decompose/implement/merge machinery).
     Because the issue carries `growth:artifact`, `/autospec-run` itself
     runs the content-quality gate (`GATE=growth`: deterministic
     `growth-content-quality-precheck.sh` pre-checks plus a `TIER_A`
     reviewer, with adaptive retry on failure) before allowing that
     issue's PR to auto-merge, alongside the standard code reviewer, the
     `growth-ethics` gate, and `autospec-secaudit`. R1 does not re-run any
     of these — the gate is single-sourced in `/autospec-run`.
   - Append a ledger line: `merged_clean` on success, `failed` on a
     per-issue failure. A per-issue failure does **not** stop the drain —
     move on to the next `growth:artifact` issue.

## R2 — Outbound draft → gate → queue

*(full pipeline and `outbound` mode — skipped in `measure` mode)*

For each open `growth:outbound` (or `growth/needs-draft`) issue, in turn:

1. **Draft** (`TIER_A` subagent) — produce outbound copy matching
   `skills/autospec-shared/schemas/growth-outbound-draft.schema.json`.
2. **Structural validation**:
   ```bash
   bash "$AUTOSPEC_SCRIPTS_DIR/validate-outbound-draft.sh" <draft.json>
   ```
   A malformed draft is **skipped and logged** — do not attempt to repair it
   in-place; log the reason and move to the next issue.
3. **Ethics gate** — deterministic disclosure pre-check, then the LLM gate:
   ```bash
   bash "$AUTOSPEC_SCRIPTS_DIR/growth-ethics-precheck.sh" --disclosure <draft.md>
   ```
   followed by a `TIER_A` `growth-ethics` reviewer subagent, both wrapped in
   a **5-attempt adaptive-retry loop** (findings fed back as re-draft
   directives). If the gate is still blocked after 5 attempts, append a
   `rejected` ledger line, **retire the source issue** (remove its
   `growth/needs-draft` label, add `growth/rejected`), and move on — do not
   queue a blocked draft. Retiring it stops the conductor's outbound tier from
   re-drafting the same hopeless issue every cycle.
4. **Cadence check**:
   ```bash
   bash "$AUTOSPEC_SCRIPTS_DIR/growth-ethics-precheck.sh" --cadence <config.json> <ledger.jsonl> <platform>
   ```
   If the venue's `cadence_cap_per_week` is already met, **refuse** to queue
   this draft this cycle (do not reject it permanently — it may queue next
   cycle).
5. **Relevance check** — the draft's rationale/body must carry the target
   venue's recorded `self_promo_rule` from `.autospec/growth.yml`
   (`targets.communities[*].self_promo_rule`); a draft that ignores the
   venue's self-promo rule is treated as an ethics-gate failure, not queued.
6. **Build the approval-queue body**:
   ```bash
   bash "$AUTOSPEC_SCRIPTS_DIR/growth-outbound-queue.sh" --build-body <draft.json>
   ```
7. **File the control issue** — create a `growth/needs-approval` issue in
   `approval.control_repo` (from config) carrying the built body.
8. **Retire the source draft issue** — once the control issue is filed,
   remove the `growth/needs-draft` label from the source `growth:outbound`
   issue (and add `growth/queued`). This is load-bearing: the conductor's
   outbound tier fires whenever an open `growth/needs-draft` issue exists, so
   a source issue left labelled would be re-drafted every cycle — filing a
   duplicate control issue and PushNotification each time, and starving the
   define/measure tiers. Do this only on a successful queue (a cadence
   *refusal* in step 4 intentionally leaves the label so the draft retries
   when the venue's window reopens).
9. Append a `pending` ledger line for the queued draft.
10. Send a `PushNotification` so a human knows an approval is waiting.

**Outbound is package-only.** This skill only ever prepares and files a
draft for human approval — it never posts, submits, or publishes outbound
content to any external platform on its own.

## R3 — Handle approvals

*(full pipeline and `outbound` mode — skipped in `measure` mode)*

For each open `growth/needs-approval` control issue in `approval.control_repo`:

1. Read the issue's labels:
   ```bash
   bash "$AUTOSPEC_SCRIPTS_DIR/growth-outbound-queue.sh" --read-state <label-csv>
   ```
2. Route by state, then **clear the serviced state** so the conductor's
   outbound tier does not re-service the same control issue every cycle (the
   tier fires whenever a `growth/needs-approval` issue still carries an
   `approved`/`edited`/`rejected`/`published` state label):
   - **`approved`** — emit a one-click-publish package (the exact copy +
     target URL/venue + any submission metadata) as a **comment** on the
     control issue. The agent **never posts on the human's behalf** — the
     comment is the deliverable. Then remove `growth/approved` and add
     `growth/awaiting-publish` so the package is not re-emitted each cycle.
     Do **not** record a `published` ledger line yet — publication is not
     confirmed. The issue now sits in `awaiting-publish` (no decision label),
     so the tier stops selecting it until the human acts.
   - **`published`** — the human has confirmed the post is live (pasted the
     URL and applied `growth/published`). Append a `published` ledger line
     carrying the live URL, then **close** the control issue. This is the
     terminal state; closing it drops it from the tier's pending count. Never
     infer publication — only this human-applied label triggers the record.
   - **`edited`** — treat the edited copy as a new draft: re-run the R2
     gate sequence (structural validation → ethics gate → cadence →
     relevance) and re-queue (which files a fresh control issue). Then close
     this superseded control issue (removing `growth/edited`).
   - **`rejected`** — append a `rejected` ledger line and close out; do not
     retry automatically.

Clearing the serviced state (relabel or close) on every branch is
load-bearing: it is the R3-side mirror of R2's source-issue retirement, and it
is what makes the shared `growth-outbound-pending` signal fall back to zero so
Tier G3 (measure) and Tier G1 (define) are not starved. Because the tier also
counts `growth/published`, a human-confirmed post self-triggers the terminal
record promptly instead of waiting for unrelated outbound work.

Reiterate the invariant: **the agent never auto-posts** anything to an
external platform under any control-issue state, including `approved` —
publication is always a human action taken outside this skill.

## R4 — Measure & attribute

*(full pipeline and `measure` mode — skipped in `outbound` mode)*

1. Run each adapter configured in `.autospec/growth.yml` (`measurement`
   block), e.g.:
   ```bash
   bash "$AUTOSPEC_SCRIPTS_DIR/growth-adapter-gsc.sh" <config.json>
   bash "$AUTOSPEC_SCRIPTS_DIR/growth-adapter-analytics.sh" <config.json>
   bash "$AUTOSPEC_SCRIPTS_DIR/growth-adapter-github.sh" <config.json>
   bash "$AUTOSPEC_SCRIPTS_DIR/growth-adapter-rank.sh" <config.json>
   ```
   **Missing credentials or a fetch error degrade that specific provider to
   `{}`** (logged with the reason) — this never aborts the run; the
   remaining adapters and the rest of R4 still proceed.

   Each adapter emits its own `{provider, metrics, ts}` envelope. **Merge all
   the adapters' envelopes for one snapshot into a single combined envelope**
   before attribution — `growth-attribute.sh` reads one envelope's `.metrics`,
   so passing a single adapter's output would silently drop the other
   providers:
   ```bash
   # combine this snapshot's per-adapter outputs (adapter-*.json) into one
   jq -s 'reduce .[] as $x ({provider:"combined", metrics:{}, ts:(.[0].ts)};
          .metrics += $x.metrics)' adapter-*.json > snapshot.json
   ```
2. Run attribution over the merged before/after snapshot envelopes (each built
   by the merge above at the start and end of the window) and the ledger:
   ```bash
   bash "$AUTOSPEC_SCRIPTS_DIR/growth-attribute.sh" <before.json> <after.json> <ledger.jsonl>
   ```
   This produces a per-lens contribution split across lenses that shipped
   (`merged_clean`/`published`) in the window.
3. Feed the attribution result into the source-weight recompute so the next
   `/autospec-grow-define` cycle biases budget toward lenses that actually
   ship:
   ```bash
   bash "$AUTOSPEC_SCRIPTS_DIR/growth-source-weights.sh"
   ```
4. Have a `TIER_A` subagent synthesize a short learnings memo from the
   attribution output (which lenses/channels are earning their budget,
   which are mostly refuted or unpublished) and store/print it alongside the
   run summary.
5. **Record the measure cycle in the ledger.** Append exactly one
   `kind:"measure"` line so the conductor's cadence gate
   (`growth-measure-due.sh`) can tell when this repo was last measured —
   without it, the measure tier fires every cycle forever. Use the canonical
   append-only writer, which enforces the full 10-key line schema:
   ```bash
   bash "$AUTOSPEC_SCRIPTS_DIR/growth-ledger.sh" --append "$(jq -n \
     --arg ts "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
     --arg reason "$MEASURE_SUMMARY" \
     '{round:0, source:"measure", title:"measure cycle",
       norm_title:("measure-" + $ts), channel:"measurement",
       kind:"measure", issue:0, outcome:"measured",
       reason:$reason, ts:$ts}')"
   ```
   `issue:0` keeps the line out of the per-issue `latest` collapse (it is
   recorded individually, like a refutation). Append this line whenever R4
   runs to completion — in both full-pipeline and `measure` mode — even when
   every adapter degraded to `{}`, because a completed (if empty) measure
   pass still resets the cadence clock. Set `$MEASURE_SUMMARY` to a one-line
   digest of the attribution result (or the degradation reason if all
   adapters returned `{}`).

## Stop

Print a run summary and hand off — one pass per invocation, do not loop:

```
Phase R4 complete on {repo}.
- artifact drain: <N> issues (<A> merged_clean, <B> failed)
- outbound: <N> drafted, <A> queued pending, <B> rejected, <C> refused (cadence)
- approvals: <N> published, <A> re-queued (edited), <B> rejected
- attribution: <summary of per-lens contribution / adapter degradations>
- ledger: <path to .autospec/growth/ledger.jsonl>

Growth runs unattended under /autospec-autonomous, which drives this skill's
outbound and measure modes on its own cadence; re-run /autospec-grow-define
to plan the next cycle, or invoke this skill directly for a one-off pass.
```
