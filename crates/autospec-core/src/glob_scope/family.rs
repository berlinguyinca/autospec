//! Family derivation and pattern audits: which names a prefix really takes in.
//!
//! The unit of a namespace is not the character, it is the **family**. A job
//! called `qwen3.8-27b-vision-1` belongs to the family `qwen3.8-27b-vision`
//! and is instance `1` of it; a job called `qwen3.8-27b-1` belongs to
//! `qwen3.8-27b`. The two families share a literal prefix, so every pattern
//! written against the shorter one takes in members of the longer one, and
//! nothing about the pattern says so.
//!
//! [`classify`] derives that structure from a name, [`prefix_collisions`]
//! reports the pairs of families where one name is a prefix of another (the
//! sweep the issue asks for: *a glob or prefix match whose literal part is
//! itself a prefix of another known identifier in the same namespace*), and
//! [`audit_pattern`] states what one pattern takes in across a namespace, per
//! family, with an anchored replacement where one exists mechanically.

use std::collections::BTreeSet;
use std::fmt;

use super::glob::{Glob, DEFAULT_BOUNDARY};

/// A name split into the family it belongs to and its instance index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identifier {
    /// The name as given (last path component).
    pub name: String,
    /// The name minus a trailing numeric instance token. `qwen3.8-27b-vision-1`
    /// → `qwen3.8-27b-vision`; `qwen3.8-27b-1` → `qwen3.8-27b`.
    pub family: String,
    /// The trailing numeric token, when the name carried one. `None` means the
    /// name is not an instance of anything — it is a bare identifier, and a
    /// pattern anchored on it is exact.
    pub instance: Option<String>,
}

/// Split a name into [`Identifier`] parts at `boundary`.
///
/// Only the last path component is considered. A trailing token of ASCII
/// digits is the instance (`qwen3.8-27b-1`); anything else (`…-vision`) is
/// part of the family, which is exactly what makes the longer family a
/// collision for the shorter one.
pub fn classify(name: &str, boundary: char) -> Identifier {
    let stem = name.rsplit('/').next().unwrap_or(name);
    match stem.rfind(boundary) {
        Some(idx) if idx + boundary.len_utf8() < stem.len() => {
            let tail = &stem[idx + boundary.len_utf8()..];
            if !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit()) {
                Identifier {
                    name: stem.to_owned(),
                    family: stem[..idx].to_owned(),
                    instance: Some(tail.to_owned()),
                }
            } else {
                Identifier {
                    name: stem.to_owned(),
                    family: stem.to_owned(),
                    instance: None,
                }
            }
        }
        _ => Identifier {
            name: stem.to_owned(),
            family: stem.to_owned(),
            instance: None,
        },
    }
}

/// Two families in one namespace where one name is a prefix of the other.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrefixCollision {
    /// The shorter family — the one a pattern is usually written against.
    pub short: String,
    /// The longer family, whose members the shorter pattern also takes in.
    pub long: String,
    /// Whether the shorter name ends where the longer one's next boundary is.
    /// `false` is worse still (`foo` vs `foobar`): the prefix runs into the
    /// middle of a token.
    pub at_boundary: bool,
}

impl fmt::Display for PrefixCollision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "family '{}' is a prefix of family '{}'{}",
            self.short,
            self.long,
            if self.at_boundary {
                " (at a boundary)"
            } else {
                " (mid-token)"
            }
        )
    }
}

/// Report every pair of distinct families in `names` where one is a prefix of
/// the other.
///
/// This is the deterministic sweep: given the inventory of identifiers in one
/// namespace, it names the prefixes that are not namespaces. Order is
/// (short, long) lexicographic; duplicate families collapse.
pub fn prefix_collisions(names: &[String], boundary: char) -> Vec<PrefixCollision> {
    let families: BTreeSet<String> = names
        .iter()
        .map(|name| classify(name, boundary).family)
        .collect();
    let mut found = Vec::new();
    for short in &families {
        for long in &families {
            if long == short || !long.starts_with(short.as_str()) {
                continue;
            }
            let rest = &long[short.len()..];
            found.push(PrefixCollision {
                short: short.clone(),
                long: long.clone(),
                at_boundary: rest.starts_with(boundary),
            });
        }
    }
    found
}

/// The members of one family matched by a pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FamilyGroup {
    pub family: String,
    pub members: Vec<String>,
}

/// What a pattern took in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuditVerdict {
    /// Nothing in the namespace matched — a pattern that matches nothing is
    /// not a scoped pattern, it is a typo.
    NoMatch,
    /// Everything matched belongs to one family.
    Scoped { family: String },
    /// The pattern spans families: the counts it feeds are wrong, and no
    /// amount of re-running fixes them.
    Overbroad {
        intended: Option<String>,
        foreign: Vec<FamilyGroup>,
    },
}

/// One pattern audited against a namespace inventory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatternAudit {
    pattern: String,
    matched: Vec<String>,
    groups: Vec<FamilyGroup>,
    intended: Option<String>,
}

impl PatternAudit {
    /// Names the pattern matched, in the order the caller enumerated them.
    pub fn matched(&self) -> &[String] {
        &self.matched
    }

    /// Matched names grouped by family, lexicographic by family.
    pub fn groups(&self) -> &[FamilyGroup] {
        &self.groups
    }

