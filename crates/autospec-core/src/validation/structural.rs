use super::catalog::StructuralCheck;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::Command;
mod startup_preflight_contract;
pub struct StructuralValidator;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefactorDraft {
    pub file: String,
    pub title: String,
    pub body: String,
    pub severity: &'static str,
}

/// Return one deterministic, behavior-preserving draft for each oversized Rust module.
pub fn oversized_module_refactor_issues(
    root: &Path,
    warning: usize,
    error: usize,
) -> Vec<RefactorDraft> {
    let mut drafts = Vec::new();
    let mut files = Vec::new();
    collect_rust_files(root, &mut files);
    files.sort();
    for file in files {
        let Ok(text) = fs::read_to_string(&file) else {
            continue;
        };
        let lines = text.lines().count();
        if lines < warning {
            continue;
        }
        let severity = if lines >= error { "error" } else { "warning" };
        let relative = file
            .strip_prefix(root)
            .unwrap_or(&file)
            .display()
            .to_string();
        let responsibility = text
            .lines()
            .find_map(|line| {
                let trimmed = line.trim();
                trimmed
                    .strip_prefix("pub fn ")
                    .or_else(|| trimmed.strip_prefix("pub async fn "))
                    .map(|name| name.split('(').next().unwrap_or(name).to_string())
            })
            .unwrap_or_else(|| "module responsibilities".to_string());
        drafts.push(RefactorDraft { file: relative.clone(), severity, title: format!("Extract {responsibility} from {relative}"), body: format!("Add a characterization test before a behavior-preserving extraction of {responsibility} from {relative}.") });
    }
    drafts
}

fn collect_rust_files(root: &Path, output: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rust_files(&path, output);
        } else if path.extension().is_some_and(|ext| ext == "rs")
            && path.to_string_lossy().contains("/src/")
        {
            output.push(path);
        }
    }
}

impl StructuralValidator {
    pub fn run(check: StructuralCheck, root: &Path) -> Result<(), String> {
        match check {
            StructuralCheck::TrioLockstep => Self::validate_trio_lockstep(root),
            StructuralCheck::DuoLockstep => Self::validate_duo_lockstep(root),
            StructuralCheck::RequiredTrioFiles => Self::validate_required_trio_files(root),
            StructuralCheck::FlagSentinelDocs => Self::validate_flag_sentinel_docs(root),
            StructuralCheck::SubagentModelTier => Self::validate_subagent_model_tiers(root),
            StructuralCheck::HarnessDetection => Self::validate_harness_detection_blocks(root),
            StructuralCheck::MonitorBatchExit => Self::validate_monitor_batch_exits(root),
            StructuralCheck::AgentsMdSubagentSection => {
                Self::validate_agents_md_subagent_section(root)
            }
            StructuralCheck::AgentsMdSubagentMatrix => {
                Self::validate_agents_md_subagent_matrix(root)
            }
            StructuralCheck::AutospecListenFiles => Self::validate_autospec_listen_files(root),
            StructuralCheck::ExamplesDirectory => Self::validate_examples_directory(root),
            StructuralCheck::GovernanceHeadings => Self::validate_governance_headings(root),
            StructuralCheck::StlDesignGuardrails => {
                Self::validate_autospec_stl_design_guardrails(root)
            }
            StructuralCheck::ExistingSpecMode => Self::validate_existing_spec_mode(root),
            StructuralCheck::DocsAmendmentPresence => Self::validate_docs_amendment_presence(root),
            StructuralCheck::AutospecReviewSkill => Self::validate_autospec_review_skill(root),
            StructuralCheck::AutospecReviewTierADirectives => {
                Self::validate_autospec_review_tier_a_directives(root)
            }
            StructuralCheck::AutospecRunPrioritySortLockstep => {
                Self::validate_autospec_run_priority_sort_lockstep(root)
            }
            StructuralCheck::AutospecRunRegressionReviewLockstep => {
                Self::validate_autospec_run_regression_review_lockstep(root)
            }
            StructuralCheck::Phase1BoundedContext => {
                Self::validate_phase1_bounded_context_contract(root)
            }
            StructuralCheck::FleetGuiSubcommandLockstep => {
                Self::validate_fleet_gui_subcommand_lockstep(root)
            }
            StructuralCheck::AgentsMdGitHygiene => Self::validate_agents_md_git_hygiene(root),
            StructuralCheck::PaletteSingleSource => Self::validate_palette_single_source(root),
            StructuralCheck::MermaidDocumentation => {
                Self::validate_mermaid_documentation_contract(root)
            }
            StructuralCheck::QaDocumentationGate => Self::validate_qa_documentation_gate(root),
            StructuralCheck::AutospecHarmonize => Self::validate_autospec_harmonize_contract(root),
            StructuralCheck::AutospecAutonomousSkill => {
                Self::validate_autospec_autonomous_skill_contract(root)
            }
            StructuralCheck::AutospecExploreUserspaceRoster => {
                Self::validate_autospec_explore_userspace_roster_contract(root)
            }
            StructuralCheck::AutospecExploreParallelValidation => {
                Self::validate_autospec_explore_parallel_validation_contract(root)
            }
            StructuralCheck::AutospecAutonomousTier4Discovery => {
                Self::validate_autospec_autonomous_tier4_discovery_contract(root)
            }
            StructuralCheck::TeamPersonalitySelection => {
                Self::validate_team_personality_selection_contract(root)
            }
            StructuralCheck::TeamPersonalityIssueTemplate => {
                Self::validate_team_personality_issue_template_contract(root)
            }
            StructuralCheck::TeamPersonalityPhase4AndDocs => {
                Self::validate_team_personality_phase4_and_docs_contract(root)
            }
            StructuralCheck::TeamPersonality => Self::validate_team_personality_contract(root),
            StructuralCheck::AutospecReleaseContract => {
                Self::validate_autospec_release_contract(root)
            }
            StructuralCheck::QaVerdictContract => Self::validate_qa_verdict_contract(root),
            StructuralCheck::BruteForceRuleIds => Self::validate_brute_force_rule_ids(root),
            StructuralCheck::CloseoutContract => Self::validate_closeout_contract(root),
            StructuralCheck::Phase4GuardianBlockLockstep => {
                Self::validate_phase4_guardian_block_lockstep(root)
            }
            StructuralCheck::Phase4IssueStartSummary => {
                Self::validate_phase4_issue_start_summary(root)
            }
            StructuralCheck::Phase4ImmediateNextIssuePickup => {
                Self::validate_phase4_immediate_next_issue_pickup(root)
            }
            StructuralCheck::AutospecRunContinuation => {
                Self::validate_autospec_run_continuation_contract(root)
            }
            StructuralCheck::AutospecRunCodexBoundedHandoff => {
                Self::validate_autospec_run_codex_bounded_handoff(root)
            }
            StructuralCheck::Phase4AdaptiveRetry => Self::validate_phase4_adaptive_retry(root),
            StructuralCheck::Phase4FullTestSuite => {
                Self::validate_phase4_full_test_suite_gate(root)
            }
            StructuralCheck::DataScopeReviewLens => Self::validate_data_scope_review_lens(root),
            StructuralCheck::Phase4CostEpicParity => {
                Self::validate_phase4_cost_epic_parity_lockstep(root)
            }
            StructuralCheck::DocsDriftGateRegenConditionalParity => {
                Self::validate_docs_drift_gate_regen_conditional_parity(root)
            }
            StructuralCheck::StopMode => Self::validate_stop_mode_sections(root),
            StructuralCheck::KeywordRouting => Self::validate_keyword_routing_section(root),
            StructuralCheck::GapRemediation => Self::validate_gap_remediation_sections(root),
            StructuralCheck::ReviewRemediation => Self::validate_review_remediation_sections(root),
            StructuralCheck::EnforcementDefaults => {
                Self::validate_enforcement_defaults_sections(root)
            }
            StructuralCheck::SelfUpdateTrio => Self::validate_trio_self_update_sections(root),
            StructuralCheck::SelfUpdateDuo => Self::validate_duo_self_update_sections(root),
            StructuralCheck::CodexSkillsInstall => Self::validate_codex_skills_install(root),
            StructuralCheck::SharedScriptInstall => Self::validate_shared_script_install(root),
            StructuralCheck::RootHelperWrapperPolicy => {
                Self::validate_root_helper_wrapper_policy(root)
            }
            StructuralCheck::ReferencePointerIntegrity => super::reference_pointer::validate(root),
            StructuralCheck::StartupPreflight => Self::validate_startup_preflight(root),
            StructuralCheck::RustOutputMacros => super::output_macros::validate(root),
        }
    }

    pub fn validate_root(root: &Path) -> Result<(), String> {
        Self::validate_required_trio_files(root)?;
        Self::validate_trio_lockstep(root)?;
        Self::validate_duo_lockstep(root)
    }

    pub fn validate_trio_lockstep(root: &Path) -> Result<(), String> {
        for skill_dir in skill_directories(root)? {
            let skill = skill_dir.join("SKILL.md");
            let codex = skill_dir.join("codex/prompt.md");
            let opencode = skill_dir.join("opencode/agent.md");
            if !skill.is_file() || !codex.is_file() || !opencode.is_file() {
                continue;
            }

            let relative = display_path(root, &skill_dir)?;
            let skill_body = strip_frontmatter(&read(&skill)?);
            if skill_body != read(&codex)? {
                return Err(format!(
                    "{relative}/codex/prompt.md: body diverges from {relative}/SKILL.md"
                ));
            }
            if skill_body != strip_frontmatter(&read(&opencode)?) {
                return Err(format!(
                    "{relative}/opencode/agent.md: body diverges from {relative}/SKILL.md"
                ));
            }
        }
        Ok(())
    }

    pub fn validate_duo_lockstep(root: &Path) -> Result<(), String> {
        for skill_dir in skill_directories(root)? {
            let skill = skill_dir.join("SKILL.md");
            let codex = skill_dir.join("codex/prompt.md");
            if !skill.is_file() || !codex.is_file() || skill_dir.join("opencode/agent.md").is_file()
            {
                continue;
            }

            let relative = display_path(root, &skill_dir)?;
            if strip_frontmatter(&read(&skill)?) != read(&codex)? {
                return Err(format!(
                    "{relative}/codex/prompt.md: body diverges from {relative}/SKILL.md"
                ));
            }
        }
        Ok(())
    }

    pub fn validate_required_trio_files(root: &Path) -> Result<(), String> {
        for skill_dir in skill_directories(root)? {
            if !skill_dir.join("SKILL.md").is_file()
                || !skill_dir.join("codex/prompt.md").is_file()
                || !skill_dir.join("opencode/agent.md").is_file()
            {
                continue;
            }

            let relative = display_path(root, &skill_dir)?;
            for required in [
                "SKILL.md",
                "README.md",
                "install.sh",
                "uninstall.sh",
                "opencode/agent.md",
                "codex/prompt.md",
            ] {
                if !skill_dir.join(required).is_file() {
                    return Err(format!("{relative}: missing required file {required}"));
                }
            }
        }
        Ok(())
    }

