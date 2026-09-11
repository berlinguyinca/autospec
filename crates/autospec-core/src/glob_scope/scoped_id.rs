//! Scope-carrying identifiers: `iw#14`, never `14`.
//!
//! A bare integer is not an identifier — it is an identifier *modulo an
//! implicit scope*, and the scope gets filled in by whatever context is
//! nearest. The incident: a patch pipeline keyed on `issue-<N>` directories
//! across four projects sharing one filesystem, opened PR #14 against the
//! wrong repository, because the repository was implied by the current
//! directory rather than carried by the key. `14` in InferWeave and `14` in
//! autospec are two different things, and nothing in the pipeline's data said
//! which one it held.
//!
//! The rule this module enforces: the scope is attached where the identifier
//! is *read*, and a bare number is resolved only when exactly one scope could
//! have meant it — [`resolve_bare`] never falls back to "the current repo".

use std::collections::BTreeMap;
use std::fmt;

/// The separator between scope and number in the canonical form `scope#number`.
pub const SCOPE_SEPARATOR: char = '#';

/// Why an identifier could not be given a scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeError {
    /// The value is a bare integer. The scope must come from the caller, never
    /// from a default.
    MissingScope { value: String },
    /// The scope part is empty (`#14`) or carries a character that cannot
    /// survive being printed into a log line, a branch name or a URL.
    BadScope { value: String },
    /// The part after `#` is absent or not a non-negative integer.
    BadNumber { value: String },
}

impl fmt::Display for ScopeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingScope { value } => write!(
                f,
                "'{value}' is a bare number, which is ambiguous across repositories: carry the scope ('iw{sep}14')",
                sep = SCOPE_SEPARATOR
            ),
            Self::BadScope { value } => write!(
                f,
                "'{value}' has an empty or unusable scope; use scope{sep}number with scope in [A-Za-z0-9._/-]",
                sep = SCOPE_SEPARATOR
            ),
            Self::BadNumber { value } => write!(
                f,
                "'{value}' has no integer after the '{sep}'",
                sep = SCOPE_SEPARATOR
            ),
        }
    }
}

impl std::error::Error for ScopeError {}

/// An identifier that carries the scope it belongs to.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ScopedId {
    /// The project the number was read from (`iw`, `owner/repo`, …).
    pub scope: String,
    /// The number within that scope.
    pub number: u64,
}

impl ScopedId {
    /// Build a scoped identifier, validating both halves.
    pub fn new(scope: &str, number: u64) -> Result<Self, ScopeError> {
        if scope.is_empty()
            || scope.contains(SCOPE_SEPARATOR)
            || !scope
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'/'))
        {
            return Err(ScopeError::BadScope {
                value: format!("{scope}{SCOPE_SEPARATOR}{number}"),
            });
        }
        Ok(Self {
            scope: scope.to_owned(),
            number,
        })
    }

    /// Parse `scope#number`.
    ///
    /// A bare integer is rejected with [`ScopeError::MissingScope`] — that
    /// rejection is the point of the type: a pipeline that cannot represent a
    /// scopeless identifier cannot leak one into another repository.
    pub fn parse(value: &str) -> Result<Self, ScopeError> {
        let Some((scope, number)) = value.split_once(SCOPE_SEPARATOR) else {
            return if value.bytes().all(|b| b.is_ascii_digit()) && !value.is_empty() {
                Err(ScopeError::MissingScope {
                    value: value.to_owned(),
                })
            } else {
                Err(ScopeError::BadScope {
                    value: value.to_owned(),
                })
            };
        };
        if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
            return Err(ScopeError::BadNumber {
                value: value.to_owned(),
            });
        }
        Self::new(scope, number.parse().unwrap_or(0))
    }

    /// The canonical `scope#number` text.
    pub fn key(&self) -> String {
        format!("{}{SCOPE_SEPARATOR}{}", self.scope, self.number)
    }
}

impl fmt::Display for ScopedId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{SCOPE_SEPARATOR}{}", self.scope, self.number)
    }
}

