use autospec_core::validation::output_macros::validate;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

fn fixture(manifest: &str, source: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("autospec-output-{}", SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
    let crate_dir = root.join("crates/sample/src");
    fs::create_dir_all(&crate_dir).unwrap();
    fs::write(root.join("crates/sample/Cargo.toml"), manifest).unwrap();
    fs::write(crate_dir.join("lib.rs"), source).unwrap();
    root
}

#[test]
fn library_output_is_blocking_and_describes_remediation() {
    let root = fixture("[package]\nname=\"sample\"\n[lib]\n", "pub fn run() { println!(\"x\"); }\n");
    let error = validate(&root).unwrap_err();
    assert!(error.contains("target_kind=library"));
    assert!(error.contains("file=") && error.contains("tracing"));
}

#[test]
fn binary_output_is_allowed() {
    let root = fixture("[package]\nname=\"sample\"\n[[bin]]\nname=\"sample\"\n", "fn run() { println!(\"x\"); }\n");
    assert!(validate(&root).is_ok());
}

#[test]
fn mixed_target_without_annotation_is_ambiguous() {
    let root = fixture("[package]\nname=\"sample\"\n[lib]\n[[bin]]\nname=\"sample\"\n", "pub fn run() { eprintln!(\"x\"); }\n");
    let error = validate(&root).unwrap_err();
    assert!(error.contains("target_kind=mixed"));
    assert!(error.contains("allow-output"));
}

#[test]
fn allow_annotation_clears_mixed_target_finding() {
    let root = fixture("[package]\nname=\"sample\"\n[lib]\n[[bin]]\nname=\"sample\"\n", "pub fn run() { println!(\"x\"); } // autospec:allow-output\n");
    assert!(validate(&root).is_ok());
}
