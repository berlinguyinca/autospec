use std::fs;
use std::path::Path;

pub struct StructuralValidator;

impl StructuralValidator {
    pub fn validate_root(root: &Path) -> Result<(), String> {
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
            Self::validate_skill(root, &skill_dir)?;
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

    fn validate_skill(root: &Path, skill_dir: &Path) -> Result<(), String> {
        let skill = skill_dir.join("SKILL.md");
        let codex = skill_dir.join("codex/prompt.md");
        if !skill.is_file() || !codex.is_file() {
            return Ok(());
        }

        let relative = display_path(root, skill_dir)?;
        let skill_body = strip_frontmatter(&read(&skill)?);
        let codex_body = read(&codex)?;
        let opencode = skill_dir.join("opencode/agent.md");

        if opencode.is_file() {
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

            if skill_body != codex_body {
                return Err(format!(
                    "{relative}/codex/prompt.md: body diverges from {relative}/SKILL.md"
                ));
            }
            if skill_body != strip_frontmatter(&read(&opencode)?) {
                return Err(format!(
                    "{relative}/opencode/agent.md: body diverges from {relative}/SKILL.md"
                ));
            }
        } else if skill_body != codex_body {
            return Err(format!(
                "{relative}/codex/prompt.md: body diverges from {relative}/SKILL.md"
            ));
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

fn require_update_flag(name: &str, installer: &Path) -> Result<(), String> {
    let has_update_flag =
        installer.is_file() && read(installer).map(|document| document.contains("--update"))?;
    if has_update_flag {
        Ok(())
    } else {
        Err(format!("{name}: install.sh missing --update flag handling"))
    }
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
