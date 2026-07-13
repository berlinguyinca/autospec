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
