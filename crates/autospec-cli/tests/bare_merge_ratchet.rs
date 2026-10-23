//! A ratchet on `gh pr merge` (T16): every merge must go through the review-quorum gate
//! (`autospec-guarded-merge.sh`, or the executor's in-process gate), so a NEW occurrence of the
//! bare command in `scripts/`, `crates/` or `skills/` is a finding. Prose that forbids the bare
//! command and the gates' own implementation count too, so each file carries a ceiling in
//! `tests/fixtures/bare-merge-allowlist.txt` (`<path> <count>`); a count may fall, never rise,
//! and a file with no entry may not mention the command at all.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const NEEDLE: &str = "gh pr merge";

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn walk(dir: &Path, root: &Path, out: &mut BTreeMap<String, usize>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            if matches!(
                name.as_str(),
                "target" | ".git" | "node_modules" | "fixtures"
            ) {
                continue;
            }
            walk(&path, root, out);
        } else if let Ok(text) = fs::read_to_string(&path) {
            let count = text.matches(NEEDLE).count();
            if count > 0 {
                let rel = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                out.insert(rel, count);
            }
        }
    }
}

#[test]
fn no_new_bare_gh_pr_merge_appears() {
    let root = repo_root();
    let mut found = BTreeMap::new();
    for top in ["scripts", "crates", "skills"] {
        walk(&root.join(top), &root, &mut found);
    }
    let allow: BTreeMap<String, usize> =
        fs::read_to_string(root.join("tests/fixtures/bare-merge-allowlist.txt"))
            .unwrap()
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
            .map(|l| {
                let (path, n) = l.rsplit_once(' ').expect("`<path> <count>`");
                (path.to_string(), n.parse().expect("count"))
            })
            .collect();
    let mut findings = Vec::new();
    for (path, count) in &found {
        let allowed = allow.get(path).copied().unwrap_or(0);
        if *count > allowed {
            findings.push(format!(
                "{path}: {count} mention(s) of `{NEEDLE}`, allowed {allowed}. Merge through \
                 scripts/autospec-guarded-merge.sh (review-quorum gate + head pin) instead."
            ));
        }
    }
    assert!(findings.is_empty(), "{}", findings.join("\n"));
}

#[test]
fn the_ratchet_sees_a_planted_bare_merge() {
    let dir = std::env::temp_dir().join(format!("bare-merge-{}", std::process::id()));
    fs::create_dir_all(dir.join("scripts")).unwrap();
    fs::write(dir.join("scripts/x.sh"), "gh pr merge 5 --admin\n").unwrap();
    let mut found = BTreeMap::new();
    walk(&dir.join("scripts"), &dir, &mut found);
    assert_eq!(found.get("scripts/x.sh"), Some(&1));
    fs::remove_dir_all(&dir).ok();
}
