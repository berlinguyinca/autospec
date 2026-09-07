//! Symbol preservation gate (issue #3565, ask 2).
//!
//! A conflict resolution that takes only one side compiles and passes the
//! test suite while silently dropping the other side's work. A
//! build-and-test gate cannot see that. This module implements the cheap,
//! effective check: **the union of the symbols added by both parents must be
//! a subset of the symbols present after resolution.** A resolution that
//! drops one is rejected with the dropped symbol named.

use super::resolution::{Resolution, SemanticConflict};
use std::collections::{BTreeSet, HashMap};

/// Language keywords and other non-symbol tokens excluded by
/// [`extract_symbols`]. Deliberately conservative: only tokens that are
/// almost certainly not the feature being preserved. Case-sensitive.
pub const KEYWORDS: &[&str] = &[
    // Rust
    "fn",
    "let",
    "mut",
    "pub",
    "struct",
    "enum",
    "impl",
    "trait",
    "use",
    "mod",
    "crate",
    "self",
    "Self",
    "super",
    "return",
    "if",
    "else",
    "match",
    "while",
    "for",
    "loop",
    "const",
    "static",
    "ref",
    "where",
    "async",
    "await",
    "move",
    "dyn",
    "true",
    "false",
    // Go
    "func",
    "package",
    "import",
    "type",
    "interface",
    "map",
    "chan",
    "range",
    "defer",
    "go",
    "nil",
    "var",
    "byte",
    "error",
    // C / Java / JS / TS
    "int",
    "char",
    "float",
    "double",
    "void",
    "long",
    "short",
    "bool",
    "unsigned",
    "signed",
    "final",
    "public",
    "private",
    "protected",
    "class",
    "extends",
    "implements",
    "try",
    "catch",
    "finally",
    "throw",
    "throws",
    "null",
    "new",
    "this",
    "typeof",
    "instanceof",
    "in",
    "of",
    "undefined",
    "switch",
    "case",
    "break",
    "continue",
    "default",
    "function",
    "string",
    "number",
    "boolean",
    "any",
    "unknown",
    "readonly",
    // Python
    "def",
    "elif",
    "lambda",
    "global",
    "nonlocal",
    "assert",
    "except",
    "raise",
    "pass",
    "with",
    "as",
    "None",
    "True",
    "False",
    "and",
    "or",
    "not",
    "is",
    "del",
    // Kotlin / Scala / Swift
    "fun",
    "val",
    "object",
    "sealed",
    "when",
    "override",
    "open",
    "internal",
    "companion",
    "data",
    "then",
    // generic control
    "do",
    "end",
    "yield",
    "export",
    "from",
    "begin",
    "done",
    // SQL
    "select",
    "insert",
    "into",
    "values",
    "update",
    "delete",
    "create",
    "table",
    "primary",
    "key",
    "foreign",
    "references",
    "unique",
    "index",
    "drop",
    "alter",
];

/// Extract the identifier symbols present in `text`.
///
/// A symbol is a run of ASCII letters and digits (starting with a letter or
/// underscore) of length at least two that is not in [`KEYWORDS`]. This is a
/// deliberately cheap textual heuristic — good enough to catch a resolution
/// that drops a struct field, a function parameter, or a guard clause,
/// which is what a green build cannot.
pub fn extract_symbols(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for line in text.lines() {
        let mut word = String::new();
        let mut push = |word: &mut String| {
            // Always take the word, keyword or not: a keyword token must not
            // linger in the buffer and merge with the identifier that follows
            // it ("func main" is two words, not "funcmain").
            let w = std::mem::take(word);
            if w.len() >= 2 && !KEYWORDS.contains(&w.as_str()) {
                out.insert(w);
            }
        };
        for ch in line.chars() {
            if ch.is_ascii_alphabetic() || ch == '_' {
                word.push(ch);
            } else if ch.is_ascii_digit() && !word.is_empty() {
                word.push(ch);
            } else {
                push(&mut word);
            }
        }
        push(&mut word);
    }
    out
}

