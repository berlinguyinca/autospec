use std::collections::BTreeMap;

use serde::Serialize;

/// A language the gateway can provision a server for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Language {
    Rust,
    Python,
    TypeScript,
    JavaScript,
    Java,
    Go,
    C,
    Cpp,
    CSharp,
    Kotlin,
    Scala,
}

/// `(language, config key, default server, marker files, source extensions)`.
///
/// One row per language keeps detection, the default server mapping, and the
/// config vocabulary in a single table instead of parallel match arms.
type LanguageRow = (
    Language,
    &'static str,
    &'static str,
    &'static [&'static str],
    &'static [&'static str],
);

const LANGUAGES: &[LanguageRow] = &[
    (
        Language::Rust,
        "rust",
        "rust-analyzer",
        &["Cargo.toml"],
        &["rs"],
    ),
    (
        Language::Python,
        "python",
        "pyright-langserver",
        &[
            "pyproject.toml",
            "setup.py",
            "setup.cfg",
            "requirements.txt",
        ],
        &["py", "pyi"],
    ),
    (
        Language::TypeScript,
        "typescript",
        "typescript-language-server",
        &["tsconfig.json"],
        &["ts", "tsx", "mts", "cts"],
    ),
    (
        Language::JavaScript,
        "javascript",
        "typescript-language-server",
        &["package.json", "jsconfig.json"],
        &["js", "jsx", "mjs", "cjs"],
    ),
    (
        Language::Java,
        "java",
        "jdtls",
        &["pom.xml", "build.gradle", "build.gradle.kts"],
        &["java"],
    ),
    (Language::Go, "go", "gopls", &["go.mod"], &["go"]),
    (
        Language::C,
        "c",
        "clangd",
        &["compile_commands.json"],
        &["c", "h"],
    ),
    (
        Language::Cpp,
        "cpp",
        "clangd",
        &["CMakeLists.txt", "compile_commands.json"],
        &["cc", "cpp", "cxx", "hpp", "hh"],
    ),
    (
        Language::CSharp,
        "csharp",
        "omnisharp",
        &["*.csproj", "*.sln"],
        &["cs"],
    ),
    (
        Language::Kotlin,
        "kotlin",
        "kotlin-language-server",
        &["build.gradle.kts"],
        &["kt", "kts"],
    ),
    (
        Language::Scala,
        "scala",
        "metals",
        // Deliberately not pom.xml: a Maven build file alone says Java, not
        // Scala. A Maven/Scala project is detected by its .scala sources.
        &["build.sbt"],
        &["scala", "sc"],
    ),
];

fn row(language: Language) -> &'static LanguageRow {
    LANGUAGES
        .iter()
        .find(|entry| entry.0 == language)
        .expect("every Language variant has a table row")
}

impl Language {
    /// The key this language is addressed by in `code-intelligence.yaml`.
    pub fn config_key(self) -> &'static str {
        row(self).1
    }

    /// The server used when the operator declares no override.
    pub fn default_server(self) -> &'static str {
        row(self).2
    }

    pub fn source_extensions(self) -> &'static [&'static str] {
        row(self).4
    }

    pub fn parse(key: &str) -> Option<Self> {
        LANGUAGES
            .iter()
            .find(|entry| entry.1 == key)
            .map(|entry| entry.0)
    }

    pub fn all() -> Vec<Self> {
        LANGUAGES.iter().map(|entry| entry.0).collect()
    }

    /// Whether starting this language's server can execute project build code
    /// (build.gradle, setup.py, build.sbt, ...). Those servers stay gated behind
    /// `security.trust_project_build_scripts`.
    pub fn server_runs_build_scripts(self) -> bool {
        matches!(self, Self::Java | Self::Kotlin | Self::Scala | Self::Python)
    }

    fn matches_marker(self, file_name: &str) -> bool {
        row(self)
            .3
            .iter()
            .any(|marker| match marker.strip_prefix('*') {
                Some(suffix) => file_name.ends_with(suffix),
                None => *marker == file_name,
            })
    }

    fn matches_extension(self, extension: &str) -> bool {
        self.source_extensions().contains(&extension)
    }
}

/// A language found in a worktree, plus the evidence that found it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DetectedLanguage {
    pub language: String,
    pub server: String,
    pub evidence: Vec<String>,
    pub source_files: usize,
}

/// Detect languages from a worktree file listing.
///
/// Paths are worktree-relative. Detection is pure so it is testable without a
/// filesystem: callers walk the worktree once and pass the listing in.
pub fn detect(paths: &[String], overrides: &BTreeMap<String, String>) -> Vec<DetectedLanguage> {
    let mut evidence: BTreeMap<Language, Vec<String>> = BTreeMap::new();
    let mut counts: BTreeMap<Language, usize> = BTreeMap::new();
    for path in paths {
        record_markers(path, &mut evidence);
        record_source_file(path, &mut counts);
    }
    let mut detected = Vec::new();
    for language in Language::all() {
        let markers = evidence.remove(&language).unwrap_or_default();
        let source_files = counts.remove(&language).unwrap_or(0);
        if markers.is_empty() && source_files == 0 {
            continue;
        }
        detected.push(DetectedLanguage {
            language: language.config_key().to_string(),
            server: resolve_server(language, overrides),
            evidence: markers,
            source_files,
        });
    }
    detected
}

