use std::fs;
use std::path::{Path, PathBuf};

use super::command::ToolCommand;
use super::results::{output_digest, CheckResult};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalCheck {
    BashSyntax,
    DeriveTrioConsistency,
    FleetScripts,
    Frontmatter,
    GapMinerContract,
    GeneratedYamlParse,
    AutospecSweepConfig,
    AutospecSweepAreaContract,
    AutospecQaClusterContract,
    AutospecQaBugClassContract,
    LoopHandoffHarnessAwareness,
    ReleaseVerdictScript,
    BatsSuite(&'static str),
    ReviewerReuseLens,
    BashHelpUsage(&'static str),
    SupersessionContract,
    RunSummaryContract,
    DbModuleInstall,
    BatsDirectory(&'static str),
    AutospecUpgradeContract,
    ClaimGuardContract,
    ClaimCasGuard,
    WatchdogWorktreeGc,
    BlockExpansion,
    AutospecExploreImplementerBase,
    AutospecExploreResearchersDeterministic,
    AutospecExploreResearchersLlm,
    AutospecExploreSpecialistsDiscovery,
    AutospecExploreStage2Intersect,
    ExploreTrioWorktreeAssert,
    AutospecExploreSpecFirst,
    AutospecExploreQaGate,
    AutospecExploreStyleNormalization,
    AutospecExploreOrchestrator,
    AutospecTestSkill,
    AutospecPlaywrightSkill,
    AutospecFabContract,
    GroomingContract,
    MutationAndNegativePath,
    LintImplementationHelpers,
    LintIssueHelpers,
    Phase4CiStatusCompare,
    DefineSpecWorktreeRouting,
    RunGroomPreflightContract,
    GrowRunContract,
    PerformanceWorkstream,
    UxUiWorkstream,
    TokenBaselineFresh,
    ArchitectureFitnessEngine,
    Phase4TestSuites,
}

impl ExternalCheck {
    pub fn run(&self, id: &str, required: bool, root: &Path) -> CheckResult {
        match self {
            Self::BashSyntax => run_bash_syntax(id, required, root),
            Self::DeriveTrioConsistency => run_derive_trio_consistency(id, required, root),
            Self::FleetScripts => run_fleet_scripts(id, required, root),
            Self::Frontmatter => run_frontmatter(id, required, root),
            Self::GapMinerContract => run_gap_miner_contract(id, required, root),
            Self::GeneratedYamlParse => run_generated_yaml_parse(id, required, root),
            Self::AutospecSweepConfig => run_autospec_sweep_config(id, required, root),
            Self::AutospecSweepAreaContract => run_autospec_sweep_area_contract(id, required, root),
            Self::AutospecQaClusterContract => run_autospec_qa_cluster_contract(id, required, root),
            Self::AutospecQaBugClassContract => {
                run_autospec_qa_bug_class_contract(id, required, root)
            }
            Self::LoopHandoffHarnessAwareness => {
                run_loop_handoff_harness_awareness(id, required, root)
            }
            Self::ReleaseVerdictScript => run_release_verdict_script(id, required, root),
            Self::BatsSuite(suite) => run_bats_suite(id, required, root, suite),
            Self::ReviewerReuseLens => run_reviewer_reuse_lens(id, required, root),
            Self::BashHelpUsage(script) => run_bash_help_usage(id, required, root, script),
            Self::SupersessionContract => run_supersession_contract(id, required, root),
            Self::RunSummaryContract => run_run_summary_contract(id, required, root),
            Self::DbModuleInstall => run_db_module_install(id, required, root),
            Self::BatsDirectory(directory) => run_bats_directory(id, required, root, directory),
            Self::AutospecUpgradeContract => run_autospec_upgrade_contract(id, required, root),
            Self::ClaimGuardContract => run_claim_guard_contract(id, required, root),
            Self::ClaimCasGuard => run_claim_cas_guard(id, required, root),
            Self::WatchdogWorktreeGc => run_watchdog_worktree_gc(id, required, root),
            Self::BlockExpansion => run_block_expansion(id, required, root),
            Self::AutospecExploreImplementerBase => {
                run_autospec_explore_implementer_base(id, required, root)
            }
            Self::AutospecExploreResearchersDeterministic => {
                run_autospec_explore_researchers_deterministic(id, required, root)
            }
            Self::AutospecExploreResearchersLlm => {
                run_autospec_explore_researchers_llm(id, required, root)
            }
            Self::AutospecExploreSpecialistsDiscovery => {
                run_autospec_explore_specialists_discovery(id, required, root)
            }
            Self::AutospecExploreStage2Intersect => {
                run_autospec_explore_stage2_intersect(id, required, root)
            }
            Self::ExploreTrioWorktreeAssert => run_explore_trio_worktree_assert(id, required, root),
            Self::AutospecExploreSpecFirst => run_autospec_explore_spec_first(id, required, root),
            Self::AutospecExploreQaGate => run_autospec_explore_qa_gate(id, required, root),
            Self::AutospecExploreStyleNormalization => {
                run_autospec_explore_style_normalization(id, required, root)
            }
            Self::AutospecExploreOrchestrator => {
                run_autospec_explore_orchestrator(id, required, root)
            }
            Self::AutospecTestSkill => run_skill_validator(id, required, root, "autospec-test"),
            Self::AutospecPlaywrightSkill => run_autospec_playwright_skill(id, required, root),
            Self::AutospecFabContract => run_autospec_fab_contract(id, required, root),
            Self::GroomingContract => run_grooming_contract(id, required, root),
            Self::MutationAndNegativePath => run_mutation_and_negative_path(id, required, root),
            Self::LintImplementationHelpers => run_lint_implementation_helpers(id, required, root),
            Self::LintIssueHelpers => run_lint_issue_helpers(id, required, root),
            Self::Phase4CiStatusCompare => run_phase4_ci_status_compare(id, required, root),
            Self::DefineSpecWorktreeRouting => run_define_spec_worktree_routing(id, required, root),
            Self::RunGroomPreflightContract => run_groom_preflight_contract(id, required, root),
            Self::GrowRunContract => run_grow_run_contract(id, required, root),
            Self::PerformanceWorkstream => run_performance_workstream(id, required, root),
            Self::UxUiWorkstream => run_ux_ui_workstream(id, required, root),
            Self::TokenBaselineFresh => run_token_baseline_fresh(id, required, root),
            Self::ArchitectureFitnessEngine => run_architecture_fitness_engine(id, required, root),
            Self::Phase4TestSuites => run_phase4_test_suites(id, required, root),
        }
    }
}

fn run_bash_syntax(id: &str, required: bool, root: &Path) -> CheckResult {
    let mut targets = Vec::new();
    for skill_dir in trio_directories(root) {
        for script in ["install.sh", "uninstall.sh"] {
            let path = skill_dir.join(script);
            if path.is_file() {
                targets.push(relative_path(root, &path));
            }
        }
    }
    for script in ["install.sh", "uninstall.sh"] {
        let path = root.join(script);
        if path.is_file() {
            targets.push(script.to_string());
        }
    }

    run_bash_syntax_targets(id, required, root, targets)
}

fn run_bash_syntax_targets(
    id: &str,
    required: bool,
    root: &Path,
    targets: Vec<String>,
) -> CheckResult {
    let mut results = Vec::new();
    for target in targets {
        let command = ToolCommand::new("bash", ["-n", target.as_str()])
            .expect("bash syntax validation has no command-string argument");
        let result = command.execute_in(id, required, root);
        let failed = result.is_failure();
        results.push(result);
        if failed {
            break;
        }
    }
    aggregate(id, required, results)
}

fn run_commands(
    id: &str,
    required: bool,
    root: &Path,
    commands: impl IntoIterator<Item = ToolCommand>,
) -> CheckResult {
    let mut results = Vec::new();
    for command in commands {
        let result = command.execute_in(id, required, root);
        let failed = result.is_failure();
        results.push(result);
        if failed {
            break;
        }
    }
    aggregate(id, required, results)
}

fn run_frontmatter(id: &str, required: bool, root: &Path) -> CheckResult {
    let python_available = program_on_path("python3");
    let mut results = Vec::new();

    for skill_dir in trio_directories(root) {
        for member in ["SKILL.md", "opencode/agent.md"] {
            let path = skill_dir.join(member);
            let relative = relative_path(root, &path);
            let Ok(document) = fs::read_to_string(&path) else {
                results.push(failure(
                    id,
                    required,
                    &format!("{relative}: missing frontmatter"),
                ));
                return aggregate(id, required, results);
            };
            let Some(frontmatter) = frontmatter_body(&document) else {
                results.push(failure(
                    id,
                    required,
                    &format!("{relative}: missing or empty frontmatter"),
                ));
                return aggregate(id, required, results);
            };

            let result = if python_available {
                ToolCommand::new(
                    "python3",
                    ["-c", PYTHON_FRONTMATTER_CHECK, relative.as_str()],
                )
                .expect("the Python frontmatter check has static source and file arguments")
                .execute_in(id, required, root)
            } else if has_frontmatter_key(&frontmatter) {
                CheckResult::completed(id, required, 0, 0, 0, 0, 0, output_digest(&[], &[]))
            } else {
                failure(
                    id,
                    required,
                    &format!("{relative}: frontmatter does not look like key: value pairs"),
                )
            };
            let failed = result.is_failure();
            results.push(result);
            if failed {
                return aggregate(id, required, results);
            }
        }
    }

    aggregate(id, required, results)
}

fn run_derive_trio_consistency(id: &str, required: bool, root: &Path) -> CheckResult {
    let derive = root.join("scripts/derive-trio.sh");
    if !derive.is_file() {
        return failure(
            id,
            required,
            "scripts/derive-trio.sh: missing (trio-derivation Phase 1 must be merged first)",
        );
    }

    let mut results = Vec::new();
    for skill_dir in trio_directories(root) {
        let relative = relative_path(root, &skill_dir);
        let command = ToolCommand::new("bash", ["scripts/derive-trio.sh", &relative, "--check"])
            .expect("bash -n style validation commands are direct argument vectors");
        let result = command.execute_in(id, required, root);
        let failed = result.is_failure();
        results.push(result);
        if failed {
            break;
        }
    }

    aggregate(id, required, results)
}

fn run_fleet_scripts(id: &str, required: bool, root: &Path) -> CheckResult {
    let targets = ["fleet-lib.sh", "fleet-init.sh", "fleet-config-lint.sh"]
        .into_iter()
        .map(|name| format!("skills/autospec-fleet/scripts/{name}"))
        .collect::<Vec<_>>();
    for target in &targets {
        if !root.join(target).is_file() {
            return failure(id, required, &format!("{target}: required file missing"));
        }
    }
    run_bash_syntax_targets(id, required, root, targets)
}

fn run_gap_miner_contract(id: &str, required: bool, root: &Path) -> CheckResult {
    let miner = root.join("scripts/autospec-gap-miner.sh");
    if !is_executable(&miner) {
        return failure(
            id,
            required,
            "scripts/autospec-gap-miner.sh missing or not executable",
        );
    }
    if !root.join("docs/memory/autospec-gap-ledger.md").is_file() {
        return failure(id, required, "docs/memory/autospec-gap-ledger.md missing");
    }
    for (member, diagnostic) in [
        (
            "SKILL.md",
            "autospec-run SKILL.md missing gap miner closeout invocation",
        ),
        (
            "codex/prompt.md",
            "autospec-run codex prompt missing gap miner closeout invocation",
        ),
        (
            "opencode/agent.md",
            "autospec-run opencode agent missing gap miner closeout invocation",
        ),
    ] {
        if !contains(
            &root.join("skills/autospec-run").join(member),
            "autospec-gap-miner.sh",
        ) {
            return failure(id, required, diagnostic);
        }
    }

    let test = root.join("tests/validate-autospec-gap-miner.sh");
    if !test.is_file() {
        return failure(id, required, "tests/validate-autospec-gap-miner.sh missing");
    }
    let commands = [
        ToolCommand::new("bash", ["-n", "scripts/autospec-gap-miner.sh"])
            .expect("bash syntax command is a direct argument vector"),
        ToolCommand::new("bash", ["tests/validate-autospec-gap-miner.sh"])
            .expect("gap-miner test command is a direct argument vector"),
    ];
    run_commands(id, required, root, commands)
}

fn run_generated_yaml_parse(id: &str, required: bool, root: &Path) -> CheckResult {
    ToolCommand::new("python3", ["-c", PYTHON_GENERATED_YAML_CHECK])
        .expect("generated YAML validation has static Python source")
        .execute_in(id, required, root)
}

fn run_autospec_sweep_config(id: &str, required: bool, root: &Path) -> CheckResult {
    let skill_dir = root.join("skills/autospec-sweep");
    if !skill_dir.is_dir() {
        return failure(id, required, "skills/autospec-sweep: directory missing");
    }
    let targets = ["wizard.sh", "run.sh", "review.sh"]
        .into_iter()
        .map(|name| format!("skills/autospec-sweep/scripts/{name}"))
        .collect::<Vec<_>>();
    for target in &targets {
        if !root.join(target).is_file() {
            return failure(id, required, &format!("{target}: required file missing"));
        }
    }
    if !root.join("schemas/autospec-config.schema.json").is_file() {
        return failure(
            id,
            required,
            "schemas/autospec-config.schema.json: required file missing",
        );
    }
    if !contains(&skill_dir.join("SKILL.md"), ".autospec/autospec.yml") {
        return failure(
            id,
            required,
            "skills/autospec-sweep/SKILL.md missing .autospec/autospec.yml contract",
        );
    }
    if !contains(&skill_dir.join("SKILL.md"), "continuous improvement") {
        return failure(
            id,
            required,
            "skills/autospec-sweep/SKILL.md missing continuous improvement contract",
        );
    }
    for member in ["SKILL.md", "codex/prompt.md", "opencode/agent.md"] {
        let path = root.join("skills/autospec").join(member);
        let display = format!("skills/autospec/{member}");
        if !contains(&path, ".autospec/autospec.yml") {
            return failure(
                id,
                required,
                &format!("{display} missing autospec.yml first-run preflight"),
            );
        }
        if !contains(&path, "/autospec-sweep init") {
            return failure(
                id,
                required,
                &format!("{display} missing /autospec-sweep init first-run route"),
            );
        }
    }
    run_bash_syntax_targets(id, required, root, targets)
}

fn run_release_verdict_script(id: &str, required: bool, root: &Path) -> CheckResult {
    let script = root.join("scripts/compute-release-verdict.sh");
    let bats = root.join("tests/compute-release-verdict.bats");
    if !script.is_file() {
        return failure(
            id,
            required,
            "scripts/compute-release-verdict.sh: required file missing",
        );
    }
    if !is_executable(&script) {
        return failure(
            id,
            required,
            "scripts/compute-release-verdict.sh: must be executable",
        );
    }
    if !bats.is_file() {
        return failure(
            id,
            required,
            "tests/compute-release-verdict.bats: required file missing",
        );
    }
    for member in ["SKILL.md", "codex/prompt.md", "opencode/agent.md"] {
        let relative = format!("skills/autospec-release/{member}");
        if !contains(&root.join(&relative), "compute-release-verdict.sh") {
            return failure(
                id,
                required,
                &format!("{relative}: missing reference to scripts/compute-release-verdict.sh"),
            );
        }
    }

    let commands = [
        ToolCommand::new("bash", ["-n", "scripts/compute-release-verdict.sh"])
            .expect("bash syntax command is a direct argument vector"),
        ToolCommand::new("bats", ["tests/compute-release-verdict.bats"])
            .expect("Bats validation has a static test-file argument"),
    ];
    run_commands(id, required, root, commands)
}

fn run_bats_suite(id: &str, required: bool, root: &Path, suite: &str) -> CheckResult {
    run_bats_suites(id, required, root, &[suite])
}

fn run_bats_suites(id: &str, required: bool, root: &Path, suites: &[&str]) -> CheckResult {
    for suite in suites {
        if !root.join(suite).is_file() {
            return failure(id, required, &format!("{suite}: bats coverage missing"));
        }
    }
    if !program_on_path("bats") {
        return CheckResult::completed(id, required, 0, 0, 0, 0, 0, output_digest(&[], &[]));
    }
    let commands = suites.iter().map(|suite| {
        ToolCommand::new("bats", [*suite]).expect("Bats validation has a static suite path")
    });
    run_commands(id, required, root, commands)
}

fn run_bats_suites_if_available(
    id: &str,
    required: bool,
    root: &Path,
    suites: &[&str],
) -> CheckResult {
    if !program_on_path("bats") {
        return CheckResult::completed(id, required, 0, 0, 0, 0, 0, output_digest(&[], &[]));
    }
    run_bats_suites(id, required, root, suites)
}

fn run_bats_directory(id: &str, required: bool, root: &Path, directory: &str) -> CheckResult {
    if !program_on_path("bats") {
        return CheckResult::completed(id, required, 0, 0, 0, 0, 0, output_digest(&[], &[]));
    }
    let directory_path = root.join(directory);
    let mut suites = fs::read_dir(&directory_path)
        .into_iter()
        .flat_map(|entries| entries.filter_map(Result::ok))
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .is_some_and(|extension| extension == "bats")
        })
        .map(|path| relative_path(root, &path))
        .collect::<Vec<_>>();
    suites.sort();
    if suites.is_empty() {
        return failure(
            id,
            required,
            &format!("{directory}/*.bats: no integration tests found"),
        );
    }
    let commands = suites.into_iter().map(|suite| {
        ToolCommand::new("bats", [suite.as_str()])
            .expect("Bats validation has a static discovered suite path")
    });
    run_commands(id, required, root, commands)
}

fn run_bats_directory_allow_empty_if_available(
    id: &str,
    required: bool,
    root: &Path,
    directory: &str,
) -> CheckResult {
    if !program_on_path("bats") {
        return CheckResult::completed(id, required, 0, 0, 0, 0, 0, output_digest(&[], &[]));
    }
    let Ok(entries) = fs::read_dir(root.join(directory)) else {
        return CheckResult::completed(id, required, 0, 0, 0, 0, 0, output_digest(&[], &[]));
    };
    let mut suites = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .is_some_and(|extension| extension == "bats")
        })
        .map(|path| relative_path(root, &path))
        .collect::<Vec<_>>();
    suites.sort();
    let commands = suites.into_iter().map(|suite| {
        ToolCommand::new("bats", [suite.as_str()])
            .expect("Bats validation has a static discovered suite path")
    });
    run_commands(id, required, root, commands)
}

