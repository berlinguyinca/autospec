use autospec_core::validation::output_macros::validate;
use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static FIXTURES_BUILT: AtomicU64 = AtomicU64::new(0);

fn fixture(manifest: &str, source: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!(
        "autospec-output-{}-{}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        std::process::id(),
        FIXTURES_BUILT.fetch_add(1, Ordering::Relaxed)
    ));
    let crate_dir = root.join("crates/sample/src");
    fs::create_dir_all(&crate_dir).unwrap();
    fs::write(root.join("crates/sample/Cargo.toml"), manifest).unwrap();
    fs::write(crate_dir.join("lib.rs"), source).unwrap();
    root
}

#[test]
fn library_output_is_blocking_and_describes_remediation() {
    let root = fixture(
        "[package]\nname=\"sample\"\n[lib]\n",
        "pub fn run() { println!(\"x\"); }\n",
    );
    let error = validate(&root).unwrap_err();
    assert!(error.contains("target_kind=library"));
    assert!(error.contains("file=") && error.contains("tracing"));
}

#[test]
fn binary_output_is_allowed() {
    let root = fixture(
        "[package]\nname=\"sample\"\n[[bin]]\nname=\"sample\"\n",
        "fn run() { println!(\"x\"); }\n",
    );
    assert!(validate(&root).is_ok());
}

#[test]
fn mixed_target_without_annotation_is_ambiguous() {
    let root = fixture(
        "[package]\nname=\"sample\"\n[lib]\n[[bin]]\nname=\"sample\"\n",
        "pub fn run() { eprintln!(\"x\"); }\n",
    );
    let error = validate(&root).unwrap_err();
    assert!(error.contains("target_kind=mixed"));
    assert!(error.contains("allow-output"));
}

#[test]
fn allow_annotation_clears_mixed_target_finding() {
    let root = fixture(
        "[package]\nname=\"sample\"\n[lib]\n[[bin]]\nname=\"sample\"\n",
        "pub fn run() { println!(\"x\"); } // autospec:allow-output\n",
    );
    assert!(validate(&root).is_ok());
}

#[test]
fn findings_name_paths_relative_to_the_validation_root() {
    let root = fixture(
        "[package]\nname=\"sample\"\n[lib]\n",
        "pub fn run() { println!(\"x\"); }\n",
    );
    let error = validate(&root).unwrap_err();
    assert!(
        error.contains("file=crates/sample/src/lib.rs"),
        "the reason must name the path relative to the validation root: {error}"
    );
    assert!(
        !error.contains(&format!("file={}", root.display())),
        "an absolute worktree path must not leak into the reason: {error}"
    );
}

#[test]
fn the_same_tree_validated_in_two_directories_yields_identical_reasons() {
    // The reason bytes must not depend on where the tree lives, so a gate comparing
    // results from two checkouts sees the same set of failing checks (#3802).
    let manifest = "[package]\nname=\"sample\"\n[lib]\n";
    let source = "pub fn run() { println!(\"x\"); }\n";
    let first = validate(&fixture(manifest, source)).unwrap_err();
    let second = validate(&fixture(manifest, source)).unwrap_err();
    assert_eq!(first, second);
}
