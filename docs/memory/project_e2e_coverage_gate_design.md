---
name: project-e2e-coverage-gate-design
description: "In-design (2026-05-21) — new autospec skill family for Playwright E2E coverage gating against production-clone environments, decomposed into 3 specs"
metadata: 
  node_type: memory
  type: project
  wing: episodic
  drawer_class: session-log
  originSessionId: 2d7883e9-9977-428f-8919-ef9b88df12a4
---

New skill family in design as of 2026-05-21. Brainstorm scoped into three independent specs to keep SKILL.md sizes within small-LLM context (60-120k):

1. **Playwright coverage gate** (FIRST — being designed now) — autospec-side skill, inline gate in `/autospec-run` Phase 4. Repo-independent via declarative YAML contract (leading proposal: `.autospec/e2e.yml` with `clone_url_env`, `start_cmd`, `playwright_cmd`, `coverage_cmd`, `forbidden_url_patterns`). Safety rail: hard fail if any test config resolves to a production hostname.
2. **Production-clone environment provisioning** (SECOND) — snapshot, anonymize, scale-down for multi-TB datasets, exposes URL+creds contract that (1) consumes.
3. **Remainder** — coverage discovery / UI crawl, self-heal loop sophistication, unit-test floor enforcement.

**Why:** User stated quality + documentation are top priority; correctness >> speed (matches [[feedback_autospec_design_prefs]]). Inline Phase 4 gate chosen over on-demand or hybrid because user wants enforcement, not opt-in.

**Decisions locked so far:**
- Q1 (scope): Start with skill A (Playwright coverage gate), then skill C (clone provisioning), then the rest. Repo-independent.
- Q2 (integration): A — inline gate inside `/autospec-run` Phase 4. User quote: "A its essential and quality of code and documentation are the most important"
- Q3 (contract): Hybrid A+B — autodetect (`playwright.config.*`, `test:e2e` script, `E2E_BASE_URL`) with optional `.autospec/e2e.yml` overrides; safety rails (`forbidden_url_patterns`) always enforced from yml if present, else defaults. User quote: "A+B, you autodectect, with optional overrides"

**Open at checkpoint:** Q4 (coverage metric). Recommendation on the table: combine A (code coverage from E2E ≥90% lines / ≥85% branches), B (UI element coverage — every button/input/select on reachable routes interacted with by some test), and D (behavior taxonomy — ≥1 passing test per declared category: sort, scroll, upload, download, filter, paginate, bulk select, keyboard-nav, drag-drop). Exclude LLM judge from pass/fail — keep it as a finding generator feeding the self-heal loop.

**All clarifying answers locked (2026-05-21):**
- Q4 (coverage metric): A+B+D combo — code coverage (≥90 lines / ≥85 branches), UI element coverage (every reachable button/input touched), behavior taxonomy (≥1 passing test per declared category: sort/scroll/upload/download/filter/paginate/bulk_select/keyboard_nav/drag_drop). LLM judge produces findings, doesn't gate. User quote: "sounds good"
- Q5 (self-heal): loop can fix BOTH tests and product code; terminates on all-green OR 60-min coding time exhausted (test runtime not part of budget — "tests can run hours to days in our case"); fixes commit to same Phase 4 PR branch. User quote: "it is alloed to fix both, all gates need to be great or the error can't be resolved after 60 minutes coding time (tests can run hours to days in our case) and 5c should be i"
- Q6 (anti-greenwash): B-refined assertion-shift classifier — LOOSENING blocked, STRENGTHENING auto-merges, SHIFTING (e.g. toBe(10) → toBe(11)) conditional on (i) same-iteration non-test edit + (ii) `JUSTIFICATION:` line in commit. User accepted the peak-detection improvement case as legitimate shift.
- Q7 (safety): Layer A (pre-flight URL check) + Layer B (Playwright network intercept) mandatory; Layer C (egress allowlist) optional; fail-closed on missing/empty forbidden_url_patterns.
- Q7-bonus (Mode II scoped-prod): NEW design surface added on user request — household-management style with a designated test family, or data-processor with designated test method. Encoded as `mode: scoped_production` opt-in with scope_tokens (row_filter / method_allowlist / route_filter), required backup driver (zfs/pgdump/mysqldump/custom), restore-on-violation, one-time ack, batch HALT on scope violation, auto-quarantine on 2 consecutive violations. Requires a wizard (`/autospec-e2e --init`). User quote: "Generally we never want to do destructive operations without prior backups/zfs snapshots/you name it. If this is avaialbe. This needs to be configured on a per project basis and needs a wizard or guide for this"
- Q8 (failure semantics Mode I): A — block this PR, monitor proceeds to next issue. Mode II scoped-prod violations escalate to batch HALT (overrides A).