fn run_bash_directory(id: &str, required: bool, root: &Path, directory: &str) -> CheckResult {
    let Ok(entries) = fs::read_dir(root.join(directory)) else {
        return CheckResult::completed(id, required, 0, 0, 0, 0, 0, output_digest(&[], &[]));
    };
    let mut scripts = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file() && path.extension().is_some_and(|extension| extension == "sh")
        })
        .map(|path| relative_path(root, &path))
        .collect::<Vec<_>>();
    scripts.sort();
    let commands = scripts.into_iter().map(|script| {
        ToolCommand::new("bash", [script.as_str()])
            .expect("self-contained shell test has a direct argument vector")
    });
    run_commands(id, required, root, commands)
}

fn run_reviewer_reuse_lens(id: &str, required: bool, root: &Path) -> CheckResult {
    const SUITE: &str = "tests/reviewer/test_reuse_lens.bats";
    if !root.join(SUITE).is_file() {
        return failure(
            id,
            required,
            "tests/reviewer/test_reuse_lens.bats: bats coverage missing (issue #1440)",
        );
    }
    if !contains(
        &root.join("scripts/gen-reviewer-prompt.sh"),
        "--reuse-flags",
    ) {
        return failure(
            id,
            required,
            "scripts/gen-reviewer-prompt.sh missing --reuse-flags support (issue #1440)",
        );
    }
    if !contains(&root.join("skills/autospec-run/SKILL.md"), "--reuse-flags") {
        return failure(
            id,
            required,
            "skills/autospec-run/SKILL.md does not pass --reuse-flags to gen-reviewer-prompt.sh (issue #1440)",
        );
    }
    run_bats_suite(id, required, root, SUITE)
}

fn run_bash_help_usage(id: &str, required: bool, root: &Path, script: &str) -> CheckResult {
    let path = root.join(script);
    if !path.is_file() {
        return failure(id, required, &format!("{script}: file missing"));
    }
    if !is_executable(&path) {
        return failure(id, required, &format!("{script}: file not executable"));
    }

    let syntax = ToolCommand::new("bash", ["-n", script])
        .expect("bash syntax command is a direct argument vector")
        .execute_in(id, required, root);
    if syntax.is_failure() {
        return aggregate(id, required, vec![syntax]);
    }

    let captured = ToolCommand::new("bash", [script, "--help"])
        .expect("bash help command is a direct argument vector")
        .execute_in_capturing(id, required, root);
    let mut help = captured.result;
    if help.is_success() && !has_usage_line(&captured.stdout) {
        const MESSAGE: &str = "--help did not print a 'Usage:' line";
        help.exit_code = Some(1);
        help.stderr_bytes += MESSAGE.len();
        help.output_digest = output_digest(&captured.stdout, MESSAGE.as_bytes());
    }
    aggregate(id, required, vec![syntax, help])
}

fn run_supersession_contract(id: &str, required: bool, root: &Path) -> CheckResult {
    const SCRIPT: &str = "scripts/resolve-spec-supersession.sh";
    const SUITE: &str = "tests/resolve-spec-supersession.bats";
    for skill in ["autospec-define", "autospec-qa", "autospec-release"] {
        for member in ["SKILL.md", "codex/prompt.md", "opencode/agent.md"] {
            let relative = format!("skills/{skill}/{member}");
            let path = root.join(&relative);
            if !has_line_prefix(&path, "## Spec supersession (recency)") {
                return failure(
                    id,
                    required,
                    &format!("{skill}: {member} missing '## Spec supersession (recency)' section"),
                );
            }
            if !contains(&path, "resolve-spec-supersession.sh") {
                return failure(
                    id,
                    required,
                    &format!(
                        "{skill}: {member} missing reference to scripts/resolve-spec-supersession.sh"
                    ),
                );
            }
        }
    }
    if !root.join(SUITE).is_file() {
        return failure(
            id,
            required,
            "tests/resolve-spec-supersession.bats: bats coverage missing",
        );
    }
    let helper = run_bash_help_usage(id, required, root, SCRIPT);
    if helper.is_failure() {
        return helper;
    }
    let bats = run_bats_suite(id, required, root, SUITE);
    aggregate(id, required, vec![helper, bats])
}

fn run_run_summary_contract(id: &str, required: bool, root: &Path) -> CheckResult {
    const HELPER: &str = "scripts/autospec-write-run-summary.sh";
    const SUITE: &str = "tests/autospec/test_run_summary.bats";
    for member in ["SKILL.md", "codex/prompt.md", "opencode/agent.md"] {
        let relative = format!("skills/autospec/{member}");
        let path = root.join(&relative);
        if !path.is_file() {
            return failure(id, required, &format!("{relative}: required file missing"));
        }
        for (required_literal, diagnostic) in [
            (
                ".autospec/run-summary.md",
                "Phase 6 missing .autospec/run-summary.md write directive",
            ),
            (
                "autospec-write-run-summary.sh",
                "Phase 6 missing autospec-write-run-summary.sh invocation",
            ),
            (
                "(none — converged)",
                "Phase 6 missing canonical convergence sentinel",
            ),
        ] {
            if !contains(&path, required_literal) {
                return failure(id, required, &format!("{relative}: {diagnostic}"));
            }
        }
    }
    if !root.join(SUITE).is_file() {
        return failure(
            id,
            required,
            "tests/autospec/test_run_summary.bats: bats coverage missing",
        );
    }
    let helper = run_bash_help_usage(id, required, root, HELPER);
    if helper.is_failure() {
        return helper;
    }
    let bats = run_bats_suite(id, required, root, SUITE);
    aggregate(id, required, vec![helper, bats])
}

fn run_db_module_install(id: &str, required: bool, root: &Path) -> CheckResult {
    const SUITE: &str = "tests/unit/install-db-module.bats";
    if !root.join(SUITE).is_file() {
        return failure(
            id,
            required,
            "tests/unit/install-db-module.bats: bats coverage missing (issue #1777)",
        );
    }
    if !contains(&root.join("install.sh"), "maybe_prompt_db_module") {
        return failure(
            id,
            required,
            "install.sh missing maybe_prompt_db_module (issue #1777)",
        );
    }
    if !contains(
        &root.join("install.sh"),
        "Install the optional database telemetry module (autospec-db)? [y/N]",
    ) {
        return failure(
            id,
            required,
            "install.sh missing autospec-db prompt text (issue #1777)",
        );
    }
    run_bats_suite(id, required, root, SUITE)
}

