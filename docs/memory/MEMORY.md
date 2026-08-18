<!--
ACTIVE USE PROTOCOL (this file is auto-loaded every session — read this header)

Before each significant decision, do a 5-second mental check:
  "Is there a memory below that applies to what I'm about to do?"

Major decision triggers (consult memory BEFORE):
  - dispatching a subagent (esp. for autospec/long-running work)
  - recovering from a failure (silent-exit, stalled batch, label conflict)
  - filing new issues (sizing caps, lockstep, decomposer gotchas)
  - design choices (skill-per-capability, ROI check, autonomy scope)
  - bash / shell scripting (set -e, return traps, heredoc gotchas)
  - validation / lockstep (validate.sh checks, trio/duo gaps)

If a memory applies and you didn't reference it, that's a miss. Call it out
to the user when you catch yourself. Memory works only if you actually use it.

Adding new memories: when you'd otherwise re-learn the same lesson, save it.
Removing old ones: when a memory cites a closed issue or "SHIPPED" status
older than 7 days AND the lesson is no longer load-bearing, archive it.
-->

# Always-relevant

- [Operational assistant provider routing](feedback_operational_assistant_provider_routing.md) — health and chat must share a relay-aware route plan for backend-only providers such as NATS

- [User role - autospec author](user_role.md) — berlinguyinca authors and maintains the multi-harness autospec skill family
- [Proactively query memory before major decisions](feedback_proactively_query_memory.md) — meta-rule: passive auto-load isn't enough; explicit "what memory applies here?" check before each dispatch/recovery/design choice

# Autospec workflow

