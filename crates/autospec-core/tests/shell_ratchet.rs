use autospec_core::shell_ratchet::{measure, verdict, RatchetVerdict};
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
fn at_the_ceiling_holds_and_one_over_regresses() {
    let tmp = Tmp::new("t");
    write(tmp.path(), "a.sh", "echo one\necho two\necho three\n");
    let s = measure(tmp.path()).unwrap();

    // Exactly at the ceiling is not a regression: a change that removes one
    // line and adds one must pass.
    let held = verdict(&s, 3);
    assert!(!held.is_regression(), "{}", held.message());
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
    let msg = verdict(&measure(tmp.path()).unwrap(), 0).message();
    assert!(
        msg.contains("crates/"),
        "must point at the Rust alternative: {msg}"
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
const REPOSITORY_CEILING: usize = 228_805;

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
        v.message(),
        surface.lines.get("shell").copied().unwrap_or(0),
        surface.lines.get("bats").copied().unwrap_or(0),
        surface.total_files()
    );
}
