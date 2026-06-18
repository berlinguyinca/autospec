# Skill Index

Every autospec capability is a skill under [`skills/`](skills). Each ships three
harness variants kept byte-identical by the lock-step rule — Claude Code
(`SKILL.md`), OpenCode (`opencode/agent.md`), and Codex CLI (`codex/prompt.md`) —
plus a per-skill `install.sh` / `uninstall.sh`. Only frontmatter differs between
harnesses; [`scripts/validate.sh`](scripts/validate.sh) enforces this.

This file lists every skill with its trigger and activation keywords. For
task-oriented "I want to…" guidance, see [`README.md`](README.md).

## Core pipeline — plan and ship

### `autospec`
- Path: [`skills/autospec`](skills/autospec)
- Trigger: ship a feature end-to-end — bootstrap a missing repo, design, decompose into linked issues (or split an existing spec), and run the autonomous implementation loop with admin auto-merge.
- Keywords: `autospec`, `ship this feature`, `auto-implement this feature`, `ship end-to-end`, `decompose and auto-implement`, `run the autonomous loop`

### `autospec-define`
- Path: [`skills/autospec-define`](skills/autospec-define)
- Trigger: planning only — bootstrap, brainstorm a spec, and decompose into classified issues; stop after Phase 3 and hand off to `/autospec-run`.
- Keywords: `autospec-define`, `plan a feature`, `design a spec`, `decompose into issues`

### `autospec-split`
- Path: [`skills/autospec-split`](skills/autospec-split)
- Trigger: turn an existing tracked `docs/specs/*.md` spec into GitHub issues, then stop after Phase 3.5.
- Keywords: `autospec-split`, `split existing spec`, `split latest spec`, `turn this spec into GitHub issues`, `roadmap this spec`, `materialize this spec`

### `autospec-run`
- Path: [`skills/autospec-run`](skills/autospec-run)
- Trigger: run the implementation half (Phases 4–6) over a populated `auto-implement` queue with admin auto-merge. `--profile <name>` filters by model profile.
- Keywords: `autospec-run`, `run the queue`, `implement the issues`, `process auto-implement`

### `autospec-classify`
- Path: [`skills/autospec-classify`](skills/autospec-classify)
- Trigger: retro-apply the Phase 3.5 model-fit rubric to existing issues — add `ctx:*` / `reasoning:*` labels and a `## Model fit` block.
- Keywords: `autospec-classify`, `classify issues`, `model-fit labels`, `add ctx labels`

## Capture and refine intent

### `autospec-listen`
- Path: [`skills/autospec-listen`](skills/autospec-listen)
- Trigger: a mid-conversation imperative to file an issue, write a spec, or build/ship something. Drafts an issue for approval, routes spec requests to `/autospec-define`, or gates build verbs to the mapped skill. Bare nouns are not triggers.
- Keywords: `file an issue`, `new issue`, `open an issue`, `create a ticket`, `write a spec`, `design spec`, `new spec`, `implement`, `build`, `ship`, `review`

### `autospec-continue`
- Path: [`skills/autospec-continue`](skills/autospec-continue)
- Trigger: `/continue` — extract the last assistant recommendation, refine it, and hand off to `/autospec --autonomous`. Supports `--skip-refine`, `--ask-confirm`, `--lens-mode`, `--from-message`.
- Keywords: `continue`, `act on that recommendation`, `do the suggested next step`

### `autospec-refine`
- Path: [`skills/autospec-refine`](skills/autospec-refine)
- Trigger: sharpen a prompt or feature request over N repo-grounded lenses before handing off to `/autospec`. Supports `--rounds`, `--lenses`, `--autonomous`, `--interactive`, `--dry-run`, `--continue`.
- Keywords: `autospec-refine`, `refine this request`, `tighten the prompt`

## Test, review, and QA

### `autospec-review`
- Path: [`skills/autospec-review`](skills/autospec-review)
- Trigger: audit design specs against open and closed issues to find gaps and file `[REGRESSION]` issues. Auto-fires after each `autospec-run` batch unless `~/.autospec/no-review.flag` exists.
- Keywords: `autospec-review`, `audit specs`, `find spec gaps`, `regression review`, `spec vs issues`

### `autospec-test`
- Path: [`skills/autospec-test`](skills/autospec-test)
- Trigger: gate every Phase 4 PR on unit + E2E coverage with a self-heal loop (≤5 iterations / 60 min), blocking assertion-loosening rewrites; also runs standalone against a branch.
- Keywords: `autospec-test`, `coverage gate`, `e2e gate`, `enforce test coverage`, `auto-heal tests`

### `autospec-qa`
- Path: [`skills/autospec-qa`](skills/autospec-qa)
- Trigger: revalidate a running app against its spec — audit UI controls, forms, validation, dropdowns, API behavior, and accessibility, and regenerate weak or missing tests.
- Keywords: `autospec-qa`, `qa revalidate`, `spec compliance audit`, `regenerate tests`, `validate dropdowns`, `validation audit`

### `autospec-playwright`
- Path: [`skills/autospec-playwright`](skills/autospec-playwright)
- Trigger: run disciplined no-mock Playwright UI-test authoring (autospec-test Stage 2A) against `.autospec/test.yml` authoring blocks and print the coverage report.
- Keywords: `autospec-playwright`, `playwright tests`, `ui test authoring`, `no-mock e2e`