/// Multiset difference of `side` against `ancestor`, preserving the order of
/// `side`.
///
/// Returns `(added, deleted)` where `added` is the list of lines in `side`
/// that are not accounted for by `ancestor`, and `deleted` is the number of
/// `ancestor` lines that `side` no longer contains.
pub fn side_additions(ancestor: &[String], side: &[String]) -> (Vec<String>, usize) {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for line in ancestor {
        *counts.entry(line.as_str()).or_insert(0) += 1;
    }
    let mut added = Vec::new();
    for line in side {
        if let Some(count) = counts.get_mut(line.as_str()) {
            if *count > 0 {
                *count -= 1;
                continue;
            }
        }
        added.push(line.clone());
    }
    let deleted: usize = counts.values().sum();
    (added, deleted)
}

/// A symbol that was added by one parent and is missing after resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingSymbol {
    pub file: String,
    pub symbol: String,
}

/// The failure produced when a resolution drops preserved work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreservationFailure {
    /// Symbols added by either parent are absent from the resolved tree.
    SymbolLoss { missing: Vec<MissingSymbol> },
    /// A conflicted file has no resolved version at all.
    FileUnresolved { path: String },
}

impl std::fmt::Display for PreservationFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SymbolLoss { missing } => write!(
                f,
                "resolution dropped symbol(s) added by a parent: {}",
                missing
                    .iter()
                    .map(|m| format!("{}: {}", m.file, m.symbol))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::FileUnresolved { path } => {
                write!(f, "conflicted file has no resolved version: {path}")
            }
        }
    }
}

/// Verify that every symbol added by either parent of every conflict is
/// still present in the resolved tree.
///
/// Returns [`None`] when the resolution preserves both sides, or the
/// concrete failure when it does not.
pub fn check_preservation(
    files: &[super::vcs::ConflictedFile],
    resolution: &Resolution,
) -> Option<PreservationFailure> {
    let mut missing: Vec<MissingSymbol> = Vec::new();
    for file in files {
        let resolved = match resolution.files.iter().find(|r| r.path == file.path) {
            Some(r) => r,
            None => {
                return Some(PreservationFailure::FileUnresolved {
                    path: file.path.clone(),
                })
            }
        };
        let mut required: BTreeSet<String> = BTreeSet::new();
        for hunk in &file.hunks {
            let (ours_added, _) = side_additions(&hunk.ancestor, &hunk.ours);
            let (theirs_added, _) = side_additions(&hunk.ancestor, &hunk.theirs);
            let text = ours_added
                .iter()
                .chain(theirs_added.iter())
                .cloned()
                .collect::<Vec<_>>()
                .join("\n");
            required.extend(extract_symbols(&text));
        }
        if required.is_empty() {
            continue;
        }
        let present = extract_symbols(&resolved.content);
        for symbol in required.difference(&present) {
            missing.push(MissingSymbol {
                file: file.path.clone(),
                symbol: symbol.clone(),
            });
        }
    }
    if missing.is_empty() {
        None
    } else {
        Some(PreservationFailure::SymbolLoss { missing })
    }
}

/// Classify and gate a resolution in one call: a hunk where either side
/// modified or deleted an ancestor line is a genuine semantic disagreement
/// (must halt, not resolve); otherwise the resolution must pass
/// [`check_preservation`].
pub fn classify_and_gate(
    files: &[super::vcs::ConflictedFile],
    resolution: &Resolution,
) -> Result<(), GateFailure> {
    let conflicts = SemanticConflict::find(files);
    if !conflicts.is_empty() {
        return Err(GateFailure::Semantic { conflicts });
    }
    match check_preservation(files, resolution) {
        None => Ok(()),
        Some(failure) => Err(GateFailure::Preservation(failure)),
    }
}

