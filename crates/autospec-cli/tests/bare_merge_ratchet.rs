//! A ratchet on merging a PR outside the review-quorum gate (T16). Every merge must go through
//! `autospec-guarded-merge.sh` (or the executor's in-process gate), so a NEW merge instruction
//! in `scripts/`, `crates/`, `skills/`, `docs/`, `tests/` or a top-level doc is a finding.
//! A "merge signal" is a line (whitespace collapsed) with `gh pr merge`, a `gh api ... /merge`
//! call, or an `--admin` flag. Each file carries TWO ceilings in
//! `tests/fixtures/bare-merge-allowlist.txt` (`<path> <executable> <prose>`): executable lines
//! (code fences, shell and Rust code) and prose (markdown outside fences, comments). A ceiling
//! may fall, never rise, so a bare merge cannot hide behind a prose mention that was removed.
//! Only the allowlist file itself is skipped; `fixtures` directories are scanned.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

const ALLOWLIST: &str = "tests/fixtures/bare-merge-allowlist.txt";

fn collapse(line: &str) -> String {
    line.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_merge_signal(line: &str) -> bool {
    let l = collapse(line);
    l.contains("gh pr merge")
        || l.contains("--admin")
        || (l.contains("gh api") && l.contains("/merge"))
}

/// `(executable, prose)` merge-signal line counts for one file.
fn classify(path: &str, text: &str) -> (usize, usize) {
    let markdown = path.ends_with(".md") || path.ends_with(".txt");
    let (mut exec, mut prose, mut in_fence) = (0, 0, false);
    for line in text.lines() {
        let bare = line.trim_start().trim_start_matches('>').trim_start();
        if markdown && bare.starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if !is_merge_signal(line) {
            continue;
        }
        let comment = ["#", "//", "*", "--"].iter().any(|c| bare.starts_with(c));
        if (markdown && !in_fence) || (!markdown && comment) {
            prose += 1;
        } else {
            exec += 1;
        }
    }
    (exec, prose)
}

fn walk(dir: &Path, root: &Path, out: &mut BTreeMap<String, (usize, usize)>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            if matches!(name.as_str(), "target" | ".git" | "node_modules") {
                continue;
            }
            walk(&path, root, out);
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        if rel == ALLOWLIST {
            continue;
        }
        if let Ok(text) = fs::read_to_string(&path) {
            let counts = classify(&rel, &text);
            if counts != (0, 0) {
                out.insert(rel, counts);
            }
        }
    }
}

fn scan(root: &Path) -> BTreeMap<String, (usize, usize)> {
    let mut found = BTreeMap::new();
    for top in ["scripts", "crates", "skills", "docs", "tests"] {
        walk(&root.join(top), root, &mut found);
    }
    if let Ok(entries) = fs::read_dir(root) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if e.path().is_file() && (name.ends_with(".md") || name.ends_with(".sh")) {
                if let Ok(text) = fs::read_to_string(e.path()) {
                    let counts = classify(&name, &text);
                    if counts != (0, 0) {
                        found.insert(name, counts);
                    }
                }
            }
        }
    }
    found
}

fn read_allowlist(root: &Path) -> BTreeMap<String, (usize, usize)> {
    fs::read_to_string(root.join(ALLOWLIST))
        .unwrap()
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .map(|l| {
            let f: Vec<&str> = l.split(' ').collect();
            assert_eq!(f.len(), 3, "`<path> <executable> <prose>`: {l}");
            (
                f[0].to_string(),
                (f[1].parse().unwrap(), f[2].parse().unwrap()),
            )
        })
        .collect()
}

#[test]
fn no_new_bare_gh_pr_merge_appears() {
    let root = repo_root();
    let found = scan(&root);
    let allow = read_allowlist(&root);
    let mut findings = Vec::new();
    for (path, (exec, prose)) in &found {
        let (a_exec, a_prose) = allow.get(path).copied().unwrap_or((0, 0));
        if *exec > a_exec || *prose > a_prose {
            findings.push(format!(
                "{path}: {exec} executable / {prose} prose merge line(s), allowed {a_exec} / \
                 {a_prose}. Merge through scripts/autospec-guarded-merge.sh (review-quorum gate \
                 + head pin) instead."
            ));
        }
    }
    assert!(findings.is_empty(), "{}", findings.join("\n"));
}

#[test]
fn the_ratchet_sees_every_bare_merge_spelling() {
    for line in [
        "gh pr merge 5 --admin",
        "gh   pr\tmerge 5",
        "gh api -X PUT repos/o/r/pulls/5/merge",
        "gh api --method PUT /repos/o/r/pulls/5/merge",
        "run it with --admin please",
    ] {
        assert!(is_merge_signal(line), "{line}");
    }
    assert!(!is_merge_signal("gh pr view 5"));
}

#[test]
fn executable_and_prose_are_counted_separately() {
    let md = "Never run `gh pr merge` bare.\n```bash\ngh pr merge 5\n```\n> ```bash\n>   gh  api -X PUT x/merge\n> ```\n";
    assert_eq!(classify("a.md", md), (2, 1));
    let sh = "# gh pr merge is forbidden\ngh pr merge 5\n";
    assert_eq!(classify("a.sh", sh), (1, 1));
}

#[test]
fn a_bare_merge_cannot_replace_a_prose_mention() {
    // ceiling 0 exec / 1 prose: swapping the prose line for a real merge must trip it
    let (exec, prose) = classify("a.sh", "gh pr merge 5\n");
    let (a_exec, a_prose) = (0, 1);
    assert!(exec > a_exec && prose <= a_prose);
}

#[test]
fn the_ratchet_scans_fixture_dirs_and_docs() {
    let dir = std::env::temp_dir().join(format!("bare-merge-{}", std::process::id()));
    fs::create_dir_all(dir.join("crates/x/tests/fixtures")).unwrap();
    fs::create_dir_all(dir.join("docs")).unwrap();
    fs::write(dir.join("crates/x/tests/fixtures/f.sh"), "gh pr merge 5\n").unwrap();
    fs::write(dir.join("docs/d.md"), "```\ngh api -X PUT a/merge\n```\n").unwrap();
    let found = scan(&dir);
    assert_eq!(found.get("crates/x/tests/fixtures/f.sh"), Some(&(1, 0)));
    assert_eq!(found.get("docs/d.md"), Some(&(1, 0)));
    fs::remove_dir_all(&dir).ok();
}

/// `cargo test regen_allowlist -- --ignored` rewrites the ceilings from the current tree.
#[test]
#[ignore]
fn regen_allowlist() {
    let root = repo_root();
    let mut out = String::from(
        "# T16: ceilings for merge-signal lines (see crates/autospec-cli/tests/bare_merge_ratchet.rs).\n# `<path> <executable> <prose>`. Counts may fall, never rise.\n",
    );
    for (path, (e, p)) in scan(&root) {
        out.push_str(&format!("{path} {e} {p}\n"));
    }
    fs::write(root.join(ALLOWLIST), out).unwrap();
}