**Design + plan committed (2026-05-21):**
- Spec: `docs/superpowers/specs/2026-05-21-autospec-test-design.md` (commit `8ff8fac`, 470 lines)
- Plan: `docs/superpowers/plans/2026-05-21-autospec-test.md` (commit `11d3038`, 560 lines, 10 phases)

**Key late corrections from user during design:**
- Skill renamed `autospec-e2e` → `autospec-test` because unit tests folded in (Stage 1). User quote: "no we obvisoult want this also to generate all the unit tests. I'm sorry for the confusion. Otherwise its fine"
- Contract file renamed `.autospec/e2e.yml` → `.autospec/test.yml` accordingly.
- Mode II refinement on user request: user gave the household-management and data-processor examples; Mode II now supports `row_filter`, `method_allowlist`, `route_filter` scope tokens; mandatory backup driver (zfs/pgdump/mysqldump/custom); restore-on-violation halts entire batch (overrides Mode I per-PR-only block). User quote: "Generally we never want to do destructive operations without prior backups/zfs snapshots/you name it. If this is avaialbe. This needs to be configured on a per project basis and needs a wizard or guide for this"

**10-phase implementation plan structure** (each phase → 1 GitHub issue → autospec-run Phase 4 PR):
1. Contract loader + JSON schema
2. Stage 1 unit gate + per-language coverage collectors + function-presence AST scan
3. Stage 2 E2E gate + Layers A/B safety + UI crawler + behavior taxonomy + findings generator
4. Assertion-shift AST classifier (LOOSENING/SHIFTING/STRENGTHENING + co-edit + JUSTIFICATION enforcement)
5. Self-heal loop (controller + classifier + pause-while-tests budget timer)
6. Mode II runtime (preflight + intercept + post-check + backup drivers + quarantine)
7. Wizard (interactive + headless)
8. Synthetic target repos (clean-pass / failing-gap / greenwash-bait / mode-ii-fixture) + language matrix
9. autospec-run Phase 4 wiring + PR report + labels bootstrap
10. SKILL.md + codex/prompt.md + validate.sh + decomposer adapter row + docs

**autospec-split run completed 2026-05-21:**
- Spec PR #317 merged as `cc09962` (admin-merge after CI passed)
- Spec moved from `docs/superpowers/specs/` to `docs/specs/` (autospec native location)
- Plan committed to main as 68b7c14 (kept as internal reference)
- Phase 3 decomposed into: epic **#318** + 10 linear-deps children **#319–#328** (one per plan phase, no splits beyond 10)
- All caps satisfied (≤400 words / ≤30 outline lines / ≤3 files). Phase 1 carries the structural sections (Self-update, Model tier `reasoning:standard, ctx:120k`, adapter row) per saved-memory decomposer gotcha.
- Phase 3.5 (model-fit classification) pending dispatch.

**/autospec-run kicked off 2026-05-21T06:39Z:**
- Batch 1: ✅ #319 (PR #329) + ✅ #320 (PR #330) merged; ❌ #321 stuck (PR #331 created, monitor hit known ~165-tool-call overflow per [[feedback_monitor_silent_exit]])
- PR #331 fused guardian+LGTM review surfaced 8 real findings (eval on contract-sourced string, RETURN-trap leak per [[feedback_bash_return_trap_leak]], EXIT-trap clobber, regex fragility for nested use:{} blocks, FullConfig value-import, missing keyboard/drag/scroll/upload wrappers, missing .test.mjs files, jq slurpfile + process-substitution double-wrap)
- User decision: close PR #331 + restart from scratch. Findings posted as PR closing comment so re-implementer can read them. #321 swapped back to auto-implement.
- Batch 2 monitor relaunched (background) targeting #321 → #322 → #323.