/// Failure of the post-resolution gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateFailure {
    /// The conflict is a genuine semantic disagreement, not an additive
    /// coexistence. The branch must halt and be reported for a human.
    Semantic { conflicts: Vec<SemanticConflict> },
    /// The resolution is additive-shaped but dropped preserved work.
    Preservation(PreservationFailure),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn extract_symbols_finds_identifiers_and_skips_keywords() {
        let symbols = extract_symbols(
            "    telemetry   *telemetryStore\n    regToken    string\nfunc main() {}\n",
        );
        assert!(symbols.contains("telemetry"));
        assert!(symbols.contains("telemetryStore"));
        assert!(symbols.contains("regToken"));
        eprintln!("SYMS {symbols:?}");
        assert!(symbols.contains("main"));
        assert!(!symbols.contains("func"));
        assert!(!symbols.contains("string"));
        assert!(!symbols.contains("main2x"));
    }

    #[test]
    fn extract_symbols_skips_numbers_and_single_letters() {
        let symbols = extract_symbols("count := 42\nx = 1\nversion2 := v3\n");
        assert!(symbols.contains("count"));
        assert!(symbols.contains("version2"));
        assert!(!symbols.contains("42"));
        assert!(!symbols.contains("x"));
    }

    #[test]
    fn side_additions_reports_added_and_deleted() {
        let ancestor = lines(&["a", "b", "c"]);
        let side = lines(&["a", "x", "b", "y"]);
        let (added, deleted) = side_additions(&ancestor, &side);
        assert_eq!(added, lines(&["x", "y"]));
        assert_eq!(deleted, 1); // "c" gone
    }

    #[test]
    fn check_preservation_accepts_union_and_rejects_dropped_side() {
        use super::super::vcs::{ConflictHunk, ConflictedFile};
        use crate::integration::resolution::Resolution;
        use crate::integration::vcs::ResolvedFile;

        let file = ConflictedFile {
            path: "store.go".to_string(),
            base:
                "type Store struct {\n\tcore        *coreClient\n\ttelemetry   *telemetryStore\n}\n"
                    .to_string(),
            hunks: vec![ConflictHunk {
                start: 1,
                ancestor: lines(&["\tcore        *coreClient"]),
                ours: lines(&["\tcore        *coreClient", "\ttelemetry   *telemetryStore"]),
                theirs: lines(&[
                    "\tcore        *coreClient",
                    "\tregToken    string",
                    "\tendpoints   endpointPolicy",
                ]),
            }],
        };
        let union = Resolution {
            files: vec![ResolvedFile {
                path: "store.go".to_string(),
                content: "type Store struct {\n\tcore        *coreClient\n\ttelemetry   *telemetryStore\n\tregToken    string\n\tendpoints   endpointPolicy\n}\n".to_string(),
            }],
        };
        assert_eq!(check_preservation(&[file.clone()], &union), None);

        // "take HEAD": the branch's fields vanish; the build would still pass.
        let take_ours = Resolution {
            files: vec![ResolvedFile {
                path: "store.go".to_string(),
                content: "type Store struct {\n\tcore        *coreClient\n\ttelemetry   *telemetryStore\n}\n".to_string(),
            }],
        };
        match check_preservation(&[file.clone()], &take_ours) {
            Some(PreservationFailure::SymbolLoss { missing }) => {
                let names: Vec<&str> = missing.iter().map(|m| m.symbol.as_str()).collect();
                assert!(names.contains(&"regToken"), "{names:?}");
                assert!(names.contains(&"endpoints"), "{names:?}");
                assert!(names.contains(&"endpointPolicy"), "{names:?}");
            }
            other => panic!("expected SymbolLoss, got {other:?}"),
        }

        // "take branch": the trunk's field vanishes.
        let take_theirs = Resolution {
            files: vec![ResolvedFile {
                path: "store.go".to_string(),
                content: "type Store struct {\n\tcore        *coreClient\n\tregToken    string\n\tendpoints   endpointPolicy\n}\n".to_string(),
            }],
        };
        match check_preservation(&[file], &take_theirs) {
            Some(PreservationFailure::SymbolLoss { missing }) => {
                let names: Vec<&str> = missing.iter().map(|m| m.symbol.as_str()).collect();
                assert!(names.contains(&"telemetry"), "{names:?}");
                assert!(names.contains(&"telemetryStore"), "{names:?}");
            }
            other => panic!("expected SymbolLoss, got {other:?}"),
        }
    }
}