    /// The family the pattern was written against — its literal prefix with a
    /// trailing boundary removed — when that family is present in the matches.
    pub fn intended(&self) -> Option<&str> {
        self.intended.as_deref()
    }

    /// True when the pattern spans more than one family.
    pub fn is_over_broad(&self) -> bool {
        self.groups.len() > 1
    }

    pub fn verdict(&self) -> AuditVerdict {
        match self.groups.len() {
            0 => AuditVerdict::NoMatch,
            1 => AuditVerdict::Scoped {
                family: self.groups[0].family.clone(),
            },
            _ => AuditVerdict::Overbroad {
                intended: self.intended.clone(),
                foreign: self
                    .groups
                    .iter()
                    .filter(|g| Some(&g.family) != self.intended.as_ref())
                    .cloned()
                    .collect(),
            },
        }
    }

    /// Matched names that belong to a family other than the intended one.
    pub fn foreign_members(&self) -> Vec<String> {
        if !self.is_over_broad() {
            return Vec::new();
        }
        self.groups
            .iter()
            .filter(|g| Some(&g.family) != self.intended.as_ref())
            .flat_map(|g| g.members.clone())
            .collect()
    }

    /// The per-family count line. A count is stated per family, never as one
    /// number over a mixed set — that is the shape that hid 6/24 vs 5/20.
    pub fn count_line(&self) -> String {
        if self.groups.is_empty() {
            return format!("pattern '{}' matched nothing", self.pattern);
        }
        let groups = self
            .groups
            .iter()
            .map(|g| format!("{} ({})", g.family, g.members.len()))
            .collect::<Vec<_>>()
            .join(", ");
        let shape = if self.is_over_broad() {
            format!(
                "{} — over-broad: {} name(s) from another family",
                self.groups.len(),
                self.foreign_members().len()
            )
        } else {
            "1 family".to_owned()
        };
        format!(
            "pattern '{}' matched {} names across {}: {}",
            self.pattern,
            self.matched.len(),
            shape,
            groups
        )
    }

    /// An anchored replacement for the pattern, when one exists mechanically.
    ///
    /// The first wildcard is replaced by a character class that keeps every
    /// intended member and excludes every foreign one: digits at that position
    /// become `[0-9]` (the `qwen3.8-27b-[0-9]*` form from the incident), other
    /// small character sets are listed. `None` means no single class separates
    /// the families — the caller must enumerate instead of tightening the glob.
    pub fn suggestion(&self) -> Option<String> {
        if !self.is_over_broad() {
            return None;
        }
        let prefix: Vec<char> = self.pattern.chars().take_while(|c| {
            *c != '*' && *c != '?' && *c != '['
        }).collect();
        let at = prefix.len();
        let mut intended_chars = BTreeSet::new();
        let mut foreign_chars = BTreeSet::new();
        for group in &self.groups {
            let target = if Some(&group.family) == self.intended.as_ref() {
                &mut intended_chars
            } else {
                &mut foreign_chars
            };
            for member in &group.members {
                target.insert(member.chars().nth(at)?);
            }
        }
        if intended_chars.is_empty() || !intended_chars.is_disjoint(&foreign_chars) {
            return None;
        }
        let class = if intended_chars.iter().all(|c| c.is_ascii_digit()) {
            "[0-9]".to_owned()
        } else if intended_chars.len() <= 4 {
            format!("[{}]", intended_chars.iter().collect::<String>())
        } else {
            return None;
        };
        Some(format!("{}{}*", prefix.iter().collect::<String>(), class))
    }
}

/// Audit `pattern` against `names`: what it takes in, grouped by family.
///
/// The intended family is read off the pattern's own literal prefix
/// (`qwen3.8-27b-*` → `qwen3.8-27b`). When that family is not among the
/// matches, `intended` is `None` and every matched family is foreign to the
/// prefix the caller wrote.
pub fn audit_pattern(pattern: &Glob, names: &[String], boundary: char) -> PatternAudit {
    let matched: Vec<String> = names
        .iter()
        .filter(|name| pattern.matches(name))
        .cloned()
        .collect();
    let mut groups: Vec<FamilyGroup> = Vec::new();
    for name in &matched {
        let family = classify(name, boundary).family;
        match groups.iter_mut().find(|g| g.family == family) {
            Some(group) => group.members.push(name.clone()),
            None => groups.push(FamilyGroup {
                family,
                members: vec![name.clone()],
            }),
        }
    }
    groups.sort_by(|a, b| a.family.cmp(&b.family));

    let trimmed = pattern
        .literal_prefix()
        .trim_end_matches(boundary)
        .to_owned();
    let intended = if pattern.is_exact() {
        groups.first().map(|g| g.family.clone())
    } else {
        groups
            .iter()
            .find(|g| g.family == trimmed)
            .map(|g| g.family.clone())
    };

    PatternAudit {
        pattern: pattern.pattern().to_owned(),
        matched,
        groups,
        intended,
    }
}

/// Convenience: audit with [`DEFAULT_BOUNDARY`].
pub fn audit(pattern: &Glob, names: &[String]) -> PatternAudit {
    audit_pattern(pattern, names, DEFAULT_BOUNDARY)
}