fn run_autospec_upgrade_contract(id: &str, required: bool, root: &Path) -> CheckResult {
    const SKILL: &str = "skills/autospec-upgrade";
    for member in ["SKILL.md", "codex/prompt.md", "opencode/agent.md"] {
        let relative = format!("{SKILL}/{member}");
        let path = root.join(&relative);
        if !path.is_file() {
            return failure(
                id,
                required,
                &format!("{relative}: required trio file missing (issue #1211)"),
            );
        }

        let document = fs::read_to_string(&path)
            .unwrap_or_default()
            .to_ascii_lowercase();
        for token in [
            "detect",
            "behavior-lock",
            "mutation",
            "codemod",
            "orchestrat",
        ] {
            if !document.contains(token) {
                return failure(
                    id,
                    required,
                    &format!("{relative}: missing upgrade-pipeline token `{token}` (issue #1211)"),
                );
            }
        }
    }

    run_bats_directory(id, required, root, "skills/autospec-upgrade/tests")
}

fn run_claim_guard_contract(id: &str, required: bool, root: &Path) -> CheckResult {
    const GUARD: &str = "scripts/claim-guard.sh";
    const SUITES: &[&str] = &[
        "tests/test_claim_guard_overlap.bats",
        "tests/test_claim_guard_stale_reclaim.bats",
        "tests/test_claim_guard_degrade.bats",
    ];

    let guard = root.join(GUARD);
    if !guard.is_file() {
        return failure(
            id,
            required,
            "scripts/claim-guard.sh: file missing (issue #1066)",
        );
    }
    if !is_executable(&guard) {
        return failure(
            id,
            required,
            "scripts/claim-guard.sh: file not executable (issue #1066)",
        );
    }

    let syntax = ToolCommand::new("bash", ["-n", GUARD])
        .expect("claim-guard syntax validation is a direct argument vector")
        .execute_in(id, required, root);
    if syntax.is_failure() {
        return syntax;
    }
    if !has_line_prefix(&guard, "        scan)") {
        return aggregate(
            id,
            required,
            vec![
                syntax,
                failure(
                    id,
                    required,
                    "scripts/claim-guard.sh: 'scan' subcommand missing from dispatch (issue #1066)",
                ),
            ],
        );
    }

    let bats = run_bats_suites(id, required, root, SUITES);
    aggregate(id, required, vec![syntax, bats])
}

fn run_claim_cas_guard(id: &str, required: bool, root: &Path) -> CheckResult {
    const TRIO: &[&str] = &[
        "skills/autospec-run/SKILL.md",
        "skills/autospec-run/opencode/agent.md",
        "skills/autospec-run/codex/prompt.md",
    ];
    const FIXTURE: &str = "tests/fixtures/gh-mock/gh";
    const SUITES: &[&str] = &[
        "tests/unit/test_autospec_claim_id_tiebreak.bats",
        "tests/unit/test_autospec_stale_lease_reclaim.bats",
    ];

    for trio in TRIO {
        let path = root.join(trio);
        if !path.is_file() {
            return failure(id, required, &format!("{trio}: required file missing"));
        }
        if !contains(&path, "claim-issue.sh") {
            return failure(
                id,
                required,
                &format!("{trio}: claim section missing reference to claim-issue.sh (issue #871)"),
            );
        }
        if contains_unguarded_claim_swap(&path) {
            return failure(
                id,
                required,
                &format!(
                    "{trio}: unguarded check-then-act 'gh issue edit --add-label in-progress-by-bot' outside the claim-issue.sh CAS path (issue #871)"
                ),
            );
        }
    }
    if !root.join(FIXTURE).is_file() {
        return failure(
            id,
            required,
            "tests/fixtures/gh-mock/gh: PATH-shadow mocked-gh fixture missing (issue #871)",
        );
    }

    run_bats_suites(id, required, root, SUITES)
}

fn run_watchdog_worktree_gc(id: &str, required: bool, root: &Path) -> CheckResult {
    const WATCHDOG: &str = "scripts/autospec-watchdog.sh";
    const SUITE: &str = "tests/resume/test_watchdog_gc.bats";

    let watchdog = root.join(WATCHDOG);
    if !watchdog.is_file() {
        return failure(id, required, "scripts/autospec-watchdog.sh: missing");
    }
    let syntax = ToolCommand::new("bash", ["-n", WATCHDOG])
        .expect("watchdog syntax validation is a direct argument vector")
        .execute_in(id, required, root);
    if syntax.is_failure() {
        return syntax;
    }

    let Ok(document) = fs::read_to_string(&watchdog) else {
        return aggregate(
            id,
            required,
            vec![
                syntax,
                failure(
                    id,
                    required,
                    "scripts/autospec-watchdog.sh: unable to read watchdog contract",
                ),
            ],
        );
    };
    let failure_message = if !document.contains("gc_orphaned_worktrees()") {
        Some("scripts/autospec-watchdog.sh must define gc_orphaned_worktrees() (issue #883)")
    } else if !document
        .lines()
        .any(|line| matches!(line, "gc_orphaned_worktrees" | "    gc_orphaned_worktrees"))
    {
        Some("scripts/autospec-watchdog.sh must invoke gc_orphaned_worktrees once per cycle (issue #883)")
    } else if !document.contains("git worktree remove --force") {
        Some("scripts/autospec-watchdog.sh GC must prune via 'git worktree remove --force' (issue #883)")
    } else if document
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .any(contains_rm_rf)
    {
        Some("scripts/autospec-watchdog.sh must never 'rm -rf' a worktree path; use 'git worktree remove --force' (issue #883)")
    } else if !document.contains("log --not --remotes") {
        Some("scripts/autospec-watchdog.sh GC must gate on 'git -C <wt> log --not --remotes' for un-pushed commits (issue #883)")
    } else if !root.join(SUITE).is_file() {
        Some("tests/resume/test_watchdog_gc.bats: GC bats fixture missing (issue #883)")
    } else {
        None
    };
    if let Some(message) = failure_message {
        return aggregate(id, required, vec![syntax, failure(id, required, message)]);
    }

    let bats = run_bats_suites(id, required, root, &[SUITE]);
    aggregate(id, required, vec![syntax, bats])
}

fn run_block_expansion(id: &str, required: bool, root: &Path) -> CheckResult {
    const EXPANDER: &str = "scripts/expand-skill-blocks.sh";
    const GOLDEN_DIRECTORY: &str = "tests/fixtures/skill-goldens";
    const MEMBERS: &[(&str, &str)] = &[
        ("SKILL.md", "SKILL.md"),
        ("codex/prompt.md", "codex.prompt.md"),
        ("opencode/agent.md", "opencode.agent.md"),
    ];

    if !root.join(EXPANDER).is_file() {
        return failure(
            id,
            required,
            "check_block_expansion: scripts/expand-skill-blocks.sh missing (fail closed)",
        );
    }

    let mut skill_directories = fs::read_dir(root.join("skills"))
        .into_iter()
        .flat_map(|entries| entries.filter_map(Result::ok))
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    skill_directories.sort();

    let mut commands = Vec::new();
    let mut mismatches = Vec::new();
    for skill_directory in skill_directories {
        let Some(skill) = skill_directory.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        for (member, suffix) in MEMBERS {
            let source = skill_directory.join(member);
            if !source.is_file() {
                continue;
            }
            let golden_relative = format!("{GOLDEN_DIRECTORY}/{skill}.{suffix}.sha256");
            let golden = root.join(&golden_relative);
            if !golden.is_file() {
                if *suffix == "SKILL.md" {
                    return block_expansion_result(
                        id,
                        required,
                        commands,
                        Some(format!(
                            "check_block_expansion: no golden for {skill} ({golden_relative} missing — fail closed)"
                        )),
                    );
                }
                if contains(&source, "<!-- autospec-block:") {
                    return block_expansion_result(
                        id,
                        required,
                        commands,
                        Some(format!(
                            "check_block_expansion: markered member {} has no golden ({golden_relative} missing — fail closed)",
                            relative_path(root, &source)
                        )),
                    );
                }
                continue;
            }

            let expected = fs::read_to_string(&golden)
                .unwrap_or_default()
                .chars()
                .filter(|character| !character.is_whitespace())
                .collect::<String>();
            let source_relative = relative_path(root, &source);
            let expanded = ToolCommand::new("bash", [EXPANDER, source_relative.as_str()])
                .expect("block expansion uses static direct argument vectors")
                .execute_in_capturing(id, required, root);
            let hashed = ToolCommand::new("shasum", ["-a", "256"])
                .expect("SHA-256 hashing uses static direct argument vectors")
                .execute_in_with_stdin_capturing(id, required, root, &expanded.stdout);
            let got = String::from_utf8_lossy(&hashed.stdout)
                .split_ascii_whitespace()
                .next()
                .unwrap_or_default()
                .to_string();
            commands.push(expanded.result);
            commands.push(hashed.result);

            if got != expected {
                mismatches.push(format!("{skill}.{suffix}"));
            }
        }
    }

    let failure = (!mismatches.is_empty()).then(|| {
        format!(
            "check_block_expansion: sha256 mismatch for members: {}",
            mismatches.join(" ")
        )
    });
    block_expansion_result(id, required, commands, failure)
}

fn block_expansion_result(
    id: &str,
    required: bool,
    commands: Vec<CheckResult>,
    failure_message: Option<String>,
) -> CheckResult {
    let mut digest_input = commands
        .iter()
        .flat_map(|result| result.output_digest.bytes().chain(std::iter::once(b'\n')))
        .collect::<Vec<_>>();
    if let Some(message) = &failure_message {
        digest_input.extend_from_slice(message.as_bytes());
    }

    CheckResult::completed(
        id,
        required,
        i32::from(failure_message.is_some()),
        commands.iter().map(|result| result.elapsed_ms).sum(),
        commands.iter().map(|result| result.spawn_count).sum(),
        commands.iter().map(|result| result.stdout_bytes).sum(),
        commands
            .iter()
            .map(|result| result.stderr_bytes)
            .sum::<usize>()
            + failure_message.as_ref().map_or(0, |message| message.len()),
        output_digest(&digest_input, &[]),
    )
}

fn run_autospec_explore_implementer_base(id: &str, required: bool, root: &Path) -> CheckResult {
    const IMPLEMENTER: &str = "skills/autospec-run/prompts/phase4-implementer.md";
    const TRIO: &[&str] = &[
        "skills/autospec-run/SKILL.md",
        "skills/autospec-run/codex/prompt.md",
        "skills/autospec-run/opencode/agent.md",
    ];
    const SUITE: &str = "tests/explore/test_explore_implementer_base.bats";

    let implementer = root.join(IMPLEMENTER);
    if !implementer.is_file() {
        return failure(
            id,
            required,
            "skills/autospec-run/prompts/phase4-implementer.md: required file missing",
        );
    }
    for (token, message) in [
        (
            ".autospec/explore-mode.json",
            "missing .autospec/explore-mode.json reference",
        ),
        (
            "EXPLORE_BASE",
            "missing EXPLORE_BASE sandbox-base resolution",
        ),
        (
            "gh pr create --base \"$EXPLORE_BASE\"",
            "missing 'gh pr create --base \"$EXPLORE_BASE\"' snippet",
        ),
        (
            "code_health:explore_main_merge_refused",
            "missing code_health:explore_main_merge_refused refusal identifier",
        ),
    ] {
        if !contains(&implementer, token) {
            return failure(id, required, &format!("{IMPLEMENTER} {message}"));
        }
    }
    for trio in TRIO {
        let path = root.join(trio);
        for (token, message) in [
            (
                ".autospec/explore-mode.json",
                "missing .autospec/explore-mode.json reference (lockstep)",
            ),
            (
                "phase4-implementer.md",
                "missing reference to phase4-implementer.md (lockstep)",
            ),
            (
                "code_health:explore_main_merge_refused",
                "missing code_health:explore_main_merge_refused (lockstep)",
            ),
        ] {
            if !contains(&path, token) {
                return failure(id, required, &format!("{trio} {message}"));
            }
        }
    }

    run_bats_suites(id, required, root, &[SUITE])
}