### `autospec-e2e-clone`
- Path: [`skills/autospec-e2e-clone`](skills/autospec-e2e-clone)
- Trigger: provision an isolated, scaled-down, PII-anonymized clone of a production environment for E2E testing (autospec-test Mode II).
- Keywords: `autospec-e2e-clone`, `clone production`, `anonymized environment`, `e2e clone`

## Docs and design

### `autospec-doc`
- Path: [`skills/autospec-doc`](skills/autospec-doc)
- Trigger: generate, regenerate, or audit per-audience documentation (user, developer, admin, general) as docs-as-tests. Supports `--full`, `--audit`, `--audience <name>`, and `init`.
- Keywords: `autospec-doc`, `generate docs`, `audit docs`, `per-audience documentation`, `docs as tests`

### `autospec-design`
- Path: [`skills/autospec-design`](skills/autospec-design)
- Trigger: adopt a vendor design language — score the repo against catalog vendors, write `DESIGN.md`, and optionally migrate existing UI. Subcommands `suggest`, `apply <vendor>`, `migrate <vendor>`.
- Keywords: `autospec-design`, `adopt design`, `design language`, `DESIGN.md`, `vendor design system`
- Catalog: [`berlinguyinca/awesome-design-md`](https://github.com/berlinguyinca/awesome-design-md) (MIT), cached 24h under `~/.autospec/design-cache/`.

## Lifecycle and reporting

### `autospec-sweep`
- Path: [`skills/autospec-sweep`](skills/autospec-sweep)
- Trigger: first-run autospec configuration, recurring spec-vs-reality sweeps, or continuous improvement across docs, tests, and code health.
- Keywords: `autospec-sweep`, `sweep`, `configure autospec`, `first-run config`, `continuous improvement`, `spec sync`

### `autospec-release`
- Path: [`skills/autospec-release`](skills/autospec-release)
- Trigger: end-to-end release-readiness sweep across specs, docs, implementation, tests, QA proof, legacy cleanup, and merge readiness; returns `PASS`, `PARTIAL`, or `FAIL`.
- Keywords: `autospec-release`, `release readiness`, `release sweep`, `release gate`, `ship current repo`

### `autospec-story`
- Path: [`skills/autospec-story`](skills/autospec-story)
- Trigger: synthesize a cited repo-level product story and implementation-state report from local specs plus GitHub issues and PRs.
- Keywords: `autospec-story`, `repo story`, `implementation state`, `what has been built`, `state of the application`

### `autospec-fleet`
- Path: [`skills/autospec-fleet`](skills/autospec-fleet)
- Trigger: supervise `autospec-run` across multiple GitHub repos from an empty workspace — config schemas, checkout planning, dry-run command generation, JSON status, stop forwarding.
- Keywords: `autospec-fleet`, `fleet init`, `fleet run`, `fleet status`, `run autospec across repos`

## Run control and recovery

### `autospec-stop`
- Path: [`skills/autospec-stop`](skills/autospec-stop)
- Trigger: halt a running monitor — `--graceful` (after the current issue), `--immediate` (next step boundary), `--status`, `--resume`. Writes `~/.autospec/stop.flag`.
- Keywords: `autospec-stop`, `stop`, `halt the monitor`, `pause autospec`, `resume autospec`

### `autospec-resume`
- Path: [`skills/autospec-resume`](skills/autospec-resume)
- Trigger: a fresh process detects an interrupted run from durable state plus heartbeats and auto-continues it after a crash — without stealing a live worker or deleting un-pushed work. Capped at `AUTOSPEC_RESUME_MAX_ATTEMPTS` (default 3).
- Keywords: `autospec-resume`, `resume interrupted run`, `recover crashed run`, `continue after crash`

### `autospec-rollover-status`
- Path: [`skills/autospec-rollover-status`](skills/autospec-rollover-status)
- Trigger: report current context % and the last rollover event for the active session monitor.
- Keywords: `rollover status`, `context status`, `how close to rollover`, `is compaction imminent`

### `autospec-loop`
- Path: [`skills/autospec-loop`](skills/autospec-loop)
- Trigger: run a single task repeatedly until a goal is reached. Freezes the request into a contract, then runs a goal-conditioned loop. Defers bare interval polling to native `/loop`.
- Keywords: `autospec-loop`, `loop until the build passes`, `keep going until done`, `run X in a loop until Y`

### `autospec-explore`
- Path: [`skills/autospec-explore`](skills/autospec-explore)
- Trigger: start a perpetual autonomous research + ship loop on an isolated sandbox branch — 7 researchers propose features from spec/code gaps, prior reports, codebase signals, open issues, source analysis, dependency health, and competitor research, then drain via `/autospec-run` with PRs targeting the sandbox branch (never `main`).
- Keywords: `autospec-explore`, `explore and ship`, `autonomous research loop`, `discovery loop`

### `autospec-harmonize`
- Path: [`skills/autospec-harmonize`](skills/autospec-harmonize)
- Trigger: discover design-token drift in a codebase, generalize a baseline, generate variants, preview them, pick one, and emit a dated migration spec ready for `/autospec-define`.
- Keywords: `autospec-harmonize`, `harmonize design`, `design tokens`, `style drift`, `palette inconsistency`, `design migration spec`

## Adding a skill

1. Create `skills/<skill-name>/SKILL.md`; keep the body concise and move large detail into `references/`.
2. Add deterministic helpers under `scripts/` when repeated shell/API work is error-prone.
3. Derive the `opencode/agent.md` and `codex/prompt.md` variants (lock-step) and regenerate goldens.
4. Validate with `bash scripts/validate.sh`.
5. Add the skill to the [`README.md`](README.md) catalog and to this index.
