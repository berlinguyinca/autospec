//! Resolution of a repository's implementation language (issue #4447).
//!
//! Agents must be told to write **Go** or **Rust** — never shell scripts — and
//! the language is a fact of the repository, not a per-issue judgement call.
//! This module is the single source of truth for the mapping, so every issue
//! generator and the shell ratchet resolve the same answer.
//!
//! Two reasons, the second not negotiable by taste: the project is too complex
//! for shell, and it may be executed under different operating systems. Shell
//! encodes host assumptions — GNU vs BSD flag spellings, `/proc`,
//! `readlink -f`, `mktemp` syntax, which `grep` is on `$PATH` — that do not
//! survive the move. A compiled Go or Rust binary carries its behaviour with it.
//!
//! The per-repository mapping, stated once:
//!
//! - `metabolomics-us/*` -> **Go**
//! - `InferWeave/*`, `berlinguyinca/autospec` -> **Rust**
//!
//! Shell remains admissible only for what genuinely cannot be a binary: a cron
//! line, and the few lines needed to launch a compiled artifact. Neither is a
//! place for logic.

/// The implementation language an agent must write in a repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImplementationLanguage {
    /// `metabolomics-us/*`
    Go,
    /// `InferWeave/*` and `berlinguyinca/autospec`
    Rust,
}

impl ImplementationLanguage {
    /// The display name, as an issue body should spell it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Go => "Go",
            Self::Rust => "Rust",
        }
    }
}

/// Resolve the implementation language for a repository (`owner/name`).
///
/// Returns `None` for a repository whose language is not settled: an issue
/// generator that cannot resolve a language must refuse to file the issue
/// rather than guess — an issue that reaches an agent without a named language
/// is a defect, not a prompt to improvise.
///
/// Matching is owner-based for the wildcard namespaces (`metabolomics-us/*`,
/// `InferWeave/*`) and exact for `berlinguyinca/autospec`. Repository and owner
/// comparisons are case-insensitive, as GitHub resolves them.
pub fn implementation_language(repo: &str) -> Option<ImplementationLanguage> {
    let repo = repo.trim();
    if repo.eq_ignore_ascii_case("berlinguyinca/autospec") {
        return Some(ImplementationLanguage::Rust);
    }
    let (owner, _) = repo.split_once('/')?;
    match owner.to_ascii_lowercase().as_str() {
        "metabolomics-us" => Some(ImplementationLanguage::Go),
        "inferweave" => Some(ImplementationLanguage::Rust),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{implementation_language, ImplementationLanguage};

    #[test]
    fn metabolomics_us_owner_resolves_to_go() {
        assert_eq!(
            implementation_language("metabolomics-us/inferweave-gateway"),
            Some(ImplementationLanguage::Go)
        );
        assert_eq!(
            implementation_language("metabolomics-us/anything"),
            Some(ImplementationLanguage::Go)
        );
    }

    #[test]
    fn inferweave_owner_resolves_to_rust() {
        assert_eq!(
            implementation_language("InferWeave/inferweave"),
            Some(ImplementationLanguage::Rust)
        );
        assert_eq!(
            implementation_language("inferweave/gateway"),
            Some(ImplementationLanguage::Rust)
        );
    }

    #[test]
    fn the_autospec_repository_resolves_to_rust() {
        assert_eq!(
            implementation_language("berlinguyinca/autospec"),
            Some(ImplementationLanguage::Rust)
        );
        assert_eq!(
            implementation_language("BERLINGUYINCA/autospec"),
            Some(ImplementationLanguage::Rust)
        );
    }

    #[test]
    fn an_unknown_repository_resolves_to_none() {
        assert_eq!(implementation_language("somebody/else"), None);
        assert_eq!(implementation_language("berlinguyinca/other"), None);
        assert_eq!(implementation_language("not a repo"), None);
        assert_eq!(implementation_language(""), None);
    }

    #[test]
    fn the_display_name_matches_the_mapping() {
        assert_eq!(ImplementationLanguage::Go.as_str(), "Go");
        assert_eq!(ImplementationLanguage::Rust.as_str(), "Rust");
    }
}