/// The server to launch for a language: the operator override wins, otherwise
/// the table default.
pub fn resolve_server(language: Language, overrides: &BTreeMap<String, String>) -> String {
    overrides
        .get(language.config_key())
        .filter(|server| !server.is_empty())
        .cloned()
        .unwrap_or_else(|| language.default_server().to_string())
}

fn record_markers(path: &str, evidence: &mut BTreeMap<Language, Vec<String>>) {
    let file_name = file_name_of(path);
    for language in Language::all() {
        if language.matches_marker(file_name) {
            evidence.entry(language).or_default().push(path.to_string());
        }
    }
}

fn record_source_file(path: &str, counts: &mut BTreeMap<Language, usize>) {
    let Some(extension) = extension_of(path) else {
        return;
    };
    for language in Language::all() {
        if language.matches_extension(extension) {
            *counts.entry(language).or_default() += 1;
        }
    }
}

fn file_name_of(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn extension_of(path: &str) -> Option<&str> {
    let file_name = file_name_of(path);
    let (stem, extension) = file_name.rsplit_once('.')?;
    if stem.is_empty() {
        return None;
    }
    Some(extension)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detect_keys(paths: &[&str]) -> Vec<String> {
        let paths = paths
            .iter()
            .map(|path| path.to_string())
            .collect::<Vec<_>>();
        detect(&paths, &BTreeMap::new())
            .into_iter()
            .map(|entry| entry.language)
            .collect()
    }

    #[test]
    fn every_language_has_a_complete_table_row() {
        for language in Language::all() {
            assert!(!language.config_key().is_empty());
            assert!(!language.default_server().is_empty());
            assert!(!language.source_extensions().is_empty());
            assert_eq!(Language::parse(language.config_key()), Some(language));
        }
    }

    #[test]
    fn manifest_files_detect_their_language() {
        assert_eq!(detect_keys(&["Cargo.toml"]), vec!["rust"]);
        assert_eq!(detect_keys(&["go.mod"]), vec!["go"]);
        assert_eq!(
            detect_keys(&["services/api/tsconfig.json"]),
            vec!["typescript"]
        );
    }

    #[test]
    fn source_files_alone_detect_a_language() {
        let detected = detect_keys(&["src/main/scala/Api.scala"]);

        assert_eq!(detected, vec!["scala"]);
    }

    #[test]
    fn detection_counts_source_files_and_records_marker_evidence() {
        let paths = ["Cargo.toml", "src/lib.rs", "src/gateway.rs"]
            .iter()
            .map(|path| path.to_string())
            .collect::<Vec<_>>();

        let detected = detect(&paths, &BTreeMap::new());

        assert_eq!(detected.len(), 1);
        assert_eq!(detected[0].source_files, 2);
        assert_eq!(detected[0].evidence, vec!["Cargo.toml".to_string()]);
        assert_eq!(detected[0].server, "rust-analyzer");
    }

    #[test]
    fn a_polyglot_worktree_detects_every_language() {
        let detected = detect_keys(&["Cargo.toml", "go.mod", "pyproject.toml", "pom.xml"]);

        assert_eq!(detected, vec!["rust", "python", "java", "go"]);
    }

    #[test]
    fn overrides_replace_the_default_server() {
        let mut overrides = BTreeMap::new();
        overrides.insert("python".to_string(), "basedpyright".to_string());
        let paths = vec!["pyproject.toml".to_string()];

        let detected = detect(&paths, &overrides);

        assert_eq!(detected[0].server, "basedpyright");
    }

    #[test]
    fn an_empty_override_falls_back_to_the_default_server() {
        let mut overrides = BTreeMap::new();
        overrides.insert("rust".to_string(), String::new());

        assert_eq!(resolve_server(Language::Rust, &overrides), "rust-analyzer");
    }

    #[test]
    fn glob_markers_match_by_suffix() {
        assert_eq!(detect_keys(&["src/Api.csproj"]), vec!["csharp"]);
    }

    #[test]
    fn dotfiles_are_not_treated_as_source_extensions() {
        assert!(detect_keys(&[".gitignore"]).is_empty());
    }

    #[test]
    fn build_script_executing_servers_are_flagged() {
        assert!(Language::Java.server_runs_build_scripts());
        assert!(Language::Scala.server_runs_build_scripts());
        assert!(!Language::Rust.server_runs_build_scripts());
        assert!(!Language::Go.server_runs_build_scripts());
    }
}
