//! Ship an archive, not a tree (issue #4577).
//!
//! A base refresh publishes an immutable generation (see
//! `immutable_base.rs`); this suite proves the per-file-cost fix: a generation
//! of many small files — dominated by loose git objects — collapses into ONE
//! tar file an agent fetches with a single sequential read, verifies against a
//! writer-recorded sha256, and extracts back to the exact working tree, with
//! `.git` never shipped.

#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use autospec_core::base_archive::{
    build_archive, extract_archive, fetch_archive, read_archive, verify_archive, ArchiveError,
    ARCHIVE_FILE, ARCHIVE_RECORD,
};

fn temp_dir(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after the epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "autospec-base-archive-{label}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&path).expect("temp dir is created");
    path
}

fn write_file(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("parent dir");
    }
    fs::write(path, content).expect("write file");
}

/// Fill a generation the way a base checkout would: a small working tree, a
/// pile of loose git objects under `.git/objects`, and a `target/` build dir.
/// Returns the number of working-tree regular files.
fn fill_generation(
    gen: &Path,
    working: &[(&str, &str)],
    loose_objects: usize,
    target_files: usize,
) {
    for (rel, content) in working {
        write_file(&gen.join(rel), content);
    }
    for i in 0..loose_objects {
        let sha = format!("{i:040x}");
        write_file(
            &gen.join(".git/objects").join(&sha[0..2]).join(&sha[2..]),
            &format!("loose object {i}\n"),
        );
    }
    for i in 0..target_files {
        write_file(
            &gen.join("target/debug").join(format!("dep_{i}")),
            &format!("build artifact {i}\n"),
        );
    }
}

/// AC (invariant 2): a generation of many files collapses into ONE tar file,
/// and the archive holds the working tree, not `.git` or the build dir.
#[test]
fn build_collapses_tree_to_one_file_and_excludes_git() {
    let root = temp_dir("collapse");
    let gen = root.join("gen-0000000001");
    let working = [
        ("src/lib.rs", "fn main() {}\n"),
        ("src/bin.rs", "pub fn run() {}\n"),
        ("README.md", "# base\n"),
        ("config.toml", "[a]\nb = 1\n"),
    ];
    fill_generation(&gen, &working, 40, 3);

    let info = build_archive(&gen).expect("build archives a generation");

    // The working tree is packed; the 40 loose objects and 3 build files are not.
    assert_eq!(
        info.files,
        working.len() as u64,
        "archive holds only the working tree"
    );
    assert_eq!(info.file, ARCHIVE_FILE);
    assert!(info.bytes > 0, "archive is not empty");
    assert_eq!(info.sha256.len(), 64, "sha256 is 64 hex chars");
    assert!(info.sha256.chars().all(|c| c.is_ascii_hexdigit()));

    // The single file and its record now sit beside the generation's files.
    assert!(gen.join(ARCHIVE_FILE).is_file(), "one tar file is produced");
    assert!(gen.join(ARCHIVE_RECORD).is_file(), "the record is written");

    // The archive itself ships no `.git` member (invariant 3): extracting it
    // yields the working tree and nothing under `.git` or `target`.
    let extract = root.join("extracted");
    let n = extract_archive(&gen.join(ARCHIVE_FILE), &extract).expect("extract");
    assert_eq!(
        n,
        working.len() as u64,
        "extract reproduces exactly the working tree"
    );
    assert!(!extract.join(".git").exists(), ".git is never shipped");
    assert!(
        !extract.join("target").exists(),
        "build dirs are never shipped"
    );
    for (rel, content) in working {
        let got = fs::read_to_string(extract.join(rel)).expect("extracted file exists");
        assert_eq!(got, *content, "extracted {rel} matches the source");
    }
}

/// AC (invariant 2): the agent fetches the single file and it verifies against
/// the writer-recorded sha256.
#[test]
fn fetch_is_one_file_and_verifies() {
    let root = temp_dir("fetch");
    let gen = root.join("gen");
    fill_generation(
        &gen,
        &[
            ("src/a.rs", "a\n"),
            ("src/b.rs", "b\n"),
            ("top.txt", "top\n"),
        ],
        25,
        2,
    );
    let info = build_archive(&gen).expect("build");

    let dest = root.join("agent-repo");
    let fetched = fetch_archive(&gen, &dest).expect("agent fetches the one file");
    assert_eq!(
        fetched,
        dest.join(ARCHIVE_FILE),
        "fetch lands the single archive file"
    );
    assert!(fetched.is_file());

    // The fetched copy hashes to the writer-recorded sha256.
    verify_archive(&info, &fetched).expect("a clean fetch verifies");
    assert_eq!(
        read_archive(&gen).expect("record reads back").sha256,
        info.sha256
    );
}

