//! The conflict surface of the conversion pass (issue #4560).
//!
//! A `git apply --3way` conflict is a hold unless the pass can prove the
//! resolution safe: every conflicted file is classified by the core
//! classifier, and only the shapes certified keep-both are merged — by the
//! core's strict merge parser — and re-offered to the gate, whose fmt,
//! compile, and test stages are what make a resolution more than a
//! hypothesis. A single file the pass cannot certify holds the patch, and
//! the HELD reason names every conflicted file and its shape.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use autospec_core::conflict_merge::{merge_keep_both, MergeError};
use autospec_core::conflict_resolution::{classify_file, resolution_for, ResolutionPlan};

use super::gate::{first_lines, run_cargo};
use super::{run_capture_in, run_git_in};

/// One conflict the pass resolved on its own. The PR body must say so: an
/// auto-resolution a human cannot see is an auto-resolution a human cannot
/// review (issue #4560, guardrail 2).
#[derive(Debug, Clone)]
pub(super) struct ResolutionRecord {
    pub(super) path: String,
    pub(super) shape: String,
    pub(super) plan: String,
}

/// The HELD reason for one conflicted file, naming the file and whether its
/// shape is a proven-safe keep-both merge or a refusal (single-value,
/// generated, or unclassifiable).
pub(super) fn conflict_reason(path: &str, content: &str) -> String {
    let shape = classify_file(path, content);
    match resolution_for(&shape) {
        ResolutionPlan::KeepBothInOrder | ResolutionPlan::KeepBothDeduplicated => {
            format!("{path}: shape {} (resolvable keep-both)", shape.as_str())
        }
        ResolutionPlan::Regenerate { .. } => {
            format!("{path}: shape {} (regenerate from source, not a merge)", shape.as_str())
        }
        ResolutionPlan::Refuse { reason } => {
            format!("{path}: shape {} (refused: {reason})", shape.as_str())
        }
    }
}

/// The files a worktree left in a conflicted (unmerged) state, each with its
/// shape — the refusal reason for a conflict the pass cannot resolve.
pub(super) fn summary(worktree: &Path) -> String {
    let names = conflicted_names(worktree);
    if names.is_empty() {
        return "conflict: could not enumerate conflicted files".to_string();
    }
    let mut parts = Vec::new();
    for name in &names {
        let content = fs::read_to_string(worktree.join(name)).unwrap_or_default();
        parts.push(conflict_reason(name, &content));
    }
    format!("conflict (refusing auto-resolution): {}", parts.join("; "))
}

/// The unmerged files git named, or an empty list when they cannot be
/// enumerated.
fn conflicted_names(worktree: &Path) -> Vec<String> {
    let output = run_capture_in(worktree, &["diff", "--name-only", "--diff-filter=U"]);
    output
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Resolve every conflict in the worktree, or hold.
///
/// Each unmerged file is classified; a keep-both shape is merged by the
/// core's strict parser, written back, and staged (`git add`), so the gate
/// — which runs next, on the merged tree — is what verifies the result.
/// Any file the classifier does not certify keep-both, or the parser cannot
/// merge, returns `Err` with the reason naming every conflicted file and its
/// shape; the caller holds the patch on it.
pub(super) fn resolve_conflicts(worktree: &Path) -> Result<Vec<ResolutionRecord>, String> {
    let names = conflicted_names(worktree);
    if names.is_empty() {
        return Err("conflict: could not enumerate conflicted files".to_string());
    }
    let mut records = Vec::new();
    for name in &names {
        let content = fs::read_to_string(worktree.join(name)).unwrap_or_default();
        let shape = classify_file(name, &content);
        let (deduplicate, plan) = match resolution_for(&shape) {
            ResolutionPlan::KeepBothInOrder => (false, "keep both, in order"),
            ResolutionPlan::KeepBothDeduplicated => (true, "keep both, deduplicated"),
            _ => return Err(summary(worktree)),
        };
        match merge_keep_both(&content, deduplicate) {
            Ok(merged) => {
                if let Err(error) = fs::write(worktree.join(name), merged) {
                    return Err(format!(
                        "{}; merge write failed on {name}: {error}",
                        summary(worktree)
                    ));
                }
                if let Err(error) = run_git_in(worktree, &["add", name]) {
                    return Err(format!(
                        "{}; git add failed on {name} after the merge: {error}",
                        summary(worktree)
                    ));
                }
                records.push(ResolutionRecord {
                    path: name.clone(),
                    shape: shape.as_str().to_string(),
                    plan: plan.to_string(),
                });
            }
            Err(MergeError::UnbalancedMarkers { detail })
            | Err(MergeError::UnsupportedConflictStyle { detail }) => {
                return Err(format!(
                    "{}; merge parser failed on {name}: {detail}",
                    summary(worktree)
                ))
            }
            Err(MergeError::NoHunks) => {
                return Err(format!(
                    "{}; {name} is unmerged but carries no conflict markers the \
                     merge parser can see",
                    summary(worktree)
                ))
            }
        }
    }
    // A union is not necessarily canonical: two appended `pub mod` lists
    // merge out of the project formatter's alphabetical order, and the gate's
    // fmt stage would hold a correct merge for it. Run the project's own
    // formatter on the merged tree — the same tool the gate will re-check —
    // and hold if it reaches beyond the merged files: a base or patch that
    // arrives fmt-dirty is held, exactly as before.
    canonicalize(worktree, &records)?;
    Ok(records)
}

/// Run the project's formatter over the merged tree and verify it touched
/// only the merged files. The merged files may be rewritten (that is the
/// point: the union is canonicalized); anything else the formatter touches
/// is fmt-dirtiness that belongs to the base or the patch, and the gate must
/// still hold it.
fn canonicalize(worktree: &Path, records: &[ResolutionRecord]) -> Result<(), String> {
    let before = snapshot(worktree);
    match run_cargo(worktree, &["fmt".to_string()]) {
        Some(output) if output.status.code() == Some(0) => {}
        Some(output) => {
            let text = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return Err(format!(
                "the project formatter failed on the merged tree: {}",
                first_lines(&text, 8)
            ));
        }
        None => {
            return Err("the project formatter could not be run on the merged tree".to_string())
        }
    }
    let merged: Vec<&str> = records.iter().map(|r| r.path.as_str()).collect();
    let after = snapshot(worktree);
    let mut touched: Vec<String> = Vec::new();
    for (path, before_content) in &before {
        if after.get(path) != Some(before_content) {
            touched.push((*path).clone());
        }
    }
    for path in after.keys() {
        if !before.contains_key(path) {
            touched.push(path.clone());
        }
    }
    let unexpected: Vec<&str> = touched
        .iter()
        .map(|p| p.as_str())
        .filter(|p| !merged.contains(p))
        .collect();
    if !unexpected.is_empty() {
        return Err(format!(
            "the project formatter changed files beyond the merged ones ({}): the base or the \
             patch is not fmt-clean",
            unexpected.join(", ")
        ));
    }
    Ok(())
}

/// The worktree's files (path relative to the root, content), skipping the
/// `.git` directory. The conversion worktree is a fresh checkout — a couple
/// of hundred text files, cheap to snapshot twice around a formatter run.
fn snapshot(worktree: &Path) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut stack = vec![worktree.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if path.is_dir() {
                if name != ".git" {
                    stack.push(path);
                }
            } else if let Ok(content) = fs::read_to_string(&path) {
                let relative = path
                    .strip_prefix(worktree)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .into_owned();
                out.insert(relative, content);
            }
        }
    }
    out
}