fn run_autospec_explore_researchers_deterministic(
    id: &str,
    required: bool,
    root: &Path,
) -> CheckResult {
    let scripts = run_required_bash_scripts(
        id,
        required,
        root,
        &[
            ("scripts/explore-research-cycle.sh", true),
            ("scripts/explore-research/spec-vs-code.sh", true),
            ("scripts/explore-research/prior-reports.sh", true),
            ("scripts/explore-research/codebase-signals.sh", true),
            ("scripts/explore-research/open-issues.sh", true),
        ],
    );
    if scripts.is_failure() {
        return scripts;
    }
    let bats = run_bats_suites(
        id,
        required,
        root,
        &[
            "tests/explore/test_explore_researchers.bats",
            "tests/explore/test_explore_research_cycle.bats",
        ],
    );
    aggregate(id, required, vec![scripts, bats])
}

fn run_autospec_explore_researchers_llm(id: &str, required: bool, root: &Path) -> CheckResult {
    let scripts = run_required_bash_scripts(
        id,
        required,
        root,
        &[
            ("scripts/lib/explore-internet-safety.sh", false),
            ("scripts/explore-research/source-analysis.sh", true),
            ("scripts/explore-research/internet.sh", true),
        ],
    );
    if scripts.is_failure() {
        return scripts;
    }
    let bats = run_bats_suites(
        id,
        required,
        root,
        &[
            "tests/explore/test_explore_researchers_llm.bats",
            "tests/explore/test_explore_internet_safety.bats",
        ],
    );
    aggregate(id, required, vec![scripts, bats])
}

fn run_autospec_explore_specialists_discovery(
    id: &str,
    required: bool,
    root: &Path,
) -> CheckResult {
    const SCAN: &str = "scripts/explore-specialist-scan.sh";
    const SCHEMA: &str = "schemas/autospec-explore-specialists.schema.json";
    const SUITE: &str = "tests/explore/test_explore_specialists.bats";

    let scripts = run_required_bash_scripts(id, required, root, &[(SCAN, true)]);
    if scripts.is_failure() {
        return scripts;
    }
    if !root.join(SCHEMA).is_file() {
        return aggregate(
            id,
            required,
            vec![
                scripts,
                failure(
                    id,
                    required,
                    "schemas/autospec-explore-specialists.schema.json: specialists roster schema missing",
                ),
            ],
        );
    }

    let mut results = vec![scripts];
    if program_on_path("jq") {
        let check = ToolCommand::new(
            "jq",
            [
                "-e",
                ".properties.domains and .properties.suggested_specialists",
                SCHEMA,
            ],
        )
        .expect("specialist schema jq validation uses static direct arguments")
        .execute_in(id, required, root);
        let failed = check.is_failure();
        results.push(check);
        if failed {
            return aggregate(id, required, results);
        }
    }
    if program_on_path("ajv") {
        let check = ToolCommand::new("ajv", ["compile", "-s", SCHEMA, "--spec=draft2020"])
            .expect("specialist schema ajv validation uses static direct arguments")
            .execute_in(id, required, root);
        let failed = check.is_failure();
        results.push(check);
        if failed {
            return aggregate(id, required, results);
        }
    }
    results.push(run_bats_suites(id, required, root, &[SUITE]));
    aggregate(id, required, results)
}

fn run_autospec_explore_stage2_intersect(id: &str, required: bool, root: &Path) -> CheckResult {
    const PREFILTER: &str = "skills/autospec-shared/scripts/discovery-intersect-prefilter.sh";
    const SCHEMA: &str = "schemas/autospec-explore-proposal.schema.json";
    const TRIO: &[&str] = &[
        "skills/autospec-explore/SKILL.md",
        "skills/autospec-explore/codex/prompt.md",
        "skills/autospec-explore/opencode/agent.md",
    ];

    let prefilter = run_required_bash_scripts(id, required, root, &[(PREFILTER, true)]);
    if prefilter.is_failure() {
        return prefilter;
    }
    if !root.join(SCHEMA).is_file() {
        return aggregate(
            id,
            required,
            vec![
                prefilter,
                failure(
                    id,
                    required,
                    "schemas/autospec-explore-proposal.schema.json: existing explore candidate schema missing",
                ),
            ],
        );
    }
    for trio in TRIO {
        let path = root.join(trio);
        if !path.is_file() {
            return aggregate(
                id,
                required,
                vec![
                    prefilter,
                    failure(
                        id,
                        required,
                        &format!("{trio}: required adapter file missing"),
                    ),
                ],
            );
        }
        if !has_heading_prefix(&path, "## Stage-2 intersect") {
            return aggregate(
                id,
                required,
                vec![
                    prefilter,
                    failure(
                        id,
                        required,
                        &format!("{trio}: missing '## Stage-2 intersect' section"),
                    ),
                ],
            );
        }
        if !contains(&path, "internet-forums") {
            return aggregate(
                id,
                required,
                vec![
                    prefilter,
                    failure(
                        id,
                        required,
                        &format!("{trio}: missing internet-forums Stage-2 discovery source"),
                    ),
                ],
            );
        }
        if !has_same_line_tokens(&path, "internet-forums", "0.4") {
            return aggregate(
                id,
                required,
                vec![
                    prefilter,
                    failure(
                        id,
                        required,
                        &format!("{trio}: internet-forums source must be weighted 0.4"),
                    ),
                ],
            );
        }
        if !contains(&path, "discovery-intersect-prefilter.sh") {
            return aggregate(
                id,
                required,
                vec![
                    prefilter,
                    failure(
                        id,
                        required,
                        &format!("{trio}: Stage-2 must cite discovery-intersect-prefilter.sh"),
                    ),
                ],
            );
        }
        if !contains_case_insensitive(&path, "repo-domain derivation") {
            return aggregate(
                id,
                required,
                vec![
                    prefilter,
                    failure(
                        id,
                        required,
                        &format!("{trio}: Stage-2 must reuse the existing repo-domain derivation"),
                    ),
                ],
            );
        }
        if !contains(&path, "autospec-explore-proposal.schema.json") {
            return aggregate(id, required, vec![prefilter, failure(id, required, &format!("{trio}: Stage-2 must emit the existing explore candidate schema (cite, not redefine)"))]);
        }
        if !contains_case_insensitive(&path, "untrusted") {
            return aggregate(
                id,
                required,
                vec![
                    prefilter,
                    failure(
                        id,
                        required,
                        &format!("{trio}: Stage-2 must state the untrusted-DATA trust boundary"),
                    ),
                ],
            );
        }
    }
    prefilter
}

fn run_explore_trio_worktree_assert(id: &str, required: bool, root: &Path) -> CheckResult {
    const TRIO: &[&str] = &[
        "skills/autospec-explore/SKILL.md",
        "skills/autospec-explore/codex/prompt.md",
        "skills/autospec-explore/opencode/agent.md",
    ];
    const SUITE: &str = "tests/explore/test_explore_worktree_assert.bats";

    let mut sections = Vec::new();
    for trio in TRIO {
        let path = root.join(trio);
        if !path.is_file() {
            return failure(
                id,
                required,
                &format!("{trio}: required adapter file missing"),
            );
        }
        if !contains(&path, "worktree-guard.sh assert") {
            return failure(
                id,
                required,
                &format!("{trio}: missing worktree-guard.sh assert before sandbox commit (D4)"),
            );
        }
        if !contains(&path, "MUST exit 0") {
            return failure(
                id,
                required,
                &format!("{trio}: assert gate must carry MUST-exit-0 wording (D4)"),
            );
        }
        if !contains_case_insensitive(&path, "primary checkout") {
            return failure(
                id,
                required,
                &format!("{trio}: missing primary-checkout hard rule (D4)"),
            );
        }
        let Some(section) = inclusive_section(
            &path,
            "## Sandbox branch contract",
            "## Research cycle contract",
        ) else {
            return failure(
                id,
                required,
                &format!("{trio}: Sandbox branch contract section missing"),
            );
        };
        sections.push(section);
    }
    if sections[0].is_empty() {
        return failure(
            id,
            required,
            "SKILL.md: Sandbox branch contract section missing",
        );
    }
    if sections[0] != sections[1] {
        return failure(
            id,
            required,
            "explore-trio: SKILL.md vs codex/prompt.md Sandbox branch contract differs (lock-step violation)",
        );
    }
    if sections[0] != sections[2] {
        return failure(
            id,
            required,
            "explore-trio: SKILL.md vs opencode/agent.md Sandbox branch contract differs (lock-step violation)",
        );
    }

    run_bats_suites(id, required, root, &[SUITE])
}

fn run_autospec_explore_spec_first(id: &str, required: bool, root: &Path) -> CheckResult {
    const ORCHESTRATOR: &str = "scripts/autospec-explore.sh";
    const SUITES: &[&str] = &[
        "tests/explore/test_explore_spec_first_filing.bats",
        "tests/explore/test_explore_define_fallback.bats",
    ];

    let orchestrator = root.join(ORCHESTRATOR);
    if !orchestrator.is_file() {
        return failure(
            id,
            required,
            "scripts/autospec-explore.sh: orchestrator missing",
        );
    }
    let syntax = ToolCommand::new("bash", ["-n", ORCHESTRATOR])
        .expect("Explore spec-first syntax validation is a direct argument vector")
        .execute_in(id, required, root);
    if syntax.is_failure() {
        return syntax;
    }
    let static_failure = [
        (
            contains(&orchestrator, "gen-explore-round-spec.sh"),
            "scripts/autospec-explore.sh: must invoke scripts/gen-explore-round-spec.sh to render the round spec (#1102)",
        ),
        (
            has_same_line_ordered_tokens(&orchestrator, &["docs/specs/", "explore-", "round-"]),
            "scripts/autospec-explore.sh: must materialize docs/specs/<date>-explore-<slug>-round-<N>-design.md (#1102)",
        ),
        (
            contains(&orchestrator, "git commit"),
            "scripts/autospec-explore.sh: must commit the round spec before filing (#1102)",
        ),
        (
            has_same_line_ordered_tokens(&orchestrator, &["git push ", "origin", "HEAD:$SANDBOX_BRANCH"]),
            "scripts/autospec-explore.sh: must push the round spec to the sandbox branch (HEAD:$SANDBOX_BRANCH) before decompose (#1102)",
        ),
        (
            contains(&orchestrator, "/autospec-define"),
            "scripts/autospec-explore.sh: round filing must decompose via /autospec-define (#1102)",
        ),
        (
            contains(&orchestrator, "--base $SANDBOX_BRANCH"),
            "scripts/autospec-explore.sh: decompose must pass --base $SANDBOX_BRANCH (never main) (#1102)",
        ),
        (
            contains(&orchestrator, "code_health:explore_define_unavailable"),
            "scripts/autospec-explore.sh: missing fallback log token code_health:explore_define_unavailable (#1102)",
        ),
        (
            contains(&orchestrator, "_explore_raw_file_round"),
            "scripts/autospec-explore.sh: must retain raw filing as the never-stall fallback (#1102)",
        ),
    ]
    .into_iter()
    .find_map(|(valid, message)| (!valid).then_some(message));
    if let Some(message) = static_failure {
        return aggregate(id, required, vec![syntax, failure(id, required, message)]);
    }

    let bats = run_bats_suites(id, required, root, SUITES);
    aggregate(id, required, vec![syntax, bats])
}

