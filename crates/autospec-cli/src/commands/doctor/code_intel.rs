use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use autospec_core::code_intel::config::CONFIG_PATH;
use autospec_core::code_intel::doctor::{report, DoctorReport, HostProbe};
use autospec_core::code_intel::language::{detect, DetectedLanguage};
use autospec_core::code_intel::{CodeIntelConfig, WorkspaceRegistry};

/// Directories never worth walking for language detection. Skipping them keeps
/// `doctor code-intel` fast on a large checkout and stops vendored trees from
/// reporting languages the project does not actually own.
const SKIPPED_DIRECTORIES: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    "dist",
    "build",
    "vendor",
    ".venv",
    "__pycache__",
];

/// How deep to walk when detecting languages.
const MAX_DEPTH: usize = 6;

pub fn run(root: &Path, as_json: bool) -> Result<String, String> {
    let config = load_config(root)?;
    let paths = walk(root);
    let languages = detect(&paths, &config.languages.overrides);
    let probe = probe_host(&config, &languages);
    let report = report(&config, &languages, &WorkspaceRegistry::new(), &probe);
    render(&report, as_json)
}

fn render(report: &DoctorReport, as_json: bool) -> Result<String, String> {
    if as_json {
        return report.to_json_string().map_err(|error| error.to_string());
    }
    Ok(report.to_text())
}

/// Read the operator configuration, falling back to documented defaults when the
/// file is absent. A malformed file is an error: silently defaulting could turn
/// off a gate the operator believes is on.
fn load_config(root: &Path) -> Result<CodeIntelConfig, String> {
    let path = root.join(CONFIG_PATH);
    if !path.is_file() {
        return Ok(CodeIntelConfig::default());
    }
    let source = std::fs::read_to_string(&path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    CodeIntelConfig::parse(&source).map_err(|error| format!("{}: {error}", path.display()))
}

fn probe_host(config: &CodeIntelConfig, languages: &[DetectedLanguage]) -> HostProbe {
    let servers: BTreeSet<String> = languages
        .iter()
        .map(|detected| detected.server.clone())
        .filter(|server| program_on_path(server))
        .collect();
    let tools: BTreeSet<String> = [
        config.fallback.structural.clone(),
        config.fallback.textual.clone(),
    ]
    .into_iter()
    .flatten()
    .filter(|tool| program_on_path(tool))
    .collect();
    let backend_present = program_on_path(&config.backend.binary);
    HostProbe {
        available_servers: servers.into_iter().collect(),
        available_fallback_tools: tools.into_iter().collect(),
        backend_present,
        backend_version: backend_present
            .then(|| backend_version(&config.backend.binary))
            .flatten(),
    }
}

fn backend_version(binary: &str) -> Option<String> {
    let output = std::process::Command::new(binary)
        .arg("--version")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    text.split_whitespace().last().map(str::to_string)
}

fn program_on_path(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|directory| directory.join(program).is_file())
    })
}

/// Collect worktree-relative paths for language detection.
fn walk(root: &Path) -> Vec<String> {
    let mut paths = Vec::new();
    let mut queue = vec![(root.to_path_buf(), 0usize)];
    while let Some((directory, depth)) = queue.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            visit(root, &entry.path(), depth, &mut queue, &mut paths);
        }
    }
    paths.sort();
    paths
}

fn visit(
    root: &Path,
    path: &Path,
    depth: usize,
    queue: &mut Vec<(PathBuf, usize)>,
    paths: &mut Vec<String>,
) {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return;
    };
    if path.is_dir() {
        if depth < MAX_DEPTH && !SKIPPED_DIRECTORIES.contains(&name) {
            queue.push((path.to_path_buf(), depth + 1));
        }
        return;
    }
    if let Some(relative) = path.strip_prefix(root).ok().and_then(Path::to_str) {
        paths.push(relative.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("autospec-code-intel-{name}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn write(root: &Path, relative: &str, contents: &str) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, contents).unwrap();
    }

    #[test]
    fn an_absent_config_file_uses_documented_defaults() {
        let root = temp_root("absent-config");

        let config = load_config(&root).unwrap();

        assert_eq!(config, CodeIntelConfig::default());
    }

    #[test]
    fn a_malformed_config_file_is_an_error_not_a_silent_default() {
        let root = temp_root("malformed-config");
        write(
            &root,
            CONFIG_PATH,
            "version: 1\nworkflow:\n  block_new_error: false\n",
        );

        let error = load_config(&root).unwrap_err();

        assert!(error.contains("unknown key in workflow"));
        assert!(error.contains("code-intelligence.yaml"));
    }

    #[test]
    fn a_valid_config_file_is_read_from_the_worktree() {
        let root = temp_root("valid-config");
        write(
            &root,
            CONFIG_PATH,
            "version: 1\nworkspace:\n  idle_ttl_minutes: 90\n",
        );

        let config = load_config(&root).unwrap();

        assert_eq!(config.idle_ttl_minutes(), 90);
    }

    #[test]
    fn the_walk_skips_build_and_vendor_directories() {
        let root = temp_root("walk-skips");
        write(&root, "Cargo.toml", "");
        write(&root, "target/debug/build.rs", "");
        write(&root, "node_modules/pkg/index.js", "");

        let paths = walk(&root);

        assert_eq!(paths, vec!["Cargo.toml".to_string()]);
    }

    #[test]
    fn the_walk_returns_worktree_relative_paths() {
        let root = temp_root("walk-relative");
        write(&root, "src/lib.rs", "");

        let paths = walk(&root);

        assert_eq!(paths, vec!["src/lib.rs".to_string()]);
    }

    #[test]
    fn the_report_detects_languages_from_the_worktree() {
        let root = temp_root("detects");
        write(&root, "Cargo.toml", "");
        write(&root, "src/lib.rs", "");

        let output = run(&root, true).unwrap();

        assert!(output.contains("\"language\":\"rust\""));
        assert!(output.contains("\"command\":\"doctor code-intel\""));
    }

    #[test]
    fn the_text_report_names_the_backend_and_mode() {
        let root = temp_root("text-report");
        write(&root, "go.mod", "");

        let output = run(&root, false).unwrap();

        assert!(output.contains("AutoSpec code intelligence:"));
        assert!(output.contains("mode local"));
        assert!(output.contains("language:go"));
    }

    #[test]
    fn a_worktree_with_no_supported_language_still_reports() {
        let root = temp_root("no-language");
        write(&root, "README.md", "# readme");

        let output = run(&root, true).unwrap();

        assert!(output.contains("\"name\":\"languages\""));
        assert!(output.contains("\"status\":\"warn\""));
    }

    #[test]
    fn a_missing_backend_binary_blocks_the_report() {
        let root = temp_root("missing-backend");
        write(
            &root,
            CONFIG_PATH,
            "version: 1\nbackend:\n  binary: agent-lsp-not-installed\n",
        );

        let output = run(&root, true).unwrap();

        assert!(output.contains("\"status\":\"blocked\""));
        assert!(output.contains("agent-lsp-not-installed"));
    }
}
