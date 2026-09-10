//! File-type classification and the per-type review checks.
//!
//! The checklist maps each changed file to a small, fixed set of the checks
//! that apply to it. The mapping is a table — one row per file type, one row
//! per check — so adding a type or a check is a data change, not a code change.
//! Types with nothing mechanical to check (prose, or a format we do not model)
//! yield no checks and are flagged so the caller can report them honestly
//! instead of silently skipping them.

use serde::Serialize;

/// The kinds of files the checklist knows how to review.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub enum FileType {
    TypeScript,
    Containerfile,
    Workflow,
    Rust,
    Python,
    Shell,
    Markdown,
    Config,
    Other,
}

impl FileType {
    pub(crate) fn label(self) -> &'static str {
        match self {
            FileType::TypeScript => "typescript",
            FileType::Containerfile => "containerfile",
            FileType::Workflow => "workflow",
            FileType::Rust => "rust",
            FileType::Python => "python",
            FileType::Shell => "shell",
            FileType::Markdown => "markdown",
            FileType::Config => "config",
            FileType::Other => "other",
        }
    }
}

/// One row of the checks table: a file type and the checks that apply to it,
/// in the order a reviewer should run them.
type CheckRow = (FileType, &'static [&'static str]);

/// The table of per-type checks, ordered by file type. A type with no
/// mechanical checks gets an empty slice; the checklist flags that separately.
const CHECKS: &[CheckRow] = &[
    (
        FileType::TypeScript,
        &[
            "typecheck: every config key must exist in the resolved schema (no unknown-key error)",
            "typecheck: no new object is typed as a catch-all that swallows unknown fields",
        ],
    ),
    (
        FileType::Containerfile,
        &[
            "build: each shell step must not abort the build",
            "build: every path a step references must exist after that step runs",
        ],
    ),
    (
        FileType::Workflow,
        &[
            "gate: every needs entry names a job that exists in this workflow",
            "gate: no job needs a job it cannot reach",
        ],
    ),
    (
        FileType::Rust,
        &[
            "compiles: build is green with no new warnings",
            "tests: new behavior has a test",
        ],
    ),
    (
        FileType::Python,
        &[
            "compiles: no syntax errors and the linter is clean",
            "tests: new behavior has a test",
        ],
    ),
    (
        FileType::Shell,
        &[
            "shell: strict mode is respected and no expansion is left unquoted",
            "shell: every referenced file and variable exists",
        ],
    ),
    (FileType::Markdown, &[]),
    (
        FileType::Config,
        &[
            "config: every key exists in the schema the consumer validates against",
            "config: no removed key is still referenced elsewhere",
        ],
    ),
    (FileType::Other, &[]),
];

fn row(file_type: FileType) -> &'static [&'static str] {
    CHECKS
        .iter()
        .find(|(kind, _)| *kind == file_type)
        .expect("every file type has a checks row")
        .1
}

/// Classify a file path into the checklist's file types.
///
/// Special names (a file that IS the build or workflow definition) are decided
/// by their basename before any extension, because a `Dockerfile` or a CI file
/// carries no extension to read.
pub fn classify_file_type(path: &str) -> FileType {
    // The .github/workflows/ directory is canonical for CI gate files.
    if path.to_ascii_lowercase().contains("/.github/workflows/") {
        return FileType::Workflow;
    }
    // A well-known file name (or a name with a build-variant suffix, e.g.
    // `Dockerfile.prod`) beats its extension, because the definition carries no
    // meaningful extension to read. Decided on the basename.
    let lower = file_name_of(path).to_ascii_lowercase();
    if lower == "dockerfile" || lower.starts_with("dockerfile.") {
        return FileType::Containerfile;
    }
    if lower == "containerfile" || lower.starts_with("containerfile.") {
        return FileType::Containerfile;
    }
    if lower == "docker-compose.yml" || lower == "docker-compose.yaml" {
        return FileType::Config;
    }
    if lower == "ci.yml" || lower == "ci.yaml" {
        return FileType::Workflow;
    }
    if lower == "makefile" || lower == "justfile" {
        return FileType::Other;
    }

    match extension_of(path) {
        Some("ts") | Some("tsx") | Some("mts") | Some("cts") => FileType::TypeScript,
        Some("rs") => FileType::Rust,
        Some("py") | Some("pyi") => FileType::Python,
        Some("sh") | Some("bash") | Some("zsh") | Some("fish") => FileType::Shell,
        Some("md") | Some("markdown") => FileType::Markdown,
        Some("yaml") | Some("yml") => FileType::Config,
        Some("json") | Some("toml") | Some("ini") | Some("conf") | Some("cfg") => FileType::Config,
        _ => FileType::Other,
    }
}

