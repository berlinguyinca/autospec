use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use super::catalog::StructuralCheck;

pub struct StructuralValidator;

impl StructuralValidator {
    pub fn run(check: StructuralCheck, root: &Path) -> Result<(), String> {
        match check {
            StructuralCheck::CatalogSlot => {
                Err("validation structural owner is not implemented".to_string())
            }
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
            StructuralCheck::StartupPreflight => Self::validate_startup_preflight(root),
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

        let missing = flags
            .into_iter()
            .filter(|flag| !documentation.contains(flag))
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
        for (heading, failure) in [
            (
                "## Subagent model selection (two-tier, cost-aware)",
                "AGENTS.md missing '## Subagent model selection (two-tier, cost-aware)' section",
            ),
            (
                "### Tier A — Specification work",
                "AGENTS.md missing '### Tier A — Specification work' subheading",
            ),
            (
                "### Tier B — Implementation work",
                "AGENTS.md missing '### Tier B — Implementation work' subheading",
            ),
        ] {
            if !document.lines().any(|line| line == heading) {
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
        for skill in ["autospec", "autospec-split", "autospec-define"] {
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
        if expected != priority_sort_excerpt(&strip_first_blank_line(&read(&codex)?)) {
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
            "skills/autospec/SKILL.md",
            "skills/autospec/codex/prompt.md",
            "skills/autospec/opencode/agent.md",
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

        for skill in ["autospec", "autospec-define"] {
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

    pub fn validate_startup_preflight(root: &Path) -> Result<(), String> {
        let template = root.join("templates/skill-blocks/startup-self-update.md");
        if !template.is_file() {
            return Err(
                "check_startup_preflight: templates/skill-blocks/startup-self-update.md missing (single source of truth)"
                    .to_string(),
            );
        }

        let canonical = startup_preflight_body(&read(&template)?);
        if canonical.is_empty() {
            return Err(
                "templates/skill-blocks/startup-self-update.md missing ## Startup self-update bash block"
                    .to_string(),
            );
        }
        if !canonical.contains("raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh")
        {
            return Err("startup preflight must call the curl-safe suite bootstrap.sh".to_string());
        }
        if !canonical.contains("--skill all --harness all --update") {
            return Err(
                "startup preflight must update all skills across all harnesses".to_string(),
            );
        }
        if canonical.contains("raw.githubusercontent.com/berlinguyinca/autospec/main/install.sh")
            || canonical.contains("raw.githubusercontent.com/berlinguyinca/autospec/main/skills/")
        {
            return Err("startup preflight must not call a raw installer directly".to_string());
        }

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
                if body != canonical {
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
            && read(&path).map(|document| document.lines().any(|line| line == section))?;
        if !has_section {
            return Err(format!(
                "skills/{skill}/{member}: missing '{section}' section"
            ));
        }
    }
    Ok(())
}

fn skill_directories(root: &Path) -> Result<Vec<std::path::PathBuf>, String> {
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

fn contains_docs_drift_note(document: &str) -> bool {
    let drift_gate = document.match_indices("drift").any(|(index, _)| {
        let remainder = &document[index + "drift".len()..];
        remainder.len() >= 5 && remainder.as_bytes()[1..].starts_with(b"gate")
    });
    let doc_before_drift = document
        .find("doc")
        .zip(document.rfind("drift"))
        .is_some_and(|(doc, drift)| doc <= drift);
    let drift_before_stage = document
        .find("drift")
        .zip(document.rfind("stage 2.5"))
        .is_some_and(|(drift, stage)| drift <= stage);

    drift_gate || document.contains("check-doc-drift") || doc_before_drift || drift_before_stage
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

fn strip_first_blank_line(document: &str) -> String {
    document
        .strip_prefix("\n")
        .or_else(|| document.strip_prefix("\r\n"))
        .unwrap_or(document)
        .to_string()
}
