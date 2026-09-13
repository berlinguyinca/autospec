//! Per-patch language classification for the conversion pass (issue #4559).
//!
//! The repo-wide `shell_ratchet` is a *surface* measurement against a
//! committed ceiling — the right tool for "is the codebase's shell debt
//! shrinking" and the wrong shape for "may this patch land", which needs a
//! per-patch verdict before any branch is created. This module is that
//! verdict.
//!
//! The defect this closes: the conversion pass selected fresh patches, ran
//! the Rust gate, and opened a PR for whatever passed — never asking what
//! language the patch is written in. A shell-only patch passes that gate
//! trivially, because it changes no Rust: `fmt`/`clippy`/`test` are all
//! green. **A green gate on a shell patch means the gate did not read the
//! patch** — the same defect as issue #4532 (a gate whose numbers do not
//! describe the change), arriving from the other direction. Measured on one
//! day's 69 fresh candidates, 35 (51%) touched shell. The standing ruling is
//! that agents write Go or Rust and never shell (#4447); it was enforced at
//! the agent prompt and at the repo-wide ratchet ceiling, but not at the one
//! step that actually lands code.
//!
//! The decision rule, from the patch's file list:
//!
//! - **Rust/Go only** — the gate can evaluate the change; proceed.
//! - **shell only** — the gate cannot fail on this patch. HELD, naming the
//!   ruling. Never gated, never branched: gating it would spend a full
//!   workspace run to produce a green that carries no information.
//! - **mixed** — the agent asked for Rust produced Rust *and* shell. HELD,
//!   naming the shell files: these are the prompt signal, worth counting
//!   separately, not just discarded.
//! - **neither** — docs, fixtures, config. The gate cannot evaluate this
//!   either, so HELD is the *explicit* decision, not a fall-through default.
//!
//! Where the gate cannot evaluate the change, the verdict is
//! **unevaluated** — never `pass`. Everything here is pure: the caller
//! supplies the file list (the `+++ b/<path>` lines of the patch).

use serde::{Deserialize, Serialize};

/// The language class of a patch, as the conversion gate sees it.
///
/// `Default` is the offerable class (the first variant); production code
/// always sets the field from [`classify`], so the default exists for test
/// fixtures, not as a verdict.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PatchLanguage {
    /// Only Rust/Go files: the Rust gate can evaluate the change.
    #[default]
    RustGo,
    /// Only shell/Bats files: the Rust gate cannot fail on this patch.
    Shell,
    /// Rust/Go plus shell: the agent asked for Rust produced shell too.
    Mixed,
    /// Neither (docs, fixtures, config, or an unparseable file list): the
    /// Rust gate cannot evaluate this patch either.
    Neither,
}

impl PatchLanguage {
    /// The machine name used in reports and the JSON plan.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RustGo => "rust-go",
            Self::Shell => "shell",
            Self::Mixed => "mixed",
            Self::Neither => "neither",
        }
    }

    /// The only class the pass may offer: the gate can actually evaluate it.
    /// A patch the gate cannot fail on must not be reported as passing.
    pub fn offerable(self) -> bool {
        matches!(self, Self::RustGo)
    }
}

/// The shell surface, exactly as the repo-wide ratchet measures it: `.sh`
/// and `.bats` files.
fn is_shell_file(path: &str) -> bool {
    ext_of(path).is_some_and(|ext| ext == "sh" || ext == "bats")
}

/// The gateable languages of the standing ruling (#4447): Rust and Go.
fn is_rust_go_file(path: &str) -> bool {
    ext_of(path).is_some_and(|ext| ext == "rs" || ext == "go")
}

/// The file's extension (lowercased, from the final path component), or
/// `None` when the name has no extension.
fn ext_of(path: &str) -> Option<String> {
    path.rsplit('/')
        .next()?
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
}

/// Classify a patch from the file list it touches.
pub fn classify(files: &[String]) -> PatchLanguage {
    let mut shell = false;
    let mut rust_go = false;
    for file in files {
        shell = shell || is_shell_file(file);
        rust_go = rust_go || is_rust_go_file(file);
    }
    match (shell, rust_go) {
        (true, true) => PatchLanguage::Mixed,
        (true, false) => PatchLanguage::Shell,
        (false, true) => PatchLanguage::RustGo,
        (false, false) => PatchLanguage::Neither,
    }
}

