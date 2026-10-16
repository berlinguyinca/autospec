//! Predicting a collision between two tasks before either is dispatched.
//!
//! A task declares the files, and optionally the symbols within them, that it
//! will change. Two tasks collide when those declarations overlap: the same
//! file, or a directory (trailing `/`) covering the other's path. When both
//! tasks name the same file and both list symbols, the symbols refine the
//! overlap: disjoint symbol sets do not collide. Symbols are only ever what a
//! task declared, never inferred.
//!
//! A task that declares nothing predicts nothing. The reactive conflict
//! handling (`held_conflicts`, `conflict_resolution`, `issue_lock`) stays the
//! backstop for what this prediction cannot see.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::ids::TaskId;
use super::task::Task;
use crate::dispatch_pipeline::IssueWriteSurface;

/// One file or directory a task declares it will change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AffectedSurface {
    /// A repo-relative file, or a directory with a trailing `/`.
    pub path: String,
    /// Symbols changed within the file; empty means the whole file.
    #[serde(default)]
    pub symbols: Vec<String>,
}

/// A predicted collision between a candidate task and one other task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PredictedCollision {
    /// The task the candidate collides with.
    pub holder: TaskId,
    /// What they share: a path, or `path#symbol` when symbols narrowed it.
    pub shared: Vec<String>,
}

fn single(path: &str) -> IssueWriteSurface {
    IssueWriteSurface {
        issue: 0,
        paths: BTreeSet::from([path.to_string()]),
    }
}

/// What two declared surfaces share, empty when they do not collide.
fn shared_between(a: &AffectedSurface, b: &AffectedSurface) -> Vec<String> {
    let entries = single(&a.path).shared_entries(&single(&b.path));
    if entries.is_empty() {
        return entries;
    }
    let same_file = a.path == b.path && !a.path.ends_with('/');
    if same_file && !a.symbols.is_empty() && !b.symbols.is_empty() {
        let theirs: BTreeSet<&String> = b.symbols.iter().collect();
        let common: BTreeSet<&String> = a.symbols.iter().filter(|s| theirs.contains(s)).collect();
        return common
            .into_iter()
            .map(|symbol| format!("{}#{symbol}", a.path))
            .collect();
    }
    entries
}

/// Everything `mine` and `theirs` share across all declared surfaces.
fn shared_with(mine: &[AffectedSurface], theirs: &[AffectedSurface]) -> BTreeSet<String> {
    let mut shared = BTreeSet::new();
    for a in mine {
        for b in theirs {
            shared.extend(shared_between(a, b));
        }
    }
    shared
}

/// The first of `others`, in order, that `candidate` collides with.
///
/// `None` when the candidate declares nothing, or nothing it declares
/// overlaps another task's declaration. The candidate itself is skipped if it
/// appears in `others`.
pub fn predict(candidate: &Task, others: &[&Task]) -> Option<PredictedCollision> {
    if candidate.affects.is_empty() {
        return None;
    }
    others
        .iter()
        .filter(|other| other.id != candidate.id)
        .find_map(|other| {
            let shared = shared_with(&candidate.affects, &other.affects);
            (!shared.is_empty()).then(|| PredictedCollision {
                holder: other.id.clone(),
                shared: shared.into_iter().collect(),
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::initiative::repository::RepositoryId;

    fn task(id: &str, affects: &[(&str, &[&str])]) -> Task {
        let mut task = Task::implementation(
            TaskId::parse(id).expect("valid task id"),
            RepositoryId::parse("github.com/InferWeave/autospec").expect("valid repository id"),
            Vec::new(),
            1,
        );
        task.affects = affects
            .iter()
            .map(|(path, symbols)| AffectedSurface {
                path: (*path).to_string(),
                symbols: symbols.iter().map(|s| (*s).to_string()).collect(),
            })
            .collect();
        task
    }

    #[test]
    fn same_file_without_symbols_collides_naming_the_file() {
        let a = task("TASK-0001", &[("src/a.rs", &[])]);
        let b = task("TASK-0002", &[("src/a.rs", &[])]);
        let hit = predict(&a, &[&b]).expect("collision");
        assert_eq!(hit.holder, b.id);
        assert_eq!(hit.shared, vec!["src/a.rs"]);
    }

    #[test]
    fn same_file_with_disjoint_symbols_does_not_collide() {
        let a = task("TASK-0001", &[("src/a.rs", &["a"])]);
        let b = task("TASK-0002", &[("src/a.rs", &["b"])]);
        assert_eq!(predict(&a, &[&b]), None);
    }

    #[test]
    fn same_file_with_overlapping_symbols_names_the_shared_symbol() {
        let a = task("TASK-0001", &[("src/a.rs", &["a", "b"])]);
        let b = task("TASK-0002", &[("src/a.rs", &["b"])]);
        let hit = predict(&a, &[&b]).expect("collision");
        assert_eq!(hit.shared, vec!["src/a.rs#b"]);
    }

    #[test]
    fn one_side_without_symbols_covers_the_whole_file() {
        let a = task("TASK-0001", &[("src/a.rs", &["a"])]);
        let b = task("TASK-0002", &[("src/a.rs", &[])]);
        assert_eq!(
            predict(&a, &[&b]).expect("collision").shared,
            vec!["src/a.rs"]
        );
    }

    #[test]
    fn directory_covers_a_file_beneath_it() {
        let a = task("TASK-0001", &[("crates/x/", &[])]);
        let b = task("TASK-0002", &[("crates/x/y.rs", &["f"])]);
        assert_eq!(
            predict(&a, &[&b]).expect("collision").shared,
            vec!["crates/x/"]
        );
        assert_eq!(
            predict(&b, &[&a]).expect("collision").shared,
            vec!["crates/x/"]
        );
    }

    #[test]
    fn near_miss_directory_prefix_is_a_different_directory() {
        let a = task("TASK-0001", &[("crates/x/", &[])]);
        let b = task("TASK-0002", &[("crates/xy/z.rs", &[])]);
        assert_eq!(predict(&a, &[&b]), None);
    }

    #[test]
    fn a_task_that_declares_nothing_predicts_nothing() {
        let a = task("TASK-0001", &[]);
        let b = task("TASK-0002", &[("src/a.rs", &[])]);
        assert_eq!(predict(&a, &[&b]), None);
        assert_eq!(predict(&b, &[&a]), None);
    }

    #[test]
    fn the_first_colliding_holder_in_order_is_named_and_self_is_skipped() {
        let a = task("TASK-0001", &[("src/a.rs", &[])]);
        let clear = task("TASK-0003", &[("src/z.rs", &[])]);
        let b = task("TASK-0002", &[("src/a.rs", &[])]);
        let d = task("TASK-0004", &[("src/a.rs", &[])]);
        let hit = predict(&a, &[&a, &clear, &b, &d]).expect("collision");
        assert_eq!(hit.holder, b.id);
    }

    #[test]
    fn a_task_json_without_affects_deserializes_to_an_empty_list() {
        let mut value = serde_json::to_value(task("TASK-0001", &[("src/a.rs", &[])])).unwrap();
        value.as_object_mut().unwrap().remove("affects");
        let parsed: Task = serde_json::from_value(value).unwrap();
        assert!(parsed.affects.is_empty());
    }
}
