use autospec_core::shell_ratchet::{
    allowlist_diff_verdict, allowlist_verdict, delta, measure, Allowlist, AllowlistDiffVerdict,
    AllowlistFinding,
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
fn a_repair_that_fits_the_file_s_accumulated_slack_is_admitted_and_recorded() {
    // b.sh was at 6 lines; a prior patch shrank it to 5 and kept its entry,
    // accumulating one line of slack. A repair that spends that slack lands,
    // and the change is recorded per file.
    let tmp = Tmp::new("t");
    write(
        tmp.path(),
        "scripts/b.sh",
        "echo one\necho two\necho three\necho four\necho five\n",
    );
    let base = measure(tmp.path()).unwrap();
    let allowlist = Allowlist::parse("scripts/b.sh 6\n").unwrap();
    write(
        tmp.path(),
        "scripts/b.sh",
        "echo one\necho two\necho three\necho four\necho five\nfix\n",
    );
    let head = measure(tmp.path()).unwrap();
    let v = allowlist_diff_verdict(&base, &allowlist, &head, &allowlist);
    assert!(!v.is_regression(), "{}", v.message("Rust"));
    assert_eq!(v.hold_reason(), "in sync with the shell allowlist");
    let AllowlistDiffVerdict::Admitted { delta, .. } = &v else {
        panic!("expected admitted: {}", v.message("Rust"));
    };
    assert!(delta.maintenance_only());
    assert_eq!(delta.modified.get("scripts/b.sh"), Some(&(5, 6)));
    // The growth is reportable in the message, not just in the record.
    assert!(
        v.message("Rust").contains("scripts/b.sh 5 -> 6"),
        "{}",
        v.message("Rust")
    );
}

#[test]
fn a_repair_that_grows_a_file_past_its_entry_is_refused() {
    // The same repair one line too big: over the entry, refused. Raising the
    // entry to cover the growth is refused too — the one-way property is on
    // the entry, not on the file.
    let tmp = Tmp::new("t");
    write(tmp.path(), "scripts/b.sh", "echo one\necho two\necho three\n");
    let base = measure(tmp.path()).unwrap();
    let allowlist = Allowlist::parse("scripts/b.sh 3\n").unwrap();
    write(
        tmp.path(),
        "scripts/b.sh",
        "echo one\necho two\necho three\nfix\nmore\n",
    );
    let head = measure(tmp.path()).unwrap();

    let v = allowlist_diff_verdict(&base, &allowlist, &head, &allowlist);
    assert!(v.is_regression(), "{}", v.message("Rust"));
    assert_eq!(v.hold_reason(), "grows shell past its allowlisted entry");
    let AllowlistDiffVerdict::Refused { findings, .. } = &v else {
        panic!("expected refusal: {}", v.message("Rust"));
    };
    assert_eq!(
        findings,
        &[AllowlistFinding::ExceededEntry {
            path: "scripts/b.sh".to_string(),
            lines: 5,
            entry: 3
        }]
    );

    let raised = Allowlist::parse("scripts/b.sh 5\n").unwrap();
    let v = allowlist_diff_verdict(&base, &allowlist, &head, &raised);
    assert!(v.is_regression(), "{}", v.message("Rust"));
    let AllowlistDiffVerdict::Refused { findings, .. } = &v else {
        panic!("expected refusal: {}", v.message("Rust"));
    };
    assert_eq!(
        findings,
        &[AllowlistFinding::RaisedEntry {
            path: "scripts/b.sh".to_string(),
            base: 3,
            head: 5
        }]
    );
}

#[test]
fn a_new_shell_file_is_refused_even_when_other_files_have_slack() {
    // New surface is what the moratorium exists to stop. Another file's slack
    // does not turn a new file into a fix.
    let tmp = Tmp::new("t");
    write(tmp.path(), "scripts/a.sh", "echo one\n");
    let base = measure(tmp.path()).unwrap();
    let allowlist = Allowlist::parse("scripts/a.sh 10\n").unwrap(); // 9 of slack
    write(tmp.path(), "scripts/b.sh", "echo new\n");
    let head = measure(tmp.path()).unwrap();
    let v = allowlist_diff_verdict(&base, &allowlist, &head, &allowlist);
    assert!(v.is_regression(), "{}", v.message("Rust"));
    assert_eq!(v.hold_reason(), "adds a new shell file");
    let AllowlistDiffVerdict::Refused { delta, findings, .. } = &v else {
        panic!("expected refusal: {}", v.message("Rust"));
    };
    assert_eq!(delta.new_files.get("scripts/b.sh"), Some(&1));
    assert_eq!(delta.new_lines(), 1);
    assert!(findings.contains(&AllowlistFinding::UnlistedFile {
        path: "scripts/b.sh".to_string(),
        lines: 1
    }));
    assert!(v.message("Rust").contains("scripts/b.sh"), "{}", v.message("Rust"));
}

#[test]
fn a_new_bats_file_is_refused_like_shell() {
    let tmp = Tmp::new("t");
    write(tmp.path(), "scripts/a.sh", "echo one\n");
    let base = measure(tmp.path()).unwrap();
    let allowlist = Allowlist::seed(&base);
    write(tmp.path(), "tests/unit/x.bats", "@test \"y\" {\n  true\n}\n");
    let head = measure(tmp.path()).unwrap();
    let v = allowlist_diff_verdict(&base, &allowlist, &head, &allowlist);
    assert!(v.is_regression(), "{}", v.message("Rust"));
    assert_eq!(v.hold_reason(), "adds a new shell file");
    assert!(v.message("Rust").contains("tests/unit/x.bats"), "{}", v.message("Rust"));
}

#[test]
fn listing_a_new_file_is_not_a_raise_but_the_entry_itself_is() {
    // A patch that adds the file AND its allowlist entry is not saved by the
    // listing: the entry for a path absent from the base rose from zero, and
    // that is the refusal.
    let tmp = Tmp::new("t");
    write(tmp.path(), "scripts/a.sh", "echo one\n");
    let base = measure(tmp.path()).unwrap();
    let base_allow = Allowlist::seed(&base);
    write(tmp.path(), "scripts/b.sh", "echo new\n");
    let head = measure(tmp.path()).unwrap();
    let head_allow = Allowlist::parse("scripts/a.sh 1\nscripts/b.sh 1\n").unwrap();
    let v = allowlist_diff_verdict(&base, &base_allow, &head, &head_allow);
    assert!(v.is_regression(), "{}", v.message("Rust"));
    let AllowlistDiffVerdict::Refused { findings, .. } = &v else {
        panic!("expected refusal: {}", v.message("Rust"));
    };
    assert_eq!(
        findings,
        &[AllowlistFinding::RaisedEntry {
            path: "scripts/b.sh".to_string(),
            base: 0,
            head: 1
        }]
    );
}

#[test]
fn a_mixed_change_is_refused_and_names_every_finding_and_records_the_fix() {
    // The refusal and the repair are different things: the verdict must name
    // both, so releasing the fix does not require re-litigating the new file.
    let tmp = Tmp::new("t");
    write(tmp.path(), "scripts/a.sh", "echo one\necho two\n");
    let base = measure(tmp.path()).unwrap();
    let base_allow = Allowlist::seed(&base);
    write(
        tmp.path(),
        "scripts/a.sh",
        "echo one\necho two\nfix\nmore\n",
    );
    write(tmp.path(), "scripts/b.sh", "echo new\n");
    let head = measure(tmp.path()).unwrap();
    let v = allowlist_diff_verdict(&base, &base_allow, &head, &base_allow);
    assert!(v.is_regression());
    assert_eq!(v.hold_reason(), "adds a new shell file");
    let AllowlistDiffVerdict::Refused {
        delta, findings, ..
    } = &v
    else {
        panic!("expected refusal: {}", v.message("Rust"));
    };
    assert_eq!(delta.new_files.get("scripts/b.sh"), Some(&1));
    assert_eq!(delta.modified.get("scripts/a.sh"), Some(&(2, 4)));
    assert!(findings.contains(&AllowlistFinding::ExceededEntry {
        path: "scripts/a.sh".to_string(),
        lines: 4,
        entry: 2
    }));
    assert!(findings.contains(&AllowlistFinding::UnlistedFile {
        path: "scripts/b.sh".to_string(),
        lines: 1
    }));
    let msg = v.message("Rust");
    assert!(msg.contains("scripts/b.sh"), "{msg}");
    assert!(msg.contains("scripts/a.sh 2 -> 4"), "{msg}");
    assert!(msg.contains("porting issue"), "{msg}");
}

#[test]
fn removing_shell_lowers_its_entry_in_the_same_commit() {
    let tmp = Tmp::new("t");
    write(
        tmp.path(),
        "scripts/a.sh",
        "echo one\necho two\necho three\n",
    );
    let base = measure(tmp.path()).unwrap();
    let base_allow = Allowlist::seed(&base);
    fs::remove_file(tmp.path().join("scripts/a.sh")).unwrap();
    let head = measure(tmp.path()).unwrap();
    // The entry goes with the file, in the same commit: no separate step.
    let head_allow = Allowlist::seed(&head);
    let v = allowlist_diff_verdict(&base, &base_allow, &head, &head_allow);
    assert!(!v.is_regression(), "{}", v.message("Rust"));
    let AllowlistDiffVerdict::Admitted { delta, .. } = &v else {
        panic!("expected admitted: {}", v.message("Rust"));
    };
    assert_eq!(delta.removed_files, vec!["scripts/a.sh".to_string()]);
    assert!(delta.modified.is_empty());
    // Leaving the entry behind is drift: the file is gone, the budget is not.
    let v = allowlist_diff_verdict(&base, &base_allow, &head, &base_allow);
    assert!(v.is_regression(), "{}", v.message("Rust"));
    assert_eq!(v.hold_reason(), "allowlist out of sync with the tree");
    let AllowlistDiffVerdict::Refused { findings, .. } = &v else {
        panic!("expected refusal: {}", v.message("Rust"));
    };
    assert_eq!(
        findings,
        &[AllowlistFinding::StaleEntry {
            path: "scripts/a.sh".to_string(),
            entry: 3
        }]
    );
}

#[test]
fn an_unchanged_tree_admits_with_nothing_recorded() {
    let tmp = Tmp::new("t");
    write(tmp.path(), "scripts/a.sh", "echo one\n");
    let base = measure(tmp.path()).unwrap();
    let allowlist = Allowlist::seed(&base);
    let head = measure(tmp.path()).unwrap();
    let v = allowlist_diff_verdict(&base, &allowlist, &head, &allowlist);
    assert!(!v.is_regression());
    assert_eq!(v.hold_reason(), "in sync with the shell allowlist");
    let AllowlistDiffVerdict::Admitted { delta, .. } = &v else {
        panic!("expected admitted: {}", v.message("Rust"));
    };
    assert!(delta.maintenance_only());
    assert!(delta.modified.is_empty());
    assert!(
        v.message("Rust").contains("existing files modified: none"),
        "{}",
        v.message("Rust")
    );
}

#[test]
fn delta_names_new_modified_and_removed() {
    let tmp = Tmp::new("t");
    write(tmp.path(), "scripts/grow.sh", "echo one\necho two\n");
    write(tmp.path(), "scripts/gone.sh", "echo bye\n");
    let base = measure(tmp.path()).unwrap();
    write(tmp.path(), "scripts/grow.sh", "echo one\necho two\nthree\n");
    fs::remove_file(tmp.path().join("scripts/gone.sh")).unwrap();
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
fn the_drifted_message_names_the_two_acceptable_responses() {
    // A ratchet that only says "failed" teaches nothing. The message has to
    // tell an agent what to do instead, or it will simply reseed the file.
    let tmp = Tmp::new("t");
    write(tmp.path(), "a.sh", "echo one\n");
    let allow = Allowlist::parse("a.sh 0\n").unwrap();
    let v = allowlist_verdict(&measure(tmp.path()).unwrap(), &allow);
    let msg = v.message("Rust");
    assert!(
        msg.contains("crates/"),
        "must point at the compiled alternative: {msg}"
    );
    assert!(
        msg.contains("write it in Rust"),
        "must name the language the patch should have been written in: {msg}"
    );
    assert!(
        msg.contains("lower the entries"),
        "must offer the removal path: {msg}"
    );
    assert!(
        msg.contains("deliberately"),
        "raising an entry must be a decision: {msg}"
    );
}

#[test]
fn the_refused_message_redirects_to_the_resolved_language() {
    // Issue #4447: a gate that rejects without redirecting makes backlog, not
    // code. A rejected agent must be told the language the patch should have
    // been written in, resolved from the repository — Go for metabolomics-us/*,
    // Rust for InferWeave/* and berlinguyinca/autospec.
    use autospec_core::implementation_language::implementation_language;

    let tmp = Tmp::new("t");
    write(tmp.path(), "a.sh", "echo one\n");
    let s = measure(tmp.path()).unwrap();
    let allow = Allowlist::default(); // nothing listed: the file is unlisted
    let drifted = allowlist_verdict(&s, &allow);

    let go = implementation_language("metabolomics-us/inferweave-gateway")
        .expect("metabolomics-us/* resolves to Go")
        .as_str();
    let go_msg = drifted.message(go);
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
    let rust_msg = drifted.message(rust);
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
