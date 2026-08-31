
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
   curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh | bash -s -- --skill all --harness all --update
   ```
   Run this one-liner once; it refreshes all autospec skills across all harnesses.
3. **Show the diff** between the prior installed file(s) and the freshly fetched copy.

## Required capabilities & harness adapter

| Capability                  | Claude Code                          | OpenCode                                 | Codex CLI                                | Fallback if missing                                |
|-----------------------------|--------------------------------------|------------------------------------------|------------------------------------------|----------------------------------------------------|
| Subagent model tier          | Tier A: `opus` + ultrathink          | Tier A: top-tier `task` + max reasoning  | Tier A: runtime/repository frontier model + `reasoning_effort=high` | Fall back UP on unavailability |
<!-- autospec-block:harness-adapter-core -->

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

**Fallback rule:** If `TIER_B` is not available in your harness (model unknown, quota/capacity failure, authorization failure, or tool call returns an error for that model), silently retry the same subagent dispatch with `TIER_A`. Preserve the needed context with a bounded handoff; for Codex native subagents, use a fresh configurable independent agent and the runtime/repository frontier model instead of moving the work into the main/authoring session. When a role-specialized reviewer has a fixed unavailable model binding, the configurable fallback must receive the identical prompt, evidence, validation steps, and output schema. Record `requested_model`, `actual_model`, `fallback_reason`, and reviewed head SHA in an orchestrator-owned sidecar artifact so the reviewer verdict remains byte-exact. Never use a weaker model or self-review. If no equal-or-stronger independent reviewer can run, fail closed for merge/release decisions. Never ask the user.

Hold `TIER_A` and `TIER_B` for the entire skill run. Every "Tier A" and "Tier B" reference below resolves to these harness-specific values.

## Relevant memory injection (run-start, once)

Before executing the main pipeline phases, call the injector to surface relevant saved lessons:

```bash
bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/inject-relevant-memory.sh" \
  --context "<skill-relevant keywords from request/issue/spec>" \
  --top-k 5