fn run_autospec_explore_qa_gate(id: &str, required: bool, root: &Path) -> CheckResult {
    const ORCHESTRATOR: &str = "scripts/autospec-explore.sh";
    const RUNNER: &str = "scripts/explore-qa-gate.sh";
    const SUITES: &[&str] = &[
        "tests/explore/test_explore_qa_gate_promotion.bats",
        "tests/explore/test_explore_qa_gate_e2e.bats",
    ];

    let orchestrator = root.join(ORCHESTRATOR);
    if !orchestrator.is_file() {
        return failure(
            id,
            required,
            "scripts/autospec-explore.sh: orchestrator missing",
        );
    }
    let syntax = ToolCommand::new("bash", ["-n", ORCHESTRATOR])
        .expect("Explore QA gate syntax validation is a direct argument vector")
        .execute_in(id, required, root);
    if syntax.is_failure() {
        return syntax;
    }
    let static_failure = [
        (contains(&orchestrator, "--qa-gate)"), "scripts/autospec-explore.sh: must parse --qa-gate (#1114)"),
        (contains(&orchestrator, "--qa-gate-pass-on-partial)"), "scripts/autospec-explore.sh: must parse --qa-gate-pass-on-partial (#1114)"),
        (contains(&orchestrator, "explore-qa-gate.sh"), "scripts/autospec-explore.sh: must invoke scripts/explore-qa-gate.sh (#1114)"),
        (contains(&orchestrator, "To merge sandbox into main"), "scripts/autospec-explore.sh: must retain the merge-instructions promotion block (#1114)"),
        (contains(&orchestrator, "Promotion WITHHELD"), "scripts/autospec-explore.sh: must withhold the merge block on a blocking verdict (#1114)"),
        (contains(&orchestrator, "sandbox QA:"), "scripts/autospec-explore.sh: must annotate the promotion output with the sandbox QA verdict (#1114)"),
        (contains(&orchestrator, "no QA config"), "scripts/autospec-explore.sh: skipped verdict must carry the no-config annotation (#1114)"),
        (contains(&orchestrator, "code_health:explore_qa_gate_failed"), "scripts/autospec-explore.sh: must emit code_health:explore_qa_gate_failed on a blocking verdict (#1114)"),
        (contains(&root.join(RUNNER), "code_health:explore_qa_gate_skipped_no_config"), "scripts/explore-qa-gate.sh: must emit code_health:explore_qa_gate_skipped_no_config (#1113/#1114)"),
        (contains(&orchestrator, "sandbox_head_sha"), "scripts/autospec-explore.sh: must warn when the sandbox advances past the gate sandbox_head_sha (#1114)"),
        (has_same_line_ordered_tokens(&orchestrator, &["QA_GATE", "-eq 0"]), "scripts/autospec-explore.sh: must guard the default-off byte-unchanged promotion path (#1114)"),
        (contains(&orchestrator, "qa_gate_verdict"), "scripts/autospec-explore.sh: must record the gate verdict in .autospec/explore-loop.json (#1114)"),
    ]
    .into_iter()
    .find_map(|(valid, message)| (!valid).then_some(message));
    if let Some(message) = static_failure {
        return aggregate(id, required, vec![syntax, failure(id, required, message)]);
    }

    let bats = run_bats_suites(id, required, root, SUITES);
    aggregate(id, required, vec![syntax, bats])
}

fn run_autospec_explore_style_normalization(id: &str, required: bool, root: &Path) -> CheckResult {
    const RESEARCHER: &str = "scripts/explore-research/style-normalization.sh";
    const TRIO: &[&str] = &[
        "skills/autospec-explore/SKILL.md",
        "skills/autospec-explore/codex/prompt.md",
        "skills/autospec-explore/opencode/agent.md",
    ];
    const SUITE: &str = "tests/explore/test_explore_style_normalization.bats";

    let researcher = run_required_bash_scripts(id, required, root, &[(RESEARCHER, true)]);
    if researcher.is_failure() {
        return researcher;
    }
    let static_checks = [
        (
            RESEARCHER,
            "AUTOSPEC_EXPLORE_STYLE_PROOF_CMD",
            "missing AUTOSPEC_EXPLORE_STYLE_PROOF_CMD invocation seam",
        ),
        (
            RESEARCHER,
            "AUTOSPEC_STYLE_PROOF_DIR",
            "missing AUTOSPEC_STYLE_PROOF_DIR artifact directory contract",
        ),
        (
            RESEARCHER,
            "best-effort",
            "generic dispatcher fallback must be documented as best-effort",
        ),
        (
            "scripts/autospec-explore.sh",
            "style-normalization",
            "default RESEARCH_SOURCES missing style-normalization",
        ),
        (
            "scripts/explore-research-cycle.sh",
            "style-normalization",
            "default source roster/weights missing style-normalization",
        ),
        (
            "skills/autospec-shared/scripts/explore-source-weights.sh",
            "style-normalization",
            "canonical priors missing style-normalization",
        ),
    ];
    for (path, token, message) in static_checks {
        if !contains(&root.join(path), token) {
            return aggregate(
                id,
                required,
                vec![
                    researcher,
                    failure(id, required, &format!("{path}: {message}")),
                ],
            );
        }
    }
    for trio in TRIO {
        let path = root.join(trio);
        if !path.is_file() {
            return aggregate(
                id,
                required,
                vec![
                    researcher,
                    failure(
                        id,
                        required,
                        &format!("{trio}: required adapter file missing"),
                    ),
                ],
            );
        }
        for (token, message) in [
            (
                "style-normalization",
                "missing style-normalization discovery researcher",
            ),
            (
                "Playwright",
                "missing Playwright auto-generation requirement",
            ),
            ("screenshot", "missing screenshot evidence requirement"),
            (
                "AUTOSPEC_EXPLORE_STYLE_PROOF_CMD",
                "missing AUTOSPEC_EXPLORE_STYLE_PROOF_CMD proof seam",
            ),
            (
                "best-effort fallback",
                "missing best-effort fallback boundary",
            ),
        ] {
            if !contains(&path, token) {
                return aggregate(
                    id,
                    required,
                    vec![
                        researcher,
                        failure(id, required, &format!("{trio}: {message}")),
                    ],
                );
            }
        }
    }
    let bats = run_bats_suites(id, required, root, &[SUITE]);
    aggregate(id, required, vec![researcher, bats])
}

fn run_autospec_explore_orchestrator(id: &str, required: bool, root: &Path) -> CheckResult {
    const ORCHESTRATOR: &str = "scripts/autospec-explore.sh";
    const REMAINING_SCRIPTS: &[(&str, bool)] = &[
        ("scripts/explore-research/spec-vs-code.sh", true),
        ("scripts/explore-research/prior-reports.sh", true),
        ("scripts/explore-research/codebase-signals.sh", true),
        ("scripts/explore-research/open-issues.sh", true),
        ("scripts/explore-research/source-analysis.sh", true),
        ("scripts/explore-research/internet.sh", true),
        ("scripts/explore-sandbox.sh", true),
        ("scripts/explore-research-cycle.sh", true),
        (ORCHESTRATOR, true),
    ];
    const TRIO: &[&str] = &[
        "skills/autospec-explore/SKILL.md",
        "skills/autospec-explore/codex/prompt.md",
        "skills/autospec-explore/opencode/agent.md",
    ];
    const SUITE: &str = "tests/explore/test_explore_e2e.bats";

    let initial_syntax = run_required_bash_scripts(id, required, root, &[(ORCHESTRATOR, true)]);
    if initial_syntax.is_failure() {
        return initial_syntax;
    }

    let orchestrator = root.join(ORCHESTRATOR);
    for (token, message) in [
        (
            "lib/autospec-loop.sh",
            "must source scripts/lib/autospec-loop.sh",
        ),
        (
            "lib/autospec-harness-detect.sh",
            "must source scripts/lib/autospec-harness-detect.sh",
        ),
        (
            "explore-sandbox.sh",
            "must invoke scripts/explore-sandbox.sh",
        ),
        (
            "explore-research-cycle.sh",
            "must invoke scripts/explore-research-cycle.sh",
        ),
        (
            "explore-stop.flag",
            "must check ~/.autospec/explore-stop.flag operator escape",
        ),
        (
            "explore-summary.md",
            "must write .autospec/explore-summary.md",
        ),
        (
            "explore-loop.json",
            "must write .autospec/explore-loop.json",
        ),
    ] {
        if !contains(&orchestrator, token) {
            return aggregate(
                id,
                required,
                vec![
                    initial_syntax,
                    failure(id, required, &format!("{ORCHESTRATOR}: {message}")),
                ],
            );
        }
    }

    let remaining_scripts = run_required_bash_scripts(id, required, root, REMAINING_SCRIPTS);
    if remaining_scripts.is_failure() {
        return aggregate(id, required, vec![initial_syntax, remaining_scripts]);
    }

    for trio in TRIO {
        let path = root.join(trio);
        if !path.is_file() {
            return aggregate(
                id,
                required,
                vec![
                    initial_syntax,
                    remaining_scripts,
                    failure(
                        id,
                        required,
                        &format!("{trio}: required adapter file missing"),
                    ),
                ],
            );
        }
        if !contains(&path, "lib/autospec-loop.sh") {
            return aggregate(
                id,
                required,
                vec![
                    initial_syntax,
                    remaining_scripts,
                    failure(
                        id,
                        required,
                        &format!("{trio}: adapter must reference scripts/lib/autospec-loop.sh"),
                    ),
                ],
            );
        }
    }

    let bats = run_bats_suites(id, required, root, &[SUITE]);
    aggregate(id, required, vec![initial_syntax, remaining_scripts, bats])
}

fn run_required_bash_scripts(
    id: &str,
    required: bool,
    root: &Path,
    scripts: &[(&str, bool)],
) -> CheckResult {
    let mut results = Vec::new();
    for (script, executable) in scripts {
        let path = root.join(script);
        if !path.is_file() {
            results.push(failure(
                id,
                required,
                &format!("{script}: required file missing"),
            ));
            return aggregate(id, required, results);
        }
        if *executable && !is_executable(&path) {
            results.push(failure(
                id,
                required,
                &format!("{script}: file not executable"),
            ));
            return aggregate(id, required, results);
        }
        let syntax = ToolCommand::new("bash", ["-n", *script])
            .expect("required Bash validation uses a direct argument vector")
            .execute_in(id, required, root);
        let failed = syntax.is_failure();
        results.push(syntax);
        if failed {
            return aggregate(id, required, results);
        }
    }
    aggregate(id, required, results)
}

fn run_autospec_sweep_area_contract(id: &str, required: bool, root: &Path) -> CheckResult {
    const AREAS_DIRECTORY: &str = "skills/autospec-sweep/areas";
    const DISPATCHER: &str = "scripts/sweep-area-dispatch.sh";
    const DEPENDENCY_RESEARCHER: &str = "scripts/explore-research/dependency-health.sh";
    const SUITE: &str = "tests/sweep/test_sweep_area_dispatch.bats";

    if !root.join(AREAS_DIRECTORY).is_dir() {
        return failure(
            id,
            required,
            "skills/autospec-sweep/areas: required directory missing",
        );
    }
    for area in [
        "spec-vs-code-drift",
        "docs-drift",
        "code-health",
        "dependency-health",
    ] {
        let relative = format!("{AREAS_DIRECTORY}/{area}.md");
        if !root.join(&relative).is_file() {
            return failure(
                id,
                required,
                &format!("{relative}: required area file missing"),
            );
        }
    }

    let mut results = Vec::new();
    for (script, missing_message) in [
        (DISPATCHER, "required dispatcher missing"),
        (DEPENDENCY_RESEARCHER, "required researcher missing"),
    ] {
        if !root.join(script).is_file() {
            results.push(failure(
                id,
                required,
                &format!("{script}: {missing_message}"),
            ));
            return aggregate(id, required, results);
        }

        let syntax = ToolCommand::new("bash", ["-n", script])
            .expect("sweep-area shell syntax validation is a direct argument vector")
            .execute_in(id, required, root);
        let failed = syntax.is_failure();
        results.push(syntax);
        if failed {
            return aggregate(id, required, results);
        }
    }

    for adapter in [
        "skills/autospec-sweep/SKILL.md",
        "skills/autospec-sweep/codex/prompt.md",
        "skills/autospec-sweep/opencode/agent.md",
    ] {
        let path = root.join(adapter);
        if !has_line_prefix(&path, "## Parallel area dispatch") {
            results.push(failure(
                id,
                required,
                &format!("{adapter}: missing '## Parallel area dispatch' section"),
            ));
            return aggregate(id, required, results);
        }
        for area in [
            "spec-vs-code-drift",
            "docs-drift",
            "code-health",
            "dependency-health",
        ] {
            if !contains(&path, area) {
                results.push(failure(
                    id,
                    required,
                    &format!("{adapter}: missing area reference '{area}'"),
                ));
                return aggregate(id, required, results);
            }
        }
        if !contains(&path, "sweep-area-dispatch.sh") {
            results.push(failure(
                id,
                required,
                &format!("{adapter}: missing reference to scripts/sweep-area-dispatch.sh"),
            ));
            return aggregate(id, required, results);
        }
    }

    results.push(run_bats_suite(id, required, root, SUITE));
    aggregate(id, required, results)
}

