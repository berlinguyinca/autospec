# autospec-run end-of-run reference

Read this file at end-of-run (after the last issue in the batch closes/merges,
queue drains to ALL_DONE) and follow each section in turn. It holds the detailed
procedures that the SKILL.md body points to: Phase 6 (final report + Done
challenge), Phase 5.5 (gap remediation), Phase 5.6 (repo quality audit), and
advisor escalation.

## Phase 6 — Final report

When the monitor terminates, challenge whether the run is really done before
posting the final summary. The **Done challenge** must explicitly re-check:

- Queue state: no ready `auto-implement`, `gap-remediation`, or
  `needs-classify` issues remain under the active profile.
- Phase 5.5 result: either 0 surviving gaps, or a named warning explaining why
  post-review could not run and what risk remains.
- Failures/deferred work: every failure is either fixed, restored to the queue,
  or listed as next work; every deferred issue is listed in the Deferred summary.
- Verification evidence: merged PRs passed the required local suite and required
  CI checks before admin-merge.

If the Done challenge finds unfinished work that can be handled safely, do not
claim completion; relaunch Phase 4/Phase 5.5 or file the missing follow-up and
drain it. Only emit the final report after the challenge says the run is really
done or after the remaining work is explicitly captured as next work.

Post a final summary to the user with:

- Work done: every issue processed, every PR merged, total elapsed wall time,
  and any failures that need human attention.
- Archived summary: stale trackers, superseded specs, closed follow-ups, or
  other archived items from this run; use `(none)` when nothing was archived.
- Done challenge: the challenge bullets and final verdict.
- Next work: what should be worked on next, or `- (none — converged)` when the
  challenge and Phase 5.5 found no remaining work.

The final report MUST include a canonical `## Next steps` section so downstream
tooling (`/autospec-refine --continue`) can deterministically harvest the next
prompt. Structure the section as a markdown list of candidate next-prompt
strings, each one a self-contained imperative phrasing of the remaining work,
blocker, or follow-up. If there is no remaining work, write
`- (none — converged)` so the harvester can detect convergence cleanly. If
evidence indicates the loop should not continue (overfitting, out-of-sample
plateau, operator policy), include a line starting with `STOP: <reason>` to
trigger an evidence-based stop in the continuous-iteration loop. Accepted
header variants the harvester recognises are `## Next steps`, `## What to do
next`, `## Remaining work`, and `## Open blockers` (case-insensitive); prefer
`## Next steps` as the canonical form. Alternatively, write the harvest-target
content inside a fenced ```autospec-next or ```next-prompt block — these are
read in fallback order by the harvester.

Also write `.autospec/run-summary.md` via the canonical helper so downstream
tools can harvest the same answer:

```bash
bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-write-run-summary.sh" \
  --repo "{repo}" \
  --prs "$PRS_MERGED_FILE_OR_CSV" \
  --issues "$ISSUES_CLOSED_FILE_OR_CSV" \
  --failures "$FAILURES_FILE_OR_CSV" \
  --archived "$ARCHIVED_FILE_OR_CSV" \
  --elapsed "$ELAPSED_HHMM" \
  --done-challenge-file "$DONE_CHALLENGE_FILE" \
  --next-steps-file "$NEXT_STEPS_FILE" \
  --quality-audit-json ".autospec/repo-quality-audit.json" \
  --output ".autospec/run-summary.md"
```

Append the **Deferred summary** when `deferred[]` is non-empty:

```
Deferred (off-profile under <profile_name>: ctx=<P.ctx>, reasoning=<P.reasoning>):
- #<N1>: <reason>
- #<N2>: <reason>
...
Re-run with --profile <larger> to pick these up, or run /autospec-run on a host that fits the larger profile.
```

If `deferred[]` is empty, omit the section.

## Phase 5.5 — End-of-run gap remediation

