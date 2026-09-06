// Unmeasured-vs-failed semantics (#3535).
//
// Lives here rather than in validation_runner.rs because that file is past
// the size ratchet's hard ceiling and may be shrunk but not lengthened.

use std::fs;
use std::path::PathBuf;

use autospec_core::validation::{
    CheckModes, CheckOwner, CheckReachability, CheckResult, ExternalCheck, Jobs, StructuralCheck,
    ToolCommand, ValidationCatalog, ValidationCheck, ValidationExecutionReport, ValidationOptions,
    ValidationPlan, ValidationRunner, ValidationStatus,
};

#[test]
fn missing_programs_are_unmeasured_rather_than_measured_failures() {
    let command = ToolCommand::new("autospec-task-two-missing-program", ["--version"])
        .expect("safe missing command definition");

    let result = command.execute("missing-tool", true);

    assert_eq!(result.exit_code, None);
    assert_eq!(result.spawn_count, 0);
    assert!(!result.is_success(), "an absent tool is never a pass");
    // Previously this asserted `is_failure()`. A tool that never ran measured nothing —
    // filing it as a measured failure claims knowledge the run does not have (#3535).
    assert!(result.is_unmeasured(), "{result:?}");
    assert!(!result.is_failure());
    assert!(result
        .unmeasured
        .as_deref()
        .is_some_and(|reason| reason.contains("autospec-task-two-missing-program")));
}
