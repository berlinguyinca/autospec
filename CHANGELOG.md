# Changelog

All notable changes to autospec are documented here.

The format is loosely based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
the repo uses conventional commits (`feat:`, `fix:`, `docs:`, `test:`, `refactor:`).

## [Unreleased]

### Added

#### Managed GitHub Projects and bounded repository onboarding (2026-08-28)
- Added one marker-verified managed GitHub Project per product, backed by a private repo-local
  binding and append-only projection journal. Reconciliation is additive and idempotent, keeps
  every generated issue and accountability epic recoverable, and retries transient item-add
  failures without duplicate issues or Project items.
- Added `autospec project resolve`, `sync`, and `onboard` for explicit repository seeds, bounded
  `--owner`/`--allow` discovery, and local workspace onboarding. Owner enumeration is capped by
  `discovery_max_repos`, and matches become exact seeds only after the command allowlist is
  applied. Discovery cannot widen the configured allowlist; deterministic relationships become
  active while ambiguous evidence stays proposed and non-blocking.
- Repository bootstrap now registers a verified remote after creation and records explicit
  `spawned-from` provenance. Existing explicit Project URLs remain compatible in external mode,
  and accountability retains its undeleted `~/.autospec/project-map.yml` compatibility fallback
  when no managed policy exists. Classifier `--apply-boards` label routing remains independent.

### Fixed

#### Generated metadata was charged to the authored word budget (2026-08-23)
- `scripts/lint-issue.sh` exempts generated blocks from the 400-word `BODY_TOO_LONG`
  cap, but the exemption is line-bounded. The `## Model fit` and `## Quality lint`
  templates emitted their heading ABOVE the opening marker, so the heading and its
  bullets were counted as authored prose. Phase 3 issues land close enough to the
  cap that Phase 3.5's own insertion tips them over: measured on the new fixture,
  the block adds 25 counted words to a 362-word body. Marker moved above the
  heading in 10 skill files (20 blocks), matching what
  `scripts/classify-model-fit.sh` and the shared-contracts template already did.
- `strip_generated_metadata` tracked only the `autospec-classify` and
  `autospec-shared-contracts` families. `autospec-quality` was never exempt at all,
  so an issue flagged once could never be brought back under the cap.
- Interaction with the UI-section exemption: `strip_ui_sections` skips lines
  after a UI heading until the next `## ` line, so running it first DELETED the
  opening marker of a generated block trailing the UI sections, leaving an
  unmatched end marker and disabling the exemption entirely - raising the counted
  total by 9 words on a `ui-feature` body, worse than before the marker moved.
  `strip_ui_sections` now terminates its skip on a generated begin marker, which
  closes the leak in both directions regardless of pipeline order: the block stays
  exempt, and authored prose following it is still counted.
- `scripts/autospec-explore.sh` and `scripts/extract-shared-contracts.sh` emitted
  generated blocks with no markers at all, so every word counted; both now wrap
  their output, the latter on both exit paths.
- The classify and quality idempotency clauses now tell the classifier to delete a
  legacy heading sitting above the begin marker, so re-running over an existing
  backlog does not leave an orphan heading plus a duplicate.
- `crates/autospec-core`'s `word_count_excluding_ui_sections` mirrored
  `strip_ui_sections` but had no generated-metadata exemption for any family, so
  `autospec lint issue` reported 535 words where the shell reported 362 on the same
  fixture. The Rust path now mirrors the full pipeline: a differential sweep of
  every `tests/fixtures/issue-quality/*.md` through both engines goes from 9
  divergent fixtures to 8, and the one that converges is the BODY_TOO_LONG case.
  The remaining 8 are unrelated rules (`AC_NOT_CHECKABLE`, `GOAL_NOT_ONE_SENTENCE`,
  `SMOKE_NOT_FENCED`, `MISSING_SECTION_DEPENDENCIES`) and are byte-identical on
  `main`.