```

Prepend the output block (if non-empty) to your working context. This surfaces lessons like
`feedback_bash_return_trap_leak.md` that prevent re-occurrence of known pitfalls.


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
| `--remediation` | Broad-dimension review (spec-coverage + correctness + test-quality + integration-wiring + docs) with false-positive filter, for the end-of-run gap-remediation loop |
| `--emit-gaps PATH` | Write the machine-readable gap JSON to PATH (implies `--remediation`) |
| `--reasoning-trial` | In remediation mode, run candidate findings through the deterministic conjecture/falsifier trial before emitting gaps |

## Remediation mode (`--remediation` / `--emit-gaps`)

The Phase 5.5 typed dispatcher passes this complete slash command as one
harness argument, so `--remediation`, `--since`, and `--emit-gaps` are not
mistaken for native harness options.

When `--remediation` (or `--emit-gaps PATH`) is passed, run a **broad-dimension** review instead of the spec-coverage-only audit, and emit a machine-readable gap list for the `/autospec-run` end-of-run gap-remediation loop.

**Model tier:** Tier A (spec work) — broad review is inline analysis and needs opus turn-stamina (per `feedback_monitor_silent_exit.md`).

Dimensions reviewed (all five, not just spec-coverage):

1. **spec-coverage** — acceptance criteria with no shipping issue/PR (the existing audit).
2. **correctness** — logic bugs, cross-platform shell portability (e.g. GNU-vs-BSD `grep`/`sed`), off-by-one, error-path gaps.
3. **test-quality** — assertions that never fail, missing negative-path coverage, untested branches.
4. **integration-wiring** — emitted-but-unconsumed artifacts, dangling references, lock-step drift.
5. **docs** — stale or contradictory documentation versus shipped behavior.

Pipeline:

1. Dispatch the broad review subagent(s) at Tier A. Collect candidate findings, each tagged with `dimension`, `severity`, `file`, `line`, `title`, `body`, and a stable `dedupe_key`.
2. **False-positive filter (required before emit):** pipe candidate findings through an evaluate-findings/critic pass. Mark each finding `verdict: keep` or `verdict: false_positive`. When uncertain, drop it (`false_positive`) — never ship a false positive into the auto-implement queue.
3. **Reasoning trial (optional, high-uncertainty gate):** when `--reasoning-trial` is passed, or when the review is autonomous and the finding is `reasoning:deep`/`priority:high`, require each kept finding to carry a falsifier:

   ```json
   {"falsifier":{"kind":"absent","needle":"expected-token","haystack":"repo-relative/path"}}
   ```

   Run the deterministic trial before emitting gaps:

   ```bash
   python3 scripts/autospec_review_audit.py reasoning-trial \
     --repo-root . \
     --candidates /tmp/autospec-review/remediation-findings.json \
     --out /tmp/autospec-review/remediation-findings-survived.json \
     --report ".autospec/reasoning-trials/${RUN_ID}/summary.json" \
     --events ".autospec/reasoning-trials/${RUN_ID}/events.jsonl"
   ```

   Use only the `--out` survivors as the findings input for `emit-gaps.sh`. Treat
   `refuted` findings as false positives. Treat `needs_evidence` as not fileable
   yet; record them in the review report so the next audit can gather better
   proof. This gives the Phase 5.5 loop a replayable conjecture/falsifier trail
   without adding another LLM call.

4. Write the surviving findings to a temp findings file, then shape them into the gap contract:

   ```bash
   bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/emit-gaps.sh" \
     --findings /tmp/autospec-review/remediation-findings.json \
     --out "${EMIT_GAPS_PATH:-$HOME/.autospec/gaps-${RUN_ID}.json}"
   ```

   The emitted gap JSON is an array of objects matching the contract:

   ```json
   [{"gap_id":"G1","dimension":"correctness","severity":"medium",
     "file":"skills/autospec-shared/scripts/cross-repo-search.sh","line":77,
     "title":"trailing pipe matches every line on BSD grep",
     "body":"<remediation issue body>",
     "dedupe_key":"cross-repo-search-trailing-pipe"}]
   ```

5. When `--emit-gaps PATH` is given, write to PATH; otherwise default to `~/.autospec/gaps-<run_id>.json`. The driver (`gap-remediation-loop.sh`) reads this file. On review-subagent failure, write an empty array (`[]`) so the driver converges cleanly, and log a warning — never block run completion.

6. Run the shared read-only repo quality audit and link its artifacts from the
   review report:

   ```bash
   bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/repo-quality-audit.sh" \
     --repo . \
     --json ".autospec/repo-quality-audit.json" \
     --markdown ".autospec/repo-quality-audit.md" \
     ${AUTOSPEC_QUALITY_AUDIT_FILE_ISSUES:+--file-issues}
   ```

   Treat audit findings as a separate repo-wide quality signal, not as
   spec-coverage rows. Findings are already classified as `app-follow-up`,
   `autospec-process-gap`, `inherited-accepted-debt`, or
   `current-branch-regression`; preserve those classifications when filing or
   reporting follow-ups.

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
python3 scripts/autospec_review_audit.py discover \
  --repo-root . \
  --since "${SINCE:-1900-01-01}" \
  ${SPEC_GLOB:+--glob "$SPEC_GLOB"} \
  --out /tmp/autospec-review/specs.json
```

Then for each spec, build the linkage:

```bash
python3 scripts/autospec_review_audit.py link \
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

1. `python3 scripts/autospec_review_audit.py validate-subagent
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
python3 scripts/autospec_review_audit.py write-csv \
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

If the configured reviewer cannot run for a qualifying model-availability reason, apply the run-start fallback rule: use a fresh configurable Tier-A reviewer with the same prompt and JSON contract. This is an availability retry only; never replace or erase substantive reviewer findings.

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
   `status=filed`. Use `python3 scripts/autospec_review_audit.py
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
