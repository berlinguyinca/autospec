//! Consecutive zero-output completion tracking per issue.
//!
//! The executor bridge records a zero-effect completion whenever a run
//! finishes without producing artifacts. The frontier consults the streaks
//! and routes an issue to human review once the consecutive count reaches
//! the review threshold, instead of re-dispatching runs that keep
//! completing with nothing. A successful run clears the streak.
//!
//! The state file lives at `~/.autospec/state/zero-output-streaks.json`
//! (override with `$AUTOSPEC_ZERO_OUTPUT_STATE_FILE`):
//!
//! ```json
//! { "owner/repo": { "42": 2 } }
//! ```
//!
//! Bookkeeping is best-effort by design: a missing or corrupt file reads as
//! "no streak" and a failed write only warns. Streak tracking must never
//! block dispatch or completion.

use std::collections::BTreeMap;
use std::path::Path;

/// `{"<owner>/<repo>": {"<issue-number>": consecutive-count}}`.
type StreakState = BTreeMap<String, BTreeMap<String, usize>>;

fn state_path() -> std::path::PathBuf {
    match std::env::var("AUTOSPEC_ZERO_OUTPUT_STATE_FILE") {
        Ok(path) if !path.is_empty() => std::path::PathBuf::from(path),
        _ => std::env::var_os("HOME")
            .map(|home| {
                std::path::PathBuf::from(home).join(".autospec/state/zero-output-streaks.json")
            })
            .unwrap_or_else(|| {
                std::path::PathBuf::from(".autospec/state/zero-output-streaks.json")
            }),
    }
}

/// Loads the per-issue zero-output streaks for one repository.
pub fn load(repo: &str) -> BTreeMap<u64, usize> {
    load_at(&state_path(), repo)
}

pub(super) fn load_at(path: &Path, repo: &str) -> BTreeMap<u64, usize> {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return BTreeMap::new();
    };
    let Ok(state) = serde_json::from_str::<StreakState>(&raw) else {
        eprintln!(
            "WARN: zero-output streak state {} is not valid JSON; treating as empty",
            path.display()
        );
        return BTreeMap::new();
    };
    state
        .get(repo)
        .map(|issues| {
            issues
                .iter()
                .filter_map(|(issue, streak)| {
                    issue.parse::<u64>().ok().map(|number| (number, *streak))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Records one zero-output completion, incrementing the issue's streak.
pub fn record_zero_output(repo: &str, issue: u64) {
    record_at(&state_path(), repo, issue);
}

pub(super) fn record_at(path: &Path, repo: &str, issue: u64) {
    let mut state = read_state(path);
    let streaks = state.entry(repo.to_string()).or_default();
    *streaks.entry(issue.to_string()).or_insert(0) += 1;
    if write_state(path, &state).is_err() {
        eprintln!(
            "WARN: could not persist zero-output streak state at {}",
            path.display()
        );
    }
}

/// Clears the issue's streak after a run that produced output.
pub fn clear(repo: &str, issue: u64) {
    clear_at(&state_path(), repo, issue);
}

pub(super) fn clear_at(path: &Path, repo: &str, issue: u64) {
    let mut state = read_state(path);
    let Some(streaks) = state.get_mut(repo) else {
        return;
    };
    if streaks.remove(&issue.to_string()).is_none() {
        return;
    }
    if streaks.is_empty() {
        state.remove(repo);
    }
    if write_state(path, &state).is_err() {
        eprintln!(
            "WARN: could not persist zero-output streak state at {}",
            path.display()
        );
    }
}

fn read_state(path: &Path) -> StreakState {
    match std::fs::read_to_string(path) {
        Ok(raw) => serde_json::from_str(&raw).unwrap_or_else(|_| {
            eprintln!(
                "WARN: zero-output streak state {} is not valid JSON; resetting",
                path.display()
            );
            StreakState::new()
        }),
        Err(_) => StreakState::new(),
    }
}

/// Atomic write: same-directory temp file then rename.
fn write_state(path: &Path, state: &StreakState) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let payload = serde_json::to_string(state)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    let tmp = format!("{}.tmp.{}", path.display(), std::process::id());
    std::fs::write(&tmp, payload)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_state_file(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "autospec-zero-output-streaks-{tag}-{}.json",
            std::process::id()
        ))
    }

    #[test]
    fn load_missing_file_is_empty() {
        let path = temp_state_file("missing");
        let streaks = load_at(&path, "owner/repo");
        assert!(streaks.is_empty());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn record_increments_the_streak() {
        let path = temp_state_file("increment");
        record_at(&path, "owner/repo", 42);
        assert_eq!(load_at(&path, "owner/repo"), {
            let mut expected = BTreeMap::new();
            expected.insert(42, 1);
            expected
        });
        record_at(&path, "owner/repo", 42);
        assert_eq!(load_at(&path, "owner/repo"), {
            let mut expected = BTreeMap::new();
            expected.insert(42, 2);
            expected
        });
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn clear_removes_the_streak_and_empty_repo() {
        let path = temp_state_file("clear");
        record_at(&path, "owner/repo", 42);
        clear_at(&path, "owner/repo", 42);
        assert!(load_at(&path, "owner/repo").is_empty());
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(
            !raw.contains("owner/repo"),
            "repo entry should be pruned: {raw}"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn clear_is_a_noop_for_unknown_issues() {
        let path = temp_state_file("noop-clear");
        clear_at(&path, "owner/repo", 99);
        assert!(load_at(&path, "owner/repo").is_empty());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn streaks_are_isolated_per_repo() {
        let path = temp_state_file("per-repo");
        record_at(&path, "owner/one", 1);
        record_at(&path, "owner/two", 1);
        assert_eq!(load_at(&path, "owner/one"), {
            let mut expected = BTreeMap::new();
            expected.insert(1, 1);
            expected
        });
        assert_eq!(load_at(&path, "owner/two"), {
            let mut expected = BTreeMap::new();
            expected.insert(1, 1);
            expected
        });
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn corrupt_state_reads_empty_and_record_recovers() {
        let path = temp_state_file("corrupt");
        std::fs::write(&path, "{not json").unwrap();
        assert!(load_at(&path, "owner/repo").is_empty());
        record_at(&path, "owner/repo", 7);
        assert_eq!(load_at(&path, "owner/repo"), {
            let mut expected = BTreeMap::new();
            expected.insert(7, 1);
            expected
        });
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn non_numeric_issue_keys_are_skipped_on_load() {
        let path = temp_state_file("bad-keys");
        std::fs::write(&path, r#"{"owner/repo": {"42": 3, "junk": 9}}"#).unwrap();
        let streaks = load_at(&path, "owner/repo");
        assert_eq!(streaks.len(), 1);
        assert_eq!(streaks.get(&42), Some(&3));
        let _ = std::fs::remove_file(&path);
    }
}