#### Five validate checks that were red for reasons unrelated to their subject (2026-08-19)
- `lint-implementation.sh` resolved `SCRIPT_DIR` with `dirname`, so on the deliberately stripped
  PATH two reuse-triage cases build, the sibling classifier file could not be found and the linter
  exited 2 instead of linting. Resolved with builtins now.
- Four assertions still demanded prose at paths #3213 and #3156 had moved it away from; they now
  check the single source it moved to, plus that the trio still points there.
- `tests/dogfood/allowlist/qa-brute-force-sweep.json` gained the row #2977 forgot when it split
  the executor-bridge test file: `codex_permission.rs` arrived allow-listed nowhere while its
  sibling `codex_sandbox.rs` was covered.
- `tests/install/test_star_prompt.sh` substituted `HOME` to sandbox the install, which also hid
  rustup's toolchain and made the build inside `install.sh` fail. It now pins `RUSTUP_HOME`/
  `CARGO_HOME` and skips the runtime build outright via a new `AUTOSPEC_SKIP_RUNTIME_BINARY=1`.

#### build-test no longer hangs in its bootstrap step (2026-08-19)
- #3241 added `python3 -m pip install pytest pyyaml` to `build-test`, which has no
  `actions/setup-python`. That targets the runner's externally-managed system Python (PEP 668);
  instead of failing, the step hung for 87 minutes on `bf70d66b` and the job was killed before
  reaching a single test. Installed through apt now, which needs no venv and cannot prompt.

#### build-test reports every failing test, and can run the python gate (2026-08-18)
- `cargo test --workspace` is fail-fast, so the job stopped at the first failing binary. One
  long-standing failure hid four other test binaries entirely — `main` reported a single failing
  test while the workspace carried seven — and hid every later step in the job, so
  `Validate repository` and `Build` had not run in CI for as long. Now `--no-fail-fast`.
- The job also never installed pytest, which `check_python_suites` shells out to from both the
  workspace suite and `autospec validate`. Only the python workflow installed it.

#### A crashed conductor no longer parks its repository for five minutes (2026-08-18)
- `decide_conductor_lease` excluded the `claimed` state from dead-owner reclamation, so a
  conductor killed inside the claim window left a lease that no replacement could take until
  `STALE_LEASE_SECS` (300) elapsed, even though the owner was provably dead on the same host.
  The store already disagreed: `release_terminated_owner` treats `status == "claimed"` with a
  dead `lock_pid` as an abandoned claim it may release. A proven-dead local owner now reclaims
  from any state; an unknown or foreign owner still reads as live. The owner's harness may still
  be running, and the replacement adopts it rather than starting a second one.
#### The frozen validation catalog records the reference-pointer gate (2026-08-18)
- #3158 added `check_reference_pointer_integrity` to the catalog but not to the frozen fixture
  every count is pinned against, so five assertions across four test binaries were wrong: the
  fixture id list, its length in two tests, the legacy top-level call totals, and both plan
  counts. #3230 separately moved the canonical guardian file to `skills/autospec-run/SKILL.md`
  while the expected error message still named `skills/autospec/SKILL.md`. None of it showed up
  in CI, because `cargo test --workspace` is fail-fast and stopped at an earlier failing binary.

#### DOC_OUT_OF_SYNC no longer reads a comment as a new CLI surface (2026-08-18)
- The rule scans added lines for a long flag and demands a touched doc file. It scanned
  comments too, so moving a doc comment that mentions `claim release --claim-id` between
  files was reported as introducing that flag — a pure code move could not be committed
  without an unrelated doc edit. A comment names a surface, it cannot introduce one, which
  is already why `*.md` is skipped wholesale; the pattern only matches a flag followed by
  whitespace or `=`, and those lines are never comments. `is_comment_line` now lives beside
  the path classifiers, with a negative-control test proving a flag in a real invocation
  still trips the rule.

