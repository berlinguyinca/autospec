use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::command::{
    bats_command, enter_fast_validation_mode, security_artifact_commands, ToolCommand,
};
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
    AutospecExploreDiscovery,
    AutospecQaContract,
    QaDeployContract,
    QaVerifyFirstDiscipline,
    QaExhaustivenessContract,
    QaIncidentContract,
    QaHealLoopContract,
    QualityDifferential,
    ReleaseAreaContract,
    ReleaseWorktreeAssert,
    FabContainerPinLint,
    RepoQualityAudit,
    AutospecAutonomousContract,
    DogfoodDetectors,
    AutospecParallelDispatch,
    GrowthShared,
    GrowthCandidatePipeline,
    GrowRunPipeline,
    DbTelemetry,
    WorktreeLadderAssertParity,
    Phase4SingleAgentDiscipline,
    Phase4FinalQualityGate,
    AutospecRefineContract,
    AutospecContinueContract,
    AutospecLoopContract,
    AutospecResumeStructure,
    AutospecSupervisorStructure,
    AutospecResumeContract,
    ImplementerContract,
    ReviewerContract,
    ConductorWiringContract,
    AutonomyGuardrailsFoundation,
    PythonSuites,
    GrowDefineContract,
    AutospecDocContract,
    ConstitutionValidation,
    InstallTests,
    ControlPlaneBootstrap,
    AutospecTestSkill,
    AutospecPlaywrightSkill,
    AutospecFabContract,
    GroomingContract,
    MutationAndNegativePath,
    LintImplementationHelpers,
    LintIssueHelpers,
    SecurityArtifactProfile,
    Phase4CiStatusCompare,
    DefineSpecWorktreeRouting,
    RunGroomPreflightContract,
    GrowRunContract,
    PerformanceWorkstream,
    UxUiWorkstream,
    TokenBaselineFresh,
    ArchitectureFitnessEngine,
    Phase4TestSuites,
    ValidationMatrixSmoke,
}