/// AC (fail closed): a corrupted fetch does not verify.
#[test]
fn verify_fails_closed_on_corruption() {
    let root = temp_dir("corrupt");
    let gen = root.join("gen");
    fill_generation(&gen, &[("src/a.rs", "a\n"), ("top.txt", "top\n")], 10, 1);
    let info = build_archive(&gen).expect("build");

    let dest = root.join("agent-repo");
    let fetched = fetch_archive(&gen, &dest).expect("fetch");
    // Flip a byte in the fetched copy (simulating a torn or bit-flipped read).
    let bytes = fs::read(&fetched).expect("read fetched");
    let mut bad = bytes;
    *bad.last_mut().expect("archive is non-empty") ^= 0xff;
    fs::write(&fetched, &bad).expect("write corrupted copy");

    let err = verify_archive(&info, &fetched).expect_err("corrupt fetch must not verify");
    assert!(
        matches!(err, ArchiveError::Corrupt { .. }),
        "corruption is a Corrupt error, got {err}"
    );
}

/// AC (invariant 2): fetching one file and extracting it reproduces the exact
/// working-tree contents byte-for-byte.
#[test]
fn extract_round_trips_working_tree() {
    let root = temp_dir("roundtrip");
    let gen = root.join("gen");
    let working = [
        ("src/deep/nested.rs", "pub fn nested() {}\n"),
        ("docs/guide.md", "line one\nline two\n"),
        ("data/points.csv", "1,2,3\n4,5,6\n"),
    ];
    fill_generation(&gen, &working, 60, 4);

    let info = build_archive(&gen).expect("build");
    assert_eq!(info.files, working.len() as u64);

    let dest = root.join("agent-repo");
    let fetched = fetch_archive(&gen, &dest).expect("fetch");
    verify_archive(&info, &fetched).expect("verify before trusting the fetch");
    let tree = root.join("tree");
    let n = extract_archive(&fetched, &tree).expect("extract");
    assert_eq!(n, working.len() as u64);
    for (rel, content) in working {
        let got = fs::read_to_string(tree.join(rel)).expect("extracted file");
        assert_eq!(got, *content, "round-trip {rel} is byte-identical");
    }
}

/// AC (fail closed): a base that packs to no regular files is a defect, not an
/// empty base — a generation whose only content is `.git`/`target` refuses.
#[test]
fn build_fails_closed_on_empty_working_tree() {
    let root = temp_dir("empty");
    let gen = root.join("gen");
    // Only excluded content: loose objects and build artifacts, no working tree.
    fill_generation(&gen, &[], 12, 2);

    let err = build_archive(&gen).expect_err("nothing to ship");
    assert!(
        matches!(err, ArchiveError::EmptyArchive { .. }),
        "empty working tree is EmptyArchive, got {err}"
    );
    assert!(
        !gen.join(ARCHIVE_RECORD).exists(),
        "no record is written for an empty archive"
    );
}

/// AC (fail closed): a non-directory generation is an error.
#[test]
fn build_fails_closed_on_missing_generation() {
    let root = temp_dir("missing");
    let err = build_archive(&root.join("nope")).expect_err("not a dir");
    assert!(matches!(err, ArchiveError::NoGeneration { .. }), "{err}");
}

/// AC (fail closed): reading a record that is absent or malformed is an error.
#[test]
fn read_archive_fails_closed() {
    let root = temp_dir("no-record");
    let gen = root.join("gen");
    fill_generation(&gen, &[("a.txt", "a\n")], 0, 0);
    // No build → no record.
    let err = read_archive(&gen).expect_err("no record yet");
    assert!(matches!(err, ArchiveError::NoArchive { .. }), "{err}");

    // A present-but-malformed record is BadRecord.
    write_file(
        &gen.join(ARCHIVE_RECORD),
        "file: base-archive.tar\nfiles: not-a-number\nbytes: 0\nsha256: 0\n",
    );
    let err = read_archive(&gen).expect_err("malformed record");
    assert!(matches!(err, ArchiveError::BadRecord { .. }), "{err}");
}

/// AC (fail closed): an agent cannot fetch a generation that has no archive.
#[test]
fn fetch_fails_closed_without_record() {
    let root = temp_dir("fetch-norec");
    let gen = root.join("gen");
    fill_generation(&gen, &[("a.txt", "a\n")], 0, 0);
    let dest = root.join("agent-repo");
    let err = fetch_archive(&gen, &dest).expect_err("no archive to fetch");
    assert!(matches!(err, ArchiveError::NoArchive { .. }), "{err}");
}
