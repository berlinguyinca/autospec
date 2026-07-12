# Skill Guide

Every AutoSpec capability is a skill under [`skills/`](skills). Each skill has:

- a human-facing README, linked below as `README`;
- a canonical execution prompt, linked below as `SKILL.md`;
- harness variants for Claude Code, OpenCode, and Codex CLI;
- install and uninstall scripts when the skill is user-installable.

The lock-step rule keeps `SKILL.md`, `opencode/agent.md`, and
`codex/prompt.md` behavior identical except for frontmatter. Run
[`scripts/validate.sh`](scripts/validate.sh) after editing any skill docs or
skill bodies.

## How To Choose A Skill

Use the narrowest skill that matches the job.

| Situation | Start Here |
| --- | --- |
| A rough idea needs a spec and issues | [`autospec-define`](#autospec-define) |
| A tracked spec already exists | [`autospec-split`](#autospec-split) |
| Issues are ready and should be implemented | [`autospec-run`](#autospec-run) |
| You want the full plan-and-ship loop | [`autospec`](#autospec) |
| You want unattended ongoing progress | [`autospec-autonomous`](#autospec-autonomous) |
| You want discovery on a sandbox branch | [`autospec-explore`](#autospec-explore) |
| You need release confidence | [`autospec-release`](#autospec-release) |
| You need running-app QA | [`autospec-qa`](#autospec-qa) |
| You need security review | [`autospec-secaudit`](#autospec-secaudit) |
| A monitor must stop, resume, or recover | [`autospec-stop`](#autospec-stop), [`autospec-resume`](#autospec-resume), [`autospec-guide`](#autospec-guide) |

## Autonomous Surfaces

These skills are the current direction of the project: long-running,
guardrail-driven autonomy that can keep making progress while preserving
operator control.

### `autospec-autonomous`

- Docs: [`README`](skills/autospec-autonomous/README.md), [`SKILL.md`](skills/autospec-autonomous/SKILL.md)
- Use when: you want AutoSpec to run unattended for a long period.
- How it works: a conductor walks a priority waterfall. It checks Tier 0 control
  commands first, then drains backlog work, promotes open issues, performs local
  discovery, improves architecture and coverage, and eventually uses broader
  discovery signals. It parks when the queue is dry, a stop/pause signal exists,
  or the usage/spend governor trips.
- How to use: run `/autospec-autonomous` with explicit budgets such as
  `--max-cycles`, `--budget-tokens`, `--budget-issues`, or `--dry-run`; steer it
  through the GitHub control channel and stop it with `/autospec-autonomous stop`
  or [`autospec-stop`](#autospec-stop).

### `autospec-run`

- Docs: [`README`](skills/autospec-run/README.md), [`SKILL.md`](skills/autospec-run/SKILL.md)
- Use when: `auto-implement` issues already exist and should be shipped.
- How it works: the monitor claims ready issues, creates isolated worktrees,
  implements with a test-first loop, opens PRs, runs validation, invokes review,
  records closeout evidence, and merges when configured gates pass.
- How to use: run `/autospec-run`; use `--profile <name>` for model-fit routing,
  `--coordination-status` to inspect distributed queue state, and `--claim` /
  `--release` for recovery or worker coordination.

### `autospec-explore`

- Docs: [`README`](skills/autospec-explore/README.md), [`SKILL.md`](skills/autospec-explore/SKILL.md)
- Use when: you want autonomous research and implementation on an isolated
  sandbox branch, not directly against `main`.
- How it works: multiple researchers propose features and defects from specs,
  code gaps, prior reports, open issues, source analysis, dependencies,
  competitor research, run-state, style drift, and domain lenses. Proposals are
  filtered, ranked, filed, and drained through `/autospec-run` against the
  sandbox branch.
- How to use: run `/autospec-explore` for discovery loops; inspect learning
  feedback with [`autospec-explore-ledger`](#autospec-explore-ledger).

### `autospec-explore-ledger`

- Docs: [`README`](skills/autospec-explore-ledger/README.md), [`SKILL.md`](skills/autospec-explore-ledger/SKILL.md)
- Use when: you want to understand which exploration sources are producing
  useful merged work.
- How it works: records explore proposal outcomes in an append-only JSONL ledger,
  computes source weights, and generates learnings from merged, reverted,
  failed, stalled, or abandoned proposals.
- How to use: run `/autospec-explore-ledger stats`, `show`, `weights`,
  `rebuild`, or `learnings`.

### `autospec-continue`

- Docs: [`README`](skills/autospec-continue/README.md), [`SKILL.md`](skills/autospec-continue/SKILL.md)
- Use when: the assistant just recommended an actionable next step and you want
  to execute it through AutoSpec without rewriting the prompt.
- How it works: extracts the latest recommendation, optionally sends it through
  `/autospec-refine`, then hands off to `/autospec --autonomous`.
- How to use: run `/continue`; use `--ask-confirm` when you want to review the
  refined prompt before execution.

### `autospec-loop`

- Docs: [`README`](skills/autospec-loop/README.md), [`SKILL.md`](skills/autospec-loop/SKILL.md)
- Use when: one task should repeat until a specific goal is reached.
- How it works: freezes the request into a goal contract, then repeats the task
  under conservative loop guardrails until success, blocker, or stop condition.
- How to use: invoke with natural language such as `/autospec-loop loop until
  the build passes`.

### `autospec-fleet`

- Docs: [`README`](skills/autospec-fleet/README.md), [`SKILL.md`](skills/autospec-fleet/SKILL.md)
- Use when: multiple repositories need coordinated `autospec-run` supervision.
- How it works: initializes fleet config, clones or syncs repos, and runs or
  dry-runs per-repo AutoSpec workers.
- How to use: start with fleet init/status/dry-run commands before allowing
  workers to mutate repositories.

### `autospec-guide`

- Docs: [`README`](skills/autospec-guide/README.md), [`SKILL.md`](skills/autospec-guide/SKILL.md)
- Use when: you need operator guidance for local AutoSpec autonomy.
- How it works: routes status, pause, resume, stuck-run guidance, supervisor
  loop, and next-command questions to existing local scripts and GitHub comments.
- How to use: ask for status or guidance; it should not merge, approve,
  schedule, or start background automation on its own.

## Planning And Intake

### `autospec`

- Docs: [`README`](skills/autospec/README.md), [`SKILL.md`](skills/autospec/SKILL.md)
- Use when: one request should be planned, decomposed, implemented, reviewed,
  and merged end to end.
- How it works: runs the full pipeline from repo bootstrap through spec,
  issue decomposition, model-fit labeling, autonomous implementation, status
  updates, and final report.
- How to use: run `/autospec <feature request>` when the feature is large enough
  to justify a spec and multiple PRs.

### `autospec-define`

- Docs: [`README`](skills/autospec-define/README.md), [`SKILL.md`](skills/autospec-define/SKILL.md)
- Use when: you want the planning half only.
- How it works: bootstraps if needed, investigates the repo, writes a design
  spec, creates linked GitHub issues, and labels them for model fit.
- How to use: run `/autospec-define <feature request>`; review the generated
  spec and queue, then run `/autospec-run`.

### `autospec-split`

- Docs: [`README`](skills/autospec-split/README.md), [`SKILL.md`](skills/autospec-split/SKILL.md)
- Use when: a tracked `docs/specs/*.md` already exists.
- How it works: skips fresh discovery/design, decomposes the selected spec into
  an epic plus implementation issues, then classifies them.
- How to use: run `/autospec-split` with an explicit spec path, `split latest
  spec`, or `split existing spec`.

### `autospec-classify`

- Docs: [`README`](skills/autospec-classify/README.md), [`SKILL.md`](skills/autospec-classify/SKILL.md)
- Use when: existing issues need `ctx:*` and `reasoning:*` labels.
- How it works: applies the Phase 3.5 model-fit rubric and inserts or replaces a
  `## Model fit` block in issue bodies.
- How to use: run `/autospec-classify`; scope with `--issues`, `--label`, or
  `--dry-run`.

### `autospec-listen`

- Docs: [`README`](skills/autospec-listen/README.md), [`SKILL.md`](skills/autospec-listen/SKILL.md)
- Use when: a conversation contains an imperative request to file an issue,
  write a spec, design, implement, build, ship, review, or run AutoSpec.
- How it works: classifies the latest message, drafts an issue when appropriate,
  hands specs to `/autospec-define`, and routes build/change verbs to the mapped
  skill with an opt-out.
- How to use: rely on natural language triggers, or invoke explicitly when you
  want conversation context turned into tracked work.

### `autospec-refine`

- Docs: [`README`](skills/autospec-refine/README.md), [`SKILL.md`](skills/autospec-refine/SKILL.md)
- Use when: a request needs sharpening before it becomes a spec or autonomous
  run.
- How it works: runs repo-grounded refinement lenses over the prompt and can hand
  off to `/autospec`.
- How to use: run `/autospec-refine <request>` with `--rounds`, `--lenses`,
  `--interactive`, `--dry-run`, or `--autonomous`.

## Validation, QA, Security, And Release

### `autospec-test`

- Docs: [`README`](skills/autospec-test/README.md), [`SKILL.md`](skills/autospec-test/SKILL.md)
- Use when: Phase 4 PRs need unit and E2E coverage gates.
- How it works: checks coverage, UI element coverage, behavior taxonomy,
  assertion-shift safety, and can self-heal within a bounded loop.
- How to use: run `/autospec-test [PR#]` or let `autospec-run` invoke it as a
  gate.

### `autospec-playwright`

- Docs: [`README`](skills/autospec-playwright/README.md), [`SKILL.md`](skills/autospec-playwright/SKILL.md)
- Use when: a project needs disciplined no-mock Playwright UI-test authoring.
- How it works: reads `.autospec/test.yml` authoring blocks, runs the
  autospec-test Stage 2A authoring flow, and prints coverage output.
- How to use: run `/autospec-playwright` after configuring the test authoring
  blocks.

### `autospec-e2e-clone`

- Docs: [`README`](skills/autospec-e2e-clone/README.md), [`SKILL.md`](skills/autospec-e2e-clone/SKILL.md)
- Use when: E2E tests need a safe production-like environment.
- How it works: provisions an isolated, scaled-down, anonymized clone and writes
  a routable URL for autospec-test Mode II.
- How to use: run before E2E validation that requires realistic data or services.

### `autospec-qa`

- Docs: [`README`](skills/autospec-qa/README.md), [`SKILL.md`](skills/autospec-qa/SKILL.md)
- Use when: you need to prove a running app matches the spec.
- How it works: audits UI controls, forms, validation, dropdowns, APIs,
  accessibility, console/network health, and missing or weak tests.
- How to use: run `/autospec-qa` against a running target, especially after
  `/autospec-run` or before release.

### `autospec-secaudit`

- Docs: [`README`](skills/autospec-secaudit/README.md), [`SKILL.md`](skills/autospec-secaudit/SKILL.md)
- Use when: generated or changed code needs security and IP review.
- How it works: combines deterministic scanners with LLM triage for secrets,
  vulnerabilities, prompt injection, license/IP issues, PII leaks, and CVEs.
- How to use: run `/autospec-secaudit` manually or let the Phase 4 gate invoke
  it; findings are written to `.autospec/secaudit.md`.

### `autospec-review`

- Docs: [`README`](skills/autospec-review/README.md), [`SKILL.md`](skills/autospec-review/SKILL.md)
- Use when: specs and issues may be out of sync.
- How it works: compares design specs with open and closed issues, identifies
  gaps, and can file high-priority regression issues.
- How to use: run manually with `/autospec-review`; it may also auto-fire after
  `autospec-run` batches unless disabled.

### `autospec-release`

- Docs: [`README`](skills/autospec-release/README.md), [`SKILL.md`](skills/autospec-release/SKILL.md)
- Use when: you need a release-readiness verdict.
- How it works: sweeps specs, docs, implementation, tests, QA proof, legacy
  cleanup, and merge readiness, then reports `PASS`, `PARTIAL`, or `FAIL`.
- How to use: run `/autospec-release` before shipping or before declaring an
  autonomous run release-ready.

### `autospec-upgrade`

- Docs: [`README`](skills/autospec-upgrade/README.md), [`SKILL.md`](skills/autospec-upgrade/SKILL.md)
- Use when: Angular, Next.js, or React projects need safe framework upgrades.
- How it works: locks observable behavior, upgrades one major at a time with
  official codemods, and gates completion on mutation score rather than coverage
  alone.
- How to use: run `/autospec-upgrade --repo .`; specify `--framework` when
  auto-detection is not enough.

### `autospec-fab`

- Docs: [`README`](skills/autospec-fab/README.md), [`SKILL.md`](skills/autospec-fab/SKILL.md)
- Use when: CAD-as-code or parametric 3D work must produce printable STL output.
- How it works: gates geometry, watertightness, pressure/vacuum, airflow,
  slicer, FEA, CFD, render, and vision checks for repos opted in via
  `.autospec/fab.yml`.
- How to use: configure `.autospec/fab.yml`, then run the fab release/validation
  flow before treating generated models as printable.

## Documentation, Design, And Product Story

### `autospec-doc`

- Docs: [`README`](skills/autospec-doc/README.md), [`SKILL.md`](skills/autospec-doc/SKILL.md)
- Use when: project documentation should be generated, regenerated, or audited
  per audience.
- How it works: routes through `doc-orchestrator.mjs`, supports user,
  developer, admin, and general audiences, verifies examples, and emits
  `llms-full.txt`.
- How to use: run `/autospec-doc init` first, then `/autospec-doc`,
  `/autospec-doc --full`, `/autospec-doc --audit`, or
  `/autospec-doc --audience <name>`.

### `autospec-design`

- Docs: [`README`](skills/autospec-design/README.md), [`SKILL.md`](skills/autospec-design/SKILL.md)
- Use when: a repo should adopt a vendor design language.
- How it works: fetches design-language docs from the external catalog, scores
  candidates against the repo, writes `DESIGN.md`, and can produce a migration
  spec.
- How to use: run `/autospec-design suggest`, `apply <vendor>`, or
  `migrate <vendor>`.

### `autospec-harmonize`

- Docs: [`README`](skills/autospec-harmonize/README.md), [`SKILL.md`](skills/autospec-harmonize/SKILL.md)
- Use when: UI design tokens have drifted.
- How it works: discovers palette, type, and spacing drift, generates variants,
  previews them, and emits a dated migration spec.
- How to use: run `/autospec-harmonize` when the goal is a design-token
  migration, then feed the resulting spec to `/autospec-define`.

### `autospec-story`

- Docs: [`README`](skills/autospec-story/README.md), [`SKILL.md`](skills/autospec-story/SKILL.md)
- Use when: you need a cited narrative of what the repo is and what has shipped.
- How it works: reconciles local specs, docs, GitHub issues, PRs, and recent git
  history into a product and implementation-state report.
- How to use: run `/autospec-story`; optionally pass `--output` to write a
  Markdown report.

### `autospec-sweep`

- Docs: [`README`](skills/autospec-sweep/README.md), [`SKILL.md`](skills/autospec-sweep/SKILL.md)
- Use when: a project needs first-run AutoSpec configuration or recurring
  spec-vs-reality sweeps.
- How it works: creates or reads `.autospec/autospec.yml`, checks docs, tests,
  specs, and code health, and emits bounded gaps that can feed the queue.
- How to use: run `/autospec-sweep init`, `configure`, or `run`.

## Operator Calibration And Recovery

### `autospec-persona`

- Docs: [`README`](skills/autospec-persona/README.md), [`SKILL.md`](skills/autospec-persona/SKILL.md)
- Use when: AutoSpec should learn the operator's preferences.
- How it works: runs a repo-grounded calibration interview with up to 50
  questions, stores answers in `~/.autospec/operator-persona.answers.json`, and
  resumes without re-asking completed batches.
- How to use: run `/autospec-persona`; use `--dry-run`, `--reset`, or
  `--max-questions N` as needed.

### `autospec-stop`

- Docs: [`README`](skills/autospec-stop/README.md), [`SKILL.md`](skills/autospec-stop/SKILL.md)
- Use when: a monitor must halt gracefully or immediately.
- How it works: writes `~/.autospec/stop.flag`; graceful mode exits after the
  current issue, immediate mode pauses at the next safe step boundary with
  recoverable state.
- How to use: run `/autospec-stop --graceful`, `/autospec-stop --immediate`,
  `/autospec-stop --status`, or `/autospec-stop --resume`.

### `autospec-resume`

- Docs: [`README`](skills/autospec-resume/README.md), [`SKILL.md`](skills/autospec-resume/SKILL.md)
- Use when: a run was interrupted by a crash, host restart, or lost session.
- How it works: reads durable run-state and heartbeats, avoids stealing live
  workers, preserves unpushed work, and retries within
  `AUTOSPEC_RESUME_MAX_ATTEMPTS`.
- How to use: run `/autospec-resume`; add `--dry-run` for inspection.

### `autospec-rollover-status`

- Docs: [`README`](skills/autospec-rollover-status/README.md), [`SKILL.md`](skills/autospec-rollover-status/SKILL.md)
- Use when: you need context rollover or compaction status.
- How it works: reports current context percentage and the last rollover event
  for the active `autospec-session` monitor.
- How to use: ask for rollover status or run the skill directly in a monitored
  session.

## Adding Or Updating Skills

1. Create or edit `skills/<skill-name>/SKILL.md`.
2. Keep harness bodies lock-step: `SKILL.md`, `opencode/agent.md`, and
   `codex/prompt.md` must differ only in frontmatter.
3. Add or update the skill README.
4. Update this guide and any README workflow table affected by the change.
5. Run `bash scripts/validate.sh`.