#### `install.sh --update` no longer exits non-zero on 12 skill pairs (2026-08-18)
- `install.sh` appends `--update` to every `skills/<skill>/install.sh` invocation, but four
  installers — `autospec-monitor`, `autospec-quality`, `autospec-rollover-status` and
  `autospec-test` — had no `--update` arm, so their argument parsers exited 2 on
  `unknown argument: --update`. That failed all three harness pairs for each of them: 12 of
  117 pairs red and a non-zero exit on every update run, while the skills themselves stayed
  stale. Their writes were already unconditional overwrites, so the flag needed no behaviour
  of its own, only parity. `tests/unit/skill-installer-flag-surface.bats` now drives every
  installer with `--dry-run --update` so a fifth cannot drift, and carries a negative control
  proving the check can fail.

#### One failing conductor test no longer takes six others with it (2026-08-18)
- The integration tests that drive the real bridge serialize on a `Mutex`, taken with
  `lock().expect(..)`. A panicking test poisons that mutex, so every later test died on
  the lock rather than running: a CI run showed one real assertion failure followed by
  six `real bridge E2E lock` panics that said nothing about anything. The guard is now
  poison-tolerant, matching `test_environment()` in the executor-bridge tests. The data
  it protects is `()`, so no invariant can have been broken by the earlier panic.

#### Conductor lease releases despite an inherited descriptor (2026-08-18)
- `LeaseTransaction` released its flock by closing the descriptor, the same way the
  evidence-attempt lease did before #3225. `with_current_lifecycle_lease` holds a
  transaction across an arbitrary operation, and a fork duplicates the descriptor into
  a child that shares the open file description the lock belongs to — so a child forked
  during that window pins the lease for its own lifetime and the next conductor is told
  the lease is `Held` (exit 20). It now unlocks explicitly. Latent: no observed symptom
  is attributed to it, unlike #3225.
- Moved `LeaseTransaction` to `resilience/lease_transaction.rs` beside
  `heartbeat_tests.rs`, because `resilience.rs` is past the size ratchet and the fix
  could not be added to it. 1,187 -> 1,147.

### Fixed

#### Evidence-attempt lease survives a fork (2026-08-18)
- An evidence-attempt lease was released by closing its descriptor. The lane launcher
  forks a supervisor that never execs, so a lease open at that moment is inherited --
  and an flock belongs to the open file description, which fork duplicates, so closing
  the owner's copy released nothing. The lane stayed owned for the supervisor's whole
  life and the next attempt was told `another evidence attempt owns this exact lane`.
  The lease now unlocks explicitly before closing, which reaches the shared
  description. This had been failing `check_block_expansion`'s sibling suite in CI on
  every PR since #3148.

#### autospec trio goldens regenerated (2026-08-18)
- `check_block_expansion` has been failing on `main` since #3213 changed the
  `autospec` trio without regenerating its three block-expansion goldens; they
  were last written by #3182 two days earlier. The bodies themselves are fine --
  `derive-trio.sh --check skills/autospec` passes -- so this is the missed
  regeneration, nothing more. Re-expanding all 113 goldens now finds zero
  mismatches.

#### Heartbeat renewal is waited for, not slept through (2026-08-18)
- `heartbeat_survives_owned_transaction_contention_and_renews_later` slept a fixed
  100 ms for a renewal published by a thread on a 10 ms interval, so a loaded machine
  missed the window and the test failed on an assertion about the code under test.
  It now polls for the renewal with a deadline, the same shape as the `wait_for`
  helper beside it. Observed as a 1-of-811 failure locally.
- Moved those ten tests into `resilience/heartbeat_tests.rs`, `include!`d the way
  `policy_tests.rs` and its siblings already are: `resilience.rs` was 1,607 lines and
  past the size ratchet, so the fix could not be added to it. Now 1,187.

### Fixed