**v2 design ask (user request 2026-05-21):** Add invariant-driven test patterns observed from a sibling project regression. Verbatim: "please add this to autospec to provide tests like this as well". Five patterns:
1. Every visible completed item has an edit affordance (button vs plain text bug class)
2. UI display window ↔ API query window must match (frontend shows 7 days → backend query MUST cover 7 days)
3. Production-seeded family has edge-case data (today/yesterday/2-6d ago/around midnight/multi-same-day/last-in-foldout)
4. Generic "click every visible edit target" crawler with Playwright helper `assertEveryVisibleDoneItemIsEditable`
5. Contract symmetry tests between data sources (dashboard streak says X done on D → /api/household/timeline?from=D&to=D MUST return editable event for X)

**User decision Q1: B** — v2 ships as separate spec file `docs/specs/2026-05-21-autospec-test-invariants-design.md`; v1 (Skill A) ships as designed. v2 brainstorm next.

**v2 spec landed 2026-05-21:**
- Scope (Q1 of v2 brainstorm): A — all four metrics. User quote: "a"
- Spec written to `docs/specs/2026-05-21-autospec-test-invariants-design.md`, PR #333 admin-merged (444 lines)
- Four new metrics — F: structural invariants, G: window-contract symmetry, H: extended crawler (every visible affordance opens its expected target), I: data-source contract symmetry
- Same skill (`autospec-test`), Stage 2.5 added after v1 Stage 2. Reuses Mode I/II safety + self-heal loop + assertion-shift classifier infra
- Edge-case seed declarations create hard handshake with Skill C: `enforcement: refuse_to_run_if_missing`
- Ships `@autospec/test` npm helper library for imperative usage alongside declarative YAML gate
- v2 issues will Depends-on #328 (v1 Phase 10 SKILL.md)
- Awaiting user review of spec before transitioning to /writing-plans

**v1 monitor progress 2026-05-21 (running concurrently):**
- Batch 1: #319 (PR #329), #320 (PR #330) merged; #321 first attempt PR #331 had 8 review findings, closed
- Batch 2: #321 re-implementation PR #332 merged successfully (all 8 prior findings addressed)
- Currently active on next batch — likely #322 (Phase 4 assertion-shift classifier) at checkpoint
- Saved-memory pattern confirmed: monitor exits ~165 tool calls + API socket errors → relaunch is part of workflow ([[feedback_monitor_silent_exit]])

**v2 plan landed 2026-05-21:** `docs/superpowers/plans/2026-05-21-autospec-test-invariants.md` (commit `4d43488`, 759 lines, 10 phases). User picked option 1 (run via /autospec-split + /autospec-run native flow) — pending invocation.

