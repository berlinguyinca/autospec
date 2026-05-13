---
description: Audit design specs against open + closed issues to find gaps, file high-priority regression issues, and feed them back through autospec. Runs as `/autospec-review` (manual) or auto-fires after each autospec-run batch unless `~/.autospec/no-review.flag` exists.
mode: primary
---

<!-- BODY START -->
## Self-update mode

Decide this purely from the request text the harness handed you. Do NOT
shell out to test the user's free-form request. Read the request, normalize
it in your reasoning (collapse whitespace, trim, lowercase), and if the result is
exactly `update`, this skill enters self-update mode and does NOT run the
normal pipeline.

1. **Detect harness** by checking which install path exists for this skill:
   - Claude Code: `~/.claude/skills/autospec-review/SKILL.md`
   - OpenCode:    `~/.config/opencode/agent/autospec-review.md`
   - Codex CLI:   `~/.codex/prompts/autospec-review.md`
2. **Re-install from `main`** by piping the canonical installer:
   ```bash
   bash <(curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-review/install.sh) --harness <detected> --update
   ```
   If multiple harness paths exist, run the one-liner once per detected harness.
3. **Show the diff** between the prior installed file(s) and the freshly fetched copy.

## Required capabilities & harness adapter

| Capability                  | Claude Code                          | OpenCode                                 | Codex CLI                                | Fallback if missing                                |
|-----------------------------|--------------------------------------|------------------------------------------|------------------------------------------|----------------------------------------------------|
| Subagent model tier          | Tier A: `opus` + ultrathink          | Tier A: top-tier `task` + max reasoning  | Tier A: `gpt-5.1-codex` + `reasoning_effort=high` | Fall back UP on unavailability |

**Model tier:** `TIER_B` (implementation work) — cheaper model with medium thinking; resolved at startup. Silently fall back to `TIER_A` if unavailable.


## Harness detection (run once at skill start, before Phase 0)

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


# autospec-review

Audit design specs against open + closed issues, write gap rows to a
CSV ledger, and route regressions back through `/autospec-split` with
`priority:high` + `regression` labels.

## When to invoke

- Manually as `/autospec-review [...flags]`.
- Automatically by `autospec-run` after the last issue in a batch
  closes — gated by `~/.autospec/no-review.flag` and the
  `--no-postreview` flag.

## CLI flags

| Flag | Effect |
|------|--------|
| `--spec PATH` | Audit one spec only |
| `--profile NAME` | Model profile from `~/.autospec/model-profiles.yml` |
| `--dry-run` | CSV + regression specs only; skip `/autospec-split` |
| `--no-autoreview` | Skip the §7a Tier-A reviewer pass |
| `--since DATE` | Only audit specs whose date prefix ≥ DATE |
| `--spec-glob PATTERN` | Override default spec discovery globs |

## Phase 0 — Preflight

1. Read repo state. Resolve `repo` from `gh repo view --json
   nameWithOwner` and `short_sha` from `git rev-parse --short HEAD`.
2. Compute `run_id = <UTC compact>-<short_sha>` (delegate to
   `scripts/autospec_review_audit.py`).
3. Acquire `~/.autospec/review.lock` (PID + start time). If lock exists
   and PID is alive AND start_time < 1h ago → exit with error
   "another autospec-review run in progress (PID X)". If stale → reclaim.
4. Ensure `reports/autospec-review/` exists in target repo.
5. Compute `audit_date = today (yyyy-mm-dd)`.

## Phase 1 — Spec discovery + linkage matrix (deterministic)

Invoke the helper script directly (no LLM):

```bash
python scripts/autospec_review_audit.py discover \
  --repo-root . \
  --since "${SINCE:-1900-01-01}" \
  ${SPEC_GLOB:+--glob "$SPEC_GLOB"} \
  --out /tmp/autospec-review/specs.json
```

Then for each spec, build the linkage:

```bash
python scripts/autospec_review_audit.py link \
  --repo "$REPO" \
  --specs /tmp/autospec-review/specs.json \
  --out  /tmp/autospec-review/linkage.json
```

Output `linkage.json` is a list of:

```json
{
  "spec_path": "...", "spec_topic": "...", "spec_text": "...",
  "linked_issues": [ ...gh issue records... ]
}
```

## Phase 2 — Audit subagent fan-out (Tier A)

For each entry in `linkage.json`, dispatch one Tier-A subagent in
batches of `${AUTOSPEC_REVIEW_BATCH_SIZE:-5}` (parallel within a batch,
serial across batches).

**Model tier:** Tier A (spec work) — top model + ultrathink.

**Subagent prompt skeleton** (verbatim, with input substitutions):

