//! Durable state for the `--issue` / run-only one-shot selector.
//!
//! Split out of `autonomous.rs` (#2946): that file is past the size ceiling, and the
//! ratchet refuses to let it grow. Everything the selector persists and reads back lives
//! here so the round trip is legible in one place.

use std::fs;
use std::path::{Path, PathBuf};

use autospec_core::execution::{OneShotIssueSelector, QueueStatus};

use super::{atomic_write, RunLayout};

pub(super) fn one_shot_selector_path(layout: &RunLayout) -> PathBuf {
    layout.state_dir.join("one-shot-selector.json")
}

/// Read the persisted selector's `consumed` flag.
///
/// This parses the document rather than probing it for `"consumed":true`. The substring
/// form agreed with `OneShotIssueSelector::status_json`'s hand-built layout only by
/// coincidence: any whitespace (`"consumed": true`) read as *not consumed*, and a nested
/// `consumed` elsewhere in the document read as consumed. Because "not consumed" is a
/// legitimate state, both mistakes surfaced as a silently re-run one-shot issue rather
/// than an error.
///
/// An unreadable or malformed document stays `false`, preserving the previous fail-safe:
/// the caller only skips work when it can prove the selector was consumed.
pub(super) fn one_shot_selector_consumed_at(path: &Path) -> bool {
    fs::read_to_string(path)
        .ok()
        .and_then(|contents| serde_json::from_str::<serde_json::Value>(&contents).ok())
        .and_then(|value| value.get("consumed").and_then(serde_json::Value::as_bool))
        .unwrap_or(false)
}

pub(super) fn load_one_shot_selector(
    layout: &RunLayout,
    issue: u64,
) -> Result<OneShotIssueSelector, String> {
    let mut selector = OneShotIssueSelector::new(issue)?;
    if one_shot_selector_consumed_at(&one_shot_selector_path(layout)) {
        let _ = selector.observe_status(issue, &QueueStatus::Passed)?;
    }
    Ok(selector)
}

pub(super) fn persist_one_shot_selector(
    layout: &RunLayout,
    selector: &OneShotIssueSelector,
) -> Result<(), String> {
    fs::create_dir_all(&layout.state_dir)
        .map_err(|error| format!("cannot create {}: {error}", layout.state_dir.display()))?;
    atomic_write(
        &one_shot_selector_path(layout),
        &format!("{}\n", selector.status_json()),
    )
}

pub(super) fn one_shot_selector_consumed(layout: &RunLayout) -> bool {
    one_shot_selector_consumed_at(&one_shot_selector_path(layout))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static FIXTURE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn fixture(document: &str) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "autospec-one-shot-selector-{}-{}",
            std::process::id(),
            FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).expect("create one-shot selector fixture");
        let path = root.join("one-shot-selector.json");
        fs::write(&path, document).expect("write one-shot selector fixture");
        (root, path)
    }

    #[test]
    fn consumed_reads_the_field_not_the_bytes() {
        // The writer emits `"consumed":true` with no space, so a substring probe agreed
        // with it by coincidence. Whitespace is the same document.
        for document in [
            r#"{"issue":42,"consumed":true,"scope":"unscoped"}"#,
            r#"{"issue": 42, "consumed": true, "scope": "unscoped"}"#,
            "{\n  \"issue\": 42,\n  \"consumed\": true,\n  \"scope\": \"unscoped\"\n}\n",
        ] {
            let (root, path) = fixture(document);
            assert!(
                one_shot_selector_consumed_at(&path),
                "consumed selector read as unconsumed: {document}"
            );
            fs::remove_dir_all(root).expect("remove one-shot selector fixture");
        }
    }

    #[test]
    fn consumed_reads_the_top_level_field_only() {
        // A nested `consumed` puts the old probe's exact bytes in the document while the
        // selector's own field is false. Escaping it into a string value would NOT
        // discriminate — JSON escapes the quotes, so the substring never matched there
        // and such a test passes against the very code this replaces.
        let (root, path) = fixture(r#"{"issue":42,"consumed":false,"prior":{"consumed":true}}"#);
        assert!(
            !one_shot_selector_consumed_at(&path),
            "a nested consumed flag was read as the selector's own"
        );
        fs::remove_dir_all(root).expect("remove one-shot selector fixture");
    }

    #[test]
    fn unconsumed_and_unreadable_documents_stay_false() {
        // `false` is the fail-safe: the caller only skips work when consumption is proven.
        for document in [
            r#"{"issue":42,"consumed":false,"scope":"active"}"#,
            r#"{"issue": 42, "consumed": false}"#,
            "{not-json\n",
            "",
        ] {
            let (root, path) = fixture(document);
            assert!(
                !one_shot_selector_consumed_at(&path),
                "unconsumed selector read as consumed: {document:?}"
            );
            fs::remove_dir_all(root).expect("remove one-shot selector fixture");
        }
        assert!(
            !one_shot_selector_consumed_at(Path::new(
                "/nonexistent/autospec/one-shot-selector.json"
            )),
            "absent selector read as consumed"
        );
    }
}