#### OpenCode subagent mode (2026-08-18)
- Declared `mode: primary` on the 14 OpenCode agent adapters that omitted it. An
  absent `mode:` means `all`, which made every one of them spawnable through the
  task tool: a child would then carry an 11k-21k-token skill body on top of its
  own preamble. Measured on a 24 GiB llama.cpp node, four such children summed
  288,970 tokens of peak against a 180,224-token KV pool and killed every live
  session with `Context size has been exceeded`.

#### Prose is not a public-surface change (2026-08-18)
- `DOC_OUT_OF_SYNC` scanned markdown for flag and env-var shapes, so a CHANGELOG
  entry that *mentions* a flag was read as that flag's introduction -- and since
  `CHANGELOG.md` is deliberately not a doc for the requirement half, a
  changelog-only change could never satisfy the gate it had just tripped.
  Markdown now joins `*.diff` in the scan exemption. `CHANGELOG.md` still earns no
  credit as documentation: if it did, every commit would satisfy the rule and the
  rule would be dead.

#### Path classifiers extracted (2026-08-18)
- The three predicates that decide whether a changed path is doc, test or fixture
  data now live in `scripts/lint-path-classifiers.sh`, sourced from
  `lint-implementation.sh`. That file is past the 600-line ratchet and may not
  grow, and the ratchet's own advice is to move code out rather than add to it.
  It ships through the existing top-level `scripts/*.sh` glob, so no installer
  change is needed -- deliberately not `scripts/lib/`, which `copy_repo_scripts`
  excludes.

#### Fixture diffs are data, not source (2026-08-18)
- `TODO_LEFT` and `MOCK_DB` scanned `*.diff` fixtures line by line, so a fixture
  that exists to contain a violation was reported as the violation -- which
  blocked landing one. `DOC_OUT_OF_SYNC` and the density scanner already skipped
  `*.diff`; these two now do too. `SECURITY` deliberately still scans them: a
  leaked key is a leaked key wherever it sits.

#### Nested tests count as tests (2026-08-18)
- `is_test_file` matched only the repo-root `tests/` tree, so the 826 test files
  living elsewhere -- 381 under `crates/autospec-cli`, 75 under
  `skills/autospec-shared`, 51 under `skills/autospec-test` -- were invisible to
  `ASSERTION_DENSITY`, `MOCK_DB` and the gated `VACUOUS_*` detectors, while
  `TODO_LEFT` and `DOC_OUT_OF_SYNC` fired on them as if they were production
  source. Measured on three real commits that touch nested tests, widening the
  glob adds no new findings; it removes false positives and closes the hole.

#### Nested docs count as docs (2026-08-18)
- `lint-implementation.sh`'s `is_doc_file` anchored `README*`, `docs/*` and
  `AGENTS.md` at the repo root, so none of the 63 non-root README files and no
  subproject `docs/` tree counted. A public-surface change documented in the right
  place still tripped `DOC_OUT_OF_SYNC`, and the only way to satisfy the gate was
  to touch an unrelated root doc. `SKILL.md` was already matched at any depth;
  the rest now are too.

### Added

#### V62-V74 final platform release candidate (2026-07-06)
- Added an additive Rust workspace with `autospec-core` and `autospec-cli` for spec parsing, dependency ordering, state transitions, validation primitives, execution queue types, agent contracts, safety policy, evidence bundles, release reports, and local-only growth reporting.
- Added the `autospec` CLI command surface with implemented `doctor` plus documented JSON/stub boundaries for `status`, `plan`, `validate`, `report`, `showcase`, and `growth-report`.
- Added JSON schemas for spec metadata, execution order, spec state, run reports, agent results, evidence bundles, and release reports.
- Added V74 release-candidate evidence under `.autospec/releases/` and `.autospec/reports/`, with public launch validation now requiring the final release candidate artifacts.
- Updated docs, demo, safety, CLI reference, and growth trackers so the public launch story reflects the V62-V73 platform slice rather than V61-only readiness.