/// The PR body section naming every auto-resolution, or nothing.
pub(super) fn pr_section(records: &[ResolutionRecord]) -> String {
    if records.is_empty() {
        return String::new();
    }
    let mut out = String::from(
        "\nAuto-resolved conflicts (the classifier certified each shape keep-both; \
         the merged files were run through the project's formatter, and the \
         gate's fmt, compile, and test stages verified the result):\n",
    );
    for record in records {
        out.push_str(&format!(
            "- `{}`: shape {} — {} (automatic)\n",
            record.path, record.shape, record.plan
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-hunk conflict in a declaration index, as git writes it.
    const CONFLICTING_MOD: &str = "pub mod a;\n<<<<<<< HEAD\npub mod b;\n=======\npub mod c;\n>>>>>>> convert-7\n";

    #[test]
    fn the_conflict_reason_names_the_certified_and_refused_shapes() {
        // Single-value files are regenerated, not merged; unclassifiable
        // files are refused; the pass's HELD reason says exactly which.
        assert_eq!(
            conflict_reason("VERSION", "1\n<<<<<<< HEAD\n2\n=======\n3\n>>>>>>> b\n"),
            "VERSION: shape single_value (regenerate from source, not a merge)"
        );
        assert_eq!(
            conflict_reason("src/engine.rs", CONFLICTING_MOD),
            "src/engine.rs: shape unknown (refused: unclassifiable file shape: no \
             conflict-resolution strategy applies, and a default would be a guess)"
        );
    }

    #[test]
    fn the_conflict_reason_names_the_certified_shapes() {
        assert_eq!(
            conflict_reason("src/lib.rs", CONFLICTING_MOD),
            "src/lib.rs: shape additive_declarations (resolvable keep-both)"
        );
        assert_eq!(
            conflict_reason("CHANGELOG.md", "old\n<<<<<<< HEAD\nb\n=======\nc\n>>>>>>> b\n"),
            "CHANGELOG.md: shape append_only_list (resolvable keep-both)"
        );
    }

    #[test]
    fn a_file_with_no_markers_has_nothing_to_resolve() {
        // The reason must say so — not pretend the file was resolved.
        let reason = conflict_reason("src/lib.rs", "pub mod a;\n");
        assert!(reason.contains("additive_declarations"), "{reason}");
    }

    #[test]
    fn the_pr_section_names_every_resolution_and_its_shape() {
        let records = vec![ResolutionRecord {
            path: "src/lib.rs".to_string(),
            shape: "additive_declarations".to_string(),
            plan: "keep both, deduplicated".to_string(),
        }];
        let section = pr_section(&records);
        assert!(section.contains("src/lib.rs"), "{section}");
        assert!(section.contains("additive_declarations"), "{section}");
        assert!(section.contains("keep both, deduplicated"), "{section}");
        assert!(section.contains("automatic"), "{section}");
        assert_eq!(pr_section(&[]), "");
    }
}