impl ExternalCheck {
    pub(crate) fn run_with_fast(
        &self,
        id: &str,
        required: bool,
        root: &Path,
        fast: bool,
    ) -> CheckResult {
        let _mode = enter_fast_validation_mode(fast);
        self.run(id, required, root)
    }

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
            Self::AutospecExploreDiscovery => run_autospec_explore_discovery(id, required, root),
            Self::AutospecQaContract => run_autospec_qa_contract(id, required, root),
            Self::QaDeployContract => run_qa_deploy_contract(id, required, root),
            Self::QaVerifyFirstDiscipline => run_qa_verify_first_discipline(id, required, root),
            Self::QaExhaustivenessContract => run_qa_exhaustiveness_contract(id, required, root),
            Self::QaIncidentContract => run_qa_incident_contract(id, required, root),
            Self::QaHealLoopContract => run_qa_heal_loop_contract(id, required, root),
            Self::QualityDifferential => run_quality_differential(id, required, root),
            Self::ReleaseAreaContract => run_release_area_contract(id, required, root),
            Self::ReleaseWorktreeAssert => run_release_worktree_assert(id, required, root),
            Self::FabContainerPinLint => run_fab_container_pin_lint(id, required, root),
            Self::RepoQualityAudit => run_repo_quality_audit(id, required, root),
            Self::AutospecAutonomousContract => {
                run_autospec_autonomous_contract(id, required, root)
            }
            Self::DogfoodDetectors => run_dogfood_detectors(id, required, root),
            Self::AutospecParallelDispatch => run_autospec_parallel_dispatch(id, required, root),
            Self::GrowthShared => run_growth_shared(id, required, root),
            Self::GrowthCandidatePipeline => run_growth_candidate_pipeline(id, required, root),
            Self::GrowRunPipeline => run_grow_run_pipeline(id, required, root),
            Self::DbTelemetry => run_db_telemetry(id, required, root),
            Self::WorktreeLadderAssertParity => {
                run_worktree_ladder_assert_parity(id, required, root)
            }
            Self::Phase4SingleAgentDiscipline => {
                run_phase4_single_agent_discipline(id, required, root)
            }
            Self::Phase4FinalQualityGate => run_phase4_final_quality_gate(id, required, root),
            Self::AutospecRefineContract => run_autospec_refine_contract(id, required, root),
            Self::AutospecContinueContract => run_autospec_continue_contract(id, required, root),
            Self::AutospecLoopContract => run_autospec_loop_contract(id, required, root),
            Self::AutospecResumeStructure => run_autospec_resume_structure(id, required, root),
            Self::AutospecSupervisorStructure => {
                run_autospec_supervisor_structure(id, required, root)
            }
            Self::AutospecResumeContract => run_autospec_resume_contract(id, required, root),
            Self::ImplementerContract => run_implementer_contract(id, required, root),
            Self::ReviewerContract => run_reviewer_contract(id, required, root),
            Self::ConductorWiringContract => run_conductor_wiring_contract(id, required, root),
            Self::AutonomyGuardrailsFoundation => {
                run_autonomy_guardrails_foundation(id, required, root)
            }
            Self::PythonSuites => run_python_suites(id, required, root),
            Self::GrowDefineContract => run_grow_define_contract(id, required, root),
            Self::AutospecDocContract => run_autospec_doc_contract(id, required, root),
            Self::ConstitutionValidation => run_constitution_validation(id, required, root),
            Self::InstallTests => run_install_tests(id, required, root),
            Self::ControlPlaneBootstrap => run_control_plane_bootstrap(id, required, root),
            Self::AutospecTestSkill => run_skill_validator(id, required, root, "autospec-test"),
            Self::AutospecPlaywrightSkill => run_autospec_playwright_skill(id, required, root),
            Self::AutospecFabContract => run_autospec_fab_contract(id, required, root),
            Self::GroomingContract => run_grooming_contract(id, required, root),
            Self::MutationAndNegativePath => run_mutation_and_negative_path(id, required, root),
            Self::LintImplementationHelpers => run_lint_implementation_helpers(id, required, root),
            Self::LintIssueHelpers => run_lint_issue_helpers(id, required, root),
            Self::SecurityArtifactProfile => {
                run_commands(id, required, root, security_artifact_commands())
            }
            Self::Phase4CiStatusCompare => run_phase4_ci_status_compare(id, required, root),
            Self::DefineSpecWorktreeRouting => run_define_spec_worktree_routing(id, required, root),
            Self::RunGroomPreflightContract => run_groom_preflight_contract(id, required, root),
            Self::GrowRunContract => run_grow_run_contract(id, required, root),
            Self::PerformanceWorkstream => run_performance_workstream(id, required, root),
            Self::UxUiWorkstream => run_ux_ui_workstream(id, required, root),
            Self::TokenBaselineFresh => run_token_baseline_fresh(id, required, root),
            Self::ArchitectureFitnessEngine => run_architecture_fitness_engine(id, required, root),
            Self::Phase4TestSuites => run_phase4_test_suites(id, required, root),
            Self::ValidationMatrixSmoke => run_validation_matrix_smoke(id, required, root),
        }
    }
}
fn run_validation_matrix_smoke(id: &str, required: bool, root: &Path) -> CheckResult {
    run_commands(
        id,
        required,
        root,
        [
            ToolCommand::new("bash", ["tests/smoke/validation-matrix.sh"])
                .expect("validation-matrix smoke is a direct script invocation"),
        ],
    )
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
    const COMPOSE_NORMALIZE_SUITE: &str = "tests/unit/test_autospec_compose_normalize_skill.bats";
    const RUNTIME_PROOF_ARTIFACTS: [&str; 6] = [
        "tests/integration/runtime-compose-40-stack.bats",
        "tests/fixtures/runtime-resources/forty-stack/.autospec/runtime.yml",
        "tests/fixtures/runtime-resources/forty-stack/compose.yaml",
        "tests/fixtures/runtime-resources/forty-stack/Dockerfile",
        "reports/runtime-isolation/compose-40-stack.csv",
        "reports/runtime-isolation/compose-40-stack.json",
    ];
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

    if results.iter().all(CheckResult::is_success) {
        if !root.join(COMPOSE_NORMALIZE_SUITE).is_file() {
            results.push(failure(
                id,
                required,
                "Compose normalizer skill contract suite is missing",
            ));
        } else {
            results.push(run_bats_suite(id, required, root, COMPOSE_NORMALIZE_SUITE));
        }
    }

    if results.iter().all(CheckResult::is_success) {
        for artifact in RUNTIME_PROOF_ARTIFACTS {
            if !root.join(artifact).is_file() {
                results.push(failure(
                    id,
                    required,
                    &format!("runtime isolation proof artifact is missing: {artifact}"),
                ));
                break;
            }
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

    ToolCommand::new("bash", ["-n", "scripts/autospec-gap-miner.sh"])
        .expect("gap-miner syntax command is a direct argument vector")
        .execute_in(id, required, root)
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
        bats_command("tests/compute-release-verdict.bats"),
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
    let commands = suites.iter().map(|suite| bats_command(suite));
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
    let commands = suites.into_iter().map(|suite| bats_command(suite.as_str()));
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
    let commands = suites.into_iter().map(|suite| bats_command(suite.as_str()));
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
        if !contains(&path, "autospec claim acquire") {
            return failure(
                id,
                required,
                &format!("{trio}: claim section missing reference to autospec claim acquire (issue #871)"),
            );
        }
        if contains_unguarded_claim_swap(&path) {
            return failure(
                id,
                required,
                &format!(
                    "{trio}: unguarded check-then-act 'gh issue edit --add-label in-progress-by-bot' bypasses autospec claim acquire (issue #871)"
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
    let failure_message = if !WATCHDOG_GC_FUNCTION.is_in(&document) {
        Some("scripts/autospec-watchdog.sh must define gc_orphaned_worktrees() (issue #883)")
    } else if !document
        .lines()
        .any(|line| matches!(line, "gc_orphaned_worktrees" | "    gc_orphaned_worktrees"))
    {
        Some("scripts/autospec-watchdog.sh must invoke gc_orphaned_worktrees once per cycle (issue #883)")
    } else if !WATCHDOG_FORCE_REMOVE.is_in(&document) {
        Some("scripts/autospec-watchdog.sh GC must prune via 'git worktree remove --force' (issue #883)")
    } else if document
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .any(contains_rm_rf)
    {
        Some("scripts/autospec-watchdog.sh must never 'rm -rf' a worktree path; use 'git worktree remove --force' (issue #883)")
    } else if !WATCHDOG_UNPUSHED_GUARD.is_in(&document) {
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
    const SCHEMA: &str = "schemas/autospec-explore-specialists.schema.json";
    const DELETED_SCAN: &str = "scripts/explore-specialist-scan.sh";
    const DELETED_SUITE: &str = "tests/explore/test_explore_specialists.bats";
    const CORE_SOURCE: &str = "crates/autospec-core/src/explore/specialists.rs";

    let mut results = Vec::new();
    if root.join(DELETED_SCAN).exists() {
        results.push(failure(
            id,
            required,
            "scripts/explore-specialist-scan.sh: deleted scanner must not be a runtime requirement",
        ));
        return aggregate(id, required, results);
    }
    if root.join(DELETED_SUITE).exists() {
        results.push(failure(
            id,
            required,
            "tests/explore/test_explore_specialists.bats: shell specialist coverage must move to Rust tests",
        ));
        return aggregate(id, required, results);
    }
    if !root.join(CORE_SOURCE).is_file() {
        results.push(failure(
            id,
            required,
            "crates/autospec-core/src/explore/specialists.rs: Rust specialist scanner missing",
        ));
        return aggregate(id, required, results);
    }
    if !root.join(SCHEMA).is_file() {
        results.push(failure(
            id,
            required,
            "schemas/autospec-explore-specialists.schema.json: specialists roster schema missing",
        ));
        return aggregate(id, required, results);
    }

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
    aggregate(id, required, results)
}

struct Stage2IntersectContract {
    passes: fn(&Path) -> bool,
    failure: &'static str,
}

fn has_stage2_intersect_heading(path: &Path) -> bool {
    has_heading_prefix(path, "## Stage-2 intersect")
}

fn has_stage2_internet_forums_source(path: &Path) -> bool {
    contains(path, "internet-forums")
}

fn has_stage2_internet_forums_weight(path: &Path) -> bool {
    has_same_line_tokens(path, "internet-forums", "0.4")
}

fn cites_stage2_intersect_prefilter(path: &Path) -> bool {
    contains(path, "discovery-intersect-prefilter.sh")
}

fn reuses_stage2_repo_domain_derivation(path: &Path) -> bool {
    contains_case_insensitive(path, "repo-domain derivation")
}

fn cites_stage2_candidate_schema(path: &Path) -> bool {
    contains(path, "autospec-explore-proposal.schema.json")
}

fn states_stage2_untrusted_boundary(path: &Path) -> bool {
    contains_case_insensitive(path, "untrusted")
}

const STAGE2_INTERSECT_CONTRACTS: &[Stage2IntersectContract] = &[
    Stage2IntersectContract {
        passes: Path::is_file,
        failure: "required adapter file missing",
    },
    Stage2IntersectContract {
        passes: has_stage2_intersect_heading,
        failure: "missing '## Stage-2 intersect' section",
    },
    Stage2IntersectContract {
        passes: has_stage2_internet_forums_source,
        failure: "missing internet-forums Stage-2 discovery source",
    },
    Stage2IntersectContract {
        passes: has_stage2_internet_forums_weight,
        failure: "internet-forums source must be weighted 0.4",
    },
    Stage2IntersectContract {
        passes: cites_stage2_intersect_prefilter,
        failure: "Stage-2 must cite discovery-intersect-prefilter.sh",
    },
    Stage2IntersectContract {
        passes: reuses_stage2_repo_domain_derivation,
        failure: "Stage-2 must reuse the existing repo-domain derivation",
    },
    Stage2IntersectContract {
        passes: cites_stage2_candidate_schema,
        failure: "Stage-2 must emit the existing explore candidate schema (cite, not redefine)",
    },
    Stage2IntersectContract {
        passes: states_stage2_untrusted_boundary,
        failure: "Stage-2 must state the untrusted-DATA trust boundary",
    },
];

fn stage2_intersect_contract_failure(path: &Path) -> Option<&'static str> {
    STAGE2_INTERSECT_CONTRACTS
        .iter()
        .find(|contract| !(contract.passes)(path))
        .map(|contract| contract.failure)
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
        if let Some(message) = stage2_intersect_contract_failure(&path) {
            return aggregate(
                id,
                required,
                vec![
                    prefilter,
                    failure(id, required, &format!("{trio}: {message}")),
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

fn run_autospec_explore_discovery(id: &str, required: bool, root: &Path) -> CheckResult {
    const RESEARCHERS: &[(&str, bool)] = &[
        ("scripts/explore-research/quality-resilience.sh", true),
        ("scripts/explore-research/dogfooding.sh", true),
        ("scripts/explore-research/self-leverage.sh", true),
    ];
    const AGGREGATOR: &str = "scripts/explore-research-cycle.sh";
    const PROPOSAL_SCHEMA: &str = "schemas/autospec-explore-proposal.schema.json";
    const SPECIALIST_SCHEMA: &str = "schemas/autospec-explore-specialists.schema.json";
    const TRIO: &[&str] = &[
        "skills/autospec-explore/SKILL.md",
        "skills/autospec-explore/codex/prompt.md",
        "skills/autospec-explore/opencode/agent.md",
    ];
    const RUNBOOK: &str = "docs/runbooks/discovery-sweep.md";
    const SPEC: &str = "docs/specs/2026-06-15-autospec-explore-discovery-enhance.md";
    const SUITES: &[&str] = &[
        "tests/explore/test_explore_quality_resilience.bats",
        "tests/explore/test_explore_dogfooding.bats",
        "tests/explore/test_explore_self_leverage.bats",
        "tests/explore/test_explore_verify_stage.bats",
        "tests/explore/test_explore_severity_roi.bats",
    ];

    let researchers = run_required_bash_scripts(id, required, root, RESEARCHERS);
    if researchers.is_failure() {
        return researchers;
    }
    let mut results = vec![researchers];
    let dogfooding = root.join("scripts/explore-research/dogfooding.sh");
    if !contains(&dogfooding, "AUTOSPEC_STATE_DIR") {
        results.push(failure(
            id,
            required,
            "scripts/explore-research/dogfooding.sh: must read ${AUTOSPEC_STATE_DIR:-$HOME/.autospec}",
        ));
        return aggregate(id, required, results);
    }

    let aggregator = run_required_bash_scripts(id, required, root, &[(AGGREGATOR, false)]);
    if aggregator.is_failure() {
        results.push(aggregator);
        return aggregate(id, required, results);
    }
    results.push(aggregator);
    const AGGREGATOR_TOKEN_CHECKS: &[DiscoveryTokenCheck] = &[
        DiscoveryTokenCheck {
            token: "proposals_after_verify",
            message: "missing verify-stage counter proposals_after_verify",
        },
        DiscoveryTokenCheck {
            token: "proposals_refuted",
            message: "missing proposals_refuted counter",
        },
        DiscoveryTokenCheck {
            token: "proposals_after_roi",
            message: "missing ROI-gate counter proposals_after_roi",
        },
        DiscoveryTokenCheck {
            token: "structural_fixes",
            message: "missing pattern-synthesis counter structural_fixes",
        },
    ];
    const SCHEMA_TOKEN_CHECKS: &[DiscoveryRequiredFileTokenCheck] = &[
        DiscoveryRequiredFileTokenCheck {
            path: PROPOSAL_SCHEMA,
            missing_label: "schema",
            token: "silent-wrong",
            message: "severity enum must include silent-wrong (load-bearing rank head)",
        },
        DiscoveryRequiredFileTokenCheck {
            path: PROPOSAL_SCHEMA,
            missing_label: "schema",
            token: "named_consumer",
            message: "missing named_consumer ROI field",
        },
        DiscoveryRequiredFileTokenCheck {
            path: SPECIALIST_SCHEMA,
            missing_label: "schema",
            token: "suggested_specialists",
            message: "missing suggested_specialists roster field",
        },
    ];
    const DOCUMENT_CHECKS: &[DiscoveryDocumentCheck] = &[
        DiscoveryDocumentCheck {
            path: RUNBOOK,
            missing_label: "runbook",
        },
        DiscoveryDocumentCheck {
            path: SPEC,
            missing_label: "source spec",
        },
    ];

    if let Some(message) = first_missing_discovery_token(root, AGGREGATOR, AGGREGATOR_TOKEN_CHECKS)
    {
        return aggregate_discovery_failure(id, required, results, message);
    }
    if !contains_case_insensitive(&root.join(AGGREGATOR), "severity") {
        return aggregate_discovery_failure(
            id,
            required,
            results,
            "scripts/explore-research-cycle.sh: missing severity-first ranking logic",
        );
    }

    if let Some(message) = first_missing_discovery_file_token(root, SCHEMA_TOKEN_CHECKS) {
        return aggregate_discovery_failure(id, required, results, message);
    }

    if let Some(message) = first_missing_discovery_trio_contract(root, TRIO) {
        return aggregate_discovery_failure(id, required, results, message);
    }

    if let Some(message) = first_missing_discovery_document(root, DOCUMENT_CHECKS) {
        return aggregate_discovery_failure(id, required, results, message);
    }
    if !contains(
        &root.join(RUNBOOK),
        "check_autospec_explore_discovery_contract",
    ) {
        return aggregate_discovery_failure(
            id,
            required,
            results,
            "docs/runbooks/discovery-sweep.md: must cite check_autospec_explore_discovery_contract as the lock-step enforcer",
        );
    }

    let bats = run_bats_suites(id, required, root, SUITES);
    results.push(bats);
    aggregate(id, required, results)
}

struct DiscoveryTokenCheck {
    token: &'static str,
    message: &'static str,
}

struct DiscoveryRequiredFileTokenCheck {
    path: &'static str,
    missing_label: &'static str,
    token: &'static str,
    message: &'static str,
}

struct DiscoveryDocumentCheck {
    path: &'static str,
    missing_label: &'static str,
}

fn aggregate_discovery_failure(
    id: &str,
    required: bool,
    mut results: Vec<CheckResult>,
    message: impl AsRef<str>,
) -> CheckResult {
    results.push(failure(id, required, message.as_ref()));
    aggregate(id, required, results)
}

fn first_missing_discovery_token(
    root: &Path,
    relative_path: &str,
    checks: &[DiscoveryTokenCheck],
) -> Option<String> {
    let path = root.join(relative_path);
    for check in checks {
        if !contains(&path, check.token) {
            return Some(format!("{relative_path}: {}", check.message));
        }
    }
    None
}

fn first_missing_discovery_file_token(
    root: &Path,
    checks: &[DiscoveryRequiredFileTokenCheck],
) -> Option<String> {
    for check in checks {
        let path = root.join(check.path);
        if !path.is_file() {
            return Some(format!(
                "{}: required {} missing",
                check.path, check.missing_label
            ));
        }
        if !contains(&path, check.token) {
            return Some(format!("{}: {}", check.path, check.message));
        }
    }
    None
}

fn first_missing_discovery_trio_contract(root: &Path, trio_paths: &[&str]) -> Option<String> {
    const HEADINGS: &[&str] = &["## Discovery enhancement", "## Domain-specialist roster"];
    const RESEARCHERS: &[&str] = &["quality-resilience", "dogfooding", "self-leverage"];
    const STAGE_ORDER: &[&str] = &[
        "dedup",
        "gap-confirm",
        "verify",
        "ROI",
        "pattern-synthesis",
        "severity-first rank",
    ];
    const SEVERITY_BANDS: &[&str] = &[
        "silent-wrong",
        "correctness",
        "stability",
        "operability",
        "feature",
        "nicety",
    ];
    const SPECIALIST_FLAGS: &[&str] = &["--specialists-mode", "--num-specialists"];

    for trio in trio_paths {
        let path = root.join(trio);
        if !path.is_file() {
            return Some(format!("{trio}: required adapter file missing"));
        }
        for heading in HEADINGS {
            if !has_heading_prefix(&path, heading) {
                return Some(format!("{trio}: missing '{heading}' section"));
            }
        }
        for researcher in RESEARCHERS {
            if !contains(&path, researcher) {
                return Some(format!(
                    "{trio}: missing discovery researcher reference '{researcher}'"
                ));
            }
        }
        if !has_same_line_ordered_tokens(&path, STAGE_ORDER) {
            return Some(format!("{trio}: missing aggregator stage order"));
        }
        for severity in SEVERITY_BANDS {
            if !contains(&path, severity) {
                return Some(format!("{trio}: missing severity band '{severity}'"));
            }
        }
        for flag in SPECIALIST_FLAGS {
            if !contains(&path, flag) {
                return Some(format!("{trio}: missing specialist flag '{flag}'"));
            }
        }
        if !contains(&path, "7 universal") {
            return Some(format!(
                "{trio}: missing '7 universal' researcher accounting"
            ));
        }
        if contains_stale_researcher_count(&path) {
            return Some(format!(
                "{trio}: stale '6 researchers' count still present (must be 7 universal)"
            ));
        }
    }
    None
}

fn first_missing_discovery_document(
    root: &Path,
    documents: &[DiscoveryDocumentCheck],
) -> Option<String> {
    const TRACKS: &[&str] = &[
        "Feature delta",
        "External",
        "Quality & resilience",
        "Dogfooding",
        "Self-leverage",
    ];

    for document in documents {
        let path = root.join(document.path);
        if !path.is_file() {
            return Some(format!(
                "{}: required {} missing",
                document.path, document.missing_label
            ));
        }
        for track in TRACKS {
            if !contains(&path, track) {
                return Some(format!(
                    "{}: missing discovery track '{}'",
                    document.path, track
                ));
            }
        }
        if !contains_case_insensitive(&path, "pattern synthesis") {
            return Some(format!(
                "{}: missing pattern-synthesis stage",
                document.path
            ));
        }
    }
    None
}

fn run_autospec_qa_contract(id: &str, required: bool, root: &Path) -> CheckResult {
    const SKILL: &str = "skills/autospec-qa/SKILL.md";
    const SCHEMAS: &[&str] = &[
        "schemas/autospec-proof-matrix.schema.json",
        "schemas/autospec-reliability.schema.json",
        "schemas/autospec-control-intent-ledger.schema.json",
        "schemas/autospec-mutation-proof.schema.json",
        "schemas/autospec-canary-results.schema.json",
    ];
    const VALIDATOR: &str = "scripts/validate-qa-artifacts.sh";

    let skill = root.join(SKILL);
    if !skill.is_file() {
        return failure(
            id,
            required,
            "skills/autospec-qa/SKILL.md: required file missing",
        );
    }
    for heading in [
        "## No-mock deployed smoke blocker handling",
        "## Presentational control classification",
        "## User input element intent prompt",
        "## Live backend blocker triage prompt",
        "## Proof artifact requirements",
        "## Generated app reliability contract prompt",
        "## Control intent ledger prompt",
        "## Duplicate-code guardian prompt",
        "## Mutation and breakage proof prompt",
        "## Post-merge deployed canary prompt",
        "## No-mock minimum coverage prompt",
        "## New code intent gate",
        "## Artifact freshness gate",
        "## Evidence provenance gate",
        "## Console and network error gate",
        "## Spec contradiction detector",
        "## Data lifecycle proof",
        "## Observability contract",
        "## Flake quarantine rule",
        "## Reliability exhaustion loop",
        "## Legacy removal and spec cleanup gate",
    ] {
        if !has_heading_prefix(&skill, heading) {
            return failure(id, required, &format!("{SKILL}: missing {heading} section"));
        }
    }
    for anchor in [
        "must be `PARTIAL`",
        "environment setup issue/task",
        "operator-blocker line with the exact missing configuration",
        "Never print secret values",
        "route-specific worker/handler failure",
        ".autospec/proof-matrix.json",
        ".autospec/reliability.yml",
        ".autospec/control-intent-ledger.json",
        ".autospec/mutation-proof.json",
        ".autospec/canary-results.json",
        "Do not populate deprecated caches",
    ] {
        if !contains(&skill, anchor) {
            return failure(
                id,
                required,
                &format!("{SKILL}: missing {anchor} requirement"),
            );
        }
    }
    for schema in SCHEMAS {
        if !root.join(schema).is_file() {
            return failure(
                id,
                required,
                &format!("{schema}: QA artifact schema missing"),
            );
        }
    }
    if !contains(
        &root.join("schemas/autospec-reliability.schema.json"),
        "deprecated_surfaces",
    ) {
        return failure(
            id,
            required,
            "schemas/autospec-reliability.schema.json: missing deprecated_surfaces contract",
        );
    }

    run_required_bash_scripts(id, required, root, &[(VALIDATOR, true)])
}

fn run_qa_deploy_contract(id: &str, required: bool, root: &Path) -> CheckResult {
    const TRIO: &[&str] = &[
        "skills/autospec-qa/SKILL.md",
        "skills/autospec-qa/codex/prompt.md",
        "skills/autospec-qa/opencode/agent.md",
    ];
    const RUNNER: &str = "scripts/qa-deploy-runner.sh";
    const SCHEMA: &str = "schemas/autospec-qa-deploy.schema.json";
    const SUITE: &str = "tests/qa/test_qa_deploy.bats";

    for trio in TRIO {
        let path = root.join(trio);
        if !path.is_file() {
            return failure(id, required, &format!("{trio}: required trio file missing"));
        }
        if !has_heading_prefix(&path, "## Deployment contract") {
            return failure(
                id,
                required,
                &format!("{trio}: missing '## Deployment contract' section"),
            );
        }
        for anchor in [
            ".autospec/qa-deploy.yml",
            "qa-deploy-runner.sh",
            "before any cluster",
            "qa_deploy_forbidden_target",
            "qa_deploy_prod_pattern",
            "qa_deploy_missing_records_cap",
            "code_health:qa_deploy_failed",
            "code_health:qa_deploy_invalid_contract",
            "deploy:",
            "head_sha_deployed",
            "--skip-teardown",
            "byte-for-byte today",
        ] {
            if !contains(&path, anchor) {
                return failure(
                    id,
                    required,
                    &format!("{trio}: deployment contract missing required anchor: {anchor}"),
                );
            }
        }
    }

    let runner = run_required_bash_scripts(id, required, root, &[(RUNNER, true)]);
    if runner.is_failure() {
        return runner;
    }
    if !root.join(SCHEMA).is_file() {
        return aggregate(
            id,
            required,
            vec![
                runner,
                failure(
                    id,
                    required,
                    "schemas/autospec-qa-deploy.schema.json: qa-deploy contract schema missing",
                ),
            ],
        );
    }

    let mut results = vec![runner];
    if program_on_path("jq") {
        let jq = ToolCommand::new("jq", ["empty", SCHEMA])
            .expect("QA deploy JSON validation uses direct arguments")
            .execute_in(id, required, root);
        if jq.is_failure() {
            results.push(jq);
            return aggregate(id, required, results);
        }
        results.push(jq);
    }
    if program_on_path("ajv") {
        let ajv = ToolCommand::new("ajv", ["compile", "-s", SCHEMA, "--spec=draft2020"])
            .expect("QA deploy schema compilation uses direct arguments")
            .execute_in(id, required, root);
        if ajv.is_failure() {
            results.push(ajv);
            return aggregate(id, required, results);
        }
        results.push(ajv);
    }
    results.push(run_bats_suites(id, required, root, &[SUITE]));
    aggregate(id, required, results)
}

fn run_qa_verify_first_discipline(id: &str, required: bool, root: &Path) -> CheckResult {
    const SCRIPTS: &[(&str, bool)] = &[
        ("scripts/qa-verify-finding.sh", true),
        ("scripts/qa-cluster-coverage.sh", true),
        ("scripts/qa-finding-filter.sh", true),
    ];
    const TRIO: &[&str] = &[
        "skills/autospec-qa/SKILL.md",
        "skills/autospec-qa/codex/prompt.md",
        "skills/autospec-qa/opencode/agent.md",
    ];
    const SUITES: &[&str] = &[
        "tests/qa/test_verify_first.bats",
        "tests/qa/test_finding_filter.bats",
    ];

    let scripts = run_required_bash_scripts(id, required, root, SCRIPTS);
    if scripts.is_failure() {
        return scripts;
    }
    for trio in TRIO {
        let path = root.join(trio);
        if !path.is_file() {
            return aggregate(
                id,
                required,
                vec![
                    scripts,
                    failure(
                        id,
                        required,
                        &format!("{trio}: required adapter file missing"),
                    ),
                ],
            );
        }
        for heading in ["## Verify-first discipline", "## Cluster sizing"] {
            if !has_heading_prefix(&path, heading) {
                return aggregate(
                    id,
                    required,
                    vec![
                        scripts,
                        failure(
                            id,
                            required,
                            &format!("{trio}: missing '{heading}' section"),
                        ),
                    ],
                );
            }
        }
        for anchor in [
            "qa-verify-finding.sh",
            "qa-cluster-coverage.sh",
            "qa-finding-filter.sh",
            "AUTOSPEC_QA_STRICT=1",
        ] {
            if !contains(&path, anchor) {
                return aggregate(
                    id,
                    required,
                    vec![
                        scripts,
                        failure(id, required, &format!("{trio}: missing {anchor} rule")),
                    ],
                );
            }
        }
    }

    let mut results = vec![scripts];
    if program_on_path("bats") {
        for suite in SUITES {
            if root.join(suite).is_file() {
                let bats = bats_command(suite).execute_in(id, required, root);
                let failed = bats.is_failure();
                results.push(bats);
                if failed {
                    return aggregate(id, required, results);
                }
            }
        }
    }
    aggregate(id, required, results)
}

fn run_qa_exhaustiveness_contract(id: &str, required: bool, root: &Path) -> CheckResult {
    const SCRIPT: &str = "scripts/qa-exhaustiveness-check.sh";
    const SUITE: &str = "tests/qa/test_exhaustiveness.bats";
    const TRIO: &[&str] = &[
        "skills/autospec-qa/SKILL.md",
        "skills/autospec-qa/codex/prompt.md",
        "skills/autospec-qa/opencode/agent.md",
    ];

    let script = run_required_bash_scripts(id, required, root, &[(SCRIPT, true)]);
    if script.is_failure() {
        return script;
    }
    if !root.join(SUITE).is_file() {
        return aggregate(
            id,
            required,
            vec![
                script,
                failure(id, required, &format!("{SUITE}: bats coverage missing")),
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
                    script,
                    failure(
                        id,
                        required,
                        &format!("{trio}: required adapter file missing"),
                    ),
                ],
            );
        }
        if !has_heading_prefix(&path, "## Exhaustiveness flags") {
            return aggregate(
                id,
                required,
                vec![
                    script,
                    failure(
                        id,
                        required,
                        &format!("{trio}: missing '## Exhaustiveness flags' section"),
                    ),
                ],
            );
        }
        for anchor in [
            "qa-exhaustiveness-check.sh",
            "--every-spec-row",
            "--mutation-each",
            "--no-mock-each",
            "--exhaustive",
        ] {
            if !contains(&path, anchor) {
                return aggregate(
                    id,
                    required,
                    vec![
                        script,
                        failure(
                            id,
                            required,
                            &format!("{trio}: missing {anchor} documentation"),
                        ),
                    ],
                );
            }
        }
    }
    let bats = run_bats_suites(id, required, root, &[SUITE]);
    aggregate(id, required, vec![script, bats])
}

fn run_qa_incident_contract(id: &str, required: bool, root: &Path) -> CheckResult {
    const SCRIPT: &str = "scripts/qa-incident-check.sh";
    const SCHEMA: &str = "schemas/autospec-production-incidents.schema.json";
    const SUITE: &str = "tests/qa/test_incident_check.bats";
    const TRIO: &[&str] = &[
        "skills/autospec-qa/SKILL.md",
        "skills/autospec-qa/codex/prompt.md",
        "skills/autospec-qa/opencode/agent.md",
    ];

    let script = run_required_bash_scripts(id, required, root, &[(SCRIPT, true)]);
    if script.is_failure() {
        return script;
    }
    for required_path in [SCHEMA, SUITE] {
        if !root.join(required_path).is_file() {
            return aggregate(
                id,
                required,
                vec![
                    script,
                    failure(
                        id,
                        required,
                        &format!("{required_path}: required file missing"),
                    ),
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
                    script,
                    failure(
                        id,
                        required,
                        &format!("{trio}: required adapter file missing"),
                    ),
                ],
            );
        }
        if !has_heading_prefix(&path, "## Production incident regression check") {
            return aggregate(
                id,
                required,
                vec![
                    script,
                    failure(
                        id,
                        required,
                        &format!("{trio}: missing incident regression heading"),
                    ),
                ],
            );
        }
        for anchor in [
            "qa-incident-check.sh",
            ".autospec/production-incidents.json",
        ] {
            if !contains(&path, anchor) {
                return aggregate(
                    id,
                    required,
                    vec![
                        script,
                        failure(id, required, &format!("{trio}: missing {anchor} reference")),
                    ],
                );
            }
        }
        if !contains_case_insensitive(&path, "never silently delete") {
            return aggregate(
                id,
                required,
                vec![
                    script,
                    failure(
                        id,
                        required,
                        &format!("{trio}: missing never-silently-delete rule"),
                    ),
                ],
            );
        }
    }
    let bats = run_bats_suites(id, required, root, &[SUITE]);
    aggregate(id, required, vec![script, bats])
}

fn run_qa_heal_loop_contract(id: &str, required: bool, root: &Path) -> CheckResult {
    const HEAL: &str = "scripts/qa-heal-loop.sh";
    const FINDING_TO_ISSUE: &str = "scripts/qa-finding-to-issue.sh";
    const TRIO: &[&str] = &[
        "skills/autospec-qa/SKILL.md",
        "skills/autospec-qa/codex/prompt.md",
        "skills/autospec-qa/opencode/agent.md",
    ];
    const SUITES: &[&str] = &[
        "tests/qa/test_qa_heal_default.bats",
        "tests/qa/test_heal_loop.bats",
    ];

    let scripts = run_required_bash_scripts(
        id,
        required,
        root,
        &[(HEAL, true), (FINDING_TO_ISSUE, true)],
    );
    if scripts.is_failure() {
        return scripts;
    }
    for suite in SUITES {
        if !root.join(suite).is_file() {
            return aggregate(
                id,
                required,
                vec![
                    scripts,
                    failure(id, required, &format!("{suite}: bats coverage missing")),
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
                    scripts,
                    failure(
                        id,
                        required,
                        &format!("{trio}: required adapter file missing"),
                    ),
                ],
            );
        }
        for heading in ["## Self-healing loop", "## --no-heal opt-out"] {
            if !has_heading_prefix(&path, heading) {
                return aggregate(
                    id,
                    required,
                    vec![
                        scripts,
                        failure(id, required, &format!("{trio}: missing {heading} section")),
                    ],
                );
            }
        }
        for anchor in [
            "qa-heal-loop.sh",
            "qa-finding-to-issue.sh",
            "oscillation_detected",
            "AUTOSPEC_HEAL_MAX_ROUNDS",
            "default-on",
            "--single-pass",
            "qa-no-heal.flag",
            "lib/autospec-loop.sh",
            "qa-heal-summary.md",
            "evidence_based_stop",
        ] {
            if !contains(&path, anchor) {
                return aggregate(
                    id,
                    required,
                    vec![
                        scripts,
                        failure(id, required, &format!("{trio}: missing {anchor} contract")),
                    ],
                );
            }
        }
    }
    let heal = root.join(HEAL);
    for anchor in [
        "lib/autospec-loop.sh",
        "--single-pass",
        "qa-no-heal.flag",
        "qa-heal-summary.md",
    ] {
        if !contains(&heal, anchor) {
            return aggregate(
                id,
                required,
                vec![
                    scripts,
                    failure(id, required, &format!("{HEAL}: missing {anchor} contract")),
                ],
            );
        }
    }
    let bats = run_bats_suites(id, required, root, SUITES);
    aggregate(id, required, vec![scripts, bats])
}

fn run_quality_differential(id: &str, required: bool, root: &Path) -> CheckResult {
    const SCRIPT: &str = "scripts/quality-differential.sh";
    const SUITE: &str = "tests/quality-differential.bats";
    const FIXTURES: &str = "tests/fixtures/quality-diff/refine-lenses";

    let script = run_required_bash_scripts(id, required, root, &[(SCRIPT, false)]);
    if script.is_failure() {
        return script;
    }
    if !root.join(SUITE).is_file() {
        return aggregate(
            id,
            required,
            vec![
                script,
                failure(id, required, &format!("{SUITE}: bats coverage missing")),
            ],
        );
    }
    let fixture_count = fs::read_dir(root.join(FIXTURES))
        .into_iter()
        .flat_map(|entries| entries.filter_map(Result::ok))
        .filter(|entry| entry.path().join("assert.sh").is_file())
        .count();
    if fixture_count < 3 {
        return aggregate(
            id,
            required,
            vec![
                script,
                failure(
                    id,
                    required,
                    &format!("{FIXTURES}: need >=3 fixtures with assert.sh (got {fixture_count})"),
                ),
            ],
        );
    }
    let bats = run_bats_suites(id, required, root, &[SUITE]);
    aggregate(id, required, vec![script, bats])
}

fn run_release_area_contract(id: &str, required: bool, root: &Path) -> CheckResult {
    const AREAS: &[&str] = &[
        "spec-completeness",
        "docs-freshness",
        "implementation-completeness",
        "test-coverage",
        "qa-artifact-integrity",
        "legacy-cleanup",
    ];
    const DIRECTORY: &str = "skills/autospec-release/areas";
    const SCRIPT: &str = "scripts/release-area-dispatch.sh";
    const TRIO: &[&str] = &[
        "skills/autospec-release/SKILL.md",
        "skills/autospec-release/codex/prompt.md",
        "skills/autospec-release/opencode/agent.md",
    ];
    const SUITE: &str = "tests/release/test_release_area_dispatch.bats";

    if !root.join(DIRECTORY).is_dir() {
        return failure(
            id,
            required,
            "skills/autospec-release/areas: directory missing",
        );
    }
    for area in AREAS {
        let path = root.join(DIRECTORY).join(format!("{area}.md"));
        if !path.is_file() {
            return failure(
                id,
                required,
                &format!("{}: required area file missing", path.display()),
            );
        }
    }
    let script = run_required_bash_scripts(id, required, root, &[(SCRIPT, true)]);
    if script.is_failure() {
        return script;
    }
    for trio in TRIO {
        if !contains(&root.join(trio), "release-area-dispatch.sh") {
            return aggregate(
                id,
                required,
                vec![
                    script,
                    failure(
                        id,
                        required,
                        &format!("{trio}: missing release-area-dispatch reference"),
                    ),
                ],
            );
        }
    }
    let mut results = vec![script];
    if program_on_path("bats") && root.join(SUITE).is_file() {
        results.push(bats_command(SUITE).execute_in(id, required, root));
    }
    aggregate(id, required, results)
}

fn run_release_worktree_assert(id: &str, required: bool, root: &Path) -> CheckResult {
    const TRIO: &[&str] = &[
        "skills/autospec-release/SKILL.md",
        "skills/autospec-release/codex/prompt.md",
        "skills/autospec-release/opencode/agent.md",
    ];
    const SUITE: &str = "tests/release/test_release_worktree_assert.bats";

    let mut expected_block = None;
    for trio in TRIO {
        let path = root.join(trio);
        if !path.is_file() {
            return failure(id, required, &format!("{trio}: required file missing"));
        }
        for anchor in [
            "## Worktree guard (commit gate)",
            "worktree-guard.sh assert",
            "MUST exit 0",
            "worktree-assert:begin",
            "worktree-assert:end",
            "git worktree remove",
            "git worktree prune",
        ] {
            if !contains(&path, anchor) {
                return failure(id, required, &format!("{trio}: missing {anchor} contract"));
            }
        }
        if !contains_case_insensitive(&path, "primary checkout") {
            return failure(
                id,
                required,
                &format!("{trio}: missing primary-checkout hard rule"),
            );
        }
        let Some(block) = inclusive_section(
            &path,
            "<!-- worktree-assert:begin -->",
            "<!-- worktree-assert:end -->",
        ) else {
            return failure(
                id,
                required,
                &format!("{trio}: missing worktree assertion block"),
            );
        };
        if let Some(expected) = &expected_block {
            if block != *expected {
                return failure(id, required, "release worktree assertion blocks differ");
            }
        } else {
            expected_block = Some(block);
        }
    }

    if program_on_path("bats") && root.join(SUITE).is_file() {
        return run_bats_suites(id, required, root, &[SUITE]);
    }
    CheckResult::completed(id, required, 0, 0, 0, 0, 0, output_digest(&[], &[]))
}

fn run_fab_container_pin_lint(id: &str, required: bool, root: &Path) -> CheckResult {
    const DOCKERFILE: &str = "skills/autospec-fab/docker/Dockerfile";
    const LINT: &str = "scripts/lint-fab-dockerfile.sh";

    if !root.join(DOCKERFILE).is_file() {
        return CheckResult::completed(id, required, 0, 0, 0, 0, 0, output_digest(&[], &[]));
    }
    let syntax = run_required_bash_scripts(id, required, root, &[(LINT, false)]);
    if syntax.is_failure() {
        return syntax;
    }
    for file in [
        DOCKERFILE,
        "skills/autospec-fab/docker/requirements.txt",
        "skills/autospec-fab/docker/wrappers/ccx_safety_wrapper.py",
        "skills/autospec-fab/docker/wrappers/foam_cfd_reduce.py",
    ] {
        if !root.join(file).is_file() {
            return aggregate(
                id,
                required,
                vec![
                    syntax,
                    failure(
                        id,
                        required,
                        &format!("{file}: required container file missing"),
                    ),
                ],
            );
        }
    }
    let lint = ToolCommand::new("bash", [LINT, DOCKERFILE])
        .expect("fab pin lint uses a direct Bash argument vector")
        .execute_in(id, required, root);
    aggregate(id, required, vec![syntax, lint])
}

#[derive(Clone, Copy)]
enum RepoQualityFailureMessage {
    SkillCallPoint,
    AnchorContract,
}

const REPO_QUALITY_AUDIT: &str = "skills/autospec-shared/scripts/repo-quality-audit.sh";
const REPO_QUALITY_CONTRACTS: &[(&str, &str, RepoQualityFailureMessage)] = &[
    (
        "skills/autospec-run/SKILL.md",
        "repo-quality-audit.sh",
        RepoQualityFailureMessage::SkillCallPoint,
    ),
    (
        "skills/autospec-qa/SKILL.md",
        "repo-quality-audit.sh",
        RepoQualityFailureMessage::SkillCallPoint,
    ),
    (
        "skills/autospec-review/SKILL.md",
        "repo-quality-audit.sh",
        RepoQualityFailureMessage::SkillCallPoint,
    ),
    (
        "skills/autospec-release/SKILL.md",
        "repo-quality-audit.sh",
        RepoQualityFailureMessage::SkillCallPoint,
    ),
    (
        "scripts/autospec-write-run-summary.sh",
        "--quality-audit-json",
        RepoQualityFailureMessage::AnchorContract,
    ),
    (
        REPO_QUALITY_AUDIT,
        "verification-contract-drift",
        RepoQualityFailureMessage::AnchorContract,
    ),
    (
        REPO_QUALITY_AUDIT,
        "verification:{",
        RepoQualityFailureMessage::AnchorContract,
    ),
    (
        REPO_QUALITY_AUDIT,
        "runtime:$runtime",
        RepoQualityFailureMessage::AnchorContract,
    ),
    (
        "scripts/autospec-write-run-summary.sh",
        "### Verification lanes",
        RepoQualityFailureMessage::AnchorContract,
    ),
    (
        "scripts/autospec-write-run-summary.sh",
        "### Runtime and engines",
        RepoQualityFailureMessage::AnchorContract,
    ),
];

fn repo_quality_failure_message(
    root: &Path,
    path: &Path,
    anchor: &str,
    message: RepoQualityFailureMessage,
) -> String {
    let mut rendered = relative_path(root, path);
    match message {
        RepoQualityFailureMessage::SkillCallPoint => {
            rendered.push_str(": missing repo quality audit call point");
        }
        RepoQualityFailureMessage::AnchorContract => {
            rendered.push_str(": missing ");
            rendered.push_str(anchor);
            rendered.push_str(" contract");
        }
    }
    rendered
}

fn run_repo_quality_audit(id: &str, required: bool, root: &Path) -> CheckResult {
    const SUITES: &[&str] = &[
        "skills/autospec-shared/tests/unit/repo-quality-audit.bats",
        "tests/autospec/test_quality_audit_summary.bats",
    ];

    let audit = run_required_bash_scripts(id, required, root, &[(REPO_QUALITY_AUDIT, true)]);
    if audit.is_failure() {
        return audit;
    }

    for &(relative, anchor, message) in REPO_QUALITY_CONTRACTS {
        let path = root.join(relative);
        if !contains(&path, anchor) {
            return aggregate(
                id,
                required,
                vec![
                    audit,
                    failure(
                        id,
                        required,
                        &repo_quality_failure_message(root, &path, anchor, message),
                    ),
                ],
            );
        }
    }
    let bats = run_bats_suites_if_available(id, required, root, SUITES);
    aggregate(id, required, vec![audit, bats])
}

fn run_autospec_autonomous_contract(id: &str, required: bool, root: &Path) -> CheckResult {
    const SKILLS: &[&str] = &[
        "autospec",
        "autospec-listen",
        "autospec-define",
        "autospec-run",
    ];
    const MEMBERS: &[&str] = &["SKILL.md", "codex/prompt.md", "opencode/agent.md"];
    const GATE: &str = "scripts/autospec-autonomy-gate.sh";
    const SUITE: &str = "tests/autospec/test_autonomous.bats";

    for skill in SKILLS {
        for member in MEMBERS {
            let relative = format!("skills/{skill}/{member}");
            let path = root.join(&relative);
            if !path.is_file() {
                return failure(
                    id,
                    required,
                    &format!("{relative}: required adapter file missing"),
                );
            }
            if !has_heading_prefix(&path, "## Autonomous mode") {
                return failure(
                    id,
                    required,
                    &format!("{relative}: missing Autonomous mode section"),
                );
            }
            if *skill == "autospec" {
                for heading in [
                    "### Autonomous spec drafting",
                    "### Safety guardrails (autonomous)",
                ] {
                    if !has_heading_prefix(&path, heading) {
                        return failure(
                            id,
                            required,
                            &format!("{relative}: missing {heading} section"),
                        );
                    }
                }
                if !contains(&path, "autospec-autonomy-gate.sh") {
                    return failure(
                        id,
                        required,
                        &format!("{relative}: missing autonomy gate reference"),
                    );
                }
            }
        }
    }
    let help = run_bash_help_usage(id, required, root, GATE);
    if help.is_failure() {
        return help;
    }
    let bats = run_bats_suites(id, required, root, &[SUITE]);
    aggregate(id, required, vec![help, bats])
}

fn run_dogfood_detectors(id: &str, required: bool, root: &Path) -> CheckResult {
    const SCRIPTS: &[(&str, bool)] = &[
        ("scripts/dogfood-detectors.sh", true),
        ("scripts/dogfood-adapter-doc-drift.sh", true),
        ("scripts/dogfood-adapter-lint.sh", true),
    ];
    const OPTIONAL_SUITES: &[&str] = &[
        "tests/dogfood/test_driver.bats",
        "tests/dogfood/test_adapter_layer.bats",
    ];

    for member in ["SKILL.md", "codex/prompt.md", "opencode/agent.md"] {
        let relative = format!("skills/autospec-qa/{member}");
        let path = root.join(&relative);
        if !has_heading_prefix(&path, "## Dogfood detectors driver") {
            return failure(
                id,
                required,
                &format!("{relative}: missing Dogfood detectors driver section"),
            );
        }
        if !contains(&path, "dogfood-detectors.sh") {
            return failure(
                id,
                required,
                &format!("{relative}: missing reference to dogfood-detectors.sh"),
            );
        }
    }
    let registry = root.join(".autospec/dogfood.yml");
    if !registry.is_file() || !has_line_prefix(&registry, "adapters:") {
        return failure(
            id,
            required,
            ".autospec/dogfood.yml: missing adapters schema",
        );
    }

    let syntax = run_required_bash_scripts(id, required, root, SCRIPTS);
    if syntax.is_failure() {
        return syntax;
    }
    let driver = ToolCommand::new("bash", ["scripts/dogfood-detectors.sh"])
        .expect("dogfood driver uses a direct Bash argument vector")
        .execute_in(id, required, root);
    if driver.is_failure() {
        return aggregate(id, required, vec![syntax, driver]);
    }

    let existing_suites = OPTIONAL_SUITES
        .iter()
        .copied()
        .filter(|suite| root.join(suite).is_file())
        .collect::<Vec<_>>();
    let bats = if existing_suites.is_empty() || !program_on_path("bats") {
        CheckResult::completed(id, required, 0, 0, 0, 0, 0, output_digest(&[], &[]))
    } else {
        run_bats_suites(id, required, root, &existing_suites)
    };
    aggregate(id, required, vec![syntax, driver, bats])
}

fn run_autospec_parallel_dispatch(id: &str, required: bool, root: &Path) -> CheckResult {
    const HELPER: &str = "scripts/dispatch-implementer.sh";
    const SUITES: &[&str] = &[
        "tests/autospec-run/test_parallel_dispatch.bats",
        "tests/autospec-run/test_batch_claim_startup_evidence.bats",
    ];

    for member in ["SKILL.md", "codex/prompt.md", "opencode/agent.md"] {
        let relative = format!("skills/autospec-run/{member}");
        let path = root.join(&relative);
        if !contains(&path, "### Parallel implementer worktree isolation") {
            return failure(
                id,
                required,
                &format!("{relative}: missing Parallel implementer worktree isolation subsection"),
            );
        }
        if !contains(&path, "dispatch-implementer.sh") {
            return failure(
                id,
                required,
                &format!("{relative}: missing dispatch-implementer.sh reference"),
            );
        }
    }

    let syntax = run_required_bash_scripts(id, required, root, &[(HELPER, true)]);
    if syntax.is_failure() {
        return syntax;
    }
    for suite in SUITES {
        if !root.join(suite).is_file() {
            return aggregate(
                id,
                required,
                vec![
                    syntax,
                    failure(id, required, &format!("{suite}: bats coverage missing")),
                ],
            );
        }
    }
    if !program_on_path("bats") {
        return syntax;
    }

    let bats = run_commands(
        id,
        required,
        root,
        [
            bats_command(SUITES[0]),
            bats_command(SUITES[1]).without_env("AUTOSPEC_RUN_ONLY_ISSUES"),
        ],
    );
    aggregate(id, required, vec![syntax, bats])
}

fn run_growth_shared(id: &str, required: bool, root: &Path) -> CheckResult {
    const SCRIPTS: &[&str] = &[
        "skills/autospec-shared/scripts/validate-growth-config.sh",
        "skills/autospec-shared/scripts/growth-ethics-blocklist.sh",
        "skills/autospec-shared/scripts/growth-ethics-precheck.sh",
        "skills/autospec-shared/scripts/growth-ledger.sh",
        "skills/autospec-shared/scripts/growth-source-weights.sh",
        "skills/autospec-shared/scripts/growth-measure.sh",
        "skills/autospec-shared/scripts/growth-measure-due.sh",
    ];
    const SUITES: &[&str] = &[
        "tests/unit/growth-config-validate.bats",
        "tests/unit/growth-ethics-blocklist.bats",
        "tests/unit/growth-ethics-precheck.bats",
        "tests/unit/growth-ledger.bats",
        "tests/unit/growth-source-weights.bats",
        "tests/unit/growth-measure.bats",
        "tests/unit/growth-measure-due.bats",
    ];
    run_scripts_with_optional_bats(id, required, root, SCRIPTS, SUITES)
}

fn run_growth_candidate_pipeline(id: &str, required: bool, root: &Path) -> CheckResult {
    const SCRIPTS: &[&str] = &[
        "skills/autospec-shared/scripts/validate-growth-candidate.sh",
        "skills/autospec-shared/scripts/growth-candidate-dedup.sh",
        "skills/autospec-shared/scripts/growth-candidate-rank.sh",
        "skills/autospec-shared/scripts/growth-candidate-verify.sh",
    ];
    const SUITES: &[&str] = &[
        "tests/unit/growth-candidate-validate.bats",
        "tests/unit/growth-candidate-dedup.bats",
        "tests/unit/growth-candidate-rank.bats",
        "tests/unit/growth-candidate-verify.bats",
        "tests/unit/growth-candidate-pipeline-integration.bats",
    ];
    run_scripts_with_optional_bats(id, required, root, SCRIPTS, SUITES)
}

fn run_grow_run_pipeline(id: &str, required: bool, root: &Path) -> CheckResult {
    const SCRIPTS: &[&str] = &[
        "skills/autospec-shared/scripts/validate-outbound-draft.sh",
        "skills/autospec-shared/scripts/growth-content-quality-precheck.sh",
        "skills/autospec-shared/scripts/growth-outbound-queue.sh",
        "skills/autospec-shared/scripts/growth-attribute.sh",
        "skills/autospec-shared/scripts/growth-adapter-github.sh",
        "skills/autospec-shared/scripts/growth-adapter-analytics.sh",
        "skills/autospec-shared/scripts/growth-adapter-gsc.sh",
        "skills/autospec-shared/scripts/growth-adapter-rank.sh",
    ];
    const SUITES: &[&str] = &[
        "tests/unit/validate-outbound-draft.bats",
        "tests/unit/growth-content-quality-precheck.bats",
        "tests/unit/growth-outbound-queue.bats",
        "tests/unit/growth-adapters.bats",
        "tests/unit/growth-attribute.bats",
        "tests/unit/growth-measure.bats",
    ];
    run_scripts_with_optional_bats(id, required, root, SCRIPTS, SUITES)
}

fn run_db_telemetry(id: &str, required: bool, root: &Path) -> CheckResult {
    const SCRIPT: &str = "skills/autospec-shared/scripts/emit-event.sh";
    const REQUIRED_FILES: &[&str] = &[
        "tests/unit/emit-event-wiring.bats",
        "skills/autospec-db-doctor/scripts/db-doctor.sh",
        "tests/unit/autospec-db-doctor.bats",
    ];
    const SUITES: &[&str] = &[
        "tests/unit/emit-event.bats",
        "tests/unit/emit-event-wiring.bats",
        "tests/unit/autospec-db-doctor.bats",
    ];

    for file in REQUIRED_FILES {
        if !root.join(file).is_file() {
            return failure(id, required, &format!("{file}: required file missing"));
        }
    }
    let syntax = run_required_bash_scripts(id, required, root, &[(SCRIPT, false)]);
    if syntax.is_failure() {
        return syntax;
    }
    let bats = run_bats_suites_if_available(id, required, root, SUITES);
    aggregate(id, required, vec![syntax, bats])
}

fn run_worktree_ladder_assert_parity(id: &str, required: bool, root: &Path) -> CheckResult {
    const TRIO_MEMBERS: &[&str] = &["SKILL.md", "codex/prompt.md", "opencode/agent.md"];
    const REQUIRED_TOKENS: &[&str] = &[
        "worktree-guard.sh resolve-branch",
        "open-pr",
        "branch-only",
        "adopt",
        "worktree-guard.sh assert",
        "MUST exit 0",
        "git worktree remove",
        "git worktree prune",
    ];

    let mut targets = Vec::new();
    for skill in ["autospec-run"] {
        for member in TRIO_MEMBERS {
            targets.push(format!("skills/{skill}/{member}"));
        }
    }
    targets.push("skills/autospec-run/prompts/phase4-implementer.md".to_string());

    for relative in &targets {
        let path = root.join(relative);
        if !path.is_file() {
            return failure(id, required, &format!("{relative}: required file missing"));
        }
        for token in REQUIRED_TOKENS {
            if !contains(&path, token) {
                return failure(id, required, &format!("{relative}: missing {token}"));
            }
        }
        if !contains_word(&path, "fresh") {
            return failure(
                id,
                required,
                &format!("{relative}: missing fresh ladder state"),
            );
        }
        if !contains_case_insensitive(&path, "skip implementation") {
            return failure(
                id,
                required,
                &format!("{relative}: missing skip implementation"),
            );
        }
        if !contains_case_insensitive(&path, "primary checkout") {
            return failure(
                id,
                required,
                &format!("{relative}: missing primary checkout rule"),
            );
        }
    }

    let mut results = Vec::new();
    for relative in ["skills/autospec-run/SKILL.md"] {
        let path = root.join(relative);
        let Some(block) = worktree_ladder_block(&path) else {
            return aggregate(
                id,
                required,
                vec![failure(
                    id,
                    required,
                    &format!("{relative}: missing worktree-ladder sentinel block"),
                )],
            );
        };
        let syntax = ToolCommand::new("bash", ["-n", "-"])
            .expect("worktree ladder syntax uses a direct Bash argument vector")
            .execute_in_with_stdin_capturing(id, required, root, block.as_bytes())
            .result;
        results.push(syntax);
    }
    aggregate(id, required, results)
}

fn run_phase4_single_agent_discipline(id: &str, required: bool, root: &Path) -> CheckResult {
    const CANONICAL: &str =
        "Subagents spawned by background `Agent` calls do NOT inherit the `Agent` tool";
    const SUITE: &str = "tests/phase4/test_single_agent_discipline.bats";

    // Router delegates Phase 4 to /autospec-run.
    for skill in ["autospec-run"] {
        for member in ["SKILL.md", "codex/prompt.md", "opencode/agent.md"] {
            let relative = format!("skills/{skill}/{member}");
            let path = root.join(&relative);
            for token in [
                CANONICAL,
                "single-agent absorbed-discipline",
                "top-level agents launched directly by the main session orchestrator",
            ] {
                if !contains(&path, token) {
                    return failure(id, required, &format!("{relative}: missing {token}"));
                }
            }
            for retired in [
                "process(ISSUE)   # foreground subagent",
                "dispatches a **foreground subagent**",
            ] {
                if contains(&path, retired) {
                    return failure(
                        id,
                        required,
                        &format!("{relative}: contains retired {retired}"),
                    );
                }
            }
        }
    }

    let prompt = root.join("skills/autospec-run/prompts/phase4-implementer.md");
    for token in ["single-agent", CANONICAL] {
        if !contains(&prompt, token) {
            return failure(
                id,
                required,
                &format!("skills/autospec-run/prompts/phase4-implementer.md: missing {token}"),
            );
        }
    }
    run_bats_suites(id, required, root, &[SUITE])
}

fn run_phase4_final_quality_gate(id: &str, required: bool, root: &Path) -> CheckResult {
    const SUITE: &str = "tests/unit/test_final_quality_gate.bats";
    for member in ["SKILL.md", "codex/prompt.md", "opencode/agent.md"] {
        let relative = format!("skills/autospec-run/{member}");
        let path = root.join(&relative);
        for token in [
            "Final quality gate",
            "cargo clippy --workspace --all-targets -- -D warnings",
            "FINAL_QUALITY_GATE_FAILED",
            "`crate`, `file`, `line`, and `rule` fields",
            "Do NOT run `gh pr merge` while the final quality gate is failing",
        ] {
            if !contains(&path, token) {
                return failure(id, required, &format!("{relative}: missing {token}"));
            }
        }
    }
    run_bats_suites(id, required, root, &[SUITE])
}

fn run_autospec_refine_contract(id: &str, required: bool, root: &Path) -> CheckResult {
    const SCRIPTS: &[(&str, bool)] = &[
        ("scripts/refine-prompt.sh", true),
        ("scripts/lib/extract-matchers.sh", false),
        ("scripts/refine-prompt-lens-llm.sh", true),
        ("scripts/refine-render-overview.sh", true),
    ];
    const SCHEMAS: &[&str] = &[
        "schemas/autospec-refinement.schema.json",
        "schemas/autospec-refinement-loop.schema.json",
    ];

    let syntax = run_required_bash_scripts(id, required, root, SCRIPTS);
    if syntax.is_failure() {
        return syntax;
    }
    for schema in SCHEMAS {
        if !root.join(schema).is_file() {
            return aggregate(
                id,
                required,
                vec![
                    syntax,
                    failure(id, required, &format!("{schema}: file missing")),
                ],
            );
        }
    }
    for member in ["SKILL.md", "codex/prompt.md", "opencode/agent.md"] {
        let relative = format!("skills/autospec-refine/{member}");
        if !root.join(&relative).is_file() {
            return aggregate(
                id,
                required,
                vec![
                    syntax,
                    failure(id, required, &format!("{relative}: file missing")),
                ],
            );
        }
    }
    // Router delegates Phase 6; Next steps directive lives in end-of-run ref.
    let next_steps_ref = "skills/autospec-run/references/end-of-run.md";
    if !contains(&root.join(next_steps_ref), "canonical `## Next steps` section") {
        return aggregate(
            id,
            required,
            vec![
                syntax,
                failure(
                    id,
                    required,
                    &format!("{next_steps_ref}: missing canonical Next steps directive"),
                ),
            ],
        );
    }

    let schema = run_optional_jsonschema_checks(id, required, root, SCHEMAS);
    if schema.is_failure() {
        return aggregate(id, required, vec![syntax, schema]);
    }
    let bats =
        run_matching_bats_suites_if_available(id, required, root, "tests/refine", "test_refine_");
    aggregate(id, required, vec![syntax, schema, bats])
}

fn run_autospec_continue_contract(id: &str, required: bool, root: &Path) -> CheckResult {
    const SCRIPTS: &[(&str, bool)] = &[
        ("scripts/autospec-continue.sh", true),
        ("scripts/extract-conversational-recommendation.sh", true),
        ("scripts/lib/extract-matchers.sh", false),
    ];
    for member in ["SKILL.md", "codex/prompt.md"] {
        let relative = format!("skills/autospec-continue/{member}");
        if !root.join(&relative).is_file() {
            return failure(id, required, &format!("{relative}: file missing"));
        }
    }
    let syntax = run_required_bash_scripts(id, required, root, SCRIPTS);
    if syntax.is_failure() {
        return syntax;
    }
    for member in ["SKILL.md", "codex/prompt.md", "opencode/agent.md"] {
        let relative = format!("skills/autospec-continue/{member}");
        let path = root.join(&relative);
        if !path.is_file() {
            continue;
        }
        for token in [
            "## Continuous loop mode",
            "--no-loop",
            "continue-no-loop.flag",
        ] {
            if !contains(&path, token) {
                return aggregate(
                    id,
                    required,
                    vec![
                        syntax,
                        failure(id, required, &format!("{relative}: missing {token}")),
                    ],
                );
            }
        }
    }
    let main = root.join("scripts/autospec-continue.sh");
    for token in [
        "--no-loop|--once",
        "continue-no-loop.flag",
        "autospec_loop_run",
    ] {
        if !contains(&main, token) {
            return aggregate(
                id,
                required,
                vec![
                    syntax,
                    failure(
                        id,
                        required,
                        &format!("scripts/autospec-continue.sh: missing {token}"),
                    ),
                ],
            );
        }
    }
    let bats = run_matching_bats_suites_if_available(
        id,
        required,
        root,
        "tests/continue",
        "test_continue_",
    );
    aggregate(id, required, vec![syntax, bats])
}

fn run_autospec_loop_contract(id: &str, required: bool, root: &Path) -> CheckResult {
    const LIBRARY: &str = "scripts/lib/autospec-loop.sh";
    const SUITE: &str = "tests/autospec/test_autospec_loop.bats";
    let syntax = run_required_bash_scripts(id, required, root, &[(LIBRARY, false)]);
    if syntax.is_failure() {
        return syntax;
    }
    for token in ["autospec_loop_run()", "autospec_loop_harvest_next_prompt()"] {
        if !contains(&root.join(LIBRARY), token) {
            return aggregate(
                id,
                required,
                vec![
                    syntax,
                    failure(id, required, &format!("{LIBRARY}: missing {token}")),
                ],
            );
        }
    }
    for script in ["scripts/refine-prompt.sh", "scripts/autospec-continue.sh"] {
        if !contains(&root.join(script), "lib/autospec-loop.sh") {
            return aggregate(
                id,
                required,
                vec![
                    syntax,
                    failure(
                        id,
                        required,
                        &format!("{script}: missing loop library source"),
                    ),
                ],
            );
        }
    }
    for member in ["SKILL.md", "codex/prompt.md", "opencode/agent.md"] {
        let relative = format!("skills/autospec/{member}");
        let path = root.join(&relative);
        if !path.is_file() {
            return aggregate(
                id,
                required,
                vec![
                    syntax,
                    failure(id, required, &format!("{relative}: file missing")),
                ],
            );
        }
        for token in ["## Continuous loop mode (--loop)", "evidence_based_stop"] {
            if !contains(&path, token) {
                return aggregate(
                    id,
                    required,
                    vec![
                        syntax,
                        failure(id, required, &format!("{relative}: missing {token}")),
                    ],
                );
            }
        }
        if !contains(&path, "autospec_loop_run") && !contains(&path, "lib/autospec-loop.sh") {
            return aggregate(
                id,
                required,
                vec![
                    syntax,
                    failure(
                        id,
                        required,
                        &format!("{relative}: missing loop driver reference"),
                    ),
                ],
            );
        }
    }
    let bats = run_bats_suites(id, required, root, &[SUITE]);
    aggregate(id, required, vec![syntax, bats])
}

fn run_autospec_resume_structure(id: &str, required: bool, root: &Path) -> CheckResult {
    const SKILL: &str = "skills/autospec-resume";
    const MEMBERS: &[&str] = &[
        "SKILL.md",
        "README.md",
        "install.sh",
        "uninstall.sh",
        "validate.sh",
        "opencode/agent.md",
        "codex/prompt.md",
        "scripts/resume-scan.sh",
        "scripts/resume-attempts.sh",
    ];
    const SCRIPTS: &[(&str, bool)] = &[
        ("skills/autospec-resume/scripts/resume-scan.sh", true),
        ("skills/autospec-resume/scripts/resume-attempts.sh", true),
        ("scripts/autospec-run-registry.sh", true),
    ];
    if !root.join(SKILL).is_dir() {
        return failure(
            id,
            required,
            "skills/autospec-resume: skill directory missing",
        );
    }
    for member in MEMBERS {
        let relative = format!("{SKILL}/{member}");
        if !root.join(&relative).is_file() {
            return failure(id, required, &format!("{relative}: required file missing"));
        }
    }
    let skill_file = root.join("skills/autospec-resume/SKILL.md");
    let valid_startup = (has_heading_prefix(&skill_file, "## Startup self-update")
        && contains(&skill_file, "SKILL_NAME=autospec-resume"))
        || contains(
            &skill_file,
            "autospec-block:startup-self-update SKILL_NAME=autospec-resume",
        );
    if !valid_startup {
        return failure(
            id,
            required,
            "skills/autospec-resume/SKILL.md: missing compatible startup self-update section",
        );
    }
    for token in [
        "## Harness detection",
        "## Required capabilities & harness adapter",
        "Subagent model tier",
    ] {
        if !contains(&skill_file, token) {
            return failure(
                id,
                required,
                &format!("skills/autospec-resume/SKILL.md: missing {token}"),
            );
        }
    }
    run_required_bash_scripts(id, required, root, SCRIPTS)
}

fn run_autospec_supervisor_structure(id: &str, required: bool, root: &Path) -> CheckResult {
    const SUPERVISOR: &str = "scripts/autospec-supervisor.sh";
    const INSTALLER: &str = "scripts/autospec-supervisor-install.sh";
    let syntax =
        run_required_bash_scripts(id, required, root, &[(SUPERVISOR, true), (INSTALLER, true)]);
    if syntax.is_failure() {
        return syntax;
    }
    let supervisor = root.join(SUPERVISOR);
    if !contains(&supervisor, "resume-scan.sh") {
        return aggregate(
            id,
            required,
            vec![
                syntax,
                failure(id, required, "supervisor: missing resume-scan delegation"),
            ],
        );
    }
    if contains_unsafe_supervisor_eval(&supervisor) {
        return aggregate(
            id,
            required,
            vec![
                syntax,
                failure(id, required, "supervisor: unsafe resume command eval"),
            ],
        );
    }
    if !contains(&supervisor, "confirm_open_run") && !contains(&supervisor, "in-progress-by-bot") {
        return aggregate(
            id,
            required,
            vec![
                syntax,
                failure(id, required, "supervisor: missing open-run confirmation"),
            ],
        );
    }
    let installer = root.join(INSTALLER);
    for token in [
        "launchd",
        "systemd",
        "@reboot",
        "install)",
        "uninstall)",
        "status)",
    ] {
        if !contains(&installer, token) {
            return aggregate(
                id,
                required,
                vec![
                    syntax,
                    failure(id, required, &format!("installer: missing {token}")),
                ],
            );
        }
    }
    syntax
}

fn run_autospec_resume_contract(id: &str, required: bool, root: &Path) -> CheckResult {
    let structure = run_autospec_resume_structure(id, required, root);
    if structure.is_failure() {
        return structure;
    }
    let supervisor = run_autospec_supervisor_structure(id, required, root);
    if supervisor.is_failure() {
        return aggregate(id, required, vec![structure, supervisor]);
    }
    let scan = root.join("skills/autospec-resume/scripts/resume-scan.sh");
    if contains(&scan, "claim state") && (contains(&scan, "upsert") || contains(&scan, "clear"))
        || contains(&scan, "gh issue comment")
    {
        return aggregate(
            id,
            required,
            vec![
                structure,
                supervisor,
                failure(
                    id,
                    required,
                    "resume scan must not mutate the durable run state",
                ),
            ],
        );
    }
    if !contains(&scan, "updated_at") {
        return aggregate(
            id,
            required,
            vec![
                structure,
                supervisor,
                failure(id, required, "resume scan: missing updated_at"),
            ],
        );
    }
    if !contains(
        &root.join("skills/autospec-run/scripts/heartbeat-write.sh"),
        "\"host\"",
    ) {
        return aggregate(
            id,
            required,
            vec![
                structure,
                supervisor,
                failure(id, required, "heartbeat write: missing host field"),
            ],
        );
    }
    for member in ["SKILL.md", "codex/prompt.md", "opencode/agent.md"] {
        let relative = format!("skills/autospec-run/{member}");
        if !contains(&root.join(&relative), "autospec-run-registry.sh") {
            return aggregate(
                id,
                required,
                vec![
                    structure,
                    supervisor,
                    failure(
                        id,
                        required,
                        &format!("{relative}: missing registry wiring"),
                    ),
                ],
            );
        }
    }
    let validate = ToolCommand::new("bash", ["skills/autospec-resume/validate.sh"])
        .expect("resume skill validation uses a direct Bash argument vector")
        .execute_in(id, required, root);
    if validate.is_failure() {
        return aggregate(id, required, vec![structure, supervisor, validate]);
    }
    if !root.join("tests/resume").is_dir() {
        return aggregate(
            id,
            required,
            vec![
                structure,
                supervisor,
                validate,
                failure(
                    id,
                    required,
                    "tests/resume: bats coverage directory missing",
                ),
            ],
        );
    }
    let bats = run_matching_bats_suites_if_available(id, required, root, "tests/resume", "");
    aggregate(id, required, vec![structure, supervisor, validate, bats])
}

fn run_implementer_contract(id: &str, required: bool, root: &Path) -> CheckResult {
    const CONTRACT: &str = "skills/autospec-run/prompts/implementer-contract.md";
    const BUNDLER: &str = "skills/autospec-shared/scripts/bundle-static-context.sh";
    let contract = root.join(CONTRACT);
    let rule_ids = match prompt_contract_rule_ids(id, required, root, &contract) {
        Ok(rule_ids) => rule_ids,
        Err(result) => return result,
    };
    let bundler = root.join(BUNDLER);
    for token in ["implementer-contract.md", "IMPLEMENTER_CONTRACT"] {
        if !contains(&bundler, token) {
            return failure(id, required, &format!("{BUNDLER}: missing {token}"));
        }
    }
    let captured = bundled_prompt(id, required, root, "implementer");
    if captured.result.is_failure() {
        return captured.result;
    }
    if contains_bytes(&captured.stdout, b"SKILL.md (implementer role)") {
        return captured_failure(
            captured.result,
            &captured.stdout,
            "bundler: implementer output still injects the SKILL.md prefix",
        );
    }
    if rule_ids.is_empty() {
        return captured_failure(
            captured.result,
            &captured.stdout,
            "AGENTS.md: RULE_ID table yielded no RULE_IDs",
        );
    }
    captured.result
}

fn run_reviewer_contract(id: &str, required: bool, root: &Path) -> CheckResult {
    const CONTRACT: &str = "skills/autospec-run/prompts/reviewer-contract.md";
    const BUNDLER: &str = "skills/autospec-shared/scripts/bundle-static-context.sh";
    let contract = root.join(CONTRACT);
    if let Err(result) = prompt_contract_rule_ids(id, required, root, &contract) {
        return result;
    }
    let bundler = root.join(BUNDLER);
    if !contains(&bundler, "REVIEWER_CONTRACT") {
        return failure(
            id,
            required,
            "bundler: missing REVIEWER_CONTRACT injection variable",
        );
    }
    let captured = bundled_prompt(id, required, root, "reviewer");
    if captured.result.is_failure() {
        return captured.result;
    }
    if contains_bytes(&captured.stdout, b"SKILL.md (reviewer role)") {
        return captured_failure(
            captured.result,
            &captured.stdout,
            "bundler: reviewer output still injects the SKILL.md prefix",
        );
    }
    let output = String::from_utf8_lossy(&captured.stdout);
    if output
        .lines()
        .filter(|line| line.starts_with("### RULE_ID table"))
        .count()
        != 1
    {
        return captured_failure(
            captured.result,
            &captured.stdout,
            "bundler: reviewer output must contain exactly one RULE_ID table",
        );
    }
    for rule_id in [
        "HALLUCINATED_API",
        "DUPLICATE_CODE",
        "STRING_MATCH_DOMAIN_LOGIC",
        "REPEATED_STRUCTURE_AS_CODE",
        "DOC_OUT_OF_SYNC",
        "INVENTED_CONFIG",
    ] {
        if !output.contains(rule_id) {
            return captured_failure(
                captured.result,
                &captured.stdout,
                &format!("bundler: reviewer output missing {rule_id}"),
            );
        }
    }
    captured.result
}

fn run_python_suites(id: &str, required: bool, root: &Path) -> CheckResult {
    if !program_on_path("python3") {
        return failure(
            id,
            required,
            "python suites: python3 unavailable; cannot gate context-monitor suites",
        );
    }
    let availability = ToolCommand::new("python3", ["-m", "pytest", "--version"])
        .expect("pytest availability uses a direct Python argument vector")
        .execute_in(id, required, root);
    if availability.is_failure() {
        return availability;
    }
    let suites = ToolCommand::new(
        "python3",
        [
            "-m",
            "pytest",
            "packages",
            "tests",
            "-q",
            "-p",
            "no:cacheprovider",
        ],
    )
    .expect("pytest suites use a direct Python argument vector")
    .with_isolated_pytest("packages/autospec_context_monitor")
    .execute_in(id, required, root);
    aggregate(id, required, vec![availability, suites])
}

fn run_grow_define_contract(id: &str, required: bool, root: &Path) -> CheckResult {
    const SKILL: &str = "skills/autospec-grow-define";
    const DERIVE_TRIO: &str = "scripts/derive-trio.sh";
    const SCRIPTS: &[&str] = &[
        "skills/autospec-shared/scripts/grow-define-pipeline.sh",
        "skills/autospec-shared/scripts/grow-define-file-issues.sh",
    ];
    const SUITES: &[&str] = &[
        "tests/unit/grow-define-pipeline.bats",
        "tests/unit/grow-define-file-issues.bats",
        "tests/unit/filing-origin-self-explore-growth.bats",
        "tests/unit/qa-filing-origin-self.bats",
        "tests/autospec-grow-define/smoke.bats",
    ];

    let syntax = run_required_bash_scripts(
        id,
        required,
        root,
        &SCRIPTS
            .iter()
            .map(|script| (*script, false))
            .collect::<Vec<_>>(),
    );
    if syntax.is_failure() {
        return syntax;
    }

    let derive_path = root.join(DERIVE_TRIO);
    if !is_executable(&derive_path) {
        return aggregate(
            id,
            required,
            vec![
                syntax,
                failure(
                    id,
                    required,
                    "scripts/derive-trio.sh missing or not executable",
                ),
            ],
        );
    }
    let derived = ToolCommand::new(DERIVE_TRIO, [SKILL, "--check"])
        .expect("derive-trio uses direct static arguments")
        .execute_in(id, required, root);
    if derived.is_failure() {
        return aggregate(id, required, vec![syntax, derived]);
    }

    for member in ["SKILL.md", "codex/prompt.md", "opencode/agent.md"] {
        let path = root.join(SKILL).join(member);
        if !path.is_file() {
            return aggregate(
                id,
                required,
                vec![
                    syntax,
                    derived,
                    failure(id, required, &format!("{SKILL}/{member}: missing")),
                ],
            );
        }
        for heading in ["## Self-update mode", "## Lens roster"] {
            if !has_heading_prefix(&path, heading) {
                return aggregate(
                    id,
                    required,
                    vec![
                        syntax,
                        derived,
                        failure(
                            id,
                            required,
                            &format!("{SKILL}/{member}: missing {heading}"),
                        ),
                    ],
                );
            }
        }
        if !contains(&path, "**Model tier:**") {
            return aggregate(
                id,
                required,
                vec![
                    syntax,
                    derived,
                    failure(
                        id,
                        required,
                        &format!("{SKILL}/{member}: missing **Model tier:** directive"),
                    ),
                ],
            );
        }
    }
    let skill = root.join(SKILL).join("SKILL.md");
    for lens in [
        "technical-seo",
        "keyword-gap",
        "content-opportunity",
        "community",
        "directory",
        "backlink",
    ] {
        if !contains(&skill, lens) {
            return aggregate(
                id,
                required,
                vec![
                    syntax,
                    derived,
                    failure(
                        id,
                        required,
                        &format!("{SKILL}/SKILL.md: lens {lens} not named in roster"),
                    ),
                ],
            );
        }
    }

    let bats = run_bats_suites_if_available(id, required, root, SUITES);
    aggregate(id, required, vec![syntax, derived, bats])
}

fn run_autospec_doc_contract(id: &str, required: bool, root: &Path) -> CheckResult {
    const SKILL: &str = "skills/autospec-doc";
    const ORCHESTRATOR: &str = "skills/autospec-doc/scripts/doc-orchestrator.mjs";
    const SUITE: &str = "skills/autospec-doc/tests/doc-orchestrator.bats";

    if !root.join(SKILL).is_dir() {
        return failure(id, required, "skills/autospec-doc: directory missing");
    }
    for member in ["SKILL.md", "codex/prompt.md", "opencode/agent.md"] {
        let path = root.join(SKILL).join(member);
        if !path.is_file() {
            return failure(
                id,
                required,
                &format!("{SKILL}/{member}: required file missing"),
            );
        }
        if !has_heading_prefix(&path, "## Subcommand contract") {
            return failure(
                id,
                required,
                &format!("{SKILL}/{member} missing '## Subcommand contract' section"),
            );
        }
        for subcommand in ["--full", "--audit", "--audience"] {
            if !contains(&path, subcommand) {
                return failure(
                    id,
                    required,
                    &format!("{SKILL}/{member} missing {subcommand} subcommand"),
                );
            }
        }
        if !contains(&path, "/autospec-doc init") && !contains(&path, "`init`") {
            return failure(
                id,
                required,
                &format!("{SKILL}/{member} missing init subcommand"),
            );
        }
        for anchor in [
            "worktree-guard.sh assert",
            "MUST exit 0",
            "doc-regen-assert:begin",
        ] {
            if !contains(&path, anchor) {
                return failure(id, required, &format!("{SKILL}/{member}: missing {anchor}"));
            }
        }
    }
    let orchestrator = root.join(ORCHESTRATOR);
    if !orchestrator.is_file() {
        return failure(
            id,
            required,
            &format!("{ORCHESTRATOR}: orchestrator stub missing"),
        );
    }

    let mut results = Vec::new();
    if program_on_path("node") {
        let syntax = ToolCommand::new("node", ["--check", ORCHESTRATOR])
            .expect("Node syntax validation uses direct static arguments")
            .execute_in(id, required, root);
        if syntax.is_failure() {
            return syntax;
        }
        results.push(syntax);

        let gate_directory = match doc_contract_gate_directory() {
            Ok(directory) => directory,
            Err(error) => return aggregate(id, required, vec![failure(id, required, &error)]),
        };
        let gate = ToolCommand::new(
            "node",
            vec![orchestrator.into_os_string(), OsString::from("--full")],
        )
        .expect("documentation gate uses direct static arguments")
        .execute_in(id, required, &gate_directory);
        let _ = fs::remove_dir_all(&gate_directory);
        if gate.exit_code != Some(2) {
            results.push(gate);
            results.push(failure(
                id,
                required,
                "doc orchestrator: non-init subcommand with no documentation config must exit 2",
            ));
            return aggregate(id, required, results);
        }
        results.push(CheckResult {
            exit_code: Some(0),
            ..gate
        });
    }

    let bats = run_bats_suites(id, required, root, &[SUITE]);
    results.push(bats);
    aggregate(id, required, results)
}

fn doc_contract_gate_directory() -> Result<PathBuf, String> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("doc orchestrator: clock unavailable: {error}"))?
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "autospec-doc-contract-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir(&directory).map_err(|error| {
        format!("doc orchestrator: could not create isolated config-gate directory: {error}")
    })?;
    Ok(directory)
}

fn run_constitution_validation(id: &str, required: bool, root: &Path) -> CheckResult {
    const PYTHON_SCRIPTS: &[&str] = &[
        "scripts/autospec-constitution-rules.py",
        "scripts/autospec-digital-twin.py",
    ];
    const BASH_SCRIPTS: &[&str] = &[
        "scripts/autospec-constitution-validate.sh",
        "scripts/autospec-baseline-compose.sh",
        "scripts/autospec-discover-metadata.sh",
        "scripts/autospec-baseline-gap.sh",
        "scripts/autospec-constitutional-gap.sh",
        "scripts/autospec-plan-issues.sh",
        "scripts/autospec-bot-state-init.sh",
        "scripts/autospec-autonomy-dry-run.sh",
        "scripts/autospec-ensure-labels.sh",
        "scripts/autospec-publish-issues.sh",
        "scripts/autospec-sync-published-issues.sh",
        "scripts/autospec-worker-v1.sh",
        "scripts/autospec-verify-worker-pr.sh",
        "scripts/autospec-promote-pr.sh",
        "scripts/autospec-plan-remediation.sh",
        "scripts/autospec-worker-one.sh",
        "scripts/autospec-publish-stuck.sh",
        "scripts/autospec-sync-guidance.sh",
        "scripts/autospec-supervisor-cycle.sh",
        "scripts/autospec-autonomy-status.sh",
        "scripts/autospec-supervisor-loop.sh",
        "scripts/autospec-autonomy-budget.sh",
        "scripts/autospec-repeated-failures.sh",
        "scripts/autospec-resume.sh",
        "scripts/autospec-guide-issue.sh",
        "scripts/autospec-build-digital-twin.sh",
        "scripts/autospec-impact-analysis.sh",
        "scripts/autospec-metadata-drift.sh",
        "scripts/autospec-extract-constitution-rules.sh",
        "scripts/autospec-check-rules.sh",
        "scripts/autospec-constitutional-gap-v1.sh",
        "scripts/autospec-constitution-audit.sh",
        "scripts/autospec-load-policy-sources.sh",
        "scripts/autospec-validate-policy-sources.sh",
        "scripts/autospec-lock-policy-sources.sh",
        "scripts/autospec-policy-compatibility.sh",
    ];
    const SUITES: &[&str] = &[
        "tests/unit/test_constitution_validation.bats",
        "tests/unit/test_baseline_composition.bats",
        "tests/unit/test_repository_intelligence.bats",
        "tests/unit/test_issue_planning.bats",
        "tests/unit/test_github_publishing.bats",
        "tests/unit/test_worker_v1.bats",
        "tests/unit/test_verifier_v0.bats",
        "tests/unit/test_autonomy_pipeline.bats",
        "tests/unit/test_local_autonomy_control.bats",
        "tests/unit/test_digital_twin.bats",
        "tests/unit/test_constitution_rules.bats",
        "tests/unit/test_policy_sources_v2.bats",
    ];

    for script in PYTHON_SCRIPTS {
        let path = root.join(script);
        if !path.is_file() {
            return failure(id, required, &format!("{script}: required file missing"));
        }
        if !is_executable(&path) {
            return failure(id, required, &format!("{script}: not executable"));
        }
    }
    let syntax = run_required_bash_scripts(
        id,
        required,
        root,
        &BASH_SCRIPTS
            .iter()
            .map(|script| (*script, true))
            .collect::<Vec<_>>(),
    );
    if syntax.is_failure() {
        return syntax;
    }
    let python = run_commands(
        id,
        required,
        root,
        [
            ToolCommand::new("python3", ["-m", "py_compile", PYTHON_SCRIPTS[1]])
                .expect("Python compilation uses direct static arguments"),
            ToolCommand::new("python3", ["-m", "py_compile", PYTHON_SCRIPTS[0]])
                .expect("Python compilation uses direct static arguments"),
        ],
    );
    if python.is_failure() {
        return aggregate(id, required, vec![syntax, python]);
    }
    let bats = run_bats_suites(id, required, root, SUITES);
    aggregate(id, required, vec![syntax, python, bats])
}

fn run_install_tests(id: &str, required: bool, root: &Path) -> CheckResult {
    let configured_pattern = std::env::var("AUTOSPEC_VALIDATE_INSTALL_TEST_GLOB")
        .ok()
        .filter(|pattern| !pattern.is_empty());
    if !root.join("tests/install").is_dir() && configured_pattern.is_none() {
        return CheckResult::completed(id, required, 0, 0, 0, 0, 0, output_digest(&[], &[]));
    }

    let patterns = configured_pattern
        .as_deref()
        .unwrap_or("tests/install/*.sh")
        .split_whitespace();
    let mut tests = patterns
        .flat_map(|pattern| expand_install_test_pattern(root, pattern))
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    tests.sort();
    tests.dedup();

    let commands = tests.into_iter().map(|test| {
        ToolCommand::new("bash", vec![test.into_os_string()])
            .expect("install tests use direct test-file arguments")
    });
    run_commands(id, required, root, commands)
}

fn run_control_plane_bootstrap(id: &str, required: bool, root: &Path) -> CheckResult {
    const HELPER: &str = "scripts/autospec-control-plane.sh";
    const EVENT_HELPER: &str = "scripts/autospec-observatory-events.sh";
    const RENDERERS: &[&str] = &[
        "scripts/lib/autospec-control-plane-render.sh",
        "scripts/lib/autospec-control-plane-observatory-render.sh",
        "scripts/lib/autospec-control-plane-observatory-events-render.sh",
        "scripts/lib/autospec-control-plane-observatory-ui-render.sh",
        "scripts/lib/autospec-control-plane-observatory-reports-render.sh",
    ];
    const SUITES: &[&str] = &[
        "tests/control-plane-bootstrap.bats",
        "tests/control-plane-governance.bats",
        "tests/control-plane-observatory.bats",
        "tests/control-plane-observatory-auth.bats",
        "tests/control-plane-events.bats",
        "tests/control-plane-ui.bats",
        "tests/integration/control-plane-events.bats",
        "tests/control-plane-reports.bats",
        "tests/integration/control-plane-reports.bats",
        "tests/smoke/control-plane-ui.bats",
        "tests/observatory-outbox.bats",
        "tests/policy-resolution.bats",
        "tests/control-plane-confirm.bats",
        "tests/control-plane-dogfood.bats",
    ];

    let mut syntax_targets = RENDERERS
        .iter()
        .map(|renderer| (*renderer, false))
        .collect::<Vec<_>>();
    syntax_targets.insert(0, (HELPER, true));
    syntax_targets.push((EVENT_HELPER, true));
    let syntax = run_required_bash_scripts(id, required, root, &syntax_targets);
    if syntax.is_failure() {
        return syntax;
    }

    let help = ToolCommand::new("bash", [HELPER, "--help"])
        .expect("control-plane help uses direct static arguments")
        .execute_in_capturing(id, required, root);
    if help.result.is_failure() {
        return aggregate(id, required, vec![syntax, help.result]);
    }
    if !contains_bytes(&help.stdout, b"bootstrap --dry-run") {
        return aggregate(
            id,
            required,
            vec![
                syntax,
                help.result,
                failure(
                    id,
                    required,
                    "scripts/autospec-control-plane.sh --help must document bootstrap --dry-run",
                ),
            ],
        );
    }
    for (document, anchor) in [
        (
            "docs/companion-repositories.md",
            "Control Plane Governance Dry Run",
        ),
        (
            "docs/runbooks/OBSERVATORY.md",
            ".autospec/observatory/outbox/<run-id>.jsonl",
        ),
    ] {
        let path = root.join(document);
        if !path.is_file() {
            return aggregate(
                id,
                required,
                vec![
                    syntax,
                    help.result,
                    failure(id, required, &format!("{document}: required file missing")),
                ],
            );
        }
        if !contains(&path, anchor) {
            return aggregate(
                id,
                required,
                vec![
                    syntax,
                    help.result,
                    failure(id, required, &format!("{document}: missing {anchor}")),
                ],
            );
        }
    }
    for suite in SUITES {
        if !root.join(suite).is_file() {
            return aggregate(
                id,
                required,
                vec![
                    syntax,
                    help.result,
                    failure(id, required, &format!("{suite}: bats coverage missing")),
                ],
            );
        }
    }
    if !program_on_path("bats") {
        return aggregate(id, required, vec![syntax, help.result]);
    }
    let suites = SUITES.iter().map(|suite| {
        let command = ToolCommand::new("bash", [*suite])
            .expect("control-plane suites use direct test-file arguments");
        if *suite == "tests/observatory-outbox.bats" {
            command.with_env("AUTOSPEC_OBSERVATORY_OFFLINE", "1")
        } else {
            command
        }
    });
    let suites = run_commands(id, required, root, suites);
    aggregate(id, required, vec![syntax, help.result, suites])
}

fn expand_install_test_pattern(root: &Path, pattern: &str) -> Vec<PathBuf> {
    let configured = Path::new(pattern);
    let path = if configured.is_absolute() {
        configured.to_path_buf()
    } else {
        root.join(configured)
    };
    let Some(name_pattern) = path.file_name().and_then(|name| name.to_str()) else {
        return Vec::new();
    };
    if !name_pattern.contains('*') {
        return vec![path.to_path_buf()];
    }
    let directory = path.parent().unwrap_or_else(|| Path::new("."));
    let mut matches = fs::read_dir(directory)
        .into_iter()
        .flat_map(|entries| entries.filter_map(Result::ok))
        .map(|entry| entry.path())
        .filter(|candidate| {
            candidate
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| wildcard_filename_matches(name_pattern, name))
        })
        .collect::<Vec<_>>();
    matches.sort();
    matches
}

fn wildcard_filename_matches(pattern: &str, candidate: &str) -> bool {
    let mut remainder = candidate;
    let mut parts = pattern.split('*');
    let first = parts.next().unwrap_or_default();
    if !remainder.starts_with(first) {
        return false;
    }
    remainder = &remainder[first.len()..];
    let suffix = parts.next_back().unwrap_or_default();
    if !remainder.ends_with(suffix) {
        return false;
    }
    remainder = &remainder[..remainder.len() - suffix.len()];
    for part in parts {
        let Some(index) = remainder.find(part) else {
            return false;
        };
        remainder = &remainder[index + part.len()..];
    }
    true
}

fn run_conductor_wiring_contract(id: &str, required: bool, root: &Path) -> CheckResult {
    const LOOP: &str = "scripts/lib/autospec-loop.sh";
    const WATERFALL: &str = "scripts/autonomous-waterfall.sh";
    const SUITE: &str = "tests/autospec/test_conductor_wiring.bats";
    let syntax =
        run_required_bash_scripts(id, required, root, &[(LOOP, false), (WATERFALL, false)]);
    if syntax.is_failure() {
        return syntax;
    }
    let loop_file = root.join(LOOP);
    if !has_line_prefix(&loop_file, "autospec_conductor_run()") {
        return aggregate(
            id,
            required,
            vec![
                syntax,
                failure(id, required, "loop driver: missing autospec_conductor_run"),
            ],
        );
    }
    for alternatives in [
        &["autonomous-premerge-gate", "premerge.gate", "premerge_gate"][..],
        &["autonomous-spend-ledger", "spend.ledger", "spend_ledger"][..],
        &["autonomous-resilience", "resilience"][..],
        &["digest", "autonomous-digest"][..],
        &[
            "autonomous-control-channel",
            "control.channel",
            "control_ch",
        ][..],
        &["autonomous-waterfall", "waterfall"][..],
    ] {
        if !contains_any(&loop_file, alternatives) {
            return aggregate(
                id,
                required,
                vec![
                    syntax,
                    failure(id, required, "loop driver: missing conductor wiring"),
                ],
            );
        }
    }
    for suite in [
        "tests/autonomous/test_waterfall_growth.bats",
        "tests/autonomous/test_loop_growth_dispatch.bats",
    ] {
        if !root.join(suite).is_file() {
            return aggregate(
                id,
                required,
                vec![
                    syntax,
                    failure(id, required, &format!("{suite}: coverage missing")),
                ],
            );
        }
    }
    let skill = root.join("skills/autospec-autonomous/SKILL.md");
    if !contains_any(&skill, &["autospec_conductor_run", "autospec-loop.sh"]) {
        return aggregate(
            id,
            required,
            vec![
                syntax,
                failure(
                    id,
                    required,
                    "autonomous skill: missing conductor reference",
                ),
            ],
        );
    }
    let bats = run_bats_suites(id, required, root, &[SUITE]);
    aggregate(id, required, vec![syntax, bats])
}

fn run_autonomy_guardrails_foundation(id: &str, required: bool, root: &Path) -> CheckResult {
    const GUARDRAILS: &str = "scripts/autonomous-guardrails.sh";
    const SUITE: &str = "tests/autonomous/test_guardrails_foundation.bats";
    let syntax = run_required_bash_scripts(id, required, root, &[(GUARDRAILS, true)]);
    if syntax.is_failure() {
        return syntax;
    }
    for (relative, token) in [
        ("scripts/autonomous-premerge-gate.sh", "diff-guard"),
        ("scripts/autonomous-premerge-gate.sh", "blast-radius"),
        ("scripts/autonomous-premerge-gate.sh", "provenance"),
        ("scripts/autonomous-resilience.sh", "post-merge-health"),
        ("scripts/autonomous-resilience.sh", "audit-log"),
        ("scripts/autonomous-resilience.sh", "FOLLOWUP_ISSUE"),
        (
            "skills/autospec-autonomous/install.sh",
            "autonomous-guardrails.sh",
        ),
        (
            "skills/autospec-autonomous/uninstall.sh",
            "autonomous-guardrails.sh",
        ),
        (
            "docs/specs/2026-07-07-autonomy-guardrails-foundation-design.md",
            "Skalse",
        ),
        (
            "docs/specs/2026-07-07-autonomy-guardrails-foundation-design.md",
            "specification gaming",
        ),
    ] {
        if !contains(&root.join(relative), token) {
            return aggregate(
                id,
                required,
                vec![
                    syntax,
                    failure(id, required, &format!("{relative}: missing {token}")),
                ],
            );
        }
    }
    let bats = run_bats_suites(id, required, root, &[SUITE]);
    aggregate(id, required, vec![syntax, bats])
}

fn prompt_contract_rule_ids(
    id: &str,
    required: bool,
    root: &Path,
    contract: &Path,
) -> Result<Vec<String>, CheckResult> {
    if !contract.is_file() {
        return Err(failure(
            id,
            required,
            &format!("{}: file missing", relative_path(root, contract)),
        ));
    }
    let content = match fs::read_to_string(contract) {
        Ok(content) => content,
        Err(_) => {
            return Err(failure(
                id,
                required,
                &format!("{}: unreadable", relative_path(root, contract)),
            ))
        }
    };
    if content.len() > 24_576 {
        return Err(failure(
            id,
            required,
            &format!("{}: exceeds 24KB budget", relative_path(root, contract)),
        ));
    }
    if content
        .lines()
        .filter(|line| line.starts_with("### RULE_ID table"))
        .count()
        != 1
    {
        return Err(failure(
            id,
            required,
            &format!(
                "{}: expected exactly one RULE_ID table header",
                relative_path(root, contract)
            ),
        ));
    }
    let agents = root.join("AGENTS.md");
    let rule_ids = rule_ids_from_agents(&agents);
    if rule_ids.is_empty() {
        return Err(failure(
            id,
            required,
            "AGENTS.md: RULE_ID table yielded no RULE_IDs",
        ));
    }
    for rule_id in &rule_ids {
        if !content.contains(rule_id) {
            return Err(failure(
                id,
                required,
                &format!(
                    "{}: RULE_ID {rule_id} missing from contract",
                    relative_path(root, contract)
                ),
            ));
        }
    }
    Ok(rule_ids)
}

fn rule_ids_from_agents(path: &Path) -> Vec<String> {
    let Ok(content) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut in_table = false;
    let mut rule_ids = Vec::new();
    for line in content.lines() {
        if line.starts_with("### RULE_ID table") {
            in_table = true;
            continue;
        }
        if in_table && (line.starts_with("### ") || line.starts_with("## ")) {
            break;
        }
        if in_table && line.starts_with("| `") {
            if let Some(rule_id) = line[3..].split('`').next() {
                if !rule_id.is_empty()
                    && rule_id
                        .chars()
                        .all(|character| character.is_ascii_uppercase() || character == '_')
                {
                    rule_ids.push(rule_id.to_string());
                }
            }
        }
    }
    rule_ids.sort();
    rule_ids.dedup();
    rule_ids
}

fn bundled_prompt(
    id: &str,
    required: bool,
    root: &Path,
    role: &str,
) -> super::command::CapturedCheckResult {
    ToolCommand::new(
        "bash",
        [
            "skills/autospec-shared/scripts/bundle-static-context.sh",
            "--role",
            role,
            "--issue-labels",
            "ctx:120k",
        ],
    )
    .expect("bundler invocation uses a direct Bash argument vector")
    .with_env("AUTOSPEC_REPO_ROOT", root.as_os_str())
    .with_env("AUTOSPEC_MEMORY_DIR", "/nonexistent-memory-dir")
    .with_env("AUTOSPEC_MANIFEST", "/nonexistent-manifest.yml")
    .execute_in_capturing(id, required, root)
}

fn captured_failure(mut result: CheckResult, stdout: &[u8], message: &str) -> CheckResult {
    result.exit_code = Some(1);
    result.stderr_bytes += message.len();
    result.output_digest = output_digest(stdout, message.as_bytes());
    result
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn contains_any(path: &Path, expected: &[&str]) -> bool {
    expected.iter().any(|token| contains(path, token))
}

fn run_optional_jsonschema_checks(
    id: &str,
    required: bool,
    root: &Path,
    schemas: &[&str],
) -> CheckResult {
    if !program_on_path("python3") {
        return CheckResult::completed(id, required, 0, 0, 0, 0, 0, output_digest(&[], &[]));
    }
    const CHECK: &str = r#"import json, sys
try:
    import jsonschema
except ImportError:
    sys.exit(0)
with open(sys.argv[1]) as file:
    schema = json.load(file)
jsonschema.Draft202012Validator.check_schema(schema)"#;
    let commands = schemas.iter().map(|schema| {
        ToolCommand::new("python3", ["-c", CHECK, *schema])
            .expect("JSON Schema validation uses a direct Python argument vector")
    });
    run_commands(id, required, root, commands)
}

fn run_matching_bats_suites_if_available(
    id: &str,
    required: bool,
    root: &Path,
    directory: &str,
    prefix: &str,
) -> CheckResult {
    if !program_on_path("bats") {
        return CheckResult::completed(id, required, 0, 0, 0, 0, 0, output_digest(&[], &[]));
    }
    let mut suites = fs::read_dir(root.join(directory))
        .into_iter()
        .flat_map(|entries| entries.filter_map(Result::ok))
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(prefix) && name.ends_with(".bats"))
        })
        .map(|path| relative_path(root, &path))
        .collect::<Vec<_>>();
    suites.sort();
    if suites.is_empty() {
        return CheckResult::completed(id, required, 0, 0, 0, 0, 0, output_digest(&[], &[]));
    }
    let suite_refs = suites.iter().map(String::as_str).collect::<Vec<_>>();
    run_bats_suites(id, required, root, &suite_refs)
}

fn run_scripts_with_optional_bats(
    id: &str,
    required: bool,
    root: &Path,
    scripts: &[&str],
    suites: &[&str],
) -> CheckResult {
    let scripts = scripts
        .iter()
        .map(|script| (*script, false))
        .collect::<Vec<_>>();
    let syntax = run_required_bash_scripts(id, required, root, &scripts);
    if syntax.is_failure() {
        return syntax;
    }
    let bats = run_bats_suites_if_available(id, required, root, suites);
    aggregate(id, required, vec![syntax, bats])
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
        "tests/autospec/groom-reconcile.bats",
        "tests/autospec/autonomous-promote-open-issues.bats",
        "tests/autospec/test_loop_grooming.bats",
        "skills/autospec-shared/tests/grooming-config.bats",
        "skills/autospec-shared/tests/unit/grooming-govern.bats",
        "skills/autospec-shared/tests/unit/grooming-observe.bats",
    ];
    let retired = retired_safety_writer_guard(id, required, root);
    if retired.is_failure() {
        return retired;
    }
    aggregate(
        id,
        required,
        vec![retired, run_bats_suites(id, required, root, SUITES)],
    )
}

