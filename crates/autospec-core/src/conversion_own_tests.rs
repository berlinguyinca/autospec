//! A patch's own newly-added tests (issue #4470).
//!
//! A patch whose own new tests fail is a different defect from one that
//! regresses existing tests: the author had the failing evidence in hand and
//! emitted anyway. The gate's hold reason must say which, so the runner fixes
//! the tests, not the code they cover. This module names the tests a patch
//! adds (`added_test_names`) and attributes a gate failure to them
//! (`own_failing_tests`), keeping that judgement in core where it is testable
//! without a ten-minute integration run.

/// Whether a diff line body starts with a test attribute: `#[test]` or
/// `#[tokio::test]` (with or without arguments), at the start of the line.
///
/// Shared with `conversion_gate` (where the added-test *count* lives); the
/// count and the names must agree on what a test is.
fn test_attribute(body: &str) -> bool {
    let trimmed = body.trim();
    trimmed == "#[test]" || trimmed.starts_with("#[tokio::test") // `]` or `(` (arguments)
}

/// The names of the test functions a patch adds, in patch order.
///
/// The attribute and its `fn` are normally on adjacent added lines; a patch
/// that formats them apart (or interposes another added attribute) yields no
/// name for that test, which is safe: the miss degrades to the generic hold
/// note, it never fabricates a false "fails its own tests" attribution.
pub fn added_test_names(patch: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut awaiting_fn = false;
    for line in patch.lines() {
        if !(line.starts_with('+') && !line.starts_with("+++")) {
            // An unchanged or removed line between the attribute and its fn
            // means the fn was not added by this patch: break the pairing.
            awaiting_fn = false;
            continue;
        }
        let body = &line[1..];
        if awaiting_fn {
            if let Some(name) = test_fn_name(body) {
                names.push(name);
            }
            awaiting_fn = false;
            continue;
        }
        if test_attribute(body) {
            awaiting_fn = true;
        }
    }
    names
}

/// The name a `fn` diff line defines, when it is a `fn <name>`.
///
/// A test `fn` may carry leading modifiers (`async fn`, `const fn`,
/// `unsafe fn`) that precede the keyword; they are stripped so the name is
/// found either way.
fn test_fn_name(body: &str) -> Option<String> {
    let mut rest = body.trim();
    for _ in 0..4 {
        let stripped = rest
            .strip_prefix("async ")
            .or_else(|| rest.strip_prefix("const "))
            .or_else(|| rest.strip_prefix("unsafe "));
        match stripped {
            Some(s) => rest = s,
            None => break,
        }
    }
    let rest = rest.strip_prefix("fn ")?;
    let name = rest
        .split(|c: char| c == '(' || c == '<' || c.is_whitespace())
        .next()?;
    (!name.is_empty()).then(|| name.to_string())
}

/// The names among `failing` that the patch itself added, deduplicated and in
/// `failing` order. Empty when none of the patch's own new tests failed.
pub fn own_failing_tests(failing: &[String], added: &[String]) -> Vec<String> {
    let mut own = Vec::new();
    for name in failing {
        if added.iter().any(|a| a == name) && !own.contains(name) {
            own.push(name.clone());
        }
    }
    own
}

/// The hold reason for a gate failure: the generic one, unless the failure is
/// in the patch's own newly-added tests, in which case it names them as the
/// author's process failure rather than a regression of existing code.
pub fn own_test_failure_reason(failure_note: &str, failing: &[String], added: &[String]) -> String {
    let own = own_failing_tests(failing, added);
    if own.is_empty() {
        format!("gate failed: {failure_note}")
    } else {
        format!(
            "gate failed: the patch fails its own newly-added test(s): {}",
            own.join(", ")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn added_test_names_names_the_tests_a_patch_adds() {
        let patch = "\
+    #[test]
+    fn the_new_test() {
+        assert_eq!(one(), 1);
+    }
+    #[tokio::test]
+    async fn async_added() {}
";
        assert_eq!(
            added_test_names(patch),
            vec!["the_new_test".to_string(), "async_added".to_string()]
        );
    }

    #[test]
    fn added_test_names_ignores_removed_and_unchanged_tests() {
        // A moved test removes then re-adds its attribute: the `fn` after the
        // removed attribute is not an added line, so only the re-add counts.
        let moved = "\
-    #[test]
-    fn moved_test() {}
+    #[test]
+    fn moved_test() {}
";
        assert_eq!(added_test_names(moved), vec!["moved_test".to_string()]);
        // Non-test added lines never produce a name.
        let plain = "\
+++ b/crates/autospec-cli/tests/a.rs
+    let x = 1;
";
        assert_eq!(added_test_names(plain), Vec::<String>::new());
        // An added attribute not followed by an added fn names nothing.
        let dangling = "\
+    #[test]
+    let x = 1;
";
        assert_eq!(added_test_names(dangling), Vec::<String>::new());
    }

    #[test]
    fn own_failing_tests_intersects_the_failing_set_with_the_added_set() {
        let failing = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let added = vec!["b".to_string(), "d".to_string()];
        assert_eq!(own_failing_tests(&failing, &added), vec!["b".to_string()]);
        // None of the patch's own tests failed: empty, never a fabrication.
        assert_eq!(
            own_failing_tests(&failing, &vec!["d".to_string()]),
            Vec::<String>::new()
        );
        // Duplicate names in the failing set are deduplicated.
        let dup = vec!["b".to_string(), "b".to_string()];
        assert_eq!(own_failing_tests(&dup, &added), vec!["b".to_string()]);
    }

    #[test]
    fn the_reason_names_own_test_failures_and_only_them() {
        let failing = vec!["a".to_string(), "own_test".to_string()];
        let added = vec!["own_test".to_string()];
        let reason = own_test_failure_reason("1 failing test(s)", &failing, &added);
        assert!(
            reason.contains("fails its own newly-added test(s)"),
            "{reason}"
        );
        assert!(reason.contains("own_test"), "{reason}");
        // A regression of existing tests keeps the generic note.
        let regressed = own_test_failure_reason("1 failing test(s)", &failing, &[]);
        assert!(!regressed.contains("fails its own"), "{regressed}");
        assert!(
            regressed.contains("gate failed: 1 failing test(s)"),
            "{regressed}"
        );
    }
}