Runs after the last issue in the batch closes/merges (queue drains, `ALL_DONE`), before the final report. It broad-reviews the shipped work and stages surviving gaps as `needs-classify` issues only after the deterministic driver renders the complete issue template and `lint-issue.sh` accepts it. This run does not promote or drain them. The existing autonomous Tier 1.5 promotion path owns classification and Rust-backed safety admission in a later cycle.

**Skip the whole phase when:**

- `~/.autospec/no-review.flag` exists, OR
- `--no-postreview` was passed to autospec-run.

Otherwise run the bounded loop. At closeout, also run the deterministic gap miner against available run evidence (review verdicts, fix commits, CI blockers, and QA/scope misses) so repeated misses become deduped `gap-remediation` issues and repeat counts land in `docs/memory/autospec-gap-ledger.md`:

```bash
if [ -s "${AUTOSPEC_RUN_GAP_EVENTS:-}" ]; then
  bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-gap-miner.sh" \
    --input "${AUTOSPEC_RUN_GAP_EVENTS}" \
    --ledger docs/memory/autospec-gap-ledger.md \
    --repo "${AUTOSPEC_REPO:-}" \
    --file
fi
```

Then continue the bounded Phase 5.5 loop:

```bash
BATCH_START_DATE="$(bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/run-batch-start.sh" --read)"
RUN_ID="$(date -u +%Y%m%dT%H%MZ)-$(git rev-parse --short HEAD)"
GAPS_FILE="$HOME/.autospec/gaps-${RUN_ID}.json"
rm -f "$HOME/.autospec/gap-round-state.json"   # fresh window per run
```

Run one staging pass:

1. **Broad review** (Tier A): run the broad-review pass with a round-scoped `--since` window (spec Phase 2 child E):

   ```bash
   REVIEW_SINCE="${BATCH_START_DATE}"
   bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/invoke-review.sh" \
     --remediation --since "${REVIEW_SINCE}" --emit-gaps "${GAPS_FILE}"
   ```

   **Harness-neutral invocation:** `invoke-review.sh` detects the active harness (Claude Code / Codex CLI / OpenCode) via `autospec-harness-detect.sh` and dispatches `/autospec-review` using the correct per-harness argv form. If the review backend is unavailable it emits `code_health:phase55_broad_review_backend_unavailable` to stderr and appends a visible diagnostic gap to `GAPS_FILE` — it never silently produces an empty gap file that would look like a clean pass. The full `BATCH_START_DATE` window preserves the per-PR-LGTM-misses-integration value (the broad pass catches cross-PR integration gaps the per-PR LGTMs missed). On review failure, log a warning, treat as 0 survivors, and fall back to the final report (never block run completion).
