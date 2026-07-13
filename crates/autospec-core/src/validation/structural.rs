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