#### V61 external launch readiness (2026-07-03)
- Rewrote the top-level README for external developers with pitch, quickstart, demo, architecture, comparison, maturity, and contribution guidance.
- Added the docs launch set: `docs/index.md`, `docs/quickstart.md`, `docs/concepts.md`, `docs/architecture.md`, `docs/workflows.md`, `docs/faq.md`, and `docs/roadmap.md`.
- Added public community and trust files: `CONTRIBUTING.md`, `SECURITY.md`, `SAFETY.md`, `ROADMAP.md`, GitHub issue templates, and a PR template.
- Added demo and launch materials under `examples/hello-autospec/`, `docs/assets/`, `scripts/demo-recording.sh`, and `marketing/`.
- Added `scripts/validate-launch-readiness.sh`, which prints `AUTOSPEC_V61_LAUNCH_READY=true` when required launch artifacts are present.
- Added V25/V60/public launch release-state gates under `.autospec/` plus `scripts/validate-v25-baseline.sh`, `scripts/validate-v60-release.sh`, and `scripts/validate-public-launch-readiness.sh`.

#### Auto context rollover — perpetual-session monitor (2026-05-31 → 2026-06-01)
- New `autospec-session` launcher wraps `claude` / `codex` / `opencode` in a tmux daemon that injects `/compact` at 50% context and `/create-handoff` → `/clear` → resume at 80%, same terminal and process.
- NORMAL/COMPACTED/ROLLED state-machine engine, per-harness transcript adapters (Claude transcript, Codex `info:null` fallback, OpenCode SQLite), Claude PreCompact hook mode (no tmux), opt-in `install.sh` prompt, cancel-window overlay, handoff validation gate, and a cost/value telemetry ledger. (#743–#776, #777, #783, #801–#819, #897)
- New `autospec-rollover-status` skill reports live context % and the last rollover event. (#770, #784)

#### autospec-e2e-clone — production-clone provisioner (tracker #423)
- New skill: an isolated, scaled-down, PII-anonymized clone of a production environment for autospec-test Mode II — snapshot capture, edge-case seeding, adapters for docker-compose / k8s / staging-slot / custom-cmd, and teardown + autospec-test integration. (#473, #474, #493)

#### autospec-doc — multi-audience documentation engine (2026-06-03)
- New docs-as-tests engine: a `documentation:` config + folder contract, per-audience generators (user / developer / admin / general), `verify-examples.mjs` (doc examples run as tests), `doc-style.mjs` palette + mermaid theming, `gen-llms-full.mjs` + manifest fill, and an incremental-by-default orchestrator with deterministic-first cost caps. Wired into `/autospec-sweep --full` and `/autospec-define` auto-docs. (#914, #921, #927–#946, #970)

#### autospec-explore — autonomous research + ship loop (2026-06-03)
- New skill: 7 researchers propose features on an isolated sandbox branch, filtered through adversarial verify + ROI + severity rank, then drained via `/autospec-run` with PRs that never target `main`; `autospec-listen` routes explore/discover intent behind a confirmation gate. (#907, #912, #913, #964, #983)

#### autospec-loop — goal-conditioned loop skill (2026-06-02)
- New skill: freeze a request into a contract, then loop until the goal is met (refine-until-go gate, goal-conditioned loop, trigger disambiguation vs native `/loop`). (#890–#905)

#### autospec-playwright — no-mock UI test authoring, Stage 2A (2026-06-04)
- New thin dispatcher skill plus autospec-test Stage 2A: control inventory + effect-assertion taxonomy, `selector-evidence.mjs` source-grep resolver, `lint-playwright-author.mjs` (AST mock-ban + adaptive retry), and reset-endpoint generation with guard-env rails. (#990–#1010)

#### autospec-fleet GUI (2026-06-01 → 2026-06-02)
- Backend launcher `fleet-gui.sh` (HTTP server, atomic config writes, constant-time auth) plus an accessible vanilla-JS repo-picker page, `--once` smoke mode, and flock-guarded concurrent config writes. (#826, #832–#865)

#### Cost & token efficiency (2026-06-03 → 2026-06-04)
- Fresh-subagent-per-issue with batch default 1, slim implementer prefix + cache-boundary fix, per-issue body fetched once with SHA-gated reviewer re-fetch, deterministic-first classify + tiered reviewer (`AUTOSPEC_REVIEWER_TIER`), `post-token-report.sh` + committed baseline, and skill-block templates with install-time expansion + version-skew guard. (#938–#950, #967, #969, #1015–#1040)

#### Git hygiene, guardrails & correctness (2026-06-01 → 2026-06-14)
- `worktree-guard.sh` (assert / resolve-branch / create) with a PR-aware worktree ladder asserted before every commit step across the run / define / doc / release / explore trios. (#959–#983)
- AGENTS.md converted to a deterministic-lint-enforced contract with `linter:allow-` escape hatches (#824); deterministic complexity gates — file/function LOC, cyclomatic, duplicate names (#822); Phase 3.75 architectural alignment between decomposition and implementation (#823); mandatory Pattern-survey before Phase 4 (#820).
- Full-suite proof required before merge (#926), autospec-run done-challenge before convergence, and a structured result-first Closeout report contract + gate (2026-06-14).

#### Phase 2 roadmap — operational hardening (2026-05-22)
- `feat(model-tier): Haiku trial profile + select-model-profile routing` (#417 / #428)
  Routes `reasoning:shallow` / `reasoning:medium` issues to Claude Haiku; `reasoning:deep` stays on Sonnet. Per-profile telemetry tracks quality.
- `feat(heartbeat): repo-scoped heartbeat directory layout` (#416 / #426)
  Restructures `~/.autospec/process-heartbeats/` to `<repo-slug>/<issue>.json`. Closes cross-repo heartbeat collisions.
- `feat(validate): duo-lockstep check for SKILL.md+codex-only skills` (#415 / #425)
  `check_lockstep()` now also runs as a duo (SKILL.md ↔ codex/prompt.md) when `opencode/agent.md` is absent.
- `feat(autospec-run): orchestrator bundle-and-dispatch helper` (#418 / #429)
  Wraps `bundle-static-context.sh` with dynamic suffix → realizes the prompt-caching gains in practice.
- `feat(autospec-run): self-enforcement CI workflow` (#419 / #430)
  Contributor PRs touching `skills/**`, `scripts/**`, `docs/specs/**` go through lint-implementation + validate.sh; blocking violations fail the workflow.
- `feat(autospec-run): telemetry dashboard` (#422 / #431)
  `gen-telemetry-dashboard.sh` emits HTML with cache hit-rate trend, per-role token cost, LGTM first-pass rate, outlier table.
- `fix(autospec-docs): drift gate warn-only on autospec self` (#427)
  Temporary operational fix while over-broad scopes get a coordinated narrow+widen PR.

#### Docs amendment — universal doc generation + drift gate (2026-05-22)
- `feat(autospec-docs): dogfood + lockstep validate.sh` (#374 / #412)
- `feat(autospec-docs): language matrix` (#373 / #411)
- `feat(autospec-docs): synthetic targets + integration harness` (#372 / #410)
- `feat(autospec-docs): drift-gate installers + CI workflow` (#371 / #409)
- `feat(autospec-docs): --init mode + Phase 4 drift gate wiring` (#369 / #408)
- `feat(autospec-docs): AI-as-reviewer + confidence routing` (#368 / #399)
- `feat(autospec-docs): screenshots + mermaid diagrams` (#367 / #383)
- `feat(autospec-docs): llms.txt + manifest + assistant prompt` (#366 / #382)
- `feat(autospec-docs): initial doc generators` (#365 / #381)
- `feat(autospec-docs): reverse-engineer pipeline` (#364 / #380)
- `feat(autospec-docs): self-heal classifier extension` (#363 / #379)
- `feat(autospec-docs): scope parser + drift checker` (#362 / #378)
- `feat(autospec-docs): tree-sitter foundation + per-language queries` (#361 / #377)

#### Prompt caching (2026-05-22)
- `feat(autospec-run): reviewer prompt cache structure` (#404 / #407)
- `feat(autospec-run): telemetry capture + cache-hit summary` (#403 / #406)
- `feat(autospec-run): bundle-static-context.sh + implementer prompt cache structure` (#402 / #405)

#### Pipeline hardening (2026-05-22)
- `feat(autospec-run): adaptive-retry loop in implementer` (#392 / #398)
- `feat(autospec-run): implementer-prompt enrichment + gen-ac-tests` (#391 / #397)
- `fix(autospec-run): batch size + reasoning:deep gating` (#390 / #396)
- `feat(autospec-run): CI-wait sentinel` (#389 / #395)
- `fix(autospec-run): pre-commit lint hook + lint-implementation modes` (#388 / #394)
- `fix(autospec-run): memory-tag frontmatter bootstrap` (#387 / #393)

#### autospec-test v2 — Stage 2.5 invariants extension (2026-05-21 → 2026-05-22)
- `feat(autospec-test): stage 2.5 orchestrator + assertion-shift v2 + SKILL.md` (#351 / #376)
- `test(autospec-test): v2 synthetic targets` (#350 / #375)
- `feat(autospec-test): @autospec/test npm helper library` (#349 / #370)
- `feat(autospec-test): edge-case seed verifier + DB driver shims` (#348 / #359)
- `feat(autospec-test): metric I contract symmetry` (#347 / #357)
- `feat(autospec-test): metric H extended crawler` (#346 / #356)
- `feat(autospec-test): metric G window-contract symmetry` (#345 / #355)
- `feat(autospec-test): metric F structural invariants runner` (#344 / #354)
- `feat(autospec-test): built-in invariant kinds` (#343 / #353)
- `feat(autospec-test): v2 contract extension + JSON Schema` (#342 / #352)

#### autospec-test v1 — unit + E2E coverage gate (2026-05-21)
- `feat(autospec-test): SKILL.md + lockstep validation + docs (phase 10)` (#328 / #340)
- `feat(autospec-run): wire autospec-test into phase 4 (phase 9)` (#327 / #339)
- `test(autospec-test): synthetic targets + language matrix (phase 8)` (#326 / #338)
- `feat(autospec-test): operator wizard (phase 7)` (#325 / #337)
- `feat(autospec-test): mode-ii scoped-prod runtime (phase 6)` (#324)
- `feat(autospec-test): self-heal loop (phase 5)` (#323 / #335)
- `feat(autospec-test): assertion-shift AST classifier (phase 4)` (#322 / #334)
- `feat(autospec-test): stage 2 e2e gate + safety layers (phase 3)` (#321 / #332)
- `feat(autospec-test): stage 1 unit gate (phase 2)` (#320 / #330)
- `feat(autospec-test): contract loader + JSON schema (phase 1)` (#319 / #329)

### Design specs landed

- `docs: autospec Phase 2 roadmap (10 strategic gaps)` (#413)
- `docs: autospec prompt caching design` (#400)
- `docs: autospec pipeline hardening design` (#385)
- `docs: autospec family docs amendment design` (#358)
- `docs: autospec-test v2 design (invariants + window contracts + contract symmetry)` (#333)
- `docs: autospec-test skill design spec` (#317)

### Tracker issues (queued for follow-on `/autospec-define` cycles)

- `#420` Mutation testing (test-of-tests, vacuous-assertion detector, mutmut/Stryker gate)
- `#421` Tooling optimization (deterministic templates, batched classifier, gen-pr-report)
- `#424` Distribution / install UX (npm package, npx, marketplace listings)

(`#423` clone provisioner shipped as `autospec-e2e-clone` — see Added above.)

---

_Earlier history predates this changelog; see `git log` for full record._