/// The review checks that apply to a file type, empty when there is nothing
/// mechanical to check for it.
pub fn checks_for(file_type: FileType) -> &'static [&'static str] {
    row(file_type)
}

/// True when a file type carries no mechanical checks; the checklist surfaces
/// such files so they are reported as uncheckable rather than dropped.
pub fn has_no_applicable_check(file_type: FileType) -> bool {
    row(file_type).is_empty()
}

/// The last path segment; an empty path yields an empty name.
fn file_name_of(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or("")
}

/// The extension after the final dot, or none.
fn extension_of(path: &str) -> Option<&str> {
    let name = file_name_of(path);
    name.rsplit_once('.').map(|(_, ext)| ext)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn special_names_win_over_extensions() {
        assert_eq!(
            classify_file_type("app/Dockerfile"),
            FileType::Containerfile
        );
        assert_eq!(
            classify_file_type("service/containerfile.prod"),
            FileType::Containerfile
        );
        assert_eq!(
            classify_file_type(".github/workflows/ci.yml"),
            FileType::Workflow
        );
        assert_eq!(classify_file_type("infra/ci.yaml"), FileType::Workflow);
    }

    #[test]
    fn extensions_map_to_their_types() {
        assert_eq!(classify_file_type("src/config.ts"), FileType::TypeScript);
        assert_eq!(classify_file_type("src/ui.tsx"), FileType::TypeScript);
        assert_eq!(classify_file_type("crates/x/src/lib.rs"), FileType::Rust);
        assert_eq!(classify_file_type("tools/gen.py"), FileType::Python);
        assert_eq!(classify_file_type("scripts/run.sh"), FileType::Shell);
        assert_eq!(classify_file_type("README.md"), FileType::Markdown);
        assert_eq!(classify_file_type("settings.yaml"), FileType::Config);
        assert_eq!(classify_file_type("config.json"), FileType::Config);
        assert_eq!(classify_file_type("Cargo.toml"), FileType::Config);
    }

    #[test]
    fn unknown_and_extensionless_files_are_other() {
        assert_eq!(classify_file_type("assets/logo.png"), FileType::Other);
        assert_eq!(classify_file_type("binary"), FileType::Other);
    }

    #[test]
    fn every_type_resolves_to_a_checks_row() {
        for file_type in [
            FileType::TypeScript,
            FileType::Containerfile,
            FileType::Workflow,
            FileType::Rust,
            FileType::Python,
            FileType::Shell,
            FileType::Markdown,
            FileType::Config,
            FileType::Other,
        ] {
            let _ = checks_for(file_type);
            let _ = has_no_applicable_check(file_type);
        }
    }

    #[test]
    fn code_types_have_checks_and_prose_does_not() {
        assert!(!has_no_applicable_check(FileType::TypeScript));
        assert!(!has_no_applicable_check(FileType::Rust));
        assert!(has_no_applicable_check(FileType::Markdown));
        assert!(has_no_applicable_check(FileType::Other));
        assert!(checks_for(FileType::TypeScript)
            .iter()
            .all(|c| !c.is_empty()));
    }
}