fn run_autospec_qa_cluster_contract(id: &str, required: bool, root: &Path) -> CheckResult {
    const CLUSTERS_DIRECTORY: &str = "skills/autospec-qa/clusters";
    const DISPATCHER: &str = "scripts/qa-cluster-dispatch.sh";
    const TRIO: &[&str] = &[
        "skills/autospec-qa/SKILL.md",
        "skills/autospec-qa/codex/prompt.md",
        "skills/autospec-qa/opencode/agent.md",
    ];
    const SUITE: &str = "tests/qa/test_qa_cluster_dispatch.bats";

    if !root.join(CLUSTERS_DIRECTORY).is_dir() {
        return failure(
            id,
            required,
            "skills/autospec-qa/clusters: missing cluster directory (issue #730)",
        );
    }
    for cluster in [
        "spec-traceability",
        "functional-coverage",
        "backend-integration",
        "reliability-contract",
        "legacy-and-cleanup",
        "benchmark-and-outsourcing",
        "accessibility-and-responsive",
        "production-incidents",
    ] {
        let path = format!("{CLUSTERS_DIRECTORY}/{cluster}.md");
        if !root.join(&path).is_file() {
            return failure(
                id,
                required,
                &format!("{path}: missing cluster file (issue #730)"),
            );
        }
    }

    let dispatcher = root.join(DISPATCHER);
    if !dispatcher.is_file() {
        return failure(
            id,
            required,
            "scripts/qa-cluster-dispatch.sh: missing (issue #730)",
        );
    }
    if !is_executable(&dispatcher) {
        return failure(
            id,
            required,
            "scripts/qa-cluster-dispatch.sh: not executable (issue #730)",
        );
    }
    for trio in TRIO {
        let path = root.join(trio);
        if !contains(&path, "qa-cluster-dispatch.sh") {
            return failure(
                id,
                required,
                &format!("{trio}: missing reference to qa-cluster-dispatch.sh (issue #730)"),
            );
        }
        if !contains(&path, "skills/autospec-qa/clusters/") {
            return failure(
                id,
                required,
                &format!("{trio}: missing reference to clusters/ directory (issue #730)"),
            );
        }
    }
    if !root.join(SUITE).is_file() {
        return failure(
            id,
            required,
            "tests/qa/test_qa_cluster_dispatch.bats: missing (issue #730)",
        );
    }

    ToolCommand::new("bash", ["-n", DISPATCHER])
        .expect("QA cluster dispatcher syntax validation is a direct argument vector")
        .execute_in(id, required, root)
}

fn run_autospec_qa_bug_class_contract(id: &str, required: bool, root: &Path) -> CheckResult {
    const HELPER: &str = "scripts/qa-bug-class-sweep.sh";
    const HEAL_LOOP: &str = "scripts/qa-heal-loop.sh";
    const TRIO: &[&str] = &[
        "skills/autospec-qa/SKILL.md",
        "skills/autospec-qa/codex/prompt.md",
        "skills/autospec-qa/opencode/agent.md",
    ];
    const SUITE: &str = "tests/qa/test_qa_bug_class_sweep.bats";

    let helper = root.join(HELPER);
    if !helper.is_file() {
        return failure(
            id,
            required,
            "scripts/qa-bug-class-sweep.sh: missing (issue #737)",
        );
    }
    if !is_executable(&helper) {
        return failure(
            id,
            required,
            "scripts/qa-bug-class-sweep.sh: not executable (issue #737)",
        );
    }
    for trio in TRIO {
        let path = root.join(trio);
        for (token, description) in [
            (
                "qa-bug-class-sweep.sh",
                "reference to qa-bug-class-sweep.sh",
            ),
            ("parent_fingerprint", "parent_fingerprint contract"),
            (
                "AUTOSPEC_QA_BUG_CLASS_MIN_CONFIDENCE",
                "confidence-threshold env var",
            ),
        ] {
            if !contains(&path, token) {
                return failure(
                    id,
                    required,
                    &format!("{trio}: missing {description} (issue #737)"),
                );
            }
        }
    }
    if !contains(&root.join(HEAL_LOOP), "parent_fingerprint") {
        return failure(
            id,
            required,
            "scripts/qa-heal-loop.sh: must group by parent_fingerprint (issue #737)",
        );
    }
    if !root.join(SUITE).is_file() {
        return failure(
            id,
            required,
            "tests/qa/test_qa_bug_class_sweep.bats: missing (issue #737)",
        );
    }

    ToolCommand::new("bash", ["-n", HELPER])
        .expect("QA bug-class helper syntax validation is a direct argument vector")
        .execute_in(id, required, root)
}

fn run_loop_handoff_harness_awareness(id: &str, required: bool, root: &Path) -> CheckResult {
    const HELPER: &str = "scripts/lib/autospec-harness-detect.sh";
    const CONSUMERS: &[&str] = &[
        "scripts/refine-prompt.sh",
        "scripts/autospec-continue.sh",
        "scripts/qa-heal-loop.sh",
        "scripts/refine-prompt-lens-llm.sh",
    ];
    const SUITES: &[&str] = &[
        "tests/lib/test_autospec_harness_detect.bats",
        "tests/refine/test_refine_loop_codex.bats",
    ];

    let helper = root.join(HELPER);
    if !helper.is_file() {
        return failure(
            id,
            required,
            "scripts/lib/autospec-harness-detect.sh: missing — required for harness-aware loop handoff",
        );
    }
    for consumer in CONSUMERS {
        let path = root.join(consumer);
        if !path.is_file() {
            return failure(id, required, &format!("{consumer}: missing"));
        }
        if !contains(&path, "lib/autospec-harness-detect.sh") {
            return failure(
                id,
                required,
                &format!(
                    "{consumer}: must source scripts/lib/autospec-harness-detect.sh (issue #723)"
                ),
            );
        }
    }
    for suite in SUITES {
        if !root.join(suite).is_file() {
            return failure(id, required, &format!("{suite}: missing (issue #723)"));
        }
    }

    ToolCommand::new("bash", ["-n", HELPER])
        .expect("harness detector syntax validation is a direct argument vector")
        .execute_in(id, required, root)
}

fn run_skill_validator(id: &str, required: bool, root: &Path, skill: &str) -> CheckResult {
    let skill_directory = format!("skills/{skill}");
    let validator = format!("{skill_directory}/validate.sh");
    if !root.join(&skill_directory).is_dir() {
        return failure(
            id,
            required,
            &format!("{skill_directory}: directory missing"),
        );
    }
    for member in ["SKILL.md", "codex/prompt.md"] {
        let relative = format!("{skill_directory}/{member}");
        if !root.join(&relative).is_file() {
            return failure(id, required, &format!("{relative}: required file missing"));
        }
    }
    if !root.join(&validator).is_file() {
        return failure(
            id,
            required,
            &format!("{validator}: per-skill validate.sh missing"),
        );
    }

    let commands = [
        ToolCommand::new("bash", ["-n", validator.as_str()])
            .expect("per-skill syntax validation is a direct argument vector"),
        ToolCommand::new("bash", [validator.as_str()])
            .expect("per-skill validator execution is a direct argument vector"),
    ];
    run_commands(id, required, root, commands)
}

fn run_autospec_playwright_skill(id: &str, required: bool, root: &Path) -> CheckResult {
    let validator = run_skill_validator(id, required, root, "autospec-playwright");
    if validator.is_failure() {
        return validator;
    }

    for member in ["SKILL.md", "codex/prompt.md", "opencode/agent.md"] {
        let relative = format!("skills/autospec-playwright/{member}");
        let path = root.join(&relative);
        if !path.is_file() {
            return aggregate(
                id,
                required,
                vec![
                    validator,
                    failure(
                        id,
                        required,
                        &format!("{relative}: required Playwright adapter body missing"),
                    ),
                ],
            );
        }

        let document = fs::read_to_string(&path).unwrap_or_default();
        for anchor in [
            "## Loaded-data dashboard evidence",
            "at least one loaded-data assertion",
            "/loading|skeleton|empty/i",
            "after catch-all mocks",
        ] {
            if !document.contains(anchor) {
                return aggregate(
                    id,
                    required,
                    vec![
                        validator,
                        failure(
                            id,
                            required,
                            &format!("{relative}: missing Playwright evidence anchor `{anchor}`"),
                        ),
                    ],
                );
            }
        }
    }
    validator
}

fn run_autospec_fab_contract(id: &str, required: bool, root: &Path) -> CheckResult {
    const SKILL: &str = "skills/autospec-fab";
    for member in ["SKILL.md", "codex/prompt.md", "opencode/agent.md"] {
        let relative = format!("{SKILL}/{member}");
        let path = root.join(&relative);
        if !path.is_file() {
            return failure(
                id,
                required,
                &format!("{relative}: required trio file missing (issue #1218)"),
            );
        }

        let document = fs::read_to_string(&path).unwrap_or_default();
        for token in [
            "release-gate",
            "watertight",
            "gasket",
            "vacuum",
            "FreeCAD",
            "STL",
        ] {
            if !document.contains(token) {
                return failure(
                    id,
                    required,
                    &format!("{relative}: missing fab token `{token}` (issue #1218)"),
                );
            }
        }
    }

    let bats = run_bats_directory(id, required, root, "skills/autospec-fab/tests");
    if bats.is_failure() {
        return bats;
    }
    aggregate(
        id,
        required,
        vec![bats, run_fab_container_dockerfile(id, required, root)],
    )
}

fn run_grooming_contract(id: &str, required: bool, root: &Path) -> CheckResult {
    const SUITES: &[&str] = &[
        "tests/autospec/list-groomable.bats",
        "tests/autospec/promote-eligibility.bats",
        "tests/autospec/groom-validate.bats",
        "tests/autospec/groom-fill.bats",
        "tests/autospec/apply-safety-review.bats",
        "tests/autospec/groom-reconcile.bats",
        "tests/autospec/autonomous-promote-open-issues.bats",
        "tests/autospec/test_loop_grooming.bats",
        "skills/autospec-shared/tests/grooming-config.bats",
        "skills/autospec-shared/tests/unit/grooming-govern.bats",
        "skills/autospec-shared/tests/unit/grooming-observe.bats",
    ];
    run_bats_suites(id, required, root, SUITES)
}

fn run_mutation_and_negative_path(id: &str, required: bool, root: &Path) -> CheckResult {
    let mut commands = Vec::new();
    const NEGATIVE_PATH_LINT: &str = "scripts/check-negative-path-pairs.sh";
    if root.join(NEGATIVE_PATH_LINT).is_file() {
        commands.push(
            ToolCommand::new("bash", [NEGATIVE_PATH_LINT])
                .expect("negative-path lint execution is a direct argument vector"),
        );
    }
    if program_on_path("bats") {
        for suite in [
            "tests/run-mutation-test.bats",
            "tests/mutation-adapters.bats",
            "tests/bash-mutate.bats",
        ] {
            if root.join(suite).is_file() {
                commands.push(
                    ToolCommand::new("bats", [suite])
                        .expect("mutation Bats execution is a direct argument vector"),
                );
            }
        }
    }
    run_commands(id, required, root, commands)
}

fn run_lint_implementation_helpers(id: &str, required: bool, root: &Path) -> CheckResult {
    const SCRIPT: &str = "scripts/lint-implementation.sh";
    let path = root.join(SCRIPT);
    if !path.is_file() {
        return failure(id, required, "scripts/lint-implementation.sh: file missing");
    }

    let syntax = ToolCommand::new("bash", ["-n", SCRIPT])
        .expect("lint-implementation syntax check is a direct argument vector")
        .execute_in(id, required, root);
    if syntax.is_failure() {
        return syntax;
    }

    let help = ToolCommand::new("bash", [SCRIPT, "--help"])
        .expect("lint-implementation help is a direct argument vector")
        .execute_in_capturing(id, required, root);
    if help.result.is_failure() {
        return aggregate(id, required, vec![syntax, help.result]);
    }

    let output = String::from_utf8_lossy(&help.stdout);
    for rule_id in [
        "OUT_OF_SCOPE",
        "MISSING_TEST",
        "COMPLEXITY",
        "SECURITY",
        "TODO_LEFT",
        "MOCK_DB",
        "DOC_OUT_OF_SYNC",
        "HALLUCINATED_API",
        "DUPLICATE_CODE",
        "INVENTED_CONFIG",
    ] {
        if !output.contains(rule_id) {
            return aggregate(
                id,
                required,
                vec![
                    syntax,
                    help.result,
                    failure(
                        id,
                        required,
                        &format!(
                            "scripts/lint-implementation.sh --help missing RULE_ID: {rule_id}"
                        ),
                    ),
                ],
            );
        }
    }
    aggregate(id, required, vec![syntax, help.result])
}