/// The HELD reason for a patch the language gate refuses. The reason names
/// the ruling (#4447) so the hold is self-explaining, and — for the
/// interesting classes — the files that decided it: the shell files, so a
/// mixed hold doubles as the prompt signal; the whole (short) list, so a
/// neither hold shows there was nothing to gate.
pub fn hold_reason(language: PatchLanguage, files: &[String]) -> String {
    let shell_files: Vec<&str> = files
        .iter()
        .filter(|f| is_shell_file(f))
        .map(String::as_str)
        .collect();
    match language {
        // The offerable class never reaches a hold reason; the line exists
        // so callers cannot treat "no hold" and "proceed" as different facts.
        PatchLanguage::RustGo => "the gate can evaluate this patch".to_string(),
        PatchLanguage::Shell => format!(
            "language: shell-only patch ({files}) — the standing ruling is that \
             agents write Go or Rust and never shell (#4447); the Rust gate cannot \
             fail on this patch, so it is held unevaluated — never gated, never \
             branched",
            files = shell_files.join(", ")
        ),
        PatchLanguage::Mixed => format!(
            "language: mixed patch — Rust/Go plus shell files ({files}): an agent \
             asked for Rust produced shell too (prompt signal); held unevaluated",
            files = shell_files.join(", ")
        ),
        PatchLanguage::Neither => {
            if files.is_empty() {
                "language: no file list could be parsed from the patch — the Rust \
                 gate cannot evaluate it, so it is held unevaluated (explicit \
                 decision, not a default)"
                    .to_string()
            } else {
                format!(
                    "language: neither Rust/Go nor shell ({files}) — the Rust gate \
                     cannot evaluate this patch, so holding is the explicit \
                     decision, not a default; held unevaluated",
                    files = files.iter().take(5).cloned().collect::<Vec<_>>().join(", ")
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn rust_only_and_go_only_are_offerable() {
        assert_eq!(
            classify(&files(&["crates/autospec-core/src/a.rs"])),
            PatchLanguage::RustGo
        );
        assert_eq!(
            classify(&files(&["cmd/tool/main.go"])),
            PatchLanguage::RustGo
        );
        assert!(PatchLanguage::RustGo.offerable());
    }

    #[test]
    fn rust_plus_docs_is_still_offerable() {
        // A changelog fragment beside the code does not make the patch
        // ungatetable: the gate evaluates the Rust.
        assert_eq!(
            classify(&files(&[
                "crates/autospec-core/src/a.rs",
                "changelog.d/4559-language-gate.md",
            ])),
            PatchLanguage::RustGo
        );
    }

    #[test]
    fn shell_only_is_not_offerable() {
        assert_eq!(classify(&files(&["scripts/x.sh"])), PatchLanguage::Shell);
        assert_eq!(classify(&files(&["tests/y.bats"])), PatchLanguage::Shell);
        assert_eq!(
            classify(&files(&["scripts/a.sh", "tests/b.bats"])),
            PatchLanguage::Shell
        );
        assert!(!PatchLanguage::Shell.offerable());
    }

    #[test]
    fn rust_plus_shell_is_mixed() {
        assert_eq!(
            classify(&files(&[
                "crates/autospec-cli/src/convert.rs",
                "scripts/x.sh"
            ])),
            PatchLanguage::Mixed
        );
        assert_eq!(
            classify(&files(&["cmd/t/main.go", "tests/y.bats"])),
            PatchLanguage::Mixed
        );
        assert!(!PatchLanguage::Mixed.offerable());
    }

    #[test]
    fn neither_is_docs_fixtures_config_or_nothing() {
        assert_eq!(
            classify(&files(&["README.md", "docs/x.md"])),
            PatchLanguage::Neither
        );
        assert_eq!(
            classify(&files(&["tests/fixtures/data.json", "config.yml"])),
            PatchLanguage::Neither
        );
        assert_eq!(classify(&[]), PatchLanguage::Neither);
        assert!(!PatchLanguage::Neither.offerable());
    }

    #[test]
    fn the_wire_form_is_kebab_case() {
        for (language, wire) in [
            (PatchLanguage::RustGo, "rust-go"),
            (PatchLanguage::Shell, "shell"),
            (PatchLanguage::Mixed, "mixed"),
            (PatchLanguage::Neither, "neither"),
        ] {
            assert_eq!(
                serde_json::to_string(&language).unwrap(),
                format!("\"{wire}\"")
            );
            let parsed: PatchLanguage = serde_json::from_str(&format!("\"{wire}\"")).unwrap();
            assert_eq!(parsed, language);
        }
    }

    #[test]
    fn the_shell_hold_names_the_ruling() {
        let reason = hold_reason(
            PatchLanguage::Shell,
            &files(&["scripts/x.sh", "tests/y.bats"]),
        );
        assert!(reason.contains("#4447"), "{reason}");
        assert!(reason.contains("scripts/x.sh"), "{reason}");
        assert!(reason.contains("tests/y.bats"), "{reason}");
        assert!(reason.contains("unevaluated"), "{reason}");
    }

    #[test]
    fn the_mixed_hold_names_the_shell_files_as_the_prompt_signal() {
        let reason = hold_reason(
            PatchLanguage::Mixed,
            &files(&["crates/autospec-cli/src/convert.rs", "scripts/x.sh"]),
        );
        // The Rust file is context, not the signal: the reason names the
        // shell file and the prompt it implicates.
        assert!(reason.contains("scripts/x.sh"), "{reason}");
        assert!(reason.contains("prompt signal"), "{reason}");
        assert!(!reason.contains("convert.rs"), "{reason}");
    }

    #[test]
    fn the_neither_hold_is_an_explicit_decision_not_a_default() {
        let reason = hold_reason(PatchLanguage::Neither, &files(&["README.md"]));
        assert!(reason.contains("README.md"), "{reason}");
        assert!(reason.contains("explicit"), "{reason}");
        let empty = hold_reason(PatchLanguage::Neither, &[]);
        assert!(empty.contains("no file list"), "{empty}");
    }
}