fn retired_safety_writer_guard(id: &str, required: bool, root: &Path) -> CheckResult {
    const RETIRED_SCRIPT: &str = "scripts/apply-safety-review.sh";
    const RUNTIME_CALLERS: &[&str] = &[
        "scripts/autonomous-promote-open-issues.sh",
        "skills/autospec-run/scripts/run-groom-preflight.sh",
        "scripts/lib/autospec-loop.sh",
    ];
    const DIRECT_SAFETY_WRITEBACK_SURFACES: &[&str] = &[
        "skills/autospec-classify/SKILL.md",
        "skills/autospec-classify/codex/prompt.md",
        "skills/autospec-classify/opencode/agent.md",
        "skills/autospec/SKILL.md",
        "skills/autospec/codex/prompt.md",
        "skills/autospec/opencode/agent.md",
        "skills/autospec-define/SKILL.md",
        "skills/autospec-define/codex/prompt.md",
        "skills/autospec-define/opencode/agent.md",
        "skills/autospec-run/SKILL.md",
        "skills/autospec-run/codex/prompt.md",
        "skills/autospec-run/opencode/agent.md",
        "skills/autospec-explore/SKILL.md",
        "skills/autospec-explore/codex/prompt.md",
        "skills/autospec-explore/opencode/agent.md",
        "scripts/autospec-explore.sh",
    ];
    const DIRECT_SAFETY_WRITEBACK_TOKENS: &[&str] = &[
        "safety:reviewed",
        "security:quarantined",
        "autospec-safety:begin",
        "autospec-safety:end",
        "autospec:needs-human",
        "autospec-safety-decision:begin",
        "autospec-safety-decision:end",
    ];

    if root.join(RETIRED_SCRIPT).exists() {
        return failure(
            id,
            required,
            "scripts/apply-safety-review.sh: retired shell safety writer must stay deleted",
        );
    }
    for relative in RUNTIME_CALLERS {
        if contains(&root.join(relative), "apply-safety-review.sh") {
            return failure(
                id,
                required,
                &format!("{relative}: references retired shell safety writer"),
            );
        }
    }
    if let Some((relative, token)) = DIRECT_SAFETY_WRITEBACK_SURFACES
        .iter()
        .find_map(|relative| {
            DIRECT_SAFETY_WRITEBACK_TOKENS
                .iter()
                .find(|token| contains(&root.join(relative), token))
                .map(|token| (*relative, *token))
        })
    {
        return failure(
            id,
            required,
            &format!(
                "{relative}: direct safety writeback token {token:?}; use autospec queue review-safety"
            ),
        );
    }
    CheckResult::completed(id, required, 0, 0, 0, 0, 0, output_digest(&[], &[]))
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
                commands.push(bats_command(suite));
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
            bats_command(FLEET_SUITE)
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
                    CLAIM_ADD_IN_PROGRESS.is_in(&line[index..])
                        && !CLAIM_ACQUIRE_COMMAND.is_in(line)
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

fn contains_word(path: &Path, expected: &str) -> bool {
    fs::read_to_string(path)
        .map(|document| {
            document.match_indices(expected).any(|(index, _)| {
                let before = document[..index].chars().next_back();
                let after = document[index + expected.len()..].chars().next();
                let is_word =
                    |character: char| character.is_ascii_alphanumeric() || character == '_';
                !before.is_some_and(is_word) && !after.is_some_and(is_word)
            })
        })
        .unwrap_or(false)
}

fn contains_unsafe_supervisor_eval(path: &Path) -> bool {
    fs::read_to_string(path)
        .map(|document| {
            document.lines().any(|line| {
                SHELL_EVAL_WORD.is_in(line)
                    && SHELL_DOUBLE_QUOTE.is_in(line)
                    && (SUPERVISOR_RESUME_SUBSTITUTION.is_in(line)
                        || SUPERVISOR_REGISTRY_SUBSTITUTION.is_in(line))
            })
        })
        .unwrap_or(false)
}

fn worktree_ladder_block(path: &Path) -> Option<String> {
    let document = fs::read_to_string(path).ok()?;
    let mut collecting = false;
    let mut lines = Vec::new();
    for line in document.lines() {
        if WORKTREE_LADDER_BEGIN.is_in(line) {
            collecting = true;
            continue;
        }
        if WORKTREE_LADDER_END.is_in(line) {
            break;
        }
        if collecting {
            lines.push(
                line.strip_prefix("> ")
                    .or_else(|| line.strip_prefix('>'))
                    .unwrap_or(line),
            );
        }
    }
    (!lines.is_empty()).then(|| lines.join("\n"))
}

fn contains_stale_researcher_count(path: &Path) -> bool {
    fs::read_to_string(path)
        .map(|document| {
            STALE_RESEARCHER_EACH_SIX.is_in(&document)
                || document
                    .match_indices(STALE_RESEARCHER_COUNT_TOKEN.as_str())
                    .any(|(index, _)| {
                        let before = document[..index].chars().next_back();
                        let after = document[index + STALE_RESEARCHER_COUNT_TOKEN.len()..]
                            .chars()
                            .next();
                        !before.is_some_and(|character| {
                            character.is_ascii_alphanumeric() || character == '_'
                        }) && !after.is_some_and(|character| {
                            character.is_ascii_alphanumeric() || character == '_'
                        })
                    })
        })
        .unwrap_or(false)
}

#[derive(Clone, Copy)]
struct TextNeedle(&'static str);

impl TextNeedle {
    const fn new(value: &'static str) -> Self {
        Self(value)
    }

    fn as_str(self) -> &'static str {
        self.0
    }

    fn len(self) -> usize {
        self.0.len()
    }

    fn is_in(self, document: &str) -> bool {
        document.contains(self.0)
    }
}

const WATCHDOG_GC_FUNCTION: TextNeedle = TextNeedle::new("gc_orphaned_worktrees()");
const WATCHDOG_FORCE_REMOVE: TextNeedle = TextNeedle::new("git worktree remove --force");
const WATCHDOG_UNPUSHED_GUARD: TextNeedle = TextNeedle::new("log --not --remotes");
const CLAIM_ADD_IN_PROGRESS: TextNeedle = TextNeedle::new("--add-label in-progress-by-bot");
const CLAIM_ACQUIRE_COMMAND: TextNeedle = TextNeedle::new("autospec claim acquire");
const SHELL_EVAL_WORD: TextNeedle = TextNeedle::new("eval");
const SHELL_DOUBLE_QUOTE: TextNeedle = TextNeedle::new("\"");
const SUPERVISOR_RESUME_SUBSTITUTION: TextNeedle = TextNeedle::new("$(resume_command");
const SUPERVISOR_REGISTRY_SUBSTITUTION: TextNeedle = TextNeedle::new("$(reg");
const WORKTREE_LADDER_BEGIN: TextNeedle = TextNeedle::new("worktree-ladder:begin");
const WORKTREE_LADDER_END: TextNeedle = TextNeedle::new("worktree-ladder:end");
const STALE_RESEARCHER_EACH_SIX: TextNeedle = TextNeedle::new("each of the 6");
const STALE_RESEARCHER_COUNT_TOKEN: TextNeedle = TextNeedle::new("6 researchers");

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
    fn block_expansion_rejects_markered_member_without_golden() {
        let root = std::env::temp_dir().join(format!(
            "autospec-block-expansion-missing-golden-{}",
            std::process::id()
        ));
        let scripts = root.join("scripts");
        let skill = root.join("skills/demo");
        let goldens = root.join("tests/fixtures/skill-goldens");
        fs::create_dir_all(skill.join("codex")).expect("create skill fixture");
        fs::create_dir_all(&scripts).expect("create scripts fixture");
        fs::create_dir_all(&goldens).expect("create golden fixture");
        fs::write(
            scripts.join("expand-skill-blocks.sh"),
            "#!/bin/sh\ncat \"$1\"\n",
        )
        .expect("write expander fixture");
        fs::write(skill.join("SKILL.md"), "# demo\n").expect("write skill fixture");
        fs::write(
            skill.join("codex/prompt.md"),
            "<!-- autospec-block:startup-self-update SKILL_NAME=demo -->\n",
        )
        .expect("write markered member fixture");
        fs::write(
            goldens.join("demo.SKILL.md.sha256"),
            "bc70e26f40b8816eb177813dda1f5f529a27a4641d45aa19cae2348a8c6a5fe9\n",
        )
        .expect("write required skill golden");

        let result = run_block_expansion("check", true, &root);
        let expected = "check_block_expansion: markered member skills/demo/codex/prompt.md has no golden (tests/fixtures/skill-goldens/demo.codex.prompt.md.sha256 missing — fail closed)";

        assert!(result.is_failure());
        assert_eq!(
            result.spawn_count, 2,
            "only SKILL expansion and hashing run"
        );
        assert_eq!(result.stderr_bytes, expected.len());
        assert_ne!(
            result.output_digest,
            output_digest(&[], expected.as_bytes())
        );
        fs::remove_dir_all(root).expect("remove block expansion fixture");
    }

    #[test]
    fn block_expansion_failure_digest_preserves_child_evidence() {
        let child = CheckResult::completed("child", true, 0, 0, 1, 5, 0, "child-output-digest");
        let mut evidence = b"child-output-digest\n".to_vec();
        evidence.extend_from_slice(b"missing golden");

        let result = block_expansion_result(
            "check_block_expansion",
            true,
            vec![child],
            Some("missing golden".to_string()),
        );

        assert_eq!(result.output_digest, output_digest(&evidence, &[]));
    }

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

    #[test]
    fn stale_researcher_count_matches_the_legacy_word_boundary_contract() {
        let path = std::env::temp_dir().join(format!(
            "autospec-stale-researcher-count-{}",
            std::process::id()
        ));

        fs::write(&path, "6 researchers").expect("temporary fixture writes");
        assert!(contains_stale_researcher_count(&path));
        fs::write(&path, "16 researchers").expect("temporary fixture rewrites");
        assert!(!contains_stale_researcher_count(&path));
        fs::write(&path, "each of the 6 sources").expect("temporary fixture rewrites");
        assert!(contains_stale_researcher_count(&path));

        fs::remove_file(path).expect("temporary fixture removes");
    }

    #[test]
    fn retired_safety_writer_guard_rejects_the_file_and_all_live_writeback_surfaces() {
        let root = std::env::temp_dir().join(format!(
            "autospec-retired-safety-writer-{}",
            std::process::id()
        ));
        let scripts = root.join("scripts");
        fs::create_dir_all(&scripts).expect("create temporary scripts directory");
        let retired = scripts.join("apply-safety-review.sh");
        fs::write(&retired, "#!/usr/bin/env bash\n").expect("write retired script fixture");

        assert!(retired_safety_writer_guard("check", true, &root).is_failure());

        fs::remove_file(&retired).expect("remove retired script fixture");
        fs::write(
            scripts.join("autonomous-promote-open-issues.sh"),
            "bash apply-safety-review.sh\n",
        )
        .expect("write live caller fixture");

        assert!(retired_safety_writer_guard("check", true, &root).is_failure());

        fs::remove_file(scripts.join("autonomous-promote-open-issues.sh"))
            .expect("remove live caller fixture");
        let prompt = root.join("skills/autospec-classify/SKILL.md");
        fs::create_dir_all(prompt.parent().expect("skill fixture has parent"))
            .expect("create temporary skill directory");
        fs::write(&prompt, "gh issue edit <N> --add-label safety:reviewed\n")
            .expect("write direct safety writer fixture");

        assert!(retired_safety_writer_guard("check", true, &root).is_failure());

        fs::remove_file(&prompt).expect("remove direct safety writer fixture");
        let explorer = scripts.join("autospec-explore.sh");
        fs::write(&explorer, "autospec-safety-decision:begin\n")
            .expect("write explorer safety writer fixture");

        assert!(retired_safety_writer_guard("check", true, &root).is_failure());

        fs::remove_file(&explorer).expect("remove explorer safety writer fixture");
        let run_prompt = root.join("skills/autospec-run/codex/prompt.md");
        fs::create_dir_all(run_prompt.parent().expect("run prompt fixture has parent"))
            .expect("create run prompt fixture directory");
        fs::write(&run_prompt, "autospec:needs-human\n")
            .expect("write run prompt safety writer fixture");

        assert!(retired_safety_writer_guard("check", true, &root).is_failure());

        fs::remove_dir_all(root).expect("remove temporary guard fixture");
    }
}