1b. **Docs-completeness dimension** (spec §D6 row 2 — runs only on round 1, after the broad review emits `${GAPS_FILE}`): the gap-remediation review also audits documentation completeness for the work shipped during this batch window. Every feature shipped in the window (scoped by `run-batch-start.sh --read`) must have a page for every configured audience (the `documentation.audiences` from #917's doc config), and there must be no outstanding `visual_stale` / `example_stale` drift signals (`check-doc-drift.sh`). Run the deterministic helper and **append** its gaps onto the existing `${GAPS_FILE}` so they file, dedupe, and converge through the SAME gap-remediation machinery used in step 2 (do NOT build a parallel loop):

   ```bash
   DOCS_GAPS="$(bash "skills/autospec-run/scripts/docs-completeness-gaps.sh" 2>/tmp/docs-completeness.err)" || DOCS_GAPS='[]'
   # Merge the docs gaps into the broad-review gap array (re-numbered by the driver).
   if [ -s "${GAPS_FILE}" ]; then
     jq -s '.[0] + .[1]' "${GAPS_FILE}" <(printf '%s' "${DOCS_GAPS}") > "${GAPS_FILE}.merged" \
       && mv "${GAPS_FILE}.merged" "${GAPS_FILE}"
   else
     printf '%s' "${DOCS_GAPS}" > "${GAPS_FILE}"
   fi
   ```

   The emitted gaps carry `dimension: "docs-completeness"`; the gap-remediation loop labels them `gap-remediation` like every other survivor, so a later round does not re-flag freshly-fixed work. Failures of the docs check itself (missing config, drift-script error, missing `node`/`jq`) only log a WARN to `/tmp/docs-completeness.err` and emit an empty array — this dimension NEVER blocks run completion (same failure semantics as `/autospec-review` above).
1c. **Security dimension** (autospec-secaudit sweep — runs on round 1, after the docs dimension merges into `${GAPS_FILE}`): run a repo-wide security sweep (full-tree `--tree --root .`) for security, secret-leak, injection, and PII issues, and **append surviving must-fix findings** to the same `${GAPS_FILE}` so they file, dedupe, and converge through the SAME gap-remediation machinery used in step 2 (do NOT build a parallel loop). Skip the dimension when `~/.autospec/no-secaudit.flag` exists. This shares its engine with `/autospec-secaudit`.

   ```bash
   if [ ! -f "$HOME/.autospec/no-secaudit.flag" ]; then
     SECSCAN="${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/security-scan.sh"
     if [ -f "$SECSCAN" ]; then
       # Auto-file only must-fix findings, and exclude `license` (copyleft/IP
       # needs human confirmation — surfaced via a manual /autospec-secaudit
       # report, not auto-filed as an implementable issue).
       SEC_GAPS="$(bash "$SECSCAN" --tree --root . 2>/tmp/secaudit-sweep.err \
         | jq -sc '[ .[] | select(.severity=="must-fix" and .dimension != "license") ]' 2>/dev/null || printf '[]')"
       if [ -s "${GAPS_FILE}" ]; then
         jq -s '.[0] + .[1]' "${GAPS_FILE}" <(printf '%s' "${SEC_GAPS}") > "${GAPS_FILE}.secmerged" \
           && mv "${GAPS_FILE}.secmerged" "${GAPS_FILE}"
       else
         printf '%s' "${SEC_GAPS}" > "${GAPS_FILE}"
       fi
     fi
   fi
   ```

   The emitted gaps carry security dimensions (`secrets` / `vuln` / `injection` / `pii` / `cve`); the gap-remediation loop labels them `gap-remediation` like every other survivor, so a later round does not re-flag freshly-fixed work. A missing scan engine, missing scanners, or `jq` error only logs to `/tmp/secaudit-sweep.err` and emits nothing — this dimension NEVER blocks run completion (same failure semantics as the docs dimension above). For deeper coverage (LLM triage of PII / prompt-injection, plus copyleft/IP review and the `.autospec/secaudit.md` report), run `/autospec-secaudit` manually.
1d. **Fab-completeness dimension** (spec §Phase 5.5 — runs only on round 1, after the security dimension merges into `${GAPS_FILE}`; runs only for a fab run, i.e. when `.autospec/fab.yml` exists): assert every printable model shipped its proof artifacts. For each printable model, `fab-completeness.sh` asserts (a) its 16-view contact sheet exists (`.autospec/fab/renders/<model>/contact-sheet.html`), (b) its `release-gate.json` exists (`.autospec/fab/gates/<model>/release-gate.json`), is GREEN (no stage `status=fail`), and is FRESH (its `geometry_hash` equals the model's current STL sha256 — never re-run stages to learn status). Each failed assertion prints one `GAP <model>: <reason>` line. Convert surviving GAP lines into gap objects carrying `dimension: "fab-completeness"` and **append** them onto `${GAPS_FILE}` so they file, dedupe, and converge through the SAME gap-remediation machinery used in step 2 (do NOT build a parallel loop):

   ```bash
   if [ -f ".autospec/fab.yml" ]; then
     FAB_GAPS="$(bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/fab-completeness.sh" \
       --fab-dir .autospec/fab --stl-dir build/stls 2>/tmp/fab-completeness.err \
       | jq -Rsc 'split("\n") | map(select(length>0)) | to_entries | map({
           gap_id: ("F" + ((.key + 1) | tostring)),
           dimension: "fab-completeness", severity: "high",
           file: ".autospec/fab/release-gate.json", line: 1,
           title: ("fab-completeness: " + .value),
           body: ("Phase 5.5 fab-completeness found: " + .value + ". Every printable model must ship a 16-view contact sheet and a green + fresh release-gate.json. Re-run the fab gate (stl-release-gate.py) and regenerate the missing artifact."),
           dedupe_key: ("fab-completeness-" + (.value | gsub("[^a-zA-Z0-9]+"; "-")))
         })' 2>/dev/null || printf '[]')"
     if [ -s "${GAPS_FILE}" ]; then
       jq -s '.[0] + .[1]' "${GAPS_FILE}" <(printf '%s' "${FAB_GAPS}") > "${GAPS_FILE}.fabmerged" \
         && mv "${GAPS_FILE}.fabmerged" "${GAPS_FILE}"
     else
       printf '%s' "${FAB_GAPS}" > "${GAPS_FILE}"
     fi
   fi
   ```

   The emitted gaps carry `dimension: "fab-completeness"`; the gap-remediation loop labels them `gap-remediation` like every other survivor, so a later round does not re-flag freshly-fixed work. A non-fab run (no `.autospec/fab.yml`), a missing helper, or a `jq` error only logs to `/tmp/fab-completeness.err` and emits nothing — this dimension NEVER blocks run completion (same failure semantics as the docs dimension above).
2. **File survivors:**

   ```bash
   bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/gap-remediation-loop.sh" \
     --gaps "${GAPS_FILE}" --file
   ```

   The driver prints `gap-remediation: survivors=<N> filed=<N> round=<N>`. Capture `<N>` survivors.
3. **Converge:** if `survivors == 0`, break — the run is clean.
4. **Stage for admission:** if `survivors > 0`, stop this Phase 5.5 loop and report the newly staged `needs-classify,gap-remediation` issues in Phase 6. Do not invoke the Phase 4 monitor for them and do not add `auto-implement` inline. The driver has already enforced the issue-quality contract; the autonomous Tier 1.5 promoter owns model-fit classification and delegates final admission to Rust. A later autospec run may review the resulting merged remediation work.

**Termination guarantees:** convergence (0 survivors) ends the loop immediately; any staged survivor ends this run's remediation loop without entering the implementation queue; the `dedupe_key` prevents a later run from re-filing the same open gap; and the driver retains its hard round-state cap as defense in depth.

**Feed Phase 6:** report (a) findings the filter suppressed, (b) gaps staged for classification, and (c) any driver/review failure. Do not report staged gaps as closed or admitted. Failures from `/autospec-review` or the driver log a warning but do NOT fail the overall run.

## Phase 5.6 — Repo quality audit

Run the shared read-only repository quality audit after Phase 5.5 converges or
hits its cap, before Phase 6 writes the final run summary:

```bash
AUTOSPEC_QUALITY_AUDIT_FILE_ISSUES="${AUTOSPEC_QUALITY_AUDIT_FILE_ISSUES:-1}" \
bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/repo-quality-audit.sh" \
  --repo . \
  --json ".autospec/repo-quality-audit.json" \
  --markdown ".autospec/repo-quality-audit.md" \
  $([ "${AUTOSPEC_QUALITY_AUDIT_FILE_ISSUES:-1}" != "0" ] && printf '%s' "--file-issues")
```

The audit writes structured findings classified as `app-follow-up`,
`autospec-process-gap`, `inherited-accepted-debt`, or
`current-branch-regression`. Probes cover dirty git status, package-manager
scripts, runtime engine compatibility, typecheck/lint/test availability, route
coverage, design/template guards, dependency audit readiness,
security-sensitive storage, focused/skipped tests, large files, TypeScript
`any` usage, and debug logging hotspots. When
`AUTOSPEC_QUALITY_AUDIT_FILE_ISSUES` defaults to `1` for this Phase 5.6 call;
set it to `0` to opt out. When issue filing is enabled and `gh` is available,
the helper may file deduplicated `quality-audit` follow-up issues; otherwise it
records unfiled residual risks in the JSON/Markdown artifacts. Failures of the audit
helper should be reported as a warning in Phase 6, not hidden.

## Advisor escalation

A bounded hard decision may be escalated to a harness-native TIER_A **advisor** that returns advice only. This is the [advisor strategy](https://claude.com/blog/the-advisor-strategy): a cheap executor runs the loop and pulls in the strong model only at the exact hard moment. The advisor never calls tools and produces no user-facing output.

Configured by the `advisor:` block in `.autospec/autospec.yml` — a single `policy: auto | on | off` plus a budget, NOT per-gate levers. **You never enumerate gates**; `advisor-escalate.sh precheck` resolves everything and returns `DISABLED` (exit 8) when a gate is not active.

**Self-governance (`policy: auto`, the default).** Autospec decides which gates are active, like an architect adjusting a standing order from results. The active set is seeded at the low-risk `impl-haiku` gate and self-tuned by `advisor-govern.sh`: it promotes the next gate in a fixed safety order (`impl-haiku → retry → reviewer → impl-decision`) only when the run's quality ≥ baseline AND cost ≤ baseline over a minimum-sample floor, and retracts the last-added gate on regression (never below the seed). `policy: on` activates every gate within budget; `policy: off` is inert.

**Governance tick (run once during the end-of-run sweep, `policy: auto` only).** Call `${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/advisor-sweep-tick.sh --main-telemetry <the run's telemetry jsonl> --json`. It observes the batch's LGTM-first-pass rate + cost/issue from telemetry (`advisor-observe.sh`), freezes a pre-advisor **baseline snapshot** the first time the advisor is active, and thereafter promotes/retracts the active gate set against that baseline via `advisor-govern.sh` — so the set self-adjusts before the next run without any operator input. It is fully fail-safe: not-`auto`, no telemetry, or no reviewer signal → a logged no-op. Inspect the telemetry behind a decision with `advisor-report.sh`.

**Protocol (every gate):**

1. Write the decision-scoped question to a temp file and the minimal relevant context to a second temp file. Do NOT dump the whole issue/diff — a bloated payload inflates `tokens_in` and erases the cost win.
2. Run `${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/advisor-escalate.sh --phase precheck --issue <N> --repo <R> --gate <id> --question-file <q> --context-file <c> --json`. A non-zero exit (`7` cap-reached, `8` disabled) means skip escalation and proceed exactly as if no advisor exists.
3. On `GO`, dispatch the advisor via the highest rung your context supports:
   - (1) the native advisor tool if your harness exposes it; else
   - (2) a read-only TIER_A subagent — Claude Code `Agent(model: opus)`, OpenCode `task` top-tier — this is the preferred path; else
   - (3) the `cli_fallback` command from the precheck output (Codex `codex exec`; `claude -p` / `opencode run` are legacy). Use rung 3 only when your context lacks a subagent tool — e.g. a background-dispatched implementer does not inherit the `Agent` tool.
   Prompt the advisor with the curated payload and: "Return advice only as one JSON object `{verdict, guidance, confidence}`. You have no tools and produce no user-facing output. guidance <= 700 tokens."
4. Run `${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/advisor-escalate.sh --phase record --issue <N> --repo <R> --gate <id> --response-file <resp> --json`. Act on the validated verdict: `plan`/`correction` → apply and continue; `stop` → soft-fail (return-to-queue + comment). An unparseable response is recorded as a fail-safe `stop`.

**`reviewer` gate:** after the LGTM reviewer forms a verdict that is neither a clean LGTM nor a hard BLOCK (a borderline call), run the protocol with `--gate reviewer` to have the advisor uphold or refine the verdict before it is issued. This complements the existing reuse-BLOCK cheap-refute pass rather than replacing it.
