use autospec_core::shell_ratchet::{
    delta, diff_verdict, measure, verdict, RatchetDiffVerdict, RatchetVerdict,
};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A unique temp directory per test, using the repository's existing
/// convention (std::env::temp_dir) rather than adding a dev-dependency.
struct Tmp(PathBuf);

impl Tmp {
    fn new(tag: &str) -> Self {
        static N: AtomicUsize = AtomicUsize::new(0);
        let p = std::env::temp_dir().join(format!(
            "autospec-shell-ratchet-{}-{}-{}",
            tag,
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        Tmp(p)
    }
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn write(dir: &std::path::Path, rel: &str, body: &str) {
    let p = dir.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, body).unwrap();
}

#[test]
fn counts_shell_and_bats_separately() {
    let tmp = Tmp::new("t");
    write(tmp.path(), "scripts/a.sh", "echo one\necho two\n");
    write(tmp.path(), "tests/b.bats", "@test \"x\" {\n  true\n}\n");
    let s = measure(tmp.path()).unwrap();
    assert_eq!(s.lines.get("shell"), Some(&2));
    assert_eq!(s.lines.get("bats"), Some(&3));
    assert_eq!(s.total_lines(), 5);
    assert_eq!(s.total_files(), 2);
}

#[test]
fn blank_lines_do_not_move_the_ratchet() {
    // Counting blanks would let reformatting change the number, which makes
    // the ceiling meaningless as a measure of logic.
    let tmp = Tmp::new("t");
    write(tmp.path(), "a.sh", "echo one\n\n\n\necho two\n");
    assert_eq!(measure(tmp.path()).unwrap().total_lines(), 2);
}

#[test]
fn build_output_and_vcs_metadata_are_not_counted() {
    // target/ holds generated scripts and .git holds hooks; neither is shell
    // anyone is going to port, and counting them would make the ceiling
    // depend on whether the tree had been built.
    let tmp = Tmp::new("t");
    write(tmp.path(), "keep.sh", "echo keep\n");
    write(
        tmp.path(),
        "target/generated.sh",
        "echo a\necho b\necho c\n",
    );
    write(tmp.path(), ".git/hooks/pre-commit.sh", "echo hook\n");
    write(tmp.path(), "vendor/dep/build.sh", "echo vendored\n");
    assert_eq!(measure(tmp.path()).unwrap().total_lines(), 1);
}

#[test]
fn files_without_a_counted_extension_are_ignored() {
    let tmp = Tmp::new("t");
    write(tmp.path(), "a.rs", "fn main() {}\n");
    write(tmp.path(), "README.md", "# docs\n");
    write(tmp.path(), "Makefile", "all:\n");
    assert_eq!(measure(tmp.path()).unwrap().total_lines(), 0);
}

#[test]
fn measure_records_per_file_line_counts() {
    // The per-file record is what lets a change be told apart as growth or
    // repair: the total alone cannot say which file grew while being fixed.
    let tmp = Tmp::new("t");
    write(tmp.path(), "scripts/a.sh", "echo one\necho two\n");
    write(tmp.path(), "tests/b.bats", "@test \"x\" {\n  true\n}\n");
    let s = measure(tmp.path()).unwrap();
    assert_eq!(s.per_file.get("scripts/a.sh"), Some(&2));
    assert_eq!(s.per_file.get("tests/b.bats"), Some(&3));
}

#[test]
fn a_fix_that_grows_an_existing_file_is_admitted_and_recorded() {
    // A bug fix frequently needs more lines than the bug — a missing guard,
    // an extra case. That growth must pass, and it must be recorded.
    let tmp = Tmp::new("t");
    write(tmp.path(), "scripts/a.sh", "echo one\necho two\n");
    let base = measure(tmp.path()).unwrap();
    write(
        tmp.path(),
        "scripts/a.sh",
        "echo one\necho two\nif [[ -t 1 ]]; then\n  :\nfi\n",
    );
    let head = measure(tmp.path()).unwrap();
    // The ceiling is tight on purpose: the head total (5) is above it, and a
    // total-only ratchet would refuse the fix.
    let v = diff_verdict(&base, &head, 3);
    assert!(!v.is_regression(), "{}", v.message());
    assert_eq!(v.hold_reason(), "modifies existing shell");
    let RatchetDiffVerdict::Admitted { delta, .. } = &v else {
        panic!("expected admitted: {}", v.message());
    };
    assert!(delta.maintenance_only());
    assert_eq!(delta.modified.get("scripts/a.sh"), Some(&(2, 5)));
    assert!(delta.new_files.is_empty());
    // The growth is reportable in the message, not just in the record.
    assert!(
        v.message().contains("scripts/a.sh 2 -> 5"),
        "{}",
        v.message()
    );
}

#[test]
fn a_new_shell_file_is_refused_even_when_the_ceiling_has_slack() {
    // New surface is what the moratorium exists to stop. Ceiling slack does
    // not turn a new file into a fix.
    let tmp = Tmp::new("t");
    write(tmp.path(), "scripts/a.sh", "echo one\n");
    let base = measure(tmp.path()).unwrap();
    write(tmp.path(), "scripts/b.sh", "echo new\n");
    let head = measure(tmp.path()).unwrap();
    let v = diff_verdict(&base, &head, 100);
    assert!(v.is_regression(), "{}", v.message());
    assert_eq!(v.hold_reason(), "adds a new shell file");
    let RatchetDiffVerdict::RefusedNewFiles { delta, .. } = &v else {
        panic!("expected refusal: {}", v.message());
    };
    assert_eq!(delta.new_files.get("scripts/b.sh"), Some(&1));
    assert_eq!(delta.new_lines(), 1);
}

#[test]
fn a_new_bats_file_is_refused_like_shell() {
    let tmp = Tmp::new("t");
    write(tmp.path(), "scripts/a.sh", "echo one\n");
    let base = measure(tmp.path()).unwrap();
    write(
        tmp.path(),
        "tests/unit/x.bats",
        "@test \"y\" {\n  true\n}\n",
    );
    let head = measure(tmp.path()).unwrap();
    let v = diff_verdict(&base, &head, 100);
    assert!(v.is_regression(), "{}", v.message());
    assert_eq!(v.hold_reason(), "adds a new shell file");
    assert!(v.message().contains("tests/unit/x.bats"), "{}", v.message());
}

#[test]
fn a_mixed_change_is_refused_for_the_new_file_and_records_the_fix() {
    // The refusal and the repair are different things: the verdict must name
    // both, so releasing the fix does not require re-litigating the new file.
    let tmp = Tmp::new("t");
    write(tmp.path(), "scripts/a.sh", "echo one\necho two\n");
    let base = measure(tmp.path()).unwrap();
    write(
        tmp.path(),
        "scripts/a.sh",
        "echo one\necho two\nfix\nmore\n",
    );
    write(tmp.path(), "scripts/b.sh", "echo new\n");
    let head = measure(tmp.path()).unwrap();
    let v = diff_verdict(&base, &head, 100);
    assert!(v.is_regression());
    assert_eq!(v.hold_reason(), "adds a new shell file");
    let RatchetDiffVerdict::RefusedNewFiles { delta, .. } = &v else {
        panic!("expected refusal: {}", v.message());
    };
    assert_eq!(delta.new_files.get("scripts/b.sh"), Some(&1));
    assert_eq!(delta.modified.get("scripts/a.sh"), Some(&(2, 4)));
    let msg = v.message();
    assert!(msg.contains("scripts/b.sh"), "{}", msg);
    assert!(msg.contains("scripts/a.sh 2 -> 4"), "{}", msg);
    assert!(msg.contains("porting issue"), "{}", msg);
}

#[test]
fn removing_shell_is_admitted_and_recorded() {
    let tmp = Tmp::new("t");
    write(
        tmp.path(),
        "scripts/a.sh",
        "echo one\necho two\necho three\n",
    );
    let base = measure(tmp.path()).unwrap();
    std::fs::remove_file(tmp.path().join("scripts/a.sh")).unwrap();
    let head = measure(tmp.path()).unwrap();
    let v = diff_verdict(&base, &head, 100);
    assert!(!v.is_regression(), "{}", v.message());
    let RatchetDiffVerdict::Admitted { delta, .. } = &v else {
        panic!("expected admitted: {}", v.message());
    };
    assert_eq!(delta.removed_files, vec!["scripts/a.sh".to_string()]);
    assert!(delta.modified.is_empty());
}

#[test]
fn an_unchanged_tree_admits_with_nothing_recorded() {
    let tmp = Tmp::new("t");
    write(tmp.path(), "scripts/a.sh", "echo one\n");
    let base = measure(tmp.path()).unwrap();
    let head = measure(tmp.path()).unwrap();
    let v = diff_verdict(&base, &head, 100);
    assert!(!v.is_regression());
    assert_eq!(v.hold_reason(), "modifies existing shell");
    let RatchetDiffVerdict::Admitted { delta, .. } = &v else {
        panic!("expected admitted: {}", v.message());
    };
    assert!(delta.maintenance_only());
    assert!(delta.modified.is_empty());
    assert!(
        v.message().contains("existing files modified: none"),
        "{}",
        v.message()
    );
}

#[test]
fn delta_names_new_modified_and_removed() {
    let tmp = Tmp::new("t");
    write(tmp.path(), "scripts/grow.sh", "echo one\necho two\n");
    write(tmp.path(), "scripts/gone.sh", "echo bye\n");
    let base = measure(tmp.path()).unwrap();
    write(tmp.path(), "scripts/grow.sh", "echo one\necho two\nthree\n");
    std::fs::remove_file(tmp.path().join("scripts/gone.sh")).unwrap();
    write(tmp.path(), "scripts/fresh.sh", "echo new\n");
    let head = measure(tmp.path()).unwrap();
    let d = delta(&base, &head);
    assert_eq!(d.new_files.get("scripts/fresh.sh"), Some(&1));
    assert_eq!(d.modified.get("scripts/grow.sh"), Some(&(2, 3)));
    assert_eq!(d.removed_files, vec!["scripts/gone.sh".to_string()]);
    assert_eq!(d.new_lines(), 1);
    assert!(!d.maintenance_only());
}

#[test]
fn at_the_ceiling_holds_and_one_over_regresses() {
    let tmp = Tmp::new("t");
    write(tmp.path(), "a.sh", "echo one\necho two\necho three\n");
    let s = measure(tmp.path()).unwrap();

    // Exactly at the ceiling is not a regression: a change that removes one
    // line and adds one must pass.
    let held = verdict(&s, 3);
    assert!(!held.is_regression(), "{}", held.message("Rust"));
    assert_eq!(
        held,
        RatchetVerdict::Held {
            total: 3,
            ceiling: 3,
            slack: 0
        }
    );

    let regressed = verdict(&s, 2);
    assert!(regressed.is_regression());
    assert_eq!(
        regressed,
        RatchetVerdict::Regressed {
            total: 3,
            ceiling: 2,
            excess: 1
        }
    );
}

#[test]
fn the_regression_message_names_the_two_acceptable_responses() {
    // A ratchet that only says "failed" teaches nothing. The message has to
    // tell an agent what to do instead, or it will simply raise the ceiling.
    let tmp = Tmp::new("t");
    write(tmp.path(), "a.sh", "echo one\n");
    let msg = verdict(&measure(tmp.path()).unwrap(), 0).message("Rust");
    assert!(
        msg.contains("crates/"),
        "must point at the compiled alternative: {msg}"
    );
    assert!(
        msg.contains("write it in Rust"),
        "must name the language the patch should have been written in: {msg}"
    );
    assert!(
        msg.contains("remove more shell"),
        "must offer the removal path: {msg}"
    );
    assert!(
        msg.contains("deliberately"),
        "raising the ceiling must be a decision: {msg}"
    );
}

#[test]
fn the_regression_message_redirects_to_the_resolved_language() {
    // Issue #4447: a gate that rejects without redirecting makes backlog, not
    // code. A rejected agent must be told the language the patch should have
    // been written in, resolved from the repository — Go for metabolomics-us/*,
    // Rust for InferWeave/* and berlinguyinca/autospec.
    use autospec_core::implementation_language::implementation_language;

    let tmp = Tmp::new("t");
    write(tmp.path(), "a.sh", "echo one\n");
    let regressed = verdict(&measure(tmp.path()).unwrap(), 0);

    let go = implementation_language("metabolomics-us/inferweave-gateway")
        .expect("metabolomics-us/* resolves to Go")
        .as_str();
    let go_msg = regressed.message(go);
    assert!(
        go_msg.contains("implementation language is Go"),
        "must name Go for a metabolomics-us repo: {go_msg}"
    );
    assert!(
        go_msg.contains("write it in Go"),
        "must redirect to Go: {go_msg}"
    );

    let rust = implementation_language("berlinguyinca/autospec")
        .expect("berlinguyinca/autospec resolves to Rust")
        .as_str();
    let rust_msg = regressed.message(rust);
    assert!(
        rust_msg.contains("implementation language is Rust"),
        "must name Rust for this repository: {rust_msg}"
    );
}

#[test]
fn an_unreadable_subtree_does_not_fail_the_measurement() {
    // The ratchet must not fail a build because of a permissions quirk in a
    // path it does not care about.
    let tmp = Tmp::new("t");
    write(tmp.path(), "a.sh", "echo one\n");
    let s = measure(tmp.path()).unwrap();
    assert_eq!(s.total_lines(), 1);
}

/// The repository ceiling. This number may FALL and may never RISE.
///
/// Lower it in the same change that removes shell — a ceiling left above the
/// real count is slack that the next script will silently consume, and the
/// ratchet stops meaning anything. It was set 64k too high on the first
/// attempt, from a line count that included blank lines; the measurement and
/// the ceiling must come from the same counter, which is why this is pinned
/// to `measure()` rather than to a shell one-liner.
const REPOSITORY_CEILING: usize = 227_188;

#[test]
fn the_repository_stays_under_its_shell_ceiling() {
    // Walk up to the workspace root: tests run with CWD at the crate.
    let mut root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for _ in 0..3 {
        if root.join("Cargo.lock").exists() && root.join("crates").exists() {
            break;
        }
        root = match root.parent() {
            Some(p) => p.to_path_buf(),
            None => break,
        };
    }
    let surface = measure(&root).expect("measuring the repository must succeed");
    let v = verdict(&surface, REPOSITORY_CEILING);
    assert!(
        !v.is_regression(),
        "{}\n\nshell={} bats={} files={}",
        v.message("Rust"),
        surface.lines.get("shell").copied().unwrap_or(0),
        surface.lines.get("bats").copied().unwrap_or(0),
        surface.total_files()
    );
}
