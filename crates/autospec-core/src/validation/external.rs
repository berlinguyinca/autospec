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
    ReleaseVerdictScript,
    BatsSuite(&'static str),
    ReviewerReuseLens,
    BashHelpUsage(&'static str),
    SupersessionContract,
    RunSummaryContract,
    DbModuleInstall,
    BatsDirectory(&'static str),
    AutospecUpgradeContract,
    AutospecTestSkill,
    AutospecPlaywrightSkill,
    AutospecFabContract,
    GroomingContract,
    MutationAndNegativePath,
    LintImplementationHelpers,
    LintIssueHelpers,
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
            Self::ReleaseVerdictScript => run_release_verdict_script(id, required, root),
            Self::BatsSuite(suite) => run_bats_suite(id, required, root, suite),
            Self::ReviewerReuseLens => run_reviewer_reuse_lens(id, required, root),
            Self::BashHelpUsage(script) => run_bash_help_usage(id, required, root, script),
            Self::SupersessionContract => run_supersession_contract(id, required, root),
            Self::RunSummaryContract => run_run_summary_contract(id, required, root),
            Self::DbModuleInstall => run_db_module_install(id, required, root),
            Self::BatsDirectory(directory) => run_bats_directory(id, required, root, directory),
            Self::AutospecUpgradeContract => run_autospec_upgrade_contract(id, required, root),
            Self::AutospecTestSkill => run_skill_validator(id, required, root, "autospec-test"),
            Self::AutospecPlaywrightSkill => run_autospec_playwright_skill(id, required, root),
            Self::AutospecFabContract => run_autospec_fab_contract(id, required, root),
            Self::GroomingContract => run_grooming_contract(id, required, root),
            Self::MutationAndNegativePath => run_mutation_and_negative_path(id, required, root),
            Self::LintImplementationHelpers => run_lint_implementation_helpers(id, required, root),
            Self::LintIssueHelpers => run_lint_issue_helpers(id, required, root),
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
