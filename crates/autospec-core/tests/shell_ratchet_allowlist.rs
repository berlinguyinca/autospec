//! Allowlist mechanics for the shell ratchet (issue #4442): parsing,
//! seeding, the per-file findings, and the invariant that pins the shipped
//! allowlist to the real tree.

use autospec_core::shell_ratchet::{
    allowlist_verdict, measure, raised_entries, Allowlist, AllowlistError, AllowlistFinding,
    AllowlistVerdict,
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
fn the_allowlist_round_trips_through_parse_and_render() {
    let text = "scripts/a.sh 10\n\n# a comment\ntests/b.bats 5\n";
    let a = Allowlist::parse(text).unwrap();
    assert_eq!(a.len(), 2);
    assert_eq!(a.get("scripts/a.sh"), Some(10));
    assert_eq!(a.get("tests/b.bats"), Some(5));
    assert_eq!(a.get("missing.sh"), None);
    assert_eq!(a.total(), 15);
    // render is canonical: sorted by path, no comments or blanks, so a
    // rendered file round-trips byte-for-byte.
    assert_eq!(a.render(), "scripts/a.sh 10\ntests/b.bats 5\n");
    assert_eq!(Allowlist::parse(&a.render()).unwrap(), a);
}

#[test]
fn the_allowlist_parses_paths_with_spaces() {
    let a = Allowlist::parse("scripts/my script.sh 3\n").unwrap();
    assert_eq!(a.get("scripts/my script.sh"), Some(3));
    assert_eq!(a.len(), 1);
}

#[test]
fn a_malformed_allowlist_line_is_rejected_with_its_number() {
    let err = Allowlist::parse("scripts/a.sh 10\nnot a count line\n").unwrap_err();
    assert!(
        matches!(err, AllowlistError::MalformedLine { line: 2, .. }),
        "expected line 2, got {err:?}"
    );
    let err = Allowlist::parse("scripts/a.sh many\n").unwrap_err();
    assert!(
        matches!(err, AllowlistError::MalformedLine { line: 1, .. }),
        "expected line 1, got {err:?}"
    );
    let err = Allowlist::parse(" 10\n").unwrap_err();
    assert!(
        matches!(err, AllowlistError::MalformedLine { line: 1, .. }),
        "a count with no path is malformed: {err:?}"
    );
}

#[test]
fn the_allowlist_seeds_every_counted_file_at_its_count() {
    let tmp = Tmp::new("t");
    write(tmp.path(), "scripts/a.sh", "echo one\necho two\n");
    write(tmp.path(), "tests/b.bats", "@test \"x\" {\n  true\n}\n");
    let s = measure(tmp.path()).unwrap();
    let a = Allowlist::seed(&s);
    assert_eq!(a.get("scripts/a.sh"), Some(2));
    assert_eq!(a.get("tests/b.bats"), Some(3));
    assert_eq!(a.total(), s.total_lines());
    // A freshly seeded allowlist keeps its own tree green.
    assert!(allowlist_verdict(&s, &a).is_clean());
}

#[test]
fn a_new_unlisted_file_is_a_blocking_finding() {
    let tmp = Tmp::new("t");
    write(tmp.path(), "scripts/a.sh", "echo one\n");
    let s = measure(tmp.path()).unwrap();
    let a = Allowlist::seed(&s);
    write(tmp.path(), "scripts/b.sh", "echo new\n");
    let head = measure(tmp.path()).unwrap();
    let v = allowlist_verdict(&head, &a);
    assert!(!v.is_clean());
    let AllowlistVerdict::Drifted { findings, .. } = &v else {
        panic!("expected drift: {}", v.message("Rust"));
    };
    assert_eq!(
        findings,
        &[AllowlistFinding::UnlistedFile {
            path: "scripts/b.sh".to_string(),
            lines: 1
        }]
    );
    assert!(v.message("Rust").contains("scripts/b.sh"), "{}", v.message("Rust"));
}

#[test]
fn a_file_at_its_entry_passes_and_one_more_line_fails() {
    let tmp = Tmp::new("t");
    write(tmp.path(), "a.sh", "echo one\n");
    let s = measure(tmp.path()).unwrap();
    let a = Allowlist::seed(&s);
    assert!(allowlist_verdict(&s, &a).is_clean());
    write(tmp.path(), "a.sh", "echo one\necho two\n");
    let head = measure(tmp.path()).unwrap();
    let v = allowlist_verdict(&head, &a);
    assert!(!v.is_clean());
    let AllowlistVerdict::Drifted { findings, .. } = &v else {
        panic!("expected drift: {}", v.message("Rust"));
    };
    assert_eq!(
        findings,
        &[AllowlistFinding::ExceededEntry {
            path: "a.sh".to_string(),
            lines: 2,
            entry: 1
        }]
    );
}

#[test]
fn a_count_below_the_entry_passes() {
    // a.sh was at 3 lines; a prior patch shrank it to 2 and kept its entry.
    // The slack is the file's own budget, and a count below the entry is the
    // normal state after a removal.
    let a = Allowlist::parse("a.sh 3\n").unwrap();
    let tmp = Tmp::new("t");
    write(tmp.path(), "a.sh", "echo one\necho two\n");
    let s = measure(tmp.path()).unwrap();
    let v = allowlist_verdict(&s, &a);
    assert!(v.is_clean(), "{}", v.message("Rust"));
    assert!(matches!(
        v,
        AllowlistVerdict::InSync { total: 2, ceiling: 3 }
    ));
}

#[test]
fn a_stale_entry_is_a_blocking_finding() {
    // The file is gone but its budget is not: the allowlist has drifted above
    // reality and silently re-opened slack. The entry has to go with the file.
    let a = Allowlist::parse("gone.sh 4\n").unwrap();
    let tmp = Tmp::new("t");
    let s = measure(tmp.path()).unwrap();
    let v = allowlist_verdict(&s, &a);
    assert!(!v.is_clean());
    let AllowlistVerdict::Drifted { findings, .. } = &v else {
        panic!("expected drift: {}", v.message("Rust"));
    };
    assert_eq!(
        findings,
        &[AllowlistFinding::StaleEntry {
            path: "gone.sh".to_string(),
            entry: 4
        }]
    );
}

#[test]
fn entries_can_only_fall() {
    let base = Allowlist::parse("a.sh 10\nb.sh 5\nc.sh 2\n").unwrap();
    // a.sh rose 10 -> 11, d.sh appeared with an entry (a rise from zero);
    // b.sh held and c.sh held: exactly two rises.
    let head = Allowlist::parse("a.sh 11\nb.sh 5\nc.sh 2\nd.sh 1\n").unwrap();
    let raised = raised_entries(&base, &head);
    assert_eq!(raised.get("a.sh"), Some(&(10, 11)));
    assert_eq!(raised.get("d.sh"), Some(&(0, 1)));
    assert_eq!(raised.len(), 2);
    // Lowering and removal are not rises.
    let head2 = Allowlist::parse("b.sh 3\n").unwrap();
    assert!(raised_entries(&base, &head2).is_empty());
}

/// The workspace root: tests run with CWD at the crate.
fn repo_root() -> PathBuf {
    let mut root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for _ in 0..3 {
        if root.join("Cargo.lock").exists() && root.join("crates").exists() {
            break;
        }
        root = match root.parent() {
            Some(p) => p.to_path_buf(),
            None => break,
        };
    }
    root
}

#[test]
fn the_shipped_allowlist_keeps_the_real_repository_scan_green() {
    // The same shape as the bats ratchet's self-enforcement
    // (tests/lint/test_bats_negation_checker.bats): scan the real tree with
    // the shipped allowlist. Red means the allowlist and the tree disagree —
    // a new shell file, a file past its entry, or an entry whose file is
    // gone. Fix the disagreement in the same commit; the gated reseed helper
    // only drops dead entries — it never changes a live one.
    let root = repo_root();
    let surface = measure(&root).expect("measuring the repository must succeed");
    let shipped = Allowlist::load(&root.join("tests/fixtures/shell-ratchet-allowlist.txt"))
        .expect("the shipped allowlist must parse");
    assert!(!shipped.is_empty(), "the shipped allowlist must list the shell surface");
    let v = allowlist_verdict(&surface, &shipped);
    assert!(
        v.is_clean(),
        "{}\n\nshell={} bats={} files={} allowlist total={}",
        v.message("Rust"),
        surface.lines.get("shell").copied().unwrap_or(0),
        surface.lines.get("bats").copied().unwrap_or(0),
        surface.total_files(),
        shipped.total()
    );
}

#[test]
fn the_reseed_helper_writes_the_allowlist_from_the_tree() {
    // Gated: CI runs without the env var and the helper is a no-op. It exists
    // for the one mechanical job a reseed is safe for: the initial seed, and
    // dropping entries whose files are gone. It refuses to change a LIVE
    // entry in either direction — a rise breaks the one-way property, and a
    // fall can erase a deliberate raise; both are decisions made in a commit
    // with a reason, not a reseed. It also refuses to mint an entry for a
    // counted file that has none: new shell surface is a finding to write in
    // Rust, not a budget to open.
    if std::env::var_os("AUTOSPEC_SHELL_RATCHET_RESEED").is_none() {
        return;
    }
    let root = repo_root();
    let path = root.join("tests/fixtures/shell-ratchet-allowlist.txt");
    let surface = measure(&root).expect("measuring the repository must succeed");
    let seeded = Allowlist::seed(&surface);
    match Allowlist::load(&path) {
        Err(_) => {} // no shipped allowlist: the initial seed writes it
        Ok(shipped) => {
            let changed: Vec<String> = shipped
                .entries()
                .iter()
                .filter_map(|(p, entry)| match seeded.get(p) {
                    None => None, // file gone: the dead entry is dropped
                    Some(count) if count != *entry => {
                        Some(format!("{p} {entry} -> {count}"))
                    }
                    Some(_) => None,
                })
                .collect();
            assert!(
                changed.is_empty(),
                "reseed refused: these live entries would change (a decision, not a reseed): {}",
                changed.join(", ")
            );
            let unlisted: Vec<String> = seeded
                .entries()
                .keys()
                .filter(|p| shipped.get(p.as_str()).is_none())
                .cloned()
                .collect();
            assert!(
                unlisted.is_empty(),
                "reseed refused: these counted files have no entry (new shell surface): {}",
                unlisted.join(", ")
            );
        }
    }
    seeded.save(&path).expect("the reseed write must succeed");
}
