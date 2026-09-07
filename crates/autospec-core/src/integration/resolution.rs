//! Conflict classification and the default (additive union) resolver
//! (issue #3565, asks 2 and 4).
//!
//! Parallel-implementer conflicts are usually *additive*: both sides keep
//! every ancestor line and merely insert different lines. Those resolve as
//! the union of both sides. A hunk where either side modified or deleted an
//! ancestor line is a genuine semantic disagreement — the two sides do not
//! coexist — and the phase must halt that branch and report the hunk for a
//! human rather than guess.

use super::symbols::side_additions;
use super::vcs::{ConflictHunk, ConflictedFile, ResolvedFile};

/// A resolved tree, one entry per previously-conflicted file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    pub files: Vec<ResolvedFile>,
}

/// A resolver failure (the default resolver cannot fail; custom resolvers
/// may).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveError(pub String);

impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ResolveError {}

/// Turns conflicted files into a resolved tree.
pub trait Resolver {
    fn resolve(&self, files: &[ConflictedFile]) -> Result<Resolution, ResolveError>;
}

/// Default resolver: keep every line of both sides (union). Only ever called
/// after [`SemanticConflict::find`] proved the conflict is additive.
#[derive(Debug, Clone, Copy, Default)]
pub struct UnionResolver;

impl Resolver for UnionResolver {
    fn resolve(&self, files: &[ConflictedFile]) -> Result<Resolution, ResolveError> {
        Ok(union_resolution(files))
    }
}

/// One hunk that is a genuine semantic disagreement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticConflict {
    pub file: String,
    /// 0-based line index of the region in the trunk-side file.
    pub start: usize,
    /// Why this hunk is not additive, e.g. which side modified ancestor
    /// lines.
    pub reason: String,
    pub ancestor: Vec<String>,
    pub ours: Vec<String>,
    pub theirs: Vec<String>,
}

impl SemanticConflict {
    /// Find every hunk in `files` where at least one side modified or
    /// deleted an ancestor line. An empty result means the whole conflict is
    /// additive and may be union-resolved.
    pub fn find(files: &[ConflictedFile]) -> Vec<SemanticConflict> {
        let mut conflicts = Vec::new();
        for file in files {
            for hunk in &file.hunks {
                let (_, ours_deleted) = side_additions(&hunk.ancestor, &hunk.ours);
                let (_, theirs_deleted) = side_additions(&hunk.ancestor, &hunk.theirs);
                let mut reasons = Vec::new();
                if ours_deleted > 0 {
                    reasons.push(format!(
                        "trunk side modified or deleted {ours_deleted} ancestor line(s)"
                    ));
                }
                if theirs_deleted > 0 {
                    reasons.push(format!(
                        "branch side modified or deleted {theirs_deleted} ancestor line(s)"
                    ));
                }
                if reasons.is_empty() {
                    continue;
                }
                conflicts.push(SemanticConflict {
                    file: file.path.clone(),
                    start: hunk.start,
                    reason: reasons.join("; "),
                    ancestor: hunk.ancestor.clone(),
                    ours: hunk.ours.clone(),
                    theirs: hunk.theirs.clone(),
                });
            }
        }
        conflicts
    }
}

/// Build the additive union resolution: the trunk-side file with each
/// branch-side insertion spliced in at its position.
pub fn union_resolution(files: &[ConflictedFile]) -> Resolution {
    let mut out = Vec::new();
    for file in files {
        let mut lines: Vec<String> = file.base.lines().map(str::to_string).collect();
        let mut hunks: Vec<&ConflictHunk> = file.hunks.iter().collect();
        hunks.sort_by_key(|h| h.start);
        for hunk in hunks {
            // The trunk-side file is the base of the merge; the branch
            // side's added lines are spliced in right after the region the
            // trunk occupied in the hunk, so both additions coexist
            // (a line-union of the hunk). Position is not part of the
            // preservation contract — the gate checks the union of
            // symbols, not line order.
            let (theirs_added, _) = side_additions(&hunk.ancestor, &hunk.theirs);
            let insert_at = (hunk.start + hunk.ours.len()).min(lines.len());
            for (offset, line) in theirs_added.iter().enumerate() {
                lines.insert(insert_at + offset, line.clone());
            }
        }
        let mut content = lines.join("\n");
        if !file.base.is_empty() && file.base.ends_with('\n') {
            content.push('\n');
        }
        out.push(ResolvedFile {
            path: file.path.clone(),
            content,
        });
    }
    Resolution { files: out }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(v: &[&str]) -> Vec<String> {
        v.iter().map(|str| str.to_string()).collect()
    }

    fn hunk(ancestor: &[&str], ours: &[&str], theirs: &[&str]) -> ConflictHunk {
        ConflictHunk {
            start: 1,
            ancestor: lines(ancestor),
            ours: lines(ours),
            theirs: lines(theirs),
        }
    }

    #[test]
    fn both_sides_adding_is_additive() {
        // `base` is the trunk-side (stage 2) file, so it already contains
        // the trunk's addition `aa`; the union splices in the branch's `bb`.
        let file = ConflictedFile {
            path: "s.go".to_string(),
            base: "type S struct {\n\tcore *Core\n\taa *A\n}\n".to_string(),
            hunks: vec![hunk(
                &["\tcore *Core"],
                &["\tcore *Core", "\tta *A"],
                &["\tcore *Core", "\tbb *B"],
            )],
        };
        assert!(SemanticConflict::find(&[file.clone()]).is_empty());
        let resolution = union_resolution(&[file]);
        assert_eq!(
            resolution.files[0].content,
            "type S struct {\n\tcore *Core\n\taa *A\n\tbb *B\n}\n"
        );
    }

    #[test]
    fn both_sides_modifying_the_same_line_is_semantic() {
        let file = ConflictedFile {
            path: "s.go".to_string(),
            base: "type S struct {\n\tcore *Core\n}\n".to_string(),
            hunks: vec![hunk(
                &["\tcore *Core"],
                &["\tcore *RenamedCore"],
                &["\tcore *OtherCore"],
            )],
        };
        let conflicts = SemanticConflict::find(&[file.clone()]);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].file, "s.go");
        assert!(
            conflicts[0].reason.contains("trunk side"),
            "{}",
            conflicts[0].reason
        );
        assert!(
            conflicts[0].reason.contains("branch side"),
            "{}",
            conflicts[0].reason
        );
    }

    #[test]
    fn one_side_deleting_an_ancestor_line_is_semantic() {
        let file = ConflictedFile {
            path: "s.go".to_string(),
            base: "type S struct {\n\tcore *Core\n\tlegacy *Legacy\n}\n".to_string(),
            hunks: vec![hunk(
                &["\tcore *Core", "\tlegacy *Legacy"],
                &["\tcore *Core", "\tlegacy *Legacy"],
                &["\tcore *Core"],
            )],
        };
        let conflicts = SemanticConflict::find(&[file]);
        assert_eq!(conflicts.len(), 1);
        assert!(conflicts[0].reason.contains("branch side"));
    }
}