fn run_lint_issue_helpers(id: &str, required: bool, root: &Path) -> CheckResult {
    const SCRIPT: &str = "scripts/lint-issue.sh";
    const FIXTURES: &str = "tests/fixtures/issue-quality";
    let path = root.join(SCRIPT);
    if !path.is_file() {
        return failure(id, required, "scripts/lint-issue.sh: file missing");
    }

    let syntax = ToolCommand::new("bash", ["-n", SCRIPT])
        .expect("lint-issue syntax check is a direct argument vector")
        .execute_in(id, required, root);
    if syntax.is_failure() {
        return syntax;
    }
    let help = ToolCommand::new("bash", [SCRIPT, "--help"])
        .expect("lint-issue help is a direct argument vector")
        .execute_in_capturing(id, required, root);
    if help.result.is_failure() {
        return aggregate(id, required, vec![syntax, help.result]);
    }
    if !has_usage_line(&help.stdout) {
        return aggregate(
            id,
            required,
            vec![
                syntax,
                help.result,
                failure(
                    id,
                    required,
                    "scripts/lint-issue.sh --help did not print a 'Usage:' line",
                ),
            ],
        );
    }

    let fixture_directory = root.join(FIXTURES);
    if !fixture_directory.is_dir() {
        return aggregate(
            id,
            required,
            vec![
                syntax,
                help.result,
                failure(
                    id,
                    required,
                    "tests/fixtures/issue-quality/: directory missing",
                ),
            ],
        );
    }
    if !fixture_directory.join("good.md").is_file() {
        return aggregate(
            id,
            required,
            vec![
                syntax,
                help.result,
                failure(
                    id,
                    required,
                    "tests/fixtures/issue-quality/good.md: fixture missing",
                ),
            ],
        );
    }
    let bad_count = fs::read_dir(&fixture_directory)
        .into_iter()
        .flat_map(|entries| entries.filter_map(Result::ok))
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with("bad-") && name.ends_with(".md"))
        })
        .count();
    if bad_count < 4 {
        return aggregate(
            id,
            required,
            vec![
                syntax,
                help.result,
                failure(
                    id,
                    required,
                    &format!(
                        "tests/fixtures/issue-quality/: need >=4 bad-*.md fixtures, found {bad_count}"
                    ),
                ),
            ],
        );
    }

    let bats = run_bats_suites_if_available(
        id,
        required,
        root,
        &["tests/unit/test_lint_issue_safety.bats"],
    );
    aggregate(id, required, vec![syntax, help.result, bats])
}

fn run_phase4_ci_status_compare(id: &str, required: bool, root: &Path) -> CheckResult {
    const HELPER: &str = "scripts/ci-status-compare.sh";
    const SMOKE_TEST: &str = "tests/smoke/ci-status-compare.sh";
    if !is_executable(&root.join(HELPER)) {
        return failure(
            id,
            required,
            "scripts/ci-status-compare.sh: required executable missing",
        );
    }
    if !root.join(SMOKE_TEST).is_file() {
        return failure(
            id,
            required,
            "tests/smoke/ci-status-compare.sh: required smoke test missing",
        );
    }
    let commands = [
        ToolCommand::new("bash", ["-n", HELPER])
            .expect("CI status helper syntax check is a direct argument vector"),
        ToolCommand::new("bash", ["-n", SMOKE_TEST])
            .expect("CI status smoke syntax check is a direct argument vector"),
        ToolCommand::new("bash", [SMOKE_TEST])
            .expect("CI status smoke execution is a direct argument vector"),
    ];
    run_commands(id, required, root, commands)
}

fn run_define_spec_worktree_routing(id: &str, required: bool, root: &Path) -> CheckResult {
    const ADAPTERS: &[&str] = &[
        "skills/autospec-define/SKILL.md",
        "skills/autospec-define/codex/prompt.md",
        "skills/autospec-define/opencode/agent.md",
    ];
    const ANCHORS: &[&str] = &[
        "worktree-guard.sh",
        "/tmp/wt-spec-",
        "MUST exit 0 before any edit",
        "resolve-branch",
        "define-spec-pr:begin",
        "define-spec-pr:end",
        "git worktree remove",
        "git worktree prune",
    ];
    for adapter in ADAPTERS {
        let path = root.join(adapter);
        if !path.is_file() {
            return failure(id, required, &format!("{adapter}: required file missing"));
        }
        let document = fs::read_to_string(&path).unwrap_or_default();
        for anchor in ANCHORS {
            if !document.contains(anchor) {
                return failure(
                    id,
                    required,
                    &format!("{adapter}: missing worktree-routing anchor `{anchor}`"),
                );
            }
        }
    }

    const SMOKE_TEST: &str = "tests/phase4/test_define_spec_worktree.sh";
    if !root.join(SMOKE_TEST).is_file() {
        return failure(
            id,
            required,
            "tests/phase4/test_define_spec_worktree.sh: required smoke test missing",
        );
    }
    ToolCommand::new("bash", [SMOKE_TEST])
        .expect("define-spec worktree smoke test is a direct argument vector")
        .execute_in(id, required, root)
}

fn run_groom_preflight_contract(id: &str, required: bool, root: &Path) -> CheckResult {
    const HELPER: &str = "skills/autospec-run/scripts/run-groom-preflight.sh";
    const SUITE: &str = "tests/unit/test_run_groom_preflight.bats";
    let helper = root.join(HELPER);
    if !helper.is_file() {
        return failure(
            id,
            required,
            "skills/autospec-run/scripts/run-groom-preflight.sh: missing",
        );
    }
    if !root.join(SUITE).is_file() {
        return failure(
            id,
            required,
            "tests/unit/test_run_groom_preflight.bats: missing",
        );
    }
    let skill = root.join("skills/autospec-run/SKILL.md");
    let document = fs::read_to_string(&skill).unwrap_or_default();
    for anchor in ["Backlog grooming preflight", "double gate", "no discovery"] {
        if !document.contains(anchor) {
            return failure(
                id,
                required,
                &format!("skills/autospec-run/SKILL.md: missing {anchor} prose"),
            );
        }
    }

    let syntax = ToolCommand::new("bash", ["-n", HELPER])
        .expect("groom preflight syntax check is a direct argument vector")
        .execute_in(id, required, root);
    if syntax.is_failure() {
        return syntax;
    }
    let bats = run_bats_suites_if_available(id, required, root, &[SUITE]);
    aggregate(id, required, vec![syntax, bats])
}

fn run_grow_run_contract(id: &str, required: bool, root: &Path) -> CheckResult {
    const SKILL: &str = "skills/autospec-grow-run/SKILL.md";
    let path = root.join(SKILL);
    if !path.is_file() {
        return failure(id, required, "skills/autospec-grow-run/SKILL.md: missing");
    }
    let document = fs::read_to_string(&path).unwrap_or_default();
    for anchor in [
        "**Model tier:**",
        "## Self-update mode",
        "## R1 — Artifact drain",
        "## R2 — Outbound",
        "## R4 — Measure",
    ] {
        if !document.contains(anchor) {
            return failure(
                id,
                required,
                &format!("skills/autospec-grow-run/SKILL.md: missing {anchor}"),
            );
        }
    }
    run_bats_suites_if_available(id, required, root, &["tests/autospec-grow-run/smoke.bats"])
}

fn run_performance_workstream(id: &str, required: bool, root: &Path) -> CheckResult {
    const CONTRACT: WorkstreamContract = WorkstreamContract {
        helper: "scripts/performance-workstream.sh",
        runbook: "docs/runbooks/performance-workstream.md",
        bats_file: "tests/autonomous/test_performance_workstream.bats",
        helper_anchors: &[
            "record-benchmark",
            "propose-regression-issue",
            "optimization-report",
            "fast-path-guard",
        ],
        runbook_anchors: &["<50ms"],
        workflow: None,
    };
    run_workstream_contract(id, required, root, &CONTRACT)
}

fn run_ux_ui_workstream(id: &str, required: bool, root: &Path) -> CheckResult {
    const CONTRACT: WorkstreamContract = WorkstreamContract {
        helper: "scripts/ux-ui-workstream.sh",
        runbook: "docs/runbooks/ux-ui-workstream.md",
        bats_file: "tests/autonomous/test_ux_ui_workstream.bats",
        helper_anchors: &[
            "record-snapshot",
            "propose-regression-issue",
            "improvement-report",
            "validate-design-doc",
        ],
        runbook_anchors: &[
            "LCP <= 2.5s",
            "INP <= 200ms",
            "CLS <= 0.1",
            "web.dev Web Vitals",
            "Lighthouse CI",
            "Nielsen Norman Group heuristics",
            "HEART",
            "light",
            "dark",
        ],
        workflow: Some((
            ".github/workflows/ux-ui-workstream.yml",
            "ux-ui-workstream.sh gate",
        )),
    };
    run_workstream_contract(id, required, root, &CONTRACT)
}

struct WorkstreamContract {
    helper: &'static str,
    runbook: &'static str,
    bats_file: &'static str,
    helper_anchors: &'static [&'static str],
    runbook_anchors: &'static [&'static str],
    workflow: Option<(&'static str, &'static str)>,
}

fn run_workstream_contract(
    id: &str,
    required: bool,
    root: &Path,
    contract: &WorkstreamContract,
) -> CheckResult {
    let WorkstreamContract {
        helper,
        runbook,
        bats_file,
        helper_anchors,
        runbook_anchors,
        workflow,
    } = contract;
    let helper_path = root.join(helper);
    if !is_executable(&helper_path) {
        return failure(
            id,
            required,
            &format!("{helper}: missing or not executable"),
        );
    }
    let helper_document = fs::read_to_string(&helper_path).unwrap_or_default();
    for anchor in *helper_anchors {
        if !helper_document.contains(anchor) {
            return failure(
                id,
                required,
                &format!("{helper}: missing `{anchor}` command"),
            );
        }
    }
    let runbook_path = root.join(runbook);
    if !runbook_path.is_file() {
        return failure(id, required, &format!("{runbook}: missing runbook"));
    }
    let runbook_document = fs::read_to_string(&runbook_path).unwrap_or_default();
    for anchor in *runbook_anchors {
        if !runbook_document.contains(anchor) {
            return failure(
                id,
                required,
                &format!("{runbook}: missing `{anchor}` guidance"),
            );
        }
    }
    if !root.join(bats_file).is_file() {
        return failure(id, required, &format!("{bats_file}: missing bats coverage"));
    }
    if let Some((workflow, anchor)) = workflow {
        let workflow_path = root.join(workflow);
        if !workflow_path.is_file() {
            return failure(id, required, &format!("{workflow}: missing PR workflow"));
        }
        if !contains(&workflow_path, anchor) {
            return failure(
                id,
                required,
                &format!("{workflow}: missing deterministic gate invocation"),
            );
        }
    }
    ToolCommand::new("bash", ["-n", helper])
        .expect("workstream syntax check is a direct argument vector")
        .execute_in(id, required, root)
}

fn run_token_baseline_fresh(id: &str, required: bool, root: &Path) -> CheckResult {
    const BASELINE: &str = "docs/reports/skill-token-baseline.md";
    const REPORT: &str = "scripts/skill-token-report.sh";
    let baseline = root.join(BASELINE);
    if !baseline.is_file() || !root.join(REPORT).is_file() {
        return CheckResult::completed(id, required, 0, 0, 0, 0, 0, output_digest(&[], &[]));
    }

    let captured = ToolCommand::new("bash", [REPORT])
        .expect("token baseline report is a direct argument vector")
        .execute_in_capturing(id, required, root);
    let _stale = captured.result.is_success()
        && String::from_utf8_lossy(&captured.stdout) != baseline_table(&baseline);

    CheckResult::completed(
        id,
        required,
        0,
        captured.result.elapsed_ms,
        captured.result.spawn_count,
        captured.result.stdout_bytes,
        captured.result.stderr_bytes,
        captured.result.output_digest,
    )
}