    pub fn validate_flag_sentinel_docs(root: &Path) -> Result<(), String> {
        let documentation = root.join("docs/FLAGS.md");
        if !documentation.is_file() {
            return Err("docs/FLAGS.md: missing flag-file reference".to_string());
        }
        let documentation = read(&documentation)?;
        let mut flags = BTreeSet::new();

        for source_root in ["scripts", "skills", "packages", "crates"] {
            let source_root = root.join(source_root);
            if !source_root.is_dir() {
                continue;
            }
            for path in files_under(&source_root)? {
                if has_ignored_flag_scan_component(root, &path) {
                    continue;
                }
                let Ok(contents) = read(&path) else {
                    continue;
                };
                flags.extend(sentinel_flags(&contents));
            }
        }

        let documented_flags = sentinel_flags(&documentation);
        let missing = flags
            .into_iter()
            .filter(|flag| !documented_flags.contains(flag))
            .collect::<Vec<_>>();
        if missing.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "docs/FLAGS.md: missing sentinel flag(s): {}",
                missing.join(" ")
            ))
        }
    }

    pub fn validate_subagent_model_tiers(root: &Path) -> Result<(), String> {
        for skill_dir in trio_skill_directories(root)? {
            let name = skill_name(&skill_dir)?;
            let document = read(&skill_dir.join("SKILL.md"))?;
            let tier_directives = document
                .lines()
                .filter(|line| line.contains("**Model tier:**"))
                .collect::<Vec<_>>();
            if tier_directives.is_empty() {
                return Err(format!(
                    "{name}: SKILL.md missing '**Model tier:**' directive on at least one subagent dispatch"
                ));
            }
            if !document
                .lines()
                .any(|line| line.starts_with("| Subagent model tier "))
                && !document.contains("<!-- autospec-block:harness-adapter -->")
            {
                return Err(format!(
                    "{name}: SKILL.md harness-adapter table missing 'Subagent model tier' row"
                ));
            }
            if tier_directives.iter().any(|line| {
                !line.contains("Tier A (spec work)")
                    && !line.contains("Tier B (implementation work)")
                    && !line.contains("TIER_A")
                    && !line.contains("TIER_B")
            }) {
                return Err(format!(
                    "{name}: SKILL.md has '**Model tier:**' directive(s) without 'Tier A (spec work)', 'Tier B (implementation work)', or TIER_A/TIER_B label"
                ));
            }
            if document.contains("TIER_A") || document.contains("TIER_B") {
                continue;
            }

            let Some((expected_a, expected_b)) = legacy_tier_counts(&name) else {
                continue;
            };
            let a_count = tier_directives
                .iter()
                .filter(|line| line.contains("Tier A (spec work)"))
                .count();
            let b_count = tier_directives
                .iter()
                .filter(|line| line.contains("Tier B (implementation work)"))
                .count();
            if a_count != expected_a {
                return Err(format!(
                    "{name}: expected {expected_a} Tier-A dispatches, found {a_count}"
                ));
            }
            if b_count != expected_b {
                return Err(format!(
                    "{name}: expected {expected_b} Tier-B dispatches, found {b_count}"
                ));
            }
        }
        Ok(())
    }

    pub fn validate_harness_detection_blocks(root: &Path) -> Result<(), String> {
        for skill_dir in trio_skill_directories(root)? {
            let name = skill_name(&skill_dir)?;
            let document = read(&skill_dir.join("SKILL.md"))?;
            if !document.contains("## Harness detection") {
                return Err(format!(
                    "{name}: missing ## Harness detection section (required; defines TIER_A/TIER_B)"
                ));
            }
            if !document.contains("TIER_A") {
                return Err(format!(
                    "{name}: ## Harness detection section present but missing TIER_A reference"
                ));
            }
            if !document.contains("TIER_B") {
                return Err(format!(
                    "{name}: ## Harness detection section present but missing TIER_B reference"
                ));
            }
            if !document.to_ascii_lowercase().contains("silently") {
                return Err(format!(
                    "{name}: ## Harness detection section present but missing 'silently' fallback reference"
                ));
            }
        }
        Ok(())
    }

    pub fn validate_monitor_batch_exits(root: &Path) -> Result<(), String> {
        for skill_dir in trio_skill_directories(root)? {
            let document = read(&skill_dir.join("SKILL.md"))?;
            if !document
                .lines()
                .any(|line| line == "## Phase 4 — Background autonomous monitor")
            {
                continue;
            }
            let name = skill_name(&skill_dir)?;
            let missing = [
                "batch_issue_count",
                "AUTOSPEC_BATCH_SIZE",
                "batch-done.json",
                "BATCH_COMPLETE",
                "ALL_DONE",
            ]
            .into_iter()
            .filter(|required| !document.contains(required))
            .collect::<Vec<_>>();
            if !missing.is_empty() {
                return Err(format!(
                    "{name}: monitor batch-exit missing: {}",
                    missing.join(" ")
                ));
            }
        }
        Ok(())
    }

    pub fn validate_agents_md_subagent_section(root: &Path) -> Result<(), String> {
        let agents = root.join("AGENTS.md");
        if !agents.is_file() {
            return Err("AGENTS.md missing at repo root".to_string());
        }
        let document = read(&agents)?;
        for (heading, qualified, failure) in [
            (
                "## Subagent model selection (two-tier, cost-aware)",
                false,
                "AGENTS.md missing '## Subagent model selection (two-tier, cost-aware)' section",
            ),
            (
                "### Tier A — Specification work",
                true,
                "AGENTS.md missing '### Tier A — Specification work' subheading",
            ),
            (
                "### Tier B — Implementation work",
                true,
                "AGENTS.md missing '### Tier B — Implementation work' subheading",
            ),
        ] {
            if !document.lines().any(|line| {
                if qualified {
                    line.starts_with(heading)
                } else {
                    line == heading
                }
            }) {
                return Err(failure.to_string());
            }
        }
        Ok(())
    }

    pub fn validate_agents_md_subagent_matrix(root: &Path) -> Result<(), String> {
        let agents = root.join("AGENTS.md");
        if !agents.is_file() {
            return Err("AGENTS.md missing at repo root".to_string());
        }
        let document = read(&agents)?;
        if !document
            .lines()
            .any(|line| line == "## Subagent vs inline decision matrix")
        {
            return Err(
                "AGENTS.md missing '## Subagent vs inline decision matrix' section".to_string(),
            );
        }
        for anchor in [
            "Read-only exploration across many files",
            "2+ independent tasks with disjoint scopes",
            "Single-purpose long-running work",
            "Risky / quarantine-worthy work",
            "Orchestration / decision-making",
            "One-shot tool calls",
            "Short edits (1-3 file modifications)",
            "Routing / control flow",
            "Multi-area fan-out",
        ] {
            if !document.contains(anchor) {
                return Err(format!(
                    "AGENTS.md decision matrix missing work-shape row: '{anchor}'"
                ));
            }
        }

        for skill_dir in skill_directories(root)? {
            if !skill_name(&skill_dir)?.starts_with("autospec") {
                continue;
            }
            for member in ["SKILL.md", "codex/prompt.md", "opencode/agent.md"] {
                let path = skill_dir.join(member);
                if !path.is_file() {
                    continue;
                }
                let content = read(&path)?;
                if !content
                    .lines()
                    .any(|line| line.starts_with("## Required capabilities & harness adapter"))
                {
                    continue;
                }
                if content.contains("<!-- autospec-block:harness-adapter-core -->") {
                    continue;
                }
                let display = display_path(root, &path)?;
                if !content
                    .lines()
                    .any(|line| line.starts_with("| Subagent dispatch policy"))
                {
                    return Err(format!(
                        "{display}: Required-capabilities table missing 'Subagent dispatch policy' row"
                    ));
                }
                if !content.contains("per AGENTS.md decision matrix") {
                    return Err(format!(
                        "{display}: dispatch-policy row missing 'per AGENTS.md decision matrix' reference"
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn validate_autospec_listen_files(root: &Path) -> Result<(), String> {
        let listen_dir = root.join("skills/autospec-listen");
        if !listen_dir.is_dir() {
            return Err("skills/autospec-listen: directory missing".to_string());
        }
        for member in [
            "SKILL.md",
            "install.sh",
            "uninstall.sh",
            "opencode/agent.md",
            "codex/prompt.md",
            "references/trigger-keywords.md",
            "README.md",
        ] {
            if !listen_dir.join(member).is_file() {
                return Err(format!(
                    "skills/autospec-listen/{member}: required file missing"
                ));
            }
        }
        Ok(())
    }

    pub fn validate_examples_directory(root: &Path) -> Result<(), String> {
        let examples = root.join("examples");
        if !examples.is_dir() {
            return Err("examples/: directory missing".to_string());
        }
        for member in ["model-profiles.yml", "project-map.yml", "README.md"] {
            if !examples.join(member).is_file() {
                return Err(format!("examples/{member}: required file missing"));
            }
        }
        Ok(())
    }

    pub fn validate_governance_headings(root: &Path) -> Result<(), String> {
        let agents = root.join("AGENTS.md");
        if !agents.is_file() {
            return Err("AGENTS.md: file missing at repo root".to_string());
        }
        let agents_document = read(&agents)?;
        for heading in [
            "## Anti-loop guardrails",
            "## Listener-filed issues lifecycle",
        ] {
            if !agents_document
                .lines()
                .any(|line| line.starts_with(heading))
            {
                return Err(format!("AGENTS.md: missing '{heading}' heading"));
            }
        }

        for (file, references) in [
            ("README.md", ["autospec-listen", "autospec-story"]),
            ("SKILLS.md", ["autospec-listen", "autospec-story"]),
        ] {
            let path = root.join(file);
            if !path.is_file() {
                return Err(format!("{file}: file missing at repo root"));
            }
            let document = read(&path)?;
            for reference in references {
                if !document.contains(reference) {
                    return Err(format!("{file}: missing '{reference}' reference"));
                }
            }
        }
        Ok(())
    }

    pub fn validate_autospec_stl_design_guardrails(root: &Path) -> Result<(), String> {
        for member in ["SKILL.md", "opencode/agent.md", "codex/prompt.md"] {
            let path = root.join("skills/autospec").join(member);
            let display = format!("skills/autospec/{member}");
            let document = read(&path).unwrap_or_default();
            for (required, diagnostic) in [
                (
                    "## Physical STL / CAD design guardrails",
                    "physical STL/CAD design guardrails section",
                ),
                (
                    "at least 5 mm of free working clearance",
                    "5 mm fitting/tool clearance rule",
                ),
                (
                    "at least 5 mm of continuous plastic margin",
                    "5 mm gasket plastic margin rule",
                ),
                (
                    "relief valves, vents, restriction",
                    "low-flow vacuum no-restrictor rule",
                ),
                (
                    "gas/fluid/vacuum/dust-flow simulation",
                    "sealed/flowing simulation rule",
                ),
                (
                    "visual QA from at least 16 angles",
                    "16-angle visual QA rule",
                ),
                (
                    "Regenerate from a clean build directory",
                    "clean-build regeneration rule",
                ),
            ] {
                if !document.contains(required) {
                    return Err(format!("{display} missing {diagnostic}"));
                }
            }
        }
        Ok(())
    }

    pub fn validate_existing_spec_mode(root: &Path) -> Result<(), String> {
        for skill in ["autospec-split", "autospec-define"] {
            for member in ["SKILL.md", "opencode/agent.md", "codex/prompt.md"] {
                let path = root.join("skills").join(skill).join(member);
                let display = format!("skills/{skill}/{member}");
                let document = read(&path).unwrap_or_default();
                if !document
                    .lines()
                    .any(|line| line.starts_with("## Existing spec mode"))
                {
                    return Err(format!("{display} missing '## Existing spec mode' section"));
                }
                if !document.contains("git cat-file -e origin/main:<spec-path>")
                    && !document.contains("git cat-file -e <base>:<spec-path>")
                {
                    return Err(format!(
                        "{display} missing selected-spec verification (git cat-file -e <base>|origin/main:<spec-path>)"
                    ));
                }
                if !document.contains("If Existing spec mode is active") {
                    return Err(format!("{display} missing Phase 3 selected-spec handoff"));
                }
            }
        }
        Ok(())
    }

    pub fn validate_docs_amendment_presence(root: &Path) -> Result<(), String> {
        for (path, failure) in [
            (
                "docs/USER_MANUAL.md",
                "docs/USER_MANUAL.md: missing — run reverse-engineer.sh + gen-docs-from-spec.mjs to regenerate",
            ),
            (
                "llms.txt",
                "llms.txt: missing — run gen-llms-txt.sh --repo-root . to regenerate",
            ),
            (
                "docs/.llm-manifest.json",
                "docs/.llm-manifest.json: missing — run gen-llm-manifest.mjs to regenerate",
            ),
        ] {
            if !root.join(path).is_file() {
                return Err(failure.to_string());
            }
        }

        let path = root.join("skills/autospec-test/SKILL.md");
        let document = read(&path).unwrap_or_default().to_ascii_lowercase();
        if !contains_docs_drift_note(&document) {
            return Err("skills/autospec-test/SKILL.md: missing Stage 2.5 drift-gate composition note (docs amendment §10)".to_string());
        }
        Ok(())
    }

    pub fn validate_autospec_review_skill(root: &Path) -> Result<(), String> {
        let skill = root.join("skills/autospec-review/SKILL.md");
        let opencode = root.join("skills/autospec-review/opencode/agent.md");
        let codex = root.join("skills/autospec-review/codex/prompt.md");
        for (path, display) in [
            (&skill, "skills/autospec-review/SKILL.md"),
            (&opencode, "skills/autospec-review/opencode/agent.md"),
            (&codex, "skills/autospec-review/codex/prompt.md"),
        ] {
            if !path.is_file() {
                return Err(format!("{display} missing"));
            }
        }

        let skill_body = strip_frontmatter(&read(&skill)?);
        if skill_body != strip_frontmatter(&read(&opencode)?) {
            return Err("autospec-review opencode lockstep".to_string());
        }
        if skill_body != read(&codex)? {
            return Err("autospec-review codex lockstep".to_string());
        }
        Ok(())
    }

    pub fn validate_autospec_review_tier_a_directives(root: &Path) -> Result<(), String> {
        let path = root.join("skills/autospec-review/SKILL.md");
        let document = read(&path).unwrap_or_default();
        let count = document.matches("Tier A (spec work)").count();
        if count == 0 {
            return Err(
                "missing 'Tier A (spec work)' directive in autospec-review/SKILL.md".to_string(),
            );
        }
        if count < 2 {
            return Err(format!(
                "expected ≥2 'Tier A (spec work)' directives in autospec-review/SKILL.md, found {count}"
            ));
        }
        Ok(())
    }

    pub fn validate_autospec_run_priority_sort_lockstep(root: &Path) -> Result<(), String> {
        let skill = root.join("skills/autospec-run/SKILL.md");
        let opencode = root.join("skills/autospec-run/opencode/agent.md");
        let codex = root.join("skills/autospec-run/codex/prompt.md");
        let expected = priority_sort_excerpt(&strip_frontmatter(&read(&skill)?));

        if expected != priority_sort_excerpt(&strip_frontmatter(&read(&opencode)?)) {
            return Err("priority sort lockstep (opencode)".to_string());
        }
        if expected != priority_sort_excerpt(&super::structural_text::strip_first_blank_line(&read(&codex)?)) {
            return Err("priority sort lockstep (codex)".to_string());
        }
        Ok(())
    }

    pub fn validate_autospec_run_regression_review_lockstep(root: &Path) -> Result<(), String> {
        for member in ["SKILL.md", "opencode/agent.md", "codex/prompt.md"] {
            let path = root.join("skills/autospec-run").join(member);
            let display = format!("skills/autospec-run/{member}");
            let document = read(&path).unwrap_or_default();
            if document.contains("dispatch a second `TIER_A` subagent") {
                return Err(format!(
                    "second TIER_A regression meta-review dispatch still present in {display} (should be folded into reviewer brief)"
                ));
            }
            for (required, failure) in [
                ("reviewer-lessons.md", "reviewer-lessons write-path missing"),
                (
                    "would the reviewer have caught the original gap",
                    "folded regression gap-check missing",
                ),
                (
                    "AUTOSPEC_REVIEWER_TIER",
                    "AUTOSPEC_REVIEWER_TIER env hatch missing",
                ),
                (
                    "`TIER_B` for ALL issues",
                    "reviewer Tier-B-for-all-issues directive missing",
                ),
            ] {
                if !document.contains(required) {
                    return Err(format!("{failure} in {display}"));
                }
            }
        }
        Ok(())
    }

    pub fn validate_phase1_bounded_context_contract(root: &Path) -> Result<(), String> {
        for member in [
            "skills/autospec-define/SKILL.md",
            "skills/autospec-define/codex/prompt.md",
            "skills/autospec-define/opencode/agent.md",
        ] {
            let document = read(&root.join(member)).unwrap_or_default();
            for (required, failure) in [
                (
                    "Phase 1 bounded-context rule",
                    "missing Phase 1 bounded-context rule",
                ),
                (
                    "Do NOT fork, inherit, or compact the full parent conversation",
                    "allows inherited parent conversation in Phase 1",
                ),
                (
                    "fork_context=false",
                    "missing Codex fresh-context directive",
                ),
                (
                    "context window or remote compact failure",
                    "missing context-overflow fallback directive",
                ),
                (
                    "bounded local read-only `rg`/file-read investigation",
                    "missing bounded local fallback directive",
                ),
            ] {
                if !document.contains(required) {
                    return Err(format!("{member} {failure}"));
                }
            }
        }
        Ok(())
    }

    pub fn validate_fleet_gui_subcommand_lockstep(root: &Path) -> Result<(), String> {
        const NEEDLE: &str = "/autospec-fleet gui";
        for member in ["SKILL.md", "codex/prompt.md", "opencode/agent.md"] {
            let path = root.join("skills/autospec-fleet").join(member);
            let display = format!("skills/autospec-fleet/{member}");
            if !contains(&path, NEEDLE) {
                return Err(format!(
                    "fleet-gui lockstep: '{NEEDLE}' missing in {display}"
                ));
            }
        }
        Ok(())
    }

    pub fn validate_agents_md_git_hygiene(root: &Path) -> Result<(), String> {
        let path = root.join("AGENTS.md");
        if !path.is_file() {
            return Err("AGENTS.md missing at repo root".to_string());
        }
        let document = read(&path)?;
        if !document
            .lines()
            .any(|line| line == "## Git hygiene (agents)")
        {
            return Err(
                "AGENTS.md missing '## Git hygiene (agents)' section (issue #962 §D5)".to_string(),
            );
        }
        for (matches, failure) in [
            (
                document.contains("Fetch-before-branch")
                    || document.contains("fetch-before-branch"),
                "AGENTS.md git hygiene section missing fetch-before-branch rule (§D5)",
            ),
            (
                document.contains("primary checkout"),
                "AGENTS.md git hygiene section missing primary-checkout read-only rule (§D5)",
            ),
            (
                document.contains("fresh-or-verified-clean")
                    || document.contains("Fresh-or-verified-clean"),
                "AGENTS.md git hygiene section missing fresh-or-verified-clean worktrees rule (§D5)",
            ),
            (
                document.contains("git worktree remove"),
                "AGENTS.md git hygiene section missing 'git worktree remove' cleanup rule (§D5)",
            ),
            (
                document.contains("git worktree prune"),
                "AGENTS.md git hygiene section missing 'git worktree prune' cleanup rule (§D5)",
            ),
            (
                document.contains("open-pr")
                    || document.contains("branch-only")
                    || document.contains("fresh"),
                "AGENTS.md git hygiene section missing PR-aware ladder states (§D5)",
            ),
            (
                document.contains("worktree-guard.sh"),
                "AGENTS.md git hygiene section missing pointer to worktree-guard.sh (§D5)",
            ),
        ] {
            if !matches {
                return Err(failure.to_string());
            }
        }
        Ok(())
    }

    pub fn validate_palette_single_source(root: &Path) -> Result<(), String> {
        let doc_style = root.join("skills/autospec-doc/scripts/doc-style.mjs");
        let doc_style_test = root.join("skills/autospec-doc/tests/doc-style.test.mjs");
        if !doc_style.is_file() {
            return Err("skills/autospec-doc/scripts/doc-style.mjs: file missing".to_string());
        }
        let hexes = hex_values(&read(&doc_style)?);
        if hexes.is_empty() {
            return Err("skills/autospec-doc/scripts/doc-style.mjs: no hex values found — palette single-source check cannot run".to_string());
        }

        for path in files_under(root)? {
            if !matches!(
                path.extension().and_then(|extension| extension.to_str()),
                Some("mjs" | "sh")
            ) || path == doc_style
                || path == doc_style_test
                || is_palette_excluded(root, &path)
            {
                continue;
            }
            let Ok(document) = read(&path) else {
                continue;
            };
            let display = display_path(root, &path)?;
            for hex in &hexes {
                if document.contains(hex) {
                    return Err(format!(
                        "palette single-source violation: {display} contains palette hex {hex}"
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn validate_mermaid_documentation_contract(root: &Path) -> Result<(), String> {
        const GENERAL_REQUIREMENTS: [(&str, &str); 8] = [
            (
                "Documentation visualization",
                "Documentation visualization contract",
            ),
            (
                "Mermaid diagrams and charts where acceptable",
                "Mermaid diagrams/charts wording",
            ),
            (
                "queues, routers, state machines, data flow, control flow, algorithms,",
                "concrete Mermaid candidate list",
            ),
            (
                "architecture boundaries, timelines",
                "Mermaid architecture/timeline candidate",
            ),
            (
                "roadmaps, or metric trends",
                "Mermaid roadmap/metric candidate",
            ),
            ("flowchart, sequence", "Mermaid flowchart/sequence taxonomy"),
            (
                "Gantt, timeline, journey, quadrant",
                "Mermaid chart taxonomy",
            ),
            (
                "Sankey, XY, pie, kanban",
                "Mermaid metric/work-queue taxonomy",
            ),
        ];
        const DOC_REQUIREMENTS: [(&str, &str); 12] = [
            ("## Mermaid diagrams", "Mermaid diagrams section"),
            ("docs/assets/diagrams", "diagram asset destination"),
            (
                "Mermaid diagrams and charts where",
                "Mermaid diagrams/charts generation wording",
            ),
            (
                "queue, router, state machine,",
                "doc-generation Mermaid candidate list",
            ),
            (
                "data flow, control flow, algorithm",
                "doc-generation Mermaid flow/algorithm candidate",
            ),
            (
                "algorithm, architecture boundary",
                "doc-generation Mermaid architecture-boundary candidate",
            ),
            (
                "roadmap, or metric trend",
                "doc-generation Mermaid roadmap/metric candidate",
            ),
            ("`flowchart`", "Mermaid flowchart type guidance"),
            ("`gantt` or `timeline`", "Mermaid roadmap chart guidance"),
            ("`sankey`", "Mermaid weighted-flow chart guidance"),
            ("`xychart` or `pie`", "Mermaid metric chart guidance"),
            ("`kanban`", "Mermaid work-queue chart guidance"),
        ];

        // Router delegates Phase 2 (incl. Documentation visualization) to /autospec-define.
        for skill in ["autospec-define"] {
            for member in ["SKILL.md", "codex/prompt.md", "opencode/agent.md"] {
                let display = format!("skills/{skill}/{member}");
                let path = root.join(&display);
                for (required, label) in GENERAL_REQUIREMENTS {
                    require_content(&path, required, format!("{display}: missing {label}"))?;
                }
            }
        }

        for member in ["SKILL.md", "codex/prompt.md", "opencode/agent.md"] {
            let display = format!("skills/autospec-doc/{member}");
            let path = root.join(&display);
            for (required, label) in DOC_REQUIREMENTS {
                require_content(&path, required, format!("{display}: missing {label}"))?;
            }
        }
        Ok(())
    }

    pub fn validate_qa_documentation_gate(root: &Path) -> Result<(), String> {
        for member in ["SKILL.md", "codex/prompt.md", "opencode/agent.md"] {
            let display = format!("skills/autospec-qa/{member}");
            let path = root.join(&display);
            for (required, label) in [
                ("## Documentation gate", "'## Documentation gate' section"),
                (
                    "doc-orchestrator.mjs --full",
                    "doc-orchestrator.mjs --full reference",
                ),
                ("verify-examples.mjs", "verify-examples.mjs reference"),
                (
                    "check-doc-drift.sh --working-tree",
                    "check-doc-drift.sh --working-tree reference",
                ),
                (
                    "orchestrator exits 2",
                    "graceful skip on orchestrator exit 2",
                ),
            ] {
                require_content(&path, required, format!("{display}: missing {label}"))?;
            }
            let has_auto_regenerate_contract = read(&path)
                .map(|document| {
                    document.lines().any(|line| {
                        line.contains("auto_regenerate") && line.contains("NOT consulted")
                    })
                })
                .unwrap_or(false);
            if !has_auto_regenerate_contract {
                return Err(format!(
                    "{display}: missing auto_regenerate NOT-consulted contract"
                ));
            }
        }

        let bats = root.join("tests/qa/test_qa_documentation_gate.bats");
        if bats.is_file() {
            Ok(())
        } else {
            Err("tests/qa/test_qa_documentation_gate.bats: bats coverage missing".to_string())
        }
    }

    pub fn validate_autospec_harmonize_contract(root: &Path) -> Result<(), String> {
        for member in ["SKILL.md", "codex/prompt.md", "opencode/agent.md"] {
            let display = format!("skills/autospec-harmonize/{member}");
            let path = root.join(&display);
            for stage in [
                "discover",
                "generalize",
                "variants",
                "preview",
                "pick",
                "gen-migration-spec",
            ] {
                require_content(
                    &path,
                    stage,
                    format!("{display}: missing stage label '{stage}'"),
                )?;
            }
            for (required, label) in [
                ("--no-live-preview", "'--no-live-preview'"),
                ("AskUserQuestion", "'AskUserQuestion' pick gate reference"),
                ("/autospec-define", "'/autospec-define' handoff"),
            ] {
                require_content(&path, required, format!("{display}: missing {label}"))?;
            }
        }
        Ok(())
    }

    pub fn validate_autospec_autonomous_skill_contract(root: &Path) -> Result<(), String> {
        for member in ["SKILL.md", "codex/prompt.md", "opencode/agent.md"] {
            let display = format!("skills/autospec-autonomous/{member}");
            let path = root.join(&display);
            for (required, label) in [
                ("Self-update mode", "'## Self-update mode' section"),
                ("Stop mode", "'## Stop mode' section"),
                (
                    "Required capabilities",
                    "'## Required capabilities & harness adapter' section",
                ),
                ("Harness detection", "'## Harness detection' section"),
                (
                    "Phase-1 waterfall contract",
                    "'## Phase-1 waterfall contract' section",
                ),
                ("Tier 0", "Tier 0 (control channel) in Phase-1 waterfall"),
                ("Tier 1", "Tier 1 (backlog→main) in Phase-1 waterfall"),
                (
                    "Tier 1.5",
                    "Tier 1.5 open-issue promotion in never-idle waterfall",
                ),
                ("Tier 2", "Tier 2 local discovery in never-idle waterfall"),
                (
                    "Tier 3",
                    "Tier 3 architecture/test-coverage improvement in never-idle waterfall",
                ),
                (
                    "Tier 4",
                    "Tier 4 internet/operator discovery in never-idle waterfall",
                ),
                (
                    "parks only when",
                    "named park-only conditions for never-idle waterfall",
                ),
            ] {
                require_content(&path, required, format!("{display}: missing {label}"))?;
            }
        }
        Ok(())
    }

    pub fn validate_autospec_explore_userspace_roster_contract(root: &Path) -> Result<(), String> {
        for member in ["SKILL.md", "codex/prompt.md", "opencode/agent.md"] {
            let display = format!("skills/autospec-explore/{member}");
            let path = root.join(&display);
            let document = read(&path).unwrap_or_default();
            if !document.contains("--no-userspace") {
                return Err(format!(
                    "{display}: missing '--no-userspace' flag (parity with --no-internet)"
                ));
            }
            for (source, weight) in [
                ("userspace-usage", "0.6"),
                ("userspace-env", "0.5"),
                ("userspace-corpus", "0.5"),
            ] {
                if !has_same_line_pair(&document, source, weight) {
                    return Err(format!(
                        "{display}: {source} source must be weighted {weight}"
                    ));
                }
            }
            if !document.to_ascii_lowercase().contains("untrusted") {
                return Err(format!(
                    "{display}: userspace roster must state the untrusted-DATA trust boundary"
                ));
            }
        }
        Ok(())
    }

    pub fn validate_autospec_explore_parallel_validation_contract(
        root: &Path,
    ) -> Result<(), String> {
        for member in ["SKILL.md", "codex/prompt.md", "opencode/agent.md"] {
            let display = format!("skills/autospec-explore/{member}");
            let path = root.join(&display);
            let document = read(&path).unwrap_or_default();
            let lower = document.to_ascii_lowercase();

            for (required, failure) in [
                (
                    "bounded parallel validation",
                    "missing bounded parallel validation directive",
                ),
                ("per-command timeout", "missing per-command timeout directive"),
                ("global sweep budget", "missing global sweep budget directive"),
                (
                    "2 other queued module validations",
                    "missing evidence that one timeout does not block at least 2 queued validations",
                ),
            ] {
                if !lower.contains(required) {
                    return Err(format!("{display}: {failure}"));
                }
            }

            for status in [
                "pass",
                "fail",
                "timeout",
                "skipped-no-command",
                "skipped-repo-class",
                "dependency-missing",
            ] {
                if !document.contains(status) {
                    return Err(format!(
                        "{display}: missing explore validation status '{status}'"
                    ));
                }
            }

            for field in ["command", "cwd", "duration", "status"] {
                if !document.contains(&format!("`{field}`")) {
                    return Err(format!("{display}: summary output must record `{field}`"));
                }
            }

            if !document.contains("first_diagnostic_lines")
                && !lower.contains("first diagnostic lines")
            {
                return Err(format!(
                    "{display}: summary output must record first diagnostic lines"
                ));
            }
        }
        Ok(())
    }

    pub fn validate_autospec_autonomous_tier4_discovery_contract(
        root: &Path,
    ) -> Result<(), String> {
        for member in ["SKILL.md", "codex/prompt.md", "opencode/agent.md"] {
            let display = format!("skills/autospec-autonomous/{member}");
            let path = root.join(&display);
            let document = read(&path).unwrap_or_default();
            for (required, failure) in [
                (
                    "/autospec-explore --once",
                    "Tier 4 must invoke '/autospec-explore --once' discovery",
                ),
                (
                    "internet-forums",
                    "Tier 4 must name the internet-forums discovery source",
                ),
                (
                    "userspace-usage",
                    "Tier 4 must name the userspace-usage discovery source",
                ),
                (
                    "userspace-env",
                    "Tier 4 must name the userspace-env discovery source",
                ),
                (
                    "userspace-corpus",
                    "Tier 4 must name the userspace-corpus discovery source",
                ),
                (
                    "discovery.enabled",
                    "Tier 4 must gate on discovery.enabled config",
                ),
                ("--no-userspace", "Tier 4 must cite the --no-userspace flag"),
                ("--no-internet", "Tier 4 must cite the --no-internet flag"),
            ] {
                if !document.contains(required) {
                    return Err(format!("{display}: {failure}"));
                }
            }
            let lower = document.to_ascii_lowercase();
            for (matches, failure) in [
                (
                    lower.contains("normal readiness gate"),
                    "Tier 4 must drain candidates via the normal readiness gate",
                ),
                (
                    lower.lines().any(|line| {
                        line.contains("never merges to")
                            && line.contains("main")
                            && line.contains("directly")
                            || line.contains("never merged directly")
                            || line.contains("never direct-merge")
                    }),
                    "Tier 4 must state candidates never merge to main directly",
                ),
                (
                    lower.contains("untrusted"),
                    "Tier 4 must state the untrusted-DATA trust boundary",
                ),
                (
                    lower.contains("accumulate"),
                    "Tier 4 must state trend memory accumulates across idle cycles",
                ),
            ] {
                if !matches {
                    return Err(format!("{display}: {failure}"));
                }
            }
        }
        Ok(())
    }

    pub fn validate_team_personality_selection_contract(root: &Path) -> Result<(), String> {
        // Router delegates Phase 2 to /autospec-define.
        for skill in ["autospec-define"] {
            let display = format!("skills/{skill}/SKILL.md");
            let path = root.join(&display);
            for (required, failure) in [
                (
                    "## Team personality selection",
                    "Team personality selection section",
                ),
                ("past specs", "past-spec inference input"),
                ("confidence is low", "low-confidence ask rule"),
                (
                    "five starter combinations",
                    "five starter combinations rule",
                ),
                (
                    "Core product engineering",
                    "Core product engineering starter team",
                ),
                ("Security-sensitive", "Security-sensitive starter team"),
                ("Team personality", "Team personality issue/spec section"),
                (
                    "Review counter-team",
                    "Review counter-team issue/spec section",
                ),
                (
                    "challenge likely blind spots",
                    "counter-team blind-spot directive",
                ),
            ] {
                require_content(&path, required, format!("{display}: missing {failure}"))?;
            }
            let has_critical_improvement = read(&path)
                .map(|document| {
                    document
                        .to_ascii_lowercase()
                        .contains("critical improvement")
                })
                .unwrap_or(false);
            if !has_critical_improvement {
                return Err(format!(
                    "{display}: missing critical improvement self-question checkpoint"
                ));
            }
        }
        Ok(())
    }

    pub fn validate_team_personality_issue_template_contract(root: &Path) -> Result<(), String> {
        const TEMPLATE_GATE: &str = "body should already contain `## Files to read first`, `## Implementation scope`, `## Team personality`, and `## Review counter-team`";
        // Router delegates Phase 3 to /autospec-define.
        for skill in ["autospec-define", "autospec-split"] {
            let display = format!("skills/{skill}/SKILL.md");
            let path = root.join(&display);
            for (required, failure) in [
                (
                    "- **Team personality**",
                    "Team personality child issue section",
                ),
                (
                    "- **Review counter-team**",
                    "Review counter-team child issue section",
                ),
                (TEMPLATE_GATE, "Phase 3.5 team-lens template gate"),
            ] {
                require_content(&path, required, format!("{display}: missing {failure}"))?;
            }
        }
        Ok(())
    }

    pub fn validate_team_personality_phase4_and_docs_contract(root: &Path) -> Result<(), String> {
        // Router delegates Phase 4 to /autospec-run.
        for skill in ["autospec-run"] {
            let display = format!("skills/{skill}/SKILL.md");
            let path = root.join(&display);
            for (required, failure) in [
                (
                    "## Team personality as execution lens",
                    "Phase 4 team personality execution lens",
                ),
                (
                    "## Review counter-team as review lens",
                    "Phase 4 review counter-team lens",
                ),
                (
                    "Critical self-question before LGTM",
                    "Phase 4 critical self-question before LGTM",
                ),
            ] {
                require_content(&path, required, format!("{display}: missing {failure}"))?;
            }
        }
        for (path, required, failure) in [
            (
                "scripts/gen-issue-skeleton.sh",
                "team_personality",
                "scripts/gen-issue-skeleton.sh missing team_personality input",
            ),
            (
                "scripts/gen-issue-skeleton.sh",
                "review_counter_team",
                "scripts/gen-issue-skeleton.sh missing review_counter_team input",
            ),
            (
                "scripts/gen-reviewer-prompt.sh",
                "--issue-body",
                "scripts/gen-reviewer-prompt.sh missing --issue-body support",
            ),
            (
                "skills/autospec-run/SKILL.md",
                "--issue-body \"/tmp/issue-<ISSUE>-body.md\"",
                "skills/autospec-run/SKILL.md reviewer prompt does not pass issue body",
            ),
            (
                "docs/API_REFERENCE.md",
                "team_personality",
                "docs/API_REFERENCE.md missing gen-issue-skeleton team_personality schema",
            ),
            (
                "docs/API_REFERENCE.md",
                "review_counter_team",
                "docs/API_REFERENCE.md missing gen-issue-skeleton review_counter_team schema",
            ),
            (
                "skills/autospec/README.md",
                "Team personality",
                "skills/autospec/README.md missing Team personality docs",
            ),
            (
                "skills/autospec-define/README.md",
                "Review counter-team",
                "skills/autospec-define/README.md missing Review counter-team docs",
            ),
        ] {
            require_content(&root.join(path), required, failure.to_string())?;
        }
        Ok(())
    }

    pub fn validate_team_personality_contract(root: &Path) -> Result<(), String> {
        Self::validate_team_personality_selection_contract(root)?;
        Self::validate_team_personality_issue_template_contract(root)?;
        Self::validate_team_personality_phase4_and_docs_contract(root)
    }

    pub fn validate_autospec_release_contract(root: &Path) -> Result<(), String> {
        let skill_relative = "skills/autospec-release/SKILL.md";
        let skill = root.join(skill_relative);
        if !skill.is_file() {
            return Err(format!("{skill_relative}: required file missing"));
        }
        let release_readme = root.join("skills/autospec-release/README.md");
        if !release_readme.is_file() {
            return Err("skills/autospec-release/README.md: required file missing".to_string());
        }

        require_line_prefix(
            &skill,
            "## Release pipeline",
            format!("{skill_relative}: missing release pipeline section"),
        )?;
        for (required, failure) in [
            ("/autospec-sweep", "missing autospec-sweep stage"),
            ("/autospec-review", "missing autospec-review stage"),
            ("/autospec-run", "missing autospec-run stage"),
            ("/autospec-test", "missing autospec-test stage"),
            ("/autospec-qa", "missing autospec-qa stage"),
            (
                "validate-qa-artifacts.sh",
                "missing QA artifact validation helper",
            ),
            (
                "PASS`, `PARTIAL`, or `FAIL`",
                "missing explicit release verdict vocabulary",
            ),
        ] {
            require_content(&skill, required, format!("{skill_relative}: {failure}"))?;
        }
        require_line_prefix(
            &skill,
            "## Legacy cleanup prompt",
            format!("{skill_relative}: missing legacy cleanup prompt"),
        )?;
        let readme = root.join("README.md");
        require_content(
            &readme,
            "autospec-release",
            "README.md: missing autospec-release getting-started entry".to_string(),
        )?;
        require_content(
            &readme,
            "Getting Started",
            "README.md: missing getting-started skill guide".to_string(),
        )
    }

    pub fn validate_qa_verdict_contract(root: &Path) -> Result<(), String> {
        for relative in [
            "skills/autospec-qa/SKILL.md",
            "skills/autospec-qa/codex/prompt.md",
            "skills/autospec-qa/opencode/agent.md",
        ] {
            let path = root.join(relative);
            for (required, failure) in [
                (
                    ".autospec/qa-verdict.json",
                    "missing reference to .autospec/qa-verdict.json",
                ),
                (
                    "live_app_proof",
                    "missing live_app_proof field in verdict schema",
                ),
                (
                    "outsourced_implementation",
                    "missing outsourced_implementation in category enum",
                ),
                (
                    "benchmark_overfit",
                    "missing benchmark_overfit in category enum",
                ),
            ] {
                require_content(&path, required, format!("{relative}: {failure}"))?;
            }
        }
        for relative in [
            "skills/autospec-release/SKILL.md",
            "skills/autospec-release/codex/prompt.md",
            "skills/autospec-release/opencode/agent.md",
        ] {
            require_content(
                &root.join(relative),
                "outsourced_implementation",
                format!("{relative}: missing outsourced_implementation remediation branch"),
            )?;
        }
        Ok(())
    }

    pub fn validate_brute_force_rule_ids(root: &Path) -> Result<(), String> {
        for rule_id in ["STRING_MATCH_DOMAIN_LOGIC", "REPEATED_STRUCTURE_AS_CODE"] {
            if !contains(&root.join("AGENTS.md"), rule_id) {
                return Err(format!("AGENTS.md missing {rule_id} in RULE_ID table"));
            }
            for relative in [
                "skills/autospec-run/SKILL.md",
                "skills/autospec-run/codex/prompt.md",
                "skills/autospec-run/opencode/agent.md",
                "skills/autospec-qa/SKILL.md",
                "skills/autospec-qa/codex/prompt.md",
                "skills/autospec-qa/opencode/agent.md",
            ] {
                if !contains(&root.join(relative), rule_id) {
                    return Err(format!(
                        "{relative} missing {rule_id} (issue #637 lockstep)"
                    ));
                }
            }
        }
        for relative in [
            "skills/autospec-qa/SKILL.md",
            "skills/autospec-qa/codex/prompt.md",
            "skills/autospec-qa/opencode/agent.md",
        ] {
            let path = root.join(relative);
            let has_sweep_heading = read(&path)
                .map(|document| {
                    document
                        .lines()
                        .any(|line| line.starts_with("## Brute-force string heuristics sweep"))
                })
                .unwrap_or(false);
            if !has_sweep_heading {
                return Err(format!(
                    "{relative} missing '## Brute-force string heuristics sweep' section (issue #637)"
                ));
            }
        }
        if !is_executable(&root.join("scripts/qa-brute-force-sweep.sh")) {
            return Err(
                "scripts/qa-brute-force-sweep.sh missing or not executable (issue #637)"
                    .to_string(),
            );
        }
        Ok(())
    }

    pub fn validate_closeout_contract(root: &Path) -> Result<(), String> {
        for (relative, required) in [
            ("AGENTS.md", "AGENTS.md missing at repo root"),
            (
                "skills/autospec-run/prompts/implementer-contract.md",
                "skills/autospec-run/prompts/implementer-contract.md: file missing",
            ),
        ] {
            if !root.join(relative).is_file() {
                return Err(required.to_string());
            }
        }
        let agents = root.join("AGENTS.md");
        let contract = root.join("skills/autospec-run/prompts/implementer-contract.md");
        for (path, relative, requirements) in [
            (
                &agents,
                "AGENTS.md",
                [
                    "## Closeout report contract",
                    "### Consumer contract",
                    "[verified]",
                    "[assumed]",
                    "[couldnt-verify]",
                    "[likely-wrong]",
                    "Before/after",
                    "One likely hidden failure",
                    "runtime proof, not",
                    "static`/build-only",
                ],
            ),
            (
                &contract,
                "skills/autospec-run/prompts/implementer-contract.md",
                [
                    "## Closeout report",
                    "Closeout evidence:",
                    "[verified]",
                    "[assumed]",
                    "[couldnt-verify]",
                    "[likely-wrong]",
                    "Before/after",
                    "One likely hidden failure",
                    "runtime proof, not",
                    "static`/build-only",
                ],
            ),
        ] {
            let document = read(path)?;
            for required in requirements {
                if !document.contains(required) {
                    return Err(format!(
                        "{relative}: closeout contract missing required anchor: {required}"
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn validate_phase4_guardian_block_lockstep(root: &Path) -> Result<(), String> {
        let canonical_relative = "skills/autospec-run/SKILL.md";
        let canonical_path = root.join(canonical_relative);
        if !canonical_path.is_file() {
            return Err(format!("guardian lockstep: {canonical_relative} missing"));
        }
        let canonical = guardian_block(&read(&canonical_path)?);
        if canonical.is_empty() {
            return Err(format!(
                "guardian lockstep: no guardian block found in {canonical_relative}"
            ));
        }
        // Router delegates Phase 4 to /autospec-run; only the run trio is checked.
        for relative in [
            "skills/autospec-run/SKILL.md",
            "skills/autospec-run/codex/prompt.md",
            "skills/autospec-run/opencode/agent.md",
        ] {
            let path = root.join(relative);
            if !path.is_file() {
                return Err(format!("guardian lockstep: {relative} missing"));
            }
            if guardian_block(&read(&path)?) != canonical {
                return Err(format!(
                    "guardian lockstep: {relative} guardian block diverges from {canonical_relative}"
                ));
            }
        }
        Ok(())
    }

    pub fn validate_phase4_issue_start_summary(root: &Path) -> Result<(), String> {
        require_literal_contract(
            root,
            &AUTOSPEC_RUN_ADAPTER_FILES,
            &[
                ("Issue start summary", "Issue start summary directive"),
                (
                    "[monitor] starting #$ISSUE:",
                    "[monitor] starting issue summary line",
                ),
                ("[monitor] goal:", "[monitor] goal summary line"),
                ("[monitor] smoke:", "[monitor] smoke summary line"),
                ("[monitor] scope:", "[monitor] scope summary line"),
            ],
        )
    }

    pub fn validate_phase4_immediate_next_issue_pickup(root: &Path) -> Result<(), String> {
        require_literal_contract(
            root,
            &AUTOSPEC_RUN_ADAPTER_FILES,
            &[
                (
                    "Immediate next-issue pickup: NO SLEEP after process(ISSUE)",
                    "immediate next-issue pickup directive",
                ),
                (
                    "fresh queue scan can pick any issue unblocked",
                    "fresh queue scan/unblocked issue rationale",
                ),
            ],
        )
    }

    pub fn validate_autospec_run_continuation_contract(root: &Path) -> Result<(), String> {
        require_literal_contract(
            root,
            &AUTOSPEC_RUN_ADAPTER_FILES,
            &[
                (
                    "BATCH_COMPLETE is a continuation signal, not a terminal state",
                    "BATCH_COMPLETE continuation contract",
                ),
                (
                    "reasoning:deep may reduce a single monitor batch to one issue",
                    "reasoning:deep single-monitor-batch wording",
                ),
                (
                    "the orchestrator MUST relaunch automatically until ALL_DONE",
                    "automatic relaunch-until-ALL_DONE directive",
                ),
                (
                    "Never tell the operator to rerun `/autospec-run` after BATCH_COMPLETE",
                    "no-manual-rerun directive",
                ),
            ],
        )
    }

    pub fn validate_autospec_run_codex_bounded_handoff(root: &Path) -> Result<(), String> {
        for relative in AUTOSPEC_RUN_ADAPTER_FILES {
            let path = root.join(relative);
            if !path.is_file() {
                return Err(format!("{relative}: required file missing"));
            }
            let document = read(&path)?;
            if !document.contains("Codex native subagents with explicit `agent_type`, `model`, or `reasoning_effort` MUST use a bounded handoff, not a full-history fork") {
                return Err(format!("{relative} missing Codex bounded handoff directive"));
            }
            if document.contains(
                "for Codex native subagents, fork/inherit the current conversation context",
            ) {
                return Err(format!("{relative} still directs Codex native subagents to fork/inherit the parent context"));
            }
        }
        Ok(())
    }

    pub fn validate_phase4_adaptive_retry(root: &Path) -> Result<(), String> {
        require_literal_contract(
            root,
            &AUTOSPEC_RUN_ADAPTER_FILES,
            &[
                ("RETRY-LOOP:begin", "RETRY-LOOP:begin marker"),
                ("MAX_IMPL_RETRIES", "MAX_IMPL_RETRIES variable"),
                ("directive_context", "directive_context variable"),
                ("Retry attempt", "'Retry attempt' findings block"),
                (
                    "Implementer hit max retries; manual intervention needed",
                    "manual-intervention exhaustion comment",
                ),
                ("auto-implement-active", "lock-label release on exhaustion"),
            ],
        )
    }

    pub fn validate_phase4_full_test_suite_gate(root: &Path) -> Result<(), String> {
        require_literal_contract(
            root,
            &AUTOSPEC_RUN_ADAPTER_FILES,
            &[
                ("Full test suite gate", "Full test suite gate section"),
                (
                    "AUTOSPEC_FULL_TEST_COMMAND",
                    "AUTOSPEC_FULL_TEST_COMMAND override",
                ),
                ("autospec validate", "direct Rust validation command"),
                (
                    "If the full suite fails, fix the failure, recommit, rerun the full suite, and repeat",
                    "fix/recommit/rerun failure loop",
                ),
                ("Do NOT dispatch LGTM review", "pre-review block on failing full suite"),
                (
                    "Do NOT run `gh pr merge`",
                    "pre-merge block on failing full suite",
                ),
                (
                    "Record the exact full-suite command and passing output summary",
                    "verification evidence requirement",
                ),
            ],
        )
    }

    pub fn validate_data_scope_review_lens(root: &Path) -> Result<(), String> {
        require_literal_contract(
            root,
            &AUTOSPEC_RUN_ADAPTER_FILES,
            &[
                ("data-scope invariant lens", "data-scope invariant lens"),
                (
                    "empty optional filters reject unless documented",
                    "empty optional filter rejection requirement",
                ),
                (
                    "unsupported-filter",
                    "unsupported-filter evidence requirement",
                ),
                ("empty-filter", "empty-filter evidence requirement"),
                ("job-only", "job-only evidence requirement"),
                ("sample-only", "sample-only evidence requirement"),
                ("job+sample", "job+sample evidence requirement"),
            ],
        )
    }

    pub fn validate_phase4_cost_epic_parity_lockstep(root: &Path) -> Result<(), String> {
        for relative in AUTOSPEC_RUN_ADAPTER_FILES {
            let document = read(&root.join(relative)).unwrap_or_default();
            if document.contains("AUTOSPEC_BATCH_SIZE:-3") {
                return Err(format!(
                    "{relative} still defaults AUTOSPEC_BATCH_SIZE to 3 (D2 requires :-1)"
                ));
            }
            if batch_size_default_three(&document) {
                return Err(format!(
                    "{relative} still states batch 'default 3' prose (D2 requires 1)"
                ));
            }
            for (required, failure) in [
                (
                    "AUTOSPEC_BATCH_SIZE:-1",
                    "AUTOSPEC_BATCH_SIZE:-1 default (D2 batch=1)",
                ),
                (
                    "Fresh-subagent-per-issue (canonical Phase 4 path, formerly single-agent absorbed-discipline)",
                    "fresh-subagent-per-issue canonical prose (D2)",
                ),
                (
                    "The orchestrator NEVER implements in its own context",
                    "'orchestrator NEVER implements in its own context' (D2)",
                ),
                (
                    "the default is 1 (one issue per subagent)",
                    "'default is 1 (one issue per subagent)' (D2)",
                ),
                (
                    "reasoning:deep` force-to-1 rule is retained",
                    "reasoning:deep force-to-1 retention (D2)",
                ),
                ("<!-- token-report:begin -->", "token-report:begin marker (D1)"),
                ("<!-- token-report:end -->", "token-report:end marker (D1)"),
                ("post-token-report.sh", "post-token-report.sh invocation (D1)"),
            ] {
                if !document.contains(required) {
                    return Err(format!("{relative} missing {failure}"));
                }
            }
            if !token_report_is_pinned(&document) {
                return Err(format!(
                    "{relative}: token-report:begin not at pinned slot (must follow admin-merge SUCCESS step 8, precede FAILURE step 9 / batch-done) (D1)"
                ));
            }
        }
        Ok(())
    }

    pub fn validate_docs_drift_gate_regen_conditional_parity(root: &Path) -> Result<(), String> {
        require_literal_contract(
            root,
            &AUTOSPEC_RUN_ADAPTER_FILES,
            &[
                (
                    "_REGEN",
                    "_REGEN gate variable (D2b auto_regenerate conditional)",
                ),
                (
                    "resolveAutoRegenerate",
                    "resolveAutoRegenerate import in gate block (D2b)",
                ),
                (
                    "docs: regeneration skipped (auto_regenerate=false)",
                    "OFF-path log 'docs: regeneration skipped (auto_regenerate=false)' (D2b)",
                ),
                (
                    "<!-- docs-drift-gate:begin -->",
                    "docs-drift-gate:begin marker (D2b)",
                ),
                (
                    "<!-- docs-drift-gate:end -->",
                    "docs-drift-gate:end marker (D2b)",
                ),
                (
                    "doc-orchestrator.mjs",
                    "doc-orchestrator.mjs inside gate (D2b regenerate path must be preserved)",
                ),
            ],
        )
    }

    pub fn validate_policy_sections(root: &Path) -> Result<(), String> {
        Self::validate_stop_mode_sections(root)?;
        Self::validate_gap_remediation_sections(root)?;
        Self::validate_review_remediation_sections(root)?;
        Self::validate_enforcement_defaults_sections(root)
    }

    pub fn validate_self_update_sections(root: &Path) -> Result<(), String> {
        Self::validate_trio_self_update_sections(root)?;
        Self::validate_duo_self_update_sections(root)
    }

    pub fn validate_trio_self_update_sections(root: &Path) -> Result<(), String> {
        Self::validate_self_update_sections_by_kind(root, true)
    }

    pub fn validate_duo_self_update_sections(root: &Path) -> Result<(), String> {
        Self::validate_self_update_sections_by_kind(root, false)
    }

    fn validate_self_update_sections_by_kind(root: &Path, expect_trio: bool) -> Result<(), String> {
        let skills_root = root.join("skills");
        if !skills_root.exists() {
            return Ok(());
        }

        let mut skill_dirs = fs::read_dir(&skills_root)
            .map_err(|error| format!("failed to read {}: {error}", skills_root.display()))?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect::<Vec<_>>();
        skill_dirs.sort();

        for skill_dir in skill_dirs {
            let skill = skill_dir.join("SKILL.md");
            let codex = skill_dir.join("codex/prompt.md");
            if !skill.is_file() || !codex.is_file() {
                continue;
            }

            let name = skill_dir
                .file_name()
                .map(|name| name.to_string_lossy())
                .ok_or_else(|| format!("invalid skill directory {}", skill_dir.display()))?;
            let opencode = skill_dir.join("opencode/agent.md");
            if opencode.is_file() && expect_trio {
                Self::validate_trio_self_update(&name, &skill_dir)?;
            } else if !opencode.is_file() && !expect_trio {
                Self::validate_duo_self_update(&name, &skill_dir)?;
            }
        }

        Ok(())
    }

    pub fn validate_stop_mode_sections(root: &Path) -> Result<(), String> {
        for skill in ["autospec", "autospec-run", "autospec-stop"] {
            require_section(root, skill, "## Stop mode")?;
        }
        Ok(())
    }

    pub fn validate_gap_remediation_sections(root: &Path) -> Result<(), String> {
        require_section(
            root,
            "autospec-run",
            "## Phase 5.5 — End-of-run gap remediation",
        )
    }

    pub fn validate_review_remediation_sections(root: &Path) -> Result<(), String> {
        require_section(root, "autospec-review", "## Remediation mode")
    }

    pub fn validate_enforcement_defaults_sections(root: &Path) -> Result<(), String> {
        require_section(root, "autospec-secaudit", "## Enforcement defaults")
    }

    pub fn validate_keyword_routing_section(root: &Path) -> Result<(), String> {
        let skill_dir = root.join("skills/autospec-listen");
        if !skill_dir.is_dir() {
            return Ok(());
        }

        let name = "autospec-listen";
        for member in ["SKILL.md", "opencode/agent.md", "codex/prompt.md"] {
            let path = skill_dir.join(member);
            require_line_prefix(
                &path,
                "## Keyword auto-routing",
                format!("{name}: {member} missing '## Keyword auto-routing' section"),
            )?;
            require_content(
                &path,
                "/autospec-explore",
                format!("{name}: {member} missing explore -> /autospec-explore verb-map row"),
            )?;
            require_content(
                &path,
                "explore-confirm",
                format!("{name}: {member} missing 'explore-confirm' gate literal"),
            )?;
            require_content(
                &path,
                "| `fix`",
                format!("{name}: {member} missing 'fix' -> /autospec verb-map row"),
            )?;
            require_content(
                &path,
                "post_approval_execution_ready",
                format!("{name}: {member} missing post-approval execution-ready routing contract"),
            )?;
            require_content(
                &path,
                "AUTOSPEC_LISTENER_AUTO_IMPLEMENT_OPEN",
                format!("{name}: {member} missing auto-implement open-count routing hint"),
            )?;
            require_content(
                &path,
                "plan_exit_ready",
                format!("{name}: {member} missing completed Plan-mode handoff route"),
            )?;
            require_content(
                &path,
                "AUTOSPEC_LISTENER_PLAN_EXIT_READY",
                format!("{name}: {member} missing Plan-exit-ready state hint"),
            )?;
        }

        let matcher = root.join("scripts/listener-match.sh");
        require_content(
            &matcher,
            "explore-confirm",
            "scripts/listener-match.sh missing 'explore-confirm' gate literal (classifier/trio drift)".to_string(),
        )?;
        require_content(
            &matcher,
            "autospec fix imperative",
            "scripts/listener-match.sh missing 'fix' imperative route (classifier/trio drift)"
                .to_string(),
        )?;
        require_content(
            &matcher,
            "post_approval_execution_ready",
            "scripts/listener-match.sh missing post-approval execution-ready route (issue #1461)"
                .to_string(),
        )?;
        require_content(
            &matcher,
            "plan_exit_ready",
            "scripts/listener-match.sh missing completed Plan-mode handoff route (issue #1462)"
                .to_string(),
        )?;

        let bats = root.join("skills/autospec-shared/tests/unit/listener-match.bats");
        require_content(
            &bats,
            "post-approval: open auto-implement issues route to autospec-run",
            "listener-match.bats missing post-approval open auto-implement route coverage (issue #1461)"
                .to_string(),
        )?;
        require_content(
            &bats,
            "plan-exit: completed saved implementation plan routes to autospec autonomous",
            "listener-match.bats missing completed Plan-mode handoff coverage (issue #1462)"
                .to_string(),
        )?;
        require_content(
            &bats,
            "plan-exit: destructive action gate does not route",
            "listener-match.bats missing Plan-exit destructive-action gate coverage (issue #1462)"
                .to_string(),
        )
    }

    pub fn validate_codex_skills_install(root: &Path) -> Result<(), String> {
        let skills_root = root.join("skills");
        let required_installer = skills_root.join("autospec-design/install.sh");
        if !required_installer.is_file() {
            return Err(
                "check_codex_skills_install: skills/autospec-design/install.sh missing".to_string(),
            );
        }

        let mut installers = fs::read_dir(&skills_root)
            .map_err(|error| format!("failed to read {}: {error}", skills_root.display()))?
            .filter_map(Result::ok)
            .map(|entry| entry.path().join("install.sh"))
            .filter(|path| path.is_file())
            .collect::<Vec<_>>();
        installers.sort();

        for installer in installers {
            let display = display_path(root, &installer)?;
            require_content(
                &installer,
                "skills/$SKILL_NAME/SKILL.md",
                format!("{display} missing Codex skills-dir install (skills/$SKILL_NAME/SKILL.md)"),
            )?;
            require_any_content(
                &installer,
                ["prompts/$SKILL_NAME.md", "CODEX_DEST"],
                format!("{display} missing legacy Codex prompts-file install"),
            )?;
        }

        Ok(())
    }

    pub fn validate_shared_script_install(root: &Path) -> Result<(), String> {
        let skills_root = root.join("skills");
        if !skills_root.is_dir() {
            return Ok(());
        }

        let design_dir = skills_root.join("autospec-design");
        if !referenced_shared_helpers(root, &design_dir)?.is_empty()
            && !contains(&design_dir.join("install.sh"), ".autospec/scripts")
        {
            return Err(
                "check_shared_script_install: autospec-design references a shared helper but does not install into ~/.autospec/scripts"
                    .to_string(),
            );
        }

        let mut installers = fs::read_dir(&skills_root)
            .map_err(|error| format!("failed to read {}: {error}", skills_root.display()))?
            .filter_map(Result::ok)
            .map(|entry| entry.path().join("install.sh"))
            .filter(|path| path.is_file())
            .collect::<Vec<_>>();
        installers.sort();

        for installer in installers {
            let skill_dir = installer
                .parent()
                .expect("an install.sh path always has its skill directory");
            if referenced_shared_helpers(root, skill_dir)?.is_empty()
                || contains(&installer, ".autospec/scripts")
            {
                continue;
            }

            let display = display_path(root, &installer)?;
            return Err(format!(
                "{display} references shared runtime helper(s) but does not install into ~/.autospec/scripts"
            ));
        }

        Ok(())
    }

    pub fn validate_root_helper_wrapper_policy(root: &Path) -> Result<(), String> {
        let scripts = root.join("scripts");
        if !scripts.is_dir() {
            return Ok(());
        }

        let output = Command::new("git")
            .args([
                "ls-files",
                "--others",
                "--exclude-standard",
                "--",
                "scripts",
            ])
            .current_dir(root)
            .output();
        let Ok(output) = output else {
            return Ok(());
        };
        if !output.status.success() {
            return Ok(());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let offenders = stdout
            .lines()
            .map(str::trim)
            .filter(|path| path.starts_with("scripts/"))
            .filter(|path| {
                let absolute = root.join(path);
                absolute.is_file() && !has_skill_canonical_source(root, path)
            })
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();

        if offenders.is_empty() {
            return Ok(());
        }

        Err(format!(
            "root helper wrapper policy: unsupported untracked root-level helper copy without canonical source: {}. Use the autospec Rust binary or install helpers from tracked skill/shared script sources instead of restoring root wrappers.",
            offenders.join(", ")
        ))
    }

    pub fn validate_startup_preflight(root: &Path) -> Result<(), String> {
        let template = root.join("templates/skill-blocks/startup-self-update.md");
        if !template.is_file() {
            return Err(
                "check_startup_preflight: templates/skill-blocks/startup-self-update.md missing (single source of truth)"
                    .to_string(),
            );
        }

        let template_body = startup_preflight_body(&read(&template)?);
        if template_body.is_empty() {
            return Err(
                "templates/skill-blocks/startup-self-update.md missing ## Startup self-update bash block"
                    .to_string(),
            );
        }
        // Asserts the block is harness-safe, then yields whichever text actually
        // carries the executable body (the extracted script, or the block itself
        // on legacy trees).
        let canonical =
            startup_preflight_contract::resolve_contract_source(root, &template_body)?;
        startup_preflight_contract::validate_reliability(&canonical)?;

        let trio_dirs = skill_directories(root)?
            .into_iter()
            .filter(|skill_dir| {
                skill_dir.join("SKILL.md").is_file()
                    && skill_dir.join("opencode/agent.md").is_file()
                    && skill_dir.join("codex/prompt.md").is_file()
            })
            .collect::<Vec<_>>();
        if !trio_dirs.iter().any(|skill_dir| {
            skill_dir
                .file_name()
                .is_some_and(|name| name == "autospec-design")
        }) {
            return Err(
                "check_startup_preflight: autospec-design not discovered (expected a complete multi-harness trio)"
                    .to_string(),
            );
        }

        for skill_dir in trio_dirs {
            for member in ["SKILL.md", "opencode/agent.md", "codex/prompt.md"] {
                let path = skill_dir.join(member);
                let document = read(&path)?;
                if document.contains("autospec-block:startup-self-update") {
                    continue;
                }
                let body = startup_preflight_body(&document);
                if body.is_empty() {
                    continue;
                }
                if body != template_body {
                    return Err(format!(
                        "{} preflight body diverges from canonical",
                        display_path(root, &path)?
                    ));
                }
            }
        }

        Ok(())
    }

    fn validate_trio_self_update(name: &str, skill_dir: &Path) -> Result<(), String> {
        for member in ["SKILL.md", "opencode/agent.md", "codex/prompt.md"] {
            let path = skill_dir.join(member);
            let has_self_update = path.is_file()
                && read(&path).map(|document| {
                    document.lines().any(|line| {
                        line.starts_with("## Self-update mode")
                            || line.contains("autospec-block:startup-self-update")
                    })
                })?;
            if !has_self_update {
                return Err(format!(
                    "{name}: {member} missing '## Self-update mode' section or autospec-block:startup-self-update marker"
                ));
            }
        }

        require_update_flag(name, &skill_dir.join("install.sh"))
    }

    fn validate_duo_self_update(name: &str, skill_dir: &Path) -> Result<(), String> {
        for member in ["SKILL.md", "codex/prompt.md"] {
            let path = skill_dir.join(member);
            let has_self_update = path.is_file()
                && read(&path).map(|document| {
                    document.lines().any(|line| {
                        line.starts_with("## Self-update")
                            || line.contains("autospec-block:startup-self-update")
                    })
                })?;
            if !has_self_update {
                return Err(format!(
                    "{name}: {member} missing '## Self-update' section or autospec-block:startup-self-update marker"
                ));
            }
        }

        let installer = skill_dir.join("install.sh");
        if installer.is_file() {
            require_update_flag(name, &installer)?;
        }
        Ok(())
    }
}

fn require_section(root: &Path, skill: &str, section: &str) -> Result<(), String> {
    let skill_dir = root.join("skills").join(skill);
    if !skill_dir.is_dir() {
        return Ok(());
    }

    for member in ["SKILL.md", "opencode/agent.md", "codex/prompt.md"] {
        let path = skill_dir.join(member);
        let has_section = path.is_file()
            && read(&path)
                .map(|document| document.lines().any(|line| line.starts_with(section)))?;
        if !has_section {
            return Err(format!(
                "skills/{skill}/{member}: missing '{section}' section"
            ));
        }
    }
    Ok(())
}

pub(super) fn skill_directories(root: &Path) -> Result<Vec<std::path::PathBuf>, String> {
    let skills_root = root.join("skills");
    if !skills_root.exists() {
        return Ok(Vec::new());
    }

    let mut skill_dirs = fs::read_dir(&skills_root)
        .map_err(|error| format!("failed to read {}: {error}", skills_root.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    skill_dirs.sort();
    Ok(skill_dirs)
}

fn trio_skill_directories(root: &Path) -> Result<Vec<std::path::PathBuf>, String> {
    skill_directories(root).map(|skill_dirs| {
        skill_dirs
            .into_iter()
            .filter(|skill_dir| {
                skill_dir.join("SKILL.md").is_file()
                    && skill_dir.join("opencode/agent.md").is_file()
                    && skill_dir.join("codex/prompt.md").is_file()
            })
            .collect()
    })
}

fn skill_name(skill_dir: &Path) -> Result<String, String> {
    skill_dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .ok_or_else(|| format!("invalid skill directory {}", skill_dir.display()))
}

fn legacy_tier_counts(name: &str) -> Option<(usize, usize)> {
    match name {
        "autospec" => Some((4, 2)),
        "autospec-define" | "autospec-split" => Some((3, 0)),
        "autospec-run" => Some((2, 3)),
        "autospec-classify" | "autospec-story" => Some((1, 0)),
        "autospec-design" => Some((2, 0)),
        _ => None,
    }
}

fn require_update_flag(name: &str, installer: &Path) -> Result<(), String> {
    let has_update_flag =
        installer.is_file() && read(installer).map(|document| document.contains("--update"))?;
    if has_update_flag {
        Ok(())
    } else {
        Err(format!("{name}: install.sh missing --update flag handling"))
    }
}

fn require_content(path: &Path, expected: &str, failure: String) -> Result<(), String> {
    let matches = path.is_file()
        && read(path)
            .map(|document| document.contains(expected))
            .unwrap_or(false);
    if matches {
        Ok(())
    } else {
        Err(failure)
    }
}

const PHASE4_ADAPTER_FILES: [&str; 6] = [
    "skills/autospec/SKILL.md",
    "skills/autospec/codex/prompt.md",
    "skills/autospec/opencode/agent.md",
    "skills/autospec-run/SKILL.md",
    "skills/autospec-run/codex/prompt.md",
    "skills/autospec-run/opencode/agent.md",
];

const AUTOSPEC_RUN_ADAPTER_FILES: [&str; 3] = [
    "skills/autospec-run/SKILL.md",
    "skills/autospec-run/codex/prompt.md",
    "skills/autospec-run/opencode/agent.md",
];

fn require_literal_contract(
    root: &Path,
    files: &[&str],
    requirements: &[(&str, &str)],
) -> Result<(), String> {
    for relative in files {
        let path = root.join(relative);
        if !path.is_file() {
            return Err(format!("{relative}: required file missing"));
        }
        let document = read(&path)?;
        for (required, failure) in requirements {
            if !document.contains(required) {
                return Err(format!("{relative} missing {failure}"));
            }
        }
    }
    Ok(())
}

fn guardian_block(document: &str) -> String {
    let mut depth = 0i32;
    let mut lines = Vec::new();
    for line in document.lines() {
        if line.contains("<!-- guardian-block:begin -->") {
            depth += 1;
            if depth == 1 { continue; }
        }
        if line.contains("<!-- guardian-block:end -->") {
            if depth == 1 { break; }
            depth -= 1;
        }
        if depth >= 1 { lines.push(line); }
    }
    lines.join("\n")
}

fn batch_size_default_three(document: &str) -> bool {
    document.lines().any(|line| {
        line.contains("AUTOSPEC_BATCH_SIZE` issues")
            && (line.contains("default 3") || line.contains("default: 3"))
    })
}

fn token_report_is_pinned(document: &str) -> bool {
    let mut after_success = false;
    for line in document.lines() {
        if line.starts_with("8. SUCCESS") || line.contains("> 8. SUCCESS") {
            after_success = true;
        }
        if line.starts_with("9. FAILURE") || line.contains("> 9. FAILURE") {
            after_success = false;
        }
        if after_success && line.contains("token-report:begin") {
            return true;
        }
    }
    false
}

fn require_line_prefix(path: &Path, expected: &str, failure: String) -> Result<(), String> {
    let matches = path.is_file()
        && read(path)
            .map(|document| document.lines().any(|line| line.starts_with(expected)))
            .unwrap_or(false);
    if matches {
        Ok(())
    } else {
        Err(failure)
    }
}

fn require_any_content<const N: usize>(
    path: &Path,
    expected: [&str; N],
    failure: String,
) -> Result<(), String> {
    let matches = path.is_file()
        && read(path)
            .map(|document| expected.iter().any(|expected| document.contains(expected)))
            .unwrap_or(false);
    if matches {
        Ok(())
    } else {
        Err(failure)
    }
}

fn has_same_line_pair(document: &str, first: &str, second: &str) -> bool {
    document
        .lines()
        .any(|line| line.contains(first) && line.contains(second))
}

fn referenced_shared_helpers(root: &Path, skill_dir: &Path) -> Result<BTreeSet<String>, String> {
    let mut helpers = BTreeSet::new();
    if !skill_dir.is_dir() {
        return Ok(helpers);
    }

    for path in files_under(skill_dir)? {
        let Ok(document) = read(&path) else {
            continue;
        };
        for helper in autospec_script_references(&document) {
            if skill_dir.join("scripts").join(&helper).is_file() {
                continue;
            }
            if root.join("scripts").join(&helper).is_file()
                || root
                    .join("skills/autospec-shared/scripts")
                    .join(&helper)
                    .is_file()
            {
                helpers.insert(helper);
            }
        }
    }

    Ok(helpers)
}

fn has_skill_canonical_source(root: &Path, script_path: &str) -> bool {
    let Some(helper_name) = Path::new(script_path)
        .file_name()
        .and_then(|name| name.to_str())
    else {
        return false;
    };

    if root
        .join("skills/autospec-shared/scripts")
        .join(helper_name)
        .is_file()
    {
        return true;
    }

    let skills_root = root.join("skills");
    let Ok(entries) = fs::read_dir(skills_root) else {
        return false;
    };
    entries
        .filter_map(Result::ok)
        .any(|entry| entry.path().join("scripts").join(helper_name).is_file())
}

fn files_under(root: &Path) -> Result<Vec<std::path::PathBuf>, String> {
    let mut files = Vec::new();
    let mut directories = vec![root.to_path_buf()];
    while let Some(directory) = directories.pop() {
        for entry in fs::read_dir(&directory)
            .map_err(|error| format!("failed to read {}: {error}", directory.display()))?
            .filter_map(Result::ok)
        {
            let path = entry.path();
            if path.is_dir() {
                directories.push(path);
            } else if path.is_file() {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

fn has_ignored_flag_scan_component(root: &Path, path: &Path) -> bool {
    path.strip_prefix(root)
        .map(|relative| {
            relative.components().any(|component| {
                matches!(
                    component.as_os_str().to_str(),
                    Some(".git" | "__pycache__" | "node_modules" | "target" | "tests")
                )
            })
        })
        .unwrap_or(false)
}

fn is_palette_excluded(root: &Path, path: &Path) -> bool {
    path.strip_prefix(root)
        .map(|relative| {
            let components = relative.components().collect::<Vec<_>>();
            components
                .iter()
                .any(|component| component.as_os_str() == ".git")
                || components.windows(2).any(|components| {
                    components[0].as_os_str() == ".claude"
                        && components[1].as_os_str() == "worktrees"
                })
        })
        .unwrap_or(false)
}

fn hex_values(document: &str) -> BTreeSet<String> {
    let bytes = document.as_bytes();
    let mut hexes = BTreeSet::new();

    for index in 0..bytes.len().saturating_sub(6) {
        if bytes[index] == b'#'
            && bytes[index + 1..index + 7]
                .iter()
                .all(u8::is_ascii_hexdigit)
        {
            hexes.insert(document[index..index + 7].to_string());
        }
    }
    hexes
}

fn sentinel_flags(document: &str) -> BTreeSet<String> {
    let mut flags = BTreeSet::new();
    let bytes = document.as_bytes();
    let mut index = 0;

    while index < bytes.len() {
        if !bytes[index].is_ascii_alphanumeric() {
            index += 1;
            continue;
        }

        let start = index;
        while index < bytes.len() && is_flag_token_byte(bytes[index]) {
            index += 1;
        }
        let candidate = &document[start..index];
        if candidate.ends_with(".flag") {
            flags.insert(candidate.to_string());
        }
    }

    flags
}

fn is_flag_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')
}

/// True when `left` is followed, after exactly one separator character
/// (e.g. a space or hyphen), by `right` — the single-gap phrase primitive
/// used to recognize both the "drift-gate" and "drift gate" spellings of
/// the same term without hard-coding either separator.
fn adjacent_with_single_gap(document: &str, left: &str, right: &str) -> bool {
    document.match_indices(left).any(|(index, _)| {
        let remainder = &document[index + left.len()..];
        remainder.len() > right.len() && remainder.as_bytes()[1..].starts_with(right.as_bytes())
    })
}

/// True when the first occurrence of `earlier` appears at or before the
/// last occurrence of `later` in the document.
fn precedes(document: &str, earlier: &str, later: &str) -> bool {
    document
        .find(earlier)
        .zip(document.rfind(later))
        .is_some_and(|(first, last)| first <= last)
}

fn contains_docs_drift_note(document: &str) -> bool {
    adjacent_with_single_gap(document, "drift", "gate")
        || document.contains("check-doc-drift")
        || precedes(document, "doc", "drift")
        || precedes(document, "drift", "stage 2.5")
}

fn autospec_script_references(document: &str) -> Vec<String> {
    const PREFIX: &str = "${AUTOSPEC_SCRIPTS_DIR";
    let mut references = Vec::new();
    let mut remaining = document;

    while let Some(start) = remaining.find(PREFIX) {
        let after_prefix = &remaining[start + PREFIX.len()..];
        let Some(end) = after_prefix.find('}') else {
            break;
        };
        let after_variable = &after_prefix[end + 1..];
        if let Some(path) = after_variable.strip_prefix('/') {
            let helper = path
                .chars()
                .take_while(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
                })
                .collect::<String>();
            if !helper.is_empty() {
                references.push(helper);
            }
        }
        remaining = after_variable;
    }

    references
}

fn contains(path: &Path, expected: &str) -> bool {
    path.is_file()
        && read(path)
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

fn startup_preflight_body(document: &str) -> String {
    let mut section_found = false;
    let mut in_block = false;
    let mut body = String::new();

    for line in document.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if !section_found {
            if trimmed.starts_with("## Startup self-update") {
                section_found = true;
            }
            continue;
        }
        if !in_block {
            if trimmed == "```bash" {
                in_block = true;
            }
            continue;
        }
        if trimmed == "```" {
            break;
        }
        if !trimmed.starts_with("SKILL_NAME=") {
            body.push_str(line);
        }
    }

    body
}

fn read(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|error| format!("failed to read {}: {error}", path.display()))
}

fn display_path(root: &Path, path: &Path) -> Result<String, String> {
    path.strip_prefix(root)
        .map(|relative| relative.to_string_lossy().replace('\\', "/"))
        .map_err(|_| format!("{} is outside {}", path.display(), root.display()))
}

fn strip_frontmatter(document: &str) -> String {
    let mut separators = 0;
    let mut body = String::new();
    for line in document.split_inclusive('\n') {
        if line.trim_end_matches(['\r', '\n']) == "---" {
            separators += 1;
            continue;
        }
        if separators >= 2 {
            body.push_str(line);
        }
    }
    body
}

fn priority_sort_excerpt(document: &str) -> String {
    let lines = document.lines().collect::<Vec<_>>();
    let Some(start) = lines
        .iter()
        .position(|line| line.contains("Queue priority sort"))
    else {
        return String::new();
    };
    lines[start..lines.len().min(start + 9)].join("\n")
}

#[cfg(test)]
mod tests {
    use super::{contains_docs_drift_note, StructuralValidator};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock after epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("autospec-{name}-{nonce}"));
        fs::create_dir_all(root.join("skills/autospec-explore/codex"))
            .expect("codex mirror directory");
        fs::create_dir_all(root.join("skills/autospec-explore/opencode"))
            .expect("opencode mirror directory");
        root
    }

    fn write_explore_trio(root: &Path, body: &str) {
        let skill = format!("---\nname: autospec-explore\n---\n{body}");
        let opencode = format!("---\nname: autospec-explore\n---\n{body}");
        fs::write(root.join("skills/autospec-explore/SKILL.md"), skill).expect("skill fixture");
        fs::write(root.join("skills/autospec-explore/codex/prompt.md"), body)
            .expect("codex fixture");
        fs::write(
            root.join("skills/autospec-explore/opencode/agent.md"),
            opencode,
        )
        .expect("opencode fixture");
    }

    #[test]
    fn explore_parallel_validation_contract_rejects_missing_timeout_attribution() {
        let root = temp_root("explore-validation-missing");
        write_explore_trio(
            &root,
            "## Explore validation\n\nRun bounded parallel validation when convenient.\n",
        );

        let error =
            StructuralValidator::validate_autospec_explore_parallel_validation_contract(&root)
                .expect_err("missing bounded parallel validation contract must fail");

        assert!(
            error.contains("per-command timeout"),
            "unexpected validation error: {error}"
        );
    }

    #[test]
    fn explore_parallel_validation_contract_accepts_timeout_attribution_vocabulary() {
        let root = temp_root("explore-validation-present");
        write_explore_trio(
            &root,
            r#"## Explore validation

Explore validation runs with bounded parallel validation, a per-command timeout,
and a global sweep budget. At least 2 other queued module validations continue
when one module times out.

Each summary row records `command`, `cwd`, `duration`, `status`, and
`first_diagnostic_lines`. The only valid status vocabulary is `pass`, `fail`,
`timeout`, `skipped-no-command`, `skipped-repo-class`, and
`dependency-missing`.
"#,
        );

        StructuralValidator::validate_autospec_explore_parallel_validation_contract(&root)
            .expect("complete bounded validation contract passes");
    }

    #[test]
    fn docs_drift_note_matches_hyphenated_and_spaced_spellings() {
        assert!(contains_docs_drift_note("stage 2.5 drift gate"));
        assert!(contains_docs_drift_note(
            "**stage 2.5 + docs drift-gate composition:** ..."
        ));
    }

    #[test]
    fn docs_drift_note_matches_the_literal_script_reference() {
        assert!(contains_docs_drift_note(
            "the drift-gate is powered by check-doc-drift.sh"
        ));
    }

    #[test]
    fn docs_drift_note_matches_doc_mentioned_before_drift() {
        assert!(contains_docs_drift_note(
            "see the doc section further below for background, then drift happens"
        ));
    }

    #[test]
    fn docs_drift_note_matches_drift_mentioned_before_stage_2_5() {
        assert!(contains_docs_drift_note(
            "drift happens sometime before stage 2.5 runs"
        ));
    }

    #[test]
    fn docs_drift_note_rejects_unrelated_prose() {
        assert!(!contains_docs_drift_note(
            "this section has nothing to do with the gate keyword at all"
        ));
    }
}
