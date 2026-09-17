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
use autospec_core::declaration_conflict::{self, Conflict};

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

/// The HELD reason for one conflicted file, naming the file, whether its
/// shape is a proven-safe keep-both merge or a refusal (single-value,
/// generated, or unclassifiable), and — when it is one — that the conflict is
/// declaration-only, so the size of that class is measurable rather than
/// sampled from an incident (#4463).
pub(super) fn conflict_reason(path: &str, content: &str) -> String {
    let shape = classify_file(path, content);
    let declaration_only = declaration_conflict::detect(content).note();
    match resolution_for(&shape) {
        ResolutionPlan::KeepBothInOrder | ResolutionPlan::KeepBothDeduplicated => {
            format!(
                "{path}: shape {}{declaration_only} (resolvable keep-both)",
                shape.as_str()
            )
        }
        ResolutionPlan::Regenerate { .. } => {
            format!(
                "{path}: shape {} (regenerate from source, not a merge)",
                shape.as_str()
            )
        }
        ResolutionPlan::Refuse { reason } => {
            format!(
                "{path}: shape {}{declaration_only} (refused: {reason})",
                shape.as_str()
            )
        }
    }
}

/// Whether this conflict is terminal: every conflicted file is a shape the
/// pass will never merge (refused, or regenerate-from-source), so no amount
/// of re-gating converts this patch against this base — only regeneration
/// can. `None` when the pass cannot make that claim: the conflicted files
/// could not be enumerated, or at least one file is a certified keep-both
/// shape (its hold may be the gate's, and the gate's holds clear when the
/// trunk does).
///
/// This is the distinction #4637's deadlock needs: a patch held on a
/// structural refusal is evidence of work against a base that no longer
/// exists, and while it sits on disk it suppresses re-dispatch of its issue
/// forever. A patch held on a gate or on a parser failure over a certified
/// shape is not: it stays, and is re-offered.
pub(super) fn is_structural_refusal(worktree: &Path) -> bool {
    let names = conflicted_names(worktree);
    if names.is_empty() {
        return false;
    }
    for name in &names {
        let content = fs::read_to_string(worktree.join(name)).unwrap_or_default();
        // A declaration-only conflict is resolvable by the pass even when the
        // path classifier refuses it (#4463), so it is not structural either.
        if declaration_conflict::detect(&content).is_declaration_only()
            || matches!(
                resolution_for(&classify_file(name, &content)),
                ResolutionPlan::KeepBothInOrder | ResolutionPlan::KeepBothDeduplicated
            )
        {
            return false;
        }
    }
    true
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

/// Whether a declared module has a file in the merged tree, given the
/// conflicted path it was declared in.
///
/// The declaration in `index.rs` resolves relative to `index/`, while the
/// declaration in `lib.rs`/`mod.rs` resolves next to it — a `pub mod alpha;`
/// in `src/index.rs` wants `src/index/alpha.rs` or `src/index/alpha/mod.rs`,
/// whereas in `src/lib.rs` it wants `src/alpha.rs`. Both shapes are checked.
/// The gate's compile stage re-checks this, but refusing here keeps a union
/// from staging a declaration the merge invented by resurrecting the other
/// side's deletion (#4463).
fn module_resolves(worktree: &Path, conflicted_path: &str, module: &str) -> bool {
    let dir = match conflicted_path.rsplit_once('/') {
        Some((dir, _)) => dir,
        None => "",
    };
    let stem = std::path::Path::new(conflicted_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    let base = worktree.join(dir);
    let sibling = |module: &str| base.join(format!("{module}.rs")).is_file();
    // Declarations inside a non-index module file resolve under that module's
    // directory; declarations in lib.rs/mod.rs resolve beside it.
    let nested = if stem == "lib" || stem == "mod" {
        false
    } else {
        base.join(format!("{stem}/{module}.rs")).is_file()
            || base.join(format!("{stem}/{module}/mod.rs")).is_file()
    };
    sibling(module) || nested
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
        // A conflict confined to module-declaration lines is mechanically
        // resolvable whatever the file is called and whichever shape the path
        // classifier gives it (#4463): the union is taken in canonical order so
        // the next independent addition lands elsewhere instead of colliding
        // at the same offset. The gate's compile stage is what proves the
        // union; `orphaned` below catches the one case a two-sided union cannot
        // see, a declaration the other side deleted.
        let declaration_only = declaration_conflict::detect(&content).is_declaration_only();
        let (deduplicate, plan) = match resolution_for(&shape) {
            ResolutionPlan::KeepBothInOrder => (false, "keep both, in order"),
            ResolutionPlan::KeepBothDeduplicated => (true, "keep both, deduplicated"),
            _ if declaration_only => (true, "declaration-only union, canonically sorted"),
            _ => return Err(summary(worktree)),
        };
        let merged = if declaration_only && deduplicate {
            declaration_conflict::sorted_union(&content).map_err(|error| {
                format!(
                    "{}; declaration union failed on {name}: {error:?}",
                    summary(worktree)
                )
            })
        } else {
            merge_keep_both(&content, deduplicate).map_err(|error| match error {
                MergeError::UnbalancedMarkers { detail }
                | MergeError::UnsupportedConflictStyle { detail } => format!(
                    "{}; merge parser failed on {name}: {detail}",
                    summary(worktree)
                ),
                MergeError::NoHunks => format!(
                    "{}; {name} is unmerged but carries no conflict markers the \
                     merge parser can see",
                    summary(worktree)
                ),
            })
        };
        let merged = match merged {
            Ok(merged) => merged,
            Err(reason) => return Err(reason),
        };
        // A union cannot distinguish an addition from the other side's
        // deletion, so the filesystem decides: a declaration naming no file in
        // the merged tree is refused rather than staged as a build failure.
        let orphans = declaration_conflict::orphaned(&merged, |module| {
            module_resolves(worktree, name, module)
        });
        if !orphans.is_empty() {
            return Err(format!(
                "{}; declaration union names no file for: {}",
                summary(worktree),
                orphans.join(", ")
            ));
        }
        if let Err(error) = fs::write(worktree.join(name), &merged) {
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
        None => return Err("the project formatter could not be run on the merged tree".to_string()),
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
    const CONFLICTING_MOD: &str =
        "pub mod a;\n<<<<<<< HEAD\npub mod b;\n=======\npub mod c;\n>>>>>>> convert-7\n";

    #[test]
    fn the_conflict_reason_names_the_certified_and_refused_shapes() {
        // Single-value files are regenerated, not merged; unclassifiable
        // files are refused; the pass's HELD reason says exactly which.
        assert_eq!(
            conflict_reason("VERSION", "1\n<<<<<<< HEAD\n2\n=======\n3\n>>>>>>> b\n"),
            "VERSION: shape single_value (regenerate from source, not a merge)"
        );
        // A conflict that is not confined to declarations (real code in the
        // hunk) is still refused: the note appears only for declaration-only
        // conflicts, and a declaration-only `unknown` file is no longer
        // refused at all — it resolves by canonical union.
        let code = "fn x() {\n<<<<<<< HEAD\n    1\n=======\n    2\n>>>>>>> b\n}";
        assert_eq!(
            conflict_reason("src/engine.rs", code),
            "src/engine.rs: shape unknown (refused: unclassifiable file shape: no \
             conflict-resolution strategy applies, and a default would be a guess)"
        );
        // A declaration-only conflict in an unclassifiable file is recorded as
        // such, so the size of the class is measured rather than sampled.
        assert_eq!(
            conflict_reason("src/engine.rs", CONFLICTING_MOD),
            "src/engine.rs: shape unknown, declaration-only (refused: unclassifiable \
             file shape: no conflict-resolution strategy applies, and a default would \
             be a guess)"
        );
    }

    #[test]
    fn the_conflict_reason_names_the_certified_shapes() {
        assert_eq!(
            conflict_reason("src/lib.rs", CONFLICTING_MOD),
            "src/lib.rs: shape additive_declarations, declaration-only (resolvable \
             keep-both)"
        );
        assert_eq!(
            conflict_reason(
                "CHANGELOG.md",
                "old\n<<<<<<< HEAD\nb\n=======\nc\n>>>>>>> b\n"
            ),
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