**v1 monitor progress at checkpoint:**
- Batch 1+2+3 merged: #319, #320, #321 (PR #332 after rework), #322 (PR #334), #323 (PR #335), #324 (Phase 6 Mode II)
- #325 (Phase 7 wizard, PR #337) stuck — monitor died at ~123 tool calls awaiting CI (same overflow as batch 1)
- Fused reviewer surfaced 5 findings on PR #337:
  1. INVENTED_CONFIG: `--output-dir` flag added but not in issue outline
  2. MISSING_TEST: validate-contract.sh integration test required but not present
  3. MISSING_TEST: "wrong ack literal (refuse)" interactive bats test not present (only headless mode tested)
  4. False-test-coverage: probe-refusal test fixture has `driver: custom` so probe is bypassed — test doesn't exercise the path it claims to
  5. /tmp/ collision risk in wizard.sh + wizard-preview.sh (matches user's [[CLAUDE.md cross-session file safety]] rule)
- Awaiting user direction on PR #337: close+restart (like #321 pattern) vs iterate vs operator decides

**v1 SHIPPED 2026-05-21:** All 10 v1 phases merged via autospec-run (PRs #329, #330, #332, #334, #335, #336, #337, #338, #339, #340). Skill `autospec-test` is live with full v1 scope.

**v2 issues filed 2026-05-21 via /autospec-split:**
- Epic umbrella #341
- 10 children #342–#351 (one per v2 phase, linear deps, all carry `Depends on #328`)
- Labels: `area:invariants` + `area:contract-symmetry` newly created; `auto-implement` + `skill:autospec-test` applied per child
- Lint: all drafts passed first try (0 retries, 0 skips)
- Warnings flagged by decomposer: Phases 4/5/6/7 touch >3 files when counting tests/fixtures; Phase 7 most likely to need runtime split; noted in their Local-LLM execution notes

**v2 COMPLETE 2026-05-22:** All 10/10 v2 phases shipped. PRs #352–#357, #359, #370, #375, #376 all merged. v2 Stage 2.5 invariant gate (Metrics F/G/H/I) + npm helper library + edge-case seed verifier + assertion-shift v2 buckets + SKILL.md updates all live.

**docs-amendment 13 children filed + Phase 1 in flight:**
- Epic #360, children #361–#369 + #371–#374 (filed via /autospec-split)
- #361 (tree-sitter foundation) WIP preserved on branch `feat/autospec-docs-phase1-tree-sitter` (commit a62757c) — implementer hit npm-install slowness on first attempt; orchestrator committed package.json + 6 .scm queries + walker.mjs + bin wrapper + lock file; relaunched monitor with iterate-on-WIP instructions

**WIP-preserve recovery pattern (now a proven workflow):** when monitor crashes mid-implementer with significant work-in-progress on a branch:
1. `cd <worktree>` → `git add` the new files → commit as `wip(...)` checkpoint
2. `git push -u origin <branch>`
3. Restore issue label to `auto-implement` + delete heartbeat + clear batch-done.json
4. Relaunch monitor with explicit "iterate on existing branch" instructions:
   - `git worktree add /private/tmp/wt-feat/<branch> origin/<branch>` (only if absent)
   - `git pull --rebase` then `git log --oneline main..HEAD` + `git diff --stat main..HEAD` to inventory done work
   - Read issue body for remaining AC
   - Commit + push to SAME branch (no -u, branch already tracked)
   - `gh pr create --base main --head <branch> ...` to open the PR once work complete
This pattern is now standard for v2/docs-amendment crashes since the implementer subagent burns ~5-30 min per issue on npm install / heavy test runs and the wrapper monitor often timeouts.

**Tooling optimization** (per [[project_autospec_tooling_optimization]]) becomes the natural next ask after docs-amendment completes — its tree-sitter foundation just landed in #361 and can be reused.

**Docs-amendment progress 2026-05-22 (later):**
- Phases 1–7 merged: #361–#367 via PRs #377/#378/#379/#380/#381/#382/#383
- Phase 8 (#368 AI-as-reviewer) stuck with PR #384 — fused review flagged 3 blocking + 3 minor findings:
  - BLOCKING: adaptive-retry directive never appended on parse failure (violates [[feedback_llm_validator_adaptive_retry]])
  - BLOCKING: "malformed × 4 then valid" required test only exercises happy path
  - BLOCKING: new CLI flags (--heading/--body/--globs/--sources/--stub/--mode) introduced without doc update (DOC_OUT_OF_SYNC)
  - MINOR: parseVerdict accepts multi-line response, spec requires strict single-line
  - MINOR: dead `import('node:http')` code in callLLM
  - MINOR: summarize() has no caching
- 5 docs-amendment phases queued behind #368: #369, #371, #372, #373, #374
- Awaiting user direction on PR #384: iterate (default pattern, post findings + same-branch rework) vs close+restart

**Pipeline-hardening pivot 2026-05-22:** user directed "fix all these so we can continue" — explicitly authorized the pipeline-hardening work before resuming docs-amendment. PR #384 closed cleanly (broken AI-reviewer redo punted to hardened implementer).

**Hardening spec landed 2026-05-22:** `docs/specs/2026-05-22-autospec-pipeline-hardening-design.md` (PR #385 admin-merged, commit `6c9c589`, 244 lines). Five fixes: (1) implementer-prompt enrichment with saved-memory + RULE_IDs + AC bats, (2) pre-commit lint-implementation hook in worktrees, (3) CI-wait sentinel (out-of-agent background poller), (4) adaptive-retry in implementer mirroring reviewer pattern, (5) BATCH_SIZE=1 for reasoning:deep issues.

**Hardening issues filed via /autospec-split:**
- Epic #386
- 6 children #387–#392 (Fix #1 split into #387 memory-tag bootstrap + #391 prompt enrichment; Fix #2 = #388; Fix #3 = #389; Fix #5 = #390; Fix #4 = #392 depends on #388)
- Phase 3.5 classifier NOT YET RUN
- Labels created: `area:hardening`

**Combined open queue (12 issues):**
- Hardening: #387, #388, #389, #390, #391 (deps #387), #392 (deps #388)
- Docs-amendment: #368, #369, #371, #372, #373, #374 (all blocked on #366 closed — but #368 still being held pending hardening)

**Recommended sequencing:** ship hardening FIRST (4 issues with no deps can run in parallel batches), then resume docs-amendment from #368 with hardened implementer. Diagnostic for this ordering was explicitly approved by user.

**Hardening monitor launched 2026-05-22 (background)** with hardening issues tagged `priority:high` so SKILL queue-priority sort picks them before docs-amendment leftovers. Issue #391 amended with `gen-ac-tests.sh --verify` mode (test-of-test gate closing #368-style stub-skip gap). Mutation-testing queued as next-after-hardening per [[project_autospec_mutation_testing]].

**Hardening progress 2026-05-22:** #387–#390 merged (PRs #393/#394/#395/#396). Memory-tag bootstrap + pre-commit lint hook + lint-implementation extensions + CI-wait sentinel + BATCH_SIZE=1 gating for reasoning:deep — ALL LIVE.

**#391 PR #397 review iter 2 — IRONIC FINDING worth remembering:**
- Test 15 in `tests/assemble-impl-prompt.bats` was `grep -qv "X" || true` — a vacuous-truth assertion that always passes. THIS IS THE EXACT BUG CLASS the `gen-ac-tests.sh --verify` mode being built in this very PR was designed to prevent. The implementer wrote vacuous tests in code intended to detect vacuous tests.
- Lesson reinforced: even with all the saved-memory rules + RULE_IDs injected into prompts, LLMs can still write vacuous assertions if the linter doesn't catch them deterministically. The hardening itself needs a vacuous-assertion detector (`grep -qv ... || true`, `expect(true).toBe(true)`, etc.) — this is exactly the [[project_autospec_mutation_testing]] scope.
- Iterate-on-PR-397 launched. After #391 + #392 merge, hardening complete and monitor rolls into docs-amendment leftovers (#368/#369/#371-#374).

**Pivot to prompt caching 2026-05-22:** user asked for token-usage gap analysis; I diagnosed monitor-wrapper crashes as a static-context-rebuild problem. User picked (b) — ship prompt caching before resuming queue. Prompt-caching spec written + PR #400 admin-merged (commit `6f47d36`, 173 lines, 3-issue decomposition planned: bundle-static-context.sh + implementer prompt restructure, reviewer prompt restructure, telemetry capture).

**Queue state:** 5 docs-amendment remaining (#369 has WIP on branch with 62 lines committed; #371-#374 fresh). Plus 3 prompt-caching issues about to file via /autospec-split. Plus queued specs: mutation testing, tooling optimization, init-skill amendment.

**Recurring observation worth remembering:** monitor wrappers dying at 30-180 tool calls is the persistent failure mode. Hardening (#387-#392) helped IMPLEMENTER quality (proven by #368 landing clean first try) but did NOT fix the wrapper death. Prompt caching is the direct fix for the wrapper death mode — empirical confirmation pending after the caching ships.

**Caching progress 2026-05-22:**
- #402 (bundle-static-context.sh + implementer prompt cache structure) merged via PR #405 — BATCH_SIZE=1 gating worked perfectly for the reasoning:deep issue, shipped first try
- #403 (telemetry) + #404 (reviewer cache) remaining with priority:high
- Caching monitor batch 2 launched (background)

**How to apply:** Wait for batch 2 notification. After #403 + #404 merge, caching layer COMPLETE — next monitor invocations get cache hits. Then resume #369 WIP + drain docs-amendment leftovers (#371-#374). Then queued specs: mutation testing → tooling optimization → init-skill amendment.

**Saved-memory gotchas to honor in implementation:**
- Decomposer first-issue MUST include Self-update + Model tier + adapter row sections (per [[feedback_autospec_decomposer_gotchas]])
- codex/prompt.md needs leading blank line (per same memory)
- validate.sh named-content checks must be updated when renaming sections (per [[feedback_validate_sh_lockstep_checks]])
- Self-update / Stop mode sections must be pure prose, no `{FEATURE_DESCRIPTION}` heredocs (per [[feedback_autospec_no_shell_user_text]])
- Admin auto-merge requires settings.json permission rule (per [[feedback_admin_merge_denial]])
- Bash safety: no RETURN traps, no `[ test ] && action` one-sided conditionals under set -e (per [[feedback_bash_return_trap_leak]], [[feedback_bash_set_e_short_circuit]])