fn run_architecture_fitness_engine(id: &str, required: bool, root: &Path) -> CheckResult {
    const RUNNER: &str = "scripts/architecture-fitness.sh";
    const REGISTRY: &str = ".autospec/architecture-fitness.yml";
    if !root.join(RUNNER).is_file() {
        return failure(
            id,
            required,
            "scripts/architecture-fitness.sh: required architecture fitness runner missing (issue #1533)",
        );
    }
    let registry = root.join(REGISTRY);
    if !registry.is_file() {
        return failure(
            id,
            required,
            ".autospec/architecture-fitness.yml: required architecture fitness registry missing (issue #1533)",
        );
    }
    let document = fs::read_to_string(&registry).unwrap_or_default();
    for anchor in ["fitness_functions:", "threshold:", "gate: true"] {
        if !document.contains(anchor) {
            return failure(
                id,
                required,
                &format!(".autospec/architecture-fitness.yml: missing `{anchor}`"),
            );
        }
    }

    let checks = [
        ToolCommand::new("bash", ["-n", RUNNER])
            .expect("architecture fitness syntax check is a direct argument vector"),
        ToolCommand::new("bash", [RUNNER, "run", "--registry", REGISTRY])
            .expect("architecture fitness execution is a direct argument vector"),
    ];
    let checks = run_commands(id, required, root, checks);
    if checks.is_failure() {
        return checks;
    }
    let bats = run_bats_directory_allow_empty_if_available(
        id,
        required,
        root,
        "tests/architecture-fitness",
    );
    aggregate(id, required, vec![checks, bats])
}

fn run_phase4_test_suites(id: &str, required: bool, root: &Path) -> CheckResult {
    let phase4 = run_bash_directory(id, required, root, "tests/phase4");
    if phase4.is_failure() {
        return phase4;
    }
    let documentation = run_bash_directory(id, required, root, "tests/docs");
    if documentation.is_failure() {
        return aggregate(id, required, vec![phase4, documentation]);
    }

    const FLEET_SUITE: &str = "tests/e2e/test_autospec_fleet_dry_run.bats";
    let fleet = if root.join(FLEET_SUITE).is_file() && root.join("tests/fixtures/fleet").is_dir() {
        if program_on_path("bats") {
            ToolCommand::new("bats", [FLEET_SUITE])
                .expect("fleet dry-run Bats suite is a direct argument vector")
                .with_env("AUTOSPEC_FLEET_LIVE_E2E", "0")
                .execute_in(id, required, root)
        } else {
            CheckResult::completed(id, required, 0, 0, 0, 0, 0, output_digest(&[], &[]))
        }
    } else {
        CheckResult::completed(id, required, 0, 0, 0, 0, 0, output_digest(&[], &[]))
    };
    aggregate(id, required, vec![phase4, documentation, fleet])
}

fn baseline_table(path: &Path) -> String {
    let Ok(document) = fs::read_to_string(path) else {
        return String::new();
    };
    let mut within_table = false;
    let mut table = Vec::new();
    for line in document.lines() {
        if line == "<!-- baseline:begin -->" {
            within_table = true;
            continue;
        }
        if line == "<!-- baseline:end -->" {
            within_table = false;
            continue;
        }
        if within_table {
            table.push(line);
        }
    }
    table.join("\n")
}

fn run_fab_container_dockerfile(id: &str, required: bool, root: &Path) -> CheckResult {
    const DOCKERFILE: &str = "skills/autospec-fab/docker/Dockerfile";
    const LINT: &str = "scripts/lint-fab-dockerfile.sh";
    if !root.join(DOCKERFILE).is_file() {
        return CheckResult::completed(id, required, 0, 0, 0, 0, 0, output_digest(&[], &[]));
    }
    if !root.join(LINT).is_file() {
        return failure(
            id,
            required,
            "scripts/lint-fab-dockerfile.sh: pin-lint script missing (issue #1300)",
        );
    }
    for required_path in [
        DOCKERFILE,
        "skills/autospec-fab/docker/requirements.txt",
        "skills/autospec-fab/docker/wrappers/ccx_safety_wrapper.py",
        "skills/autospec-fab/docker/wrappers/foam_cfd_reduce.py",
    ] {
        if !root.join(required_path).is_file() {
            return failure(
                id,
                required,
                &format!("{required_path}: required container file missing (issue #1300)"),
            );
        }
    }

    let commands = [
        ToolCommand::new("bash", ["-n", LINT])
            .expect("fab pin lint syntax check is a direct argument vector"),
        ToolCommand::new("bash", [LINT, DOCKERFILE])
            .expect("fab pin lint execution is a direct argument vector"),
    ];
    run_commands(id, required, root, commands)
}

fn trio_directories(root: &Path) -> Vec<PathBuf> {
    let skills_root = root.join("skills");
    let Ok(entries) = fs::read_dir(skills_root) else {
        return Vec::new();
    };

    let mut directories = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_dir()
                && path.join("SKILL.md").is_file()
                && path.join("opencode/agent.md").is_file()
                && path.join("codex/prompt.md").is_file()
        })
        .collect::<Vec<_>>();
    directories.sort();
    directories
}

fn aggregate(id: &str, required: bool, results: Vec<CheckResult>) -> CheckResult {
    if results.is_empty() {
        return CheckResult::completed(id, required, 0, 0, 0, 0, 0, output_digest(&[], &[]));
    }

    let failure = results.iter().find(|result| result.is_failure());
    let combined_digests = results
        .iter()
        .flat_map(|result| result.output_digest.bytes().chain(std::iter::once(b'\n')))
        .collect::<Vec<_>>();
    CheckResult {
        id: id.to_string(),
        required,
        exit_code: match failure {
            Some(result) => result.exit_code,
            None => Some(0),
        },
        elapsed_ms: results.iter().map(|result| result.elapsed_ms).sum(),
        spawn_count: results.iter().map(|result| result.spawn_count).sum(),
        stdout_bytes: results.iter().map(|result| result.stdout_bytes).sum(),
        stderr_bytes: results.iter().map(|result| result.stderr_bytes).sum(),
        output_digest: output_digest(&combined_digests, &[]),
    }
}

fn failure(id: &str, required: bool, message: &str) -> CheckResult {
    CheckResult::completed(
        id,
        required,
        1,
        0,
        0,
        0,
        message.len(),
        output_digest(&[], message.as_bytes()),
    )
}

fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .expect("discovered trio directory is inside the validation root")
        .to_string_lossy()
        .replace('\\', "/")
}

fn frontmatter_body(document: &str) -> Option<String> {
    let mut separators = 0;
    let mut body = String::new();
    for line in document.split_inclusive('\n') {
        if line.trim_end_matches(['\r', '\n']) == "---" {
            separators += 1;
            if separators == 2 {
                break;
            }
            continue;
        }
        if separators == 1 {
            body.push_str(line);
        }
    }
    (separators >= 2 && !body.trim_end_matches(['\r', '\n']).is_empty()).then_some(body)
}

fn has_frontmatter_key(document: &str) -> bool {
    document.lines().any(|line| {
        let candidate = line.trim_start_matches(char::is_whitespace);
        let key_length = candidate
            .chars()
            .take_while(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '_' | '-')
            })
            .count();
        key_length > 0 && candidate[key_length..].starts_with(':')
    })
}

fn program_on_path(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|directory| directory.join(program).is_file())
    })
}

fn has_usage_line(stdout: &[u8]) -> bool {
    String::from_utf8_lossy(stdout)
        .lines()
        .any(|line| line.starts_with("Usage:"))
}

fn has_line_prefix(path: &Path, expected: &str) -> bool {
    path.is_file()
        && fs::read_to_string(path)
            .map(|document| document.lines().any(|line| line.starts_with(expected)))
            .unwrap_or(false)
}

fn contains_unguarded_claim_swap(path: &Path) -> bool {
    fs::read_to_string(path)
        .map(|document| {
            document.lines().any(|line| {
                line.find("gh issue edit ").is_some_and(|index| {
                    line[index..].contains("--add-label in-progress-by-bot")
                        && !line.contains("claim-issue.sh")
                })
            })
        })
        .unwrap_or(false)
}

fn has_heading_prefix(path: &Path, expected: &str) -> bool {
    fs::read_to_string(path)
        .map(|document| document.lines().any(|line| line.starts_with(expected)))
        .unwrap_or(false)
}

fn has_same_line_tokens(path: &Path, first: &str, second: &str) -> bool {
    fs::read_to_string(path)
        .map(|document| {
            document
                .lines()
                .any(|line| line.contains(first) && line.contains(second))
        })
        .unwrap_or(false)
}

fn has_same_line_ordered_tokens(path: &Path, tokens: &[&str]) -> bool {
    fs::read_to_string(path)
        .map(|document| {
            document.lines().any(|line| {
                tokens
                    .iter()
                    .try_fold(0, |offset, token| {
                        line[offset..]
                            .find(token)
                            .map(|index| offset + index + token.len())
                    })
                    .is_some()
            })
        })
        .unwrap_or(false)
}

fn inclusive_section(path: &Path, start_heading: &str, end_heading: &str) -> Option<String> {
    let document = fs::read_to_string(path).ok()?;
    let lines = document.lines().collect::<Vec<_>>();
    let start = lines
        .iter()
        .position(|line| line.starts_with(start_heading))?;
    let end = lines
        .iter()
        .enumerate()
        .skip(start)
        .find_map(|(index, line)| line.starts_with(end_heading).then_some(index))?;
    Some(lines[start..=end].join("\n"))
}

fn contains_case_insensitive(path: &Path, expected: &str) -> bool {
    fs::read_to_string(path)
        .map(|document| {
            document
                .to_ascii_lowercase()
                .contains(&expected.to_ascii_lowercase())
        })
        .unwrap_or(false)
}

fn contains_rm_rf(line: &str) -> bool {
    let mut remaining = line;
    while let Some(index) = remaining.find("rm") {
        let suffix = &remaining[index + "rm".len()..];
        let trimmed = suffix.trim_start_matches(char::is_whitespace);
        if trimmed.len() < suffix.len() && trimmed.starts_with("-rf") {
            return true;
        }
        remaining = suffix;
    }
    false
}

fn contains(path: &Path, expected: &str) -> bool {
    path.is_file()
        && fs::read_to_string(path)
            .map(|document| document.contains(expected))
            .unwrap_or(false)
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    path.is_file()
        && fs::metadata(path)
            .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

const PYTHON_FRONTMATTER_CHECK: &str = r#"import sys
try:
    import yaml
except ImportError:
    raise SystemExit(0)
document = open(sys.argv[1], encoding="utf-8").read()
separators = 0
body = []
for line in document.splitlines(True):
    if line.rstrip("\r\n") == "---":
        separators += 1
        if separators == 2:
            break
        continue
    if separators == 1:
        body.append(line)
try:
    yaml.safe_load("".join(body))
except Exception:
    raise SystemExit(1)
"#;

const PYTHON_GENERATED_YAML_CHECK: &str = r#"import pathlib
import sys
try:
    import yaml
except Exception as exc:
    print(f"PyYAML unavailable: {exc}", file=sys.stderr)
    raise SystemExit(1)
failures = []
root = pathlib.Path(".autospec")
if root.exists():
    for path in sorted(list(root.rglob("*.yml")) + list(root.rglob("*.yaml"))):
        try:
            with path.open(encoding="utf-8") as handle:
                yaml.safe_load(handle)
        except Exception as exc:
            failures.append(f"{path}: {exc}")
if failures:
    print("\n".join(failures), file=sys.stderr)
    raise SystemExit(1)
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregate_preserves_a_missing_child_tool_failure() {
        let result = aggregate(
            "check",
            true,
            vec![CheckResult {
                id: "child".to_string(),
                required: true,
                exit_code: None,
                elapsed_ms: 0,
                spawn_count: 0,
                stdout_bytes: 0,
                stderr_bytes: 0,
                output_digest: "child".to_string(),
            }],
        );

        assert_eq!(result.exit_code, None);
        assert!(result.is_failure());
    }
}
