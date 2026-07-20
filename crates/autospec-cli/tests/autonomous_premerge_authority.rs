use std::fs;
use std::path::Path;

#[test]
fn source_has_no_legacy_authority() {
    let source = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands/autonomous/premerge.rs"),
    )
    .expect("read premerge command source");
    for required in [
        "Command::new(\"git\")",
        "&[\"symbolic-ref\", \"--quiet\", \"--short\", \"HEAD\"]",
        "&[\"rev-parse\", \"HEAD\"]",
        "&[\"status\", \"--porcelain\", \"--untracked-files=no\"]",
        "evaluate_premerge(&lane, qa, security)",
    ] {
        assert!(
            source.contains(required),
            "missing Rust authority: {required}"
        );
    }
    for forbidden in [
        "Command::new(\"bash\")",
        "Command::new(\"sh\")",
        "omx",
        "/autospec-run",
        "AUTOSPEC_AUTONOMOUS_PREMERGE_CMD",
        "autonomous-premerge-gate.sh",
        "scripts/autospec-autonomous.sh",
    ] {
        assert!(!source.contains(forbidden), "legacy authority: {forbidden}");
    }
}