- [Autospec gap ledger](autospec-gap-ledger.md) — stable repeat-count ledger for mined review/CI/scope/QA misses filed as gap-remediation work
- [Sync repo before /autospec design phases](feedback_pre_pipeline_sync.md) — check git fetch/status before brainstorming; stale local edits often duplicate landed upstream work
- [autospec-split origin/main gate](feedback_autospec_split_origin_main_gate.md) — /autospec-split halts if spec not on origin/main; land spec first to avoid mid-pipeline PR detour
- [Per-PR LGTM misses integration](feedback_per_pr_lgtm_misses_integration.md) — Phase 5.5 broad-audit caught 7 high-sev integration bugs that 19 per-PR LGTMs missed; never skip 5.5
- [Autospec design preferences](feedback_autospec_design_prefs.md) — small-LLM target (60-120k ctx), correctness>>speed, tight imperative triggers, conservative guardrails, lock-step rule sacred
- [Autospec monitor exit modes + recovery](feedback_monitor_silent_exit.md) — Phase 4 monitor silent-exits on complex integration work; relaunch is part of the workflow; subprocess mocks for tmux/osascript prevent test stalls
- [Autospec decomposer + lockstep gotchas](feedback_autospec_decomposer_gotchas.md) — first new-skill issue MUST include structural sections (Self-update + Model tier + adapter row); codex/prompt.md needs leading blank line for lockstep; decomposer should NOT apply needs-autospec-template
- [lint BODY_TOO_LONG counts injected metadata](feedback_lint_body_too_long_counts_injected_metadata.md) — Phase 3.5/3.75 append Model-fit + Shared-contracts blocks after the ≤400-word trim, so every classified child trips needs-quality-bar; blanket BODY_TOO_LONG-only flags are benign
- [Admin-merge denial during autospec](feedback_admin_merge_denial.md) — `gh pr merge --admin` blocked by harness hook; needs settings.json permission rule for Phase 4 to flow
- [/autospec autonomy scope](feedback_autospec_autonomy_scope.md) — auto-merge spec PRs, collapse low-stakes brainstorm to default-locks, surface only run/defer/refine + destructive-remote actions
- [Skill per capability](feedback_autospec_skill_per_capability.md) — operator-facing capabilities ship as top-level /autospec-<verb> skills; inline sub-modes are convenience shortcuts only
- [Perpetual capabilities extend the conductor](feedback_capabilities_are_conductor_tiers_not_new_conductors.md) — autonomous workstreams reuse autospec-autonomous tiers instead of duplicating conductor control planes
- [One-shot tiers must clear trigger state](feedback_oneshot_to_tier_must_clear_trigger_state.md) — a conductor tier must retire the signal it consumes or it will re-fire and starve lower tiers
- [Autospec mode-dispatch must not shell out user text](feedback_autospec_no_shell_user_text.md) — Self-update/Stop mode sections must be pure prose; no bash heredocs of `{FEATURE_DESCRIPTION}`, and no inlined `$1`/`$2`/`$3` (the harness DOES substitute those; #3177 fixed, #3101 is the un-gated residual)
- [Refine-then-run workflow shorthand](feedback_refine_then_run_workflow.md) — user's normal autospec flow is refine, review checkpoint, then run; route imperative shorthand through autospec-listen

# Quality / framework discipline

- [Generic failure diagnostics flow](feedback_generic_failure_diagnostics_flow.md) — separate state from reason, group correlated failures, and excerpt logs around high-signal anchors
- [ROI-check new components](feedback_roi_check_new_components.md) — every new skill/fork/schema needs a named consumer that benefits today; default to invoking upstream over forking
- [LLM validator + adaptive retry](feedback_llm_validator_adaptive_retry.md) — pair every LLM-output validator with a 5-attempt retry loop that feeds findings back as directives
- [Pre-commit gate shapes the commit split](feedback_precommit_gate_commit_shaping.md) — logical_units=3 binds before the 400-line cap; every source commit needs its own doc touch; oversized files may not gain a line; never wrap bats `run` in a helper
- [validate.sh has named-content checks](feedback_validate_sh_lockstep_checks.md) — renaming SKILL.md prose sections requires updating validate.sh checks too
- [validate.sh lockstep duo gap](feedback_validate_sh_lockstep_duo_gap.md) — check_lockstep() must guard SKILL.md+codex/prompt.md duos, not just full trios
- [Skill golden + derivation workflow](feedback_skill_golden_derivation_workflow.md) — editing any trio skill (esp. with autospec-block markers) needs re-derive codex/opencode AND regenerate tests/fixtures/skill-goldens sha256, or validate.sh fails closed
- [Decompose: trio prose + goldens must be one issue](feedback_decompose_trio_prose_goldens_atomic.md) — never split "edit trio prose" and "regen goldens" into separate auto-implement issues; the prose-only intermediate fails validate closed (bit 3x in one run); combine into one implementer that Closes both
- [Trio derivation tooling (use it)](reference_trio_derivation_tooling.md) — edit SKILL.md then `derive-trio.sh --in-place` + `gen-skill-goldens.sh`; stop hand-maintaining codex/opencode + goldens; validate's check_derive_trio_consistency enforces it; decomposer now treats trio+goldens as one unit
- [Golden generator takes a bare skill name](feedback_gen_skill_goldens_bare_name.md) — `derive-trio.sh` takes a skill path, but `gen-skill-goldens.sh` takes a bare name; exit 3 means regeneration did not happen
- [jq test() regex metachar injection](feedback_jq_test_regex_metachar_injection.md) — interpolating host/user-derived values into jq test() is regex injection; dotted hostnames made claim self-clean delete the wrong worker's lock comment; use capture()+==
- [Self-consistent test fixtures mask bugs](feedback_self_consistent_test_fixtures_mask_bugs.md) — tests that build fixtures with the SUT's own derivation expression can't catch a bug in it; the Claude transcript-slug `lstrip` bug shipped green for months. Pin against the real convention / live values; reproduce end-to-end

# Infrastructure gotchas

- [Context floor kills small tiers](feedback_context_floor_kills_small_tiers.md) — measure the before-any-work floor (OpenCode p90 37,873) before picking a window; it is client-specific and invisible in conversation length

- [Shared KV pool has no admission control](feedback_shared_kv_pool_has_no_admission_control.md) — llama.cpp `kv-unified` lets sessions differ in size but over-subscription kills every live session; ration client-side

- [Background pipeline exit masking](feedback_background_pipeline_exit_masking.md) — `cmd | tail; echo` background tasks report exit 0 even when the gate failed; parse the gate's own final status line, and zsh uses lowercase `pipestatus`
- [No tree mutation during background validate](feedback_no_tree_mutation_during_bg_validate.md) — switching/deleting branches while a background validate.sh runs corrupts its checkout mid-run → false "required file missing"; run it in a dedicated detached worktree and confirm the gate's OK line, not just the (echo-masked) exit code
- [Validate baseline diffing](feedback_validate_baseline_diff.md) — validate is red on main, so compare failure SETS against a clean origin/main worktree; per-suite spot checks gave a false all-clear and let 2 regressions through CI

- [Installer excludes runtime libs](feedback_installer_excludes_runtime_libs.md) — install.sh drops scripts/lib/ runtime libs; autospec-explore hard-crashes on a clean install; ship-completeness doesn't catch it
- [Install shared-library helpers from their own source root](feedback_install_shared_lib_scripts_dir.md) — helpers under `skills/autospec-shared/scripts` belong in `SHARED_LIB_SCRIPT_FILES`, not the repo-root script group
- [Codex exec needs the repository-check override](feedback_codex_exec_needs_skip_git_repo_check.md) — headless integrations must pass `--skip-git-repo-check`; argument-blind stubs hide the refusal
- [Explore codebase-signals false positives](feedback_explore_codebase_signals_false_positives.md) — TODO/FIXME grep matches prose, assets, and its own source; noise dominates ranking; constitution gate drops 0/45
- [Explore --once unverified ~0% precision](feedback_explore_once_unverified_near_zero_precision.md) — local discovery on autospec repo: 183 raw → 0 verified; even source-analysis was 4/4 false on direct check; never auto-file --once bare-subprocess output, verify evidence against files first

- [Heartbeat cross-repo collision](feedback_heartbeat_cross_repo_collision.md) — ~/.autospec/process-heartbeats/ is shared across repos; use path-scoped slug subdirs
- [Bash RETURN trap leaks](feedback_bash_return_trap_leak.md) — RETURN traps leak into caller frames under set -u; use inline cleanup
- [Bash set -e short-circuit aborts](feedback_bash_set_e_short_circuit.md) — `[ test ] && action` aborts under set -e when test fails; use if/then/fi for one-sided conditionals
- [Indirect environment lookup without eval](feedback_indirect_env_lookup_no_eval.md) — `${!name:-}` is Bash 3.2-safe and avoids command injection from config-derived variable names
- [bash 3.2 process-sub + [ -f ] in tests](feedback_bash32_process_sub_test_file.md) — macOS bash 3.2 `[ -f <(...) ]` is false; bats tests must write to a real temp file before a --validate-file/[ -f ] helper
- [Subagent cwd pinned to main checkout](feedback_subagent_cwd_pinned_to_main_checkout.md) — subagents may write through absolute worktree paths, but the main agent must own git operations to avoid cross-branch contamination
- [Per-session worktree isolation](feedback_per_session_worktree_isolation.md) — concurrent autospec/claude sessions stomp each other; start every edit in a fresh worktree off origin/main + pre-flight overlap scan + file/skill claim, never edit on a shared/in-flight branch
- [Worktree resource isolation proof](feedback_worktree_resource_isolation.md) — use an isolated real Docker engine for scale evidence, measure label-scoped resources, fail cleanup loudly, and reject symlinked private state roots
- [OMC autopilot magic-keyword misfire](feedback_omc_autopilot_misfire.md) — system reminders containing "AUTOPILOT" auto-activate OMC autopilot mid-session; recover via state_write(active=false) + state_clear(skill-active)
- [Mempalace miner flat-form gap](feedback_mempalace_miner_flat_form.md) — M3 miner matches both `metadata.type:` (spec) and `type:` (real CC files); fixture both variants
- [Harness session-id env vars](reference_harness_session_id_envs.md) — `CLAUDE_CODE_SESSION_ID` is the stable per-session id (ps -o sess=0, no tty under tool calls); fallback chain for harness-neutral per-session locks; PPID fallback is unreliable
- [Worktree/main topology](reference_worktree_main_topology.md) — check `git worktree list` before assuming primary can hold `main`; sibling worktrees may own it, and stale N-ahead branches usually need fresh origin/main branches

# Active project state (review weekly; archive when shipped)

- [Autospec tooling optimization — tracker #421](project_autospec_tooling_optimization.md) — convert LLM-driven steps to deterministic tools
- [Autospec mutation testing — tracker #420](project_autospec_mutation_testing.md) — test-of-tests layer: mutation gate + assertion-density floor + negative-path-pair lint
- [Babysit-tax → Autonomy Charter](project_babysit_tax_autonomy_charter.md) — session mining: operator confirmations are ~always rubber-stamps of the agent's own recommendation; build a standing recommendation=action charter, auto-chain define→run→explore, push notifications on async waits
- [Tier 1.5 promotion activation](project_tier15_promotion_activation.md) — autospec-autonomous issue-promotion gates, stale-install caveats, and explore verifier bridge status

<!-- Archived (shipped or session-historical; files kept on disk, removed from index):
- project_2026_05_22_23_session_close.md — historical session close 2026-05-22→23
- project_e2e_coverage_gate_design.md — autospec-test family SHIPPED 2026-05-22
- project_autospec_phase2_roadmap.md — Phase 2 fixes shipped May 2026; trackers folded into items above
- project_autospec_init_skill.md — folded into docs amendment; residual under #424
- project_autospec_review_status.md — autospec-review shipped 2026-05-07
- project_harness_aware_model_tier.md — harness-aware tier shipped 2026-05-07
- project_monitor_session_reset.md — monitor session-reset + guardian fusion shipped
- project_turbo_integration_design.md — turbo/autospec integration shipped
- project_cross_session_ci_rot.md — cross-session CI rot shipped 2026-05-18
- project_cross_tool_memory_brainstorm.md — cross-tool memory M1-M5 shipped 2026-05-23 (PRs #503-#507)
- project_gap_remediation_keyword_routing.md — gap-remediation + keyword-routing shipped 2026-05-24
- feedback_mempalace_integration_boundary.md — mempalace-specific, archived to keep index focused
- project_memory_consumers_epic.md — superseded by current session's framework improvements (#820-#824)
- Auto-context-rollover feature SHIPPED 2026-06-01 — see .turbo/handoff/2026-06-01-auto-context-rollover-shipped.md for the full PR list (43 PRs incl. framework #820-#824)
-->