```
You are an autospec audit subagent. Read references/gap-taxonomy.md
and references/subagent-contract.md (loaded inline below). Apply the
taxonomy to the supplied spec + linked issues. Output JSON matching
the contract. Do not ship false positives — when uncertain, omit.

== gap-taxonomy.md ==
<verbatim contents>

== subagent-contract.md ==
<verbatim contents>

== input ==
<yaml block from §4 of the design spec>
```

For each subagent's JSON return:

1. `python scripts/autospec_review_audit.py validate-subagent
   --input /tmp/.../subagent-NN.json
   --spec-path "..." --linked-numbers "1 2 3" --spec-text-file ...`
2. On schema failure, retry the subagent ONCE with the validation error
   prepended to the prompt. On second failure, write a fallback row
   `{gap_type: ac_no_issue, severity: blocker, title: "subagent
   schema failure", notes: <error>, ...}`.

## Phase 3 — CSV merge + per-run snapshot

Aggregate all subagent outputs into one rows list. For each gap:

- compute `gap_id = sha1(spec_path + spec_anchor + gap_type)[:10]`
- set `status = open`, `remediation_issue = ""`, `remediation_pr = ""`
- copy `run_id`, `audit_date`, `repo` from preflight

Then:

```bash
python scripts/autospec_review_audit.py write-csv \
  --rows /tmp/autospec-review/rows.json \
  --snapshot reports/autospec-review/${AUDIT_DATE}-${RUN_ID}.csv \
  --ledger   reports/autospec-review/gaps.csv
```

The script writes the per-run snapshot (overwrite-on-same-run_id) and
merges into the ledger preserving manual `wontfix` / `false_positive`
edits.

## Phase 4 — Render regression specs

Group rows by `spec_path`. For each group with ≥1 `status=open` row:

1. Render `templates/regression-spec.md.tmpl` with substitutions
   (`audit_date`, `spec_path`, `run_id`, `spec_topic`,
   `parent_spec_summary` (2-3 line auto-generated summary from
   spec_text headings), and the iterated `gaps`).
2. Write to `docs/specs/${AUDIT_DATE}-${SPEC_TOPIC}-regressions.md`
   in the TARGET repo (NOT the autospec repo).
3. Do NOT commit yet — Phase 5 reviews and may modify.

## Phase 5 — Reviewer subagent (§7a Tier A)

Skip if `--no-autoreview` was passed.

For each rendered regression spec, dispatch one Tier-A reviewer
subagent.

**Model tier:** Tier A (spec work).

**Prompt:** load `references/reviewer-prompt.md` verbatim and append
the input yaml block.

For each reviewer JSON return:

1. For each `gap_id` in `false_positive_gap_ids`:
   - Update CSV row: `status=false_positive`, prepend
     `Reviewer flagged: <reason>` to `notes`.
   - Strip the corresponding `### Gap <id>` section from the
     regression spec.
2. For each `gap_id` in `scope_concern_gap_ids`:
   - Prepend `Reviewer scope-concern: <reason>` to `notes`. Keep
     `status=open`.
3. For each `ac_tightening` entry: replace the AC bullet in the
   regression spec body.
4. Append `reviewer_notes_md` under a new heading
   `### Reviewer notes (autospec-review §7a, Tier A, <run_id>)`.

After all reviewers finish:

```bash
git checkout -b "autospec-review/${RUN_ID}"
git add docs/specs/${AUDIT_DATE}-*-regressions.md
git commit -m "docs(autospec-review): regression specs from run ${RUN_ID}"
```

## Phase 6 — /autospec-split handoff + post-process

Skip if `--dry-run` was passed.

For each regression spec file:

1. Invoke `/autospec-split docs/specs/<file>` and capture the issue
   numbers it returns (parse from gh output).
2. For each new issue number:
   - `gh issue edit <num> --add-label priority:high --add-label
     regression --add-label <topic-label>`
   - `gh issue edit <num> --title "[REGRESSION] $(gh issue view <num>
     --json title -q .title)"` (idempotent — strip duplicate prefix
     first via shell prefix-test)
   - `gh issue comment <num> --body "Generated by autospec-review run
     ${RUN_ID}. See gap_id <id> in
     reports/autospec-review/gaps.csv."`
3. Update CSV rows: write `remediation_issue=#<num>`, flip
   `status=filed`. Use `python scripts/autospec_review_audit.py
   update-status --gap-id <id> --status filed --issue <num>`.

## Finalization

1. Append run summary to `reports/autospec-review/runs.md` (newest
   first).
2. Print to console: `run_id`, gaps by type, gaps by severity,
   regression issues filed, paths to per-run CSV + ledger.
3. If env `AUTOSPEC_REVIEW_NOTIFY` set, POST the same summary as JSON
   to that webhook.
4. Release `~/.autospec/review.lock`.
<!-- BODY END -->