/// What a bare number resolves to, given the identifiers actually present.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// Exactly one scope holds that number.
    Unique(ScopedId),
    /// More than one scope holds it. The scopes are named; there is no winner.
    Ambiguous {
        number: u64,
        scopes: Vec<String>,
        ids: Vec<ScopedId>,
    },
    /// No known scope holds it.
    Unknown { number: u64 },
}

impl Resolution {
    /// The resolved identifier, if there is exactly one candidate. Never a
    /// default: ambiguity and ignorance both return `None`.
    pub fn id(&self) -> Option<&ScopedId> {
        match self {
            Self::Unique(id) => Some(id),
            _ => None,
        }
    }
}

impl fmt::Display for Resolution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unique(id) => write!(f, "{id}"),
            Self::Ambiguous { number, scopes, .. } => write!(
                f,
                "{number} is ambiguous: held by {} ({})",
                scopes.len(),
                scopes.join(", ")
            ),
            Self::Unknown { number } => write!(f, "{number} is not known in any scope"),
        }
    }
}

/// Resolve a bare number against known scoped identifiers.
///
/// Resolution is a *report*, not a choice: with two candidates the caller must
/// pick a scope explicitly, because guessing the current repository is exactly
/// the bug this type exists to prevent.
pub fn resolve_bare(number: u64, known: &[ScopedId]) -> Resolution {
    let mut ids: Vec<ScopedId> = known
        .iter()
        .filter(|id| id.number == number)
        .cloned()
        .collect();
    ids.sort();
    ids.dedup();
    match ids.len() {
        0 => Resolution::Unknown { number },
        1 => Resolution::Unique(ids.into_iter().next().expect("len 1")),
        _ => Resolution::Ambiguous {
            number,
            scopes: ids.iter().map(|id| id.scope.clone()).collect(),
            ids,
        },
    }
}

/// A set of artifacts keyed by scoped identifier, for the collision sweep.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScopeLedger {
    entries: Vec<(ScopedId, String)>,
}

impl ScopeLedger {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record `artifact` (a path, a branch, a PR reference) under its scope.
    pub fn add(&mut self, id: ScopedId, artifact: impl Into<String>) {
        self.entries.push((id, artifact.into()));
    }

    /// Every artifact recorded under the bare `number`, across all scopes.
    pub fn artifacts_for(&self, number: u64) -> Vec<(&ScopedId, &str)> {
        self.entries
            .iter()
            .filter(|(id, _)| id.number == number)
            .map(|(id, artifact)| (id, artifact.as_str()))
            .collect()
    }

    /// Numbers that appear under more than one scope: the keys a bare-integer
    /// pipeline cannot resolve without guessing.
    pub fn ambiguous_numbers(&self) -> Vec<(u64, Vec<String>)> {
        let mut by_number: BTreeMap<u64, Vec<String>> = BTreeMap::new();
        for (id, _) in &self.entries {
            let scopes = by_number.entry(id.number).or_default();
            if !scopes.contains(&id.scope) {
                scopes.push(id.scope.clone());
            }
        }
        by_number
            .into_iter()
            .filter(|(_, scopes)| scopes.len() > 1)
            .map(|(number, mut scopes)| {
                scopes.sort();
                (number, scopes)
            })
            .collect()
    }

    /// How many entries each scope holds — the `uniq -c` view of the ledger.
    pub fn scope_counts(&self) -> Vec<(String, usize)> {
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for (id, _) in &self.entries {
            *counts.entry(id.scope.clone()).or_default() += 1;
        }
        counts.into_iter().collect()
    }

    /// One line per ambiguous number, naming the scopes that hold it.
    pub fn collision_line(&self) -> String {
        let ambiguous = self.ambiguous_numbers();
        if ambiguous.is_empty() {
            return "no bare-number collisions".to_owned();
        }
        let items = ambiguous
            .iter()
            .map(|(number, scopes)| format!("{} in {} ({})", number, scopes.len(), scopes.join(", ")))
            .collect::<Vec<_>>()
            .join("; ");
        format!(
            "{} bare number(s) collide across scopes: {}",
            ambiguous.len(),
            items
        )
    }

    /// Entries in insertion order.
    pub fn entries(&self) -> &[(ScopedId, String)] {
        &self.entries
    }
}
