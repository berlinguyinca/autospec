//! `autospec dispatch authority` (#3947): the gate that asks which program is
//! current before dispatching a run against it.
//!
//! Every test asserts on the exit code first. A dispatch that proceeds against
//! a superseded spec set is not a wrong answer to correct input — it is forty
//! merges in the wrong direction, and the exit code is the only thing a wrapper
//! can branch on.

use std::fs;
use std::path::PathBuf;
use std::process::Output;

#[path = "support/temp_directory.rs"]
mod temp_directory;
use temp_directory::unique as temp_dir;

const V2_PROGRAM: &str = "\
# Edge and gateway program
Spec-Set: inferweave/v2
Spec-Version: V2
Supersedes: inferweave/v1
Decision-Record: docs/adr/0007-permanent-ownership.md
Authority-Over: gateway, edge
";

const CLEAN_SLATE: &str = "\
# Clean-slate implementation charter
Spec-Set: inferweave/mono
Spec-Version: V1
Supersedes: inferweave/v2
Decision-Record: docs/adr/0011-clean-slate.md
Authority-Over: gateway, edge, control-plane
";

const SUPERSEDED_CHARTER: &str = "\
# Former product charter
Spec-Set: inferweave/legacy
Spec-Version: V1
Superseded-By: inferweave/mono
Decision-Record: docs/adr/0011-clean-slate.md
Authority-Over: registry
";

const NO_CURRENCY: &str = "\
# Former product charter
This charter describes the original single-repository product.
";

struct Harness {
    temp: PathBuf,
    specs: PathBuf,
}

impl Harness {
    fn new(tag: &str) -> Self {
        let temp = temp_dir(tag);
        let specs = temp.join("specs");
        fs::create_dir_all(&specs).expect("specs dir");
        Self { temp, specs }
    }

    fn write(&self, name: &str, body: &str) {
        let path = self.specs.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("spec parent dir");
        }
        fs::write(&path, body).expect("write spec document");
    }

    fn tasks(&self, body: &str) -> String {
        let path = self.temp.join("tasks.tsv");
        fs::write(&path, body).expect("write task records");
        path.to_string_lossy().into_owned()
    }

    /// `args[0]` is the subcommand path; the spec directory is the default source.
    fn gate(&self, extra: &[&str]) -> Output {
        self.gate_sources(&[&self.specs.to_string_lossy()], extra)
    }

    fn gate_sources(&self, sources: &[&str], extra: &[&str]) -> Output {
        let mut argv: Vec<String> = vec!["dispatch".to_string(), "authority".to_string()];
        for source in sources {
            argv.push("--spec-dir".to_string());
            argv.push((*source).to_string());
        }
        argv.extend(extra.iter().map(|arg| arg.to_string()));
        run(&argv)
    }
}

fn run(argv: &[String]) -> Output {
    let binary = env!("CARGO_BIN_EXE_autospec");
    std::process::Command::new(binary)
        .args(argv)
        .output()
        .expect("invoke autospec")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

/// Both the report and the one-line status the wrapper branches on.
fn combined(output: &Output) -> String {
    format!("{}{}", stdout(output), stderr(output))
}

/// A current spec set with a decision record dispatches, and says which
/// authority it dispatched under.
#[test]
fn current_authority_allows_dispatch() {
    let h = Harness::new("dispatch-authority-current");
    h.write("v2.md", V2_PROGRAM);
    let output = h.gate(&[]);
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    let out = stdout(&output);
    assert!(out.contains("ALLOWED"), "{out}");
    assert!(out.contains("inferweave/v2"), "{out}");
    assert!(out.contains("current"), "{out}");
}

/// A superseded spec set stops the dispatch and names its successor.
#[test]
fn superseded_spec_set_refuses_dispatch() {
    let h = Harness::new("dispatch-authority-superseded");
    h.write("legacy.md", SUPERSEDED_CHARTER);
    let output = h.gate(&[]);
    assert_eq!(output.status.code(), Some(1), "{}", combined(&output));
    let text = combined(&output);
    assert!(text.contains("SPEC_SUPERSEDED"), "{text}");
    assert!(text.contains("inferweave/mono"), "{text}");
    assert!(text.contains("REFUSED"), "{text}");
}

/// No currency marker anywhere is a refusal, not a default of "current".
#[test]
fn currency_less_spec_set_refuses_dispatch() {
    let h = Harness::new("dispatch-authority-no-marker");
    h.write("charter.md", NO_CURRENCY);
    let output = h.gate(&[]);
    assert_eq!(output.status.code(), Some(1), "{}", combined(&output));
    let text = combined(&output);
    assert!(text.contains("CURRENCY_MISSING"), "{text}");
    assert!(text.contains("charter.md"), "{text}");
}

/// Two documents claiming the same component is a hard error for a human.
#[test]
fn conflicting_claims_refuse_dispatch_and_name_both_authorities() {
    let h = Harness::new("dispatch-authority-conflict");
    h.write("v2.md", V2_PROGRAM);
    h.write("mono.md", CLEAN_SLATE);
    let output = h.gate(&[]);
    assert_eq!(output.status.code(), Some(1), "{}", combined(&output));
    let text = combined(&output);
    assert!(text.contains("AUTHORITY_CONFLICT"), "{text}");
    assert!(text.contains("gateway"), "{text}");
    assert!(text.contains("inferweave/v2"), "{text}");
    assert!(text.contains("inferweave/mono"), "{text}");
}

/// A component only one current set claims dispatches even while another part
/// of the problem space is disputed.
#[test]
fn component_scope_narrows_the_conflict() {
    let h = Harness::new("dispatch-authority-scope");
    h.write("v2.md", V2_PROGRAM);
    h.write("legacy.md", SUPERSEDED_CHARTER);
    let output = h.gate(&["--component", "gateway"]);
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    assert!(
        stdout(&output).contains("inferweave/v2"),
        "{}",
        stdout(&output)
    );
}

/// Volume is reported per authority, and merges that derive from no determined
/// authority produce a warning rather than a clean total.
#[test]
fn throughput_is_reported_by_authority() {
    let h = Harness::new("dispatch-authority-throughput");
    h.write("v2.md", V2_PROGRAM);
    let tasks =
        h.tasks("1\tinferweave/v2\tmerged\n2\tinferweave/v2\tmerged\n3\tundetermined\tmerged\n");
    let output = h.gate(&["--tasks", &tasks]);
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    let out = stdout(&output);
    assert!(out.contains("THROUGHPUT BY SPEC AUTHORITY"), "{out}");
    assert!(out.contains("WARNING"), "{out}");
    assert!(out.contains("no determined spec authority"), "{out}");
}

/// A task row whose authority column is blank is the undetermined bucket, not a
/// row with an empty name: the table and its warning must both name it.
#[test]
fn blank_task_authority_reports_under_undetermined() {
    let h = Harness::new("dispatch-authority-blank-authority");
    h.write("v2.md", V2_PROGRAM);
    let tasks = h.tasks("1\tinferweave/v2\tmerged\n2\t\tmerged\n");
    let output = h.gate(&["--tasks", &tasks]);
    let out = stdout(&output);
    assert!(
        out.contains("undetermined"),
        "blank column did not bucket as undetermined:\n{out}"
    );
    assert!(
        !out.contains("spec set  whose"),
        "warning rendered a blank spec set name:\n{out}"
    );
}

/// A malformed record is reported instead of being dropped: a dropped row is a
/// silently wrong merge count.
#[test]
fn malformed_task_record_is_reported() {
    let h = Harness::new("dispatch-authority-malformed");
    h.write("v2.md", V2_PROGRAM);
    let tasks = h.tasks("1\tinferweave/v2\tmerged\n2 inferweave/v2\n");
    let output = h.gate(&["--tasks", &tasks]);
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    let out = stdout(&output);
    assert!(out.contains("MALFORMED"), "{out}");
    assert!(out.contains("line 2"), "{out}");
}

/// The JSON payload carries the verdict and its codes for a wrapper to read.
#[test]
fn json_reports_the_verdict_and_codes() {
    let h = Harness::new("dispatch-authority-json");
    h.write("v2.md", V2_PROGRAM);
    h.write("mono.md", CLEAN_SLATE);
    let output = h.gate(&["--json"]);
    assert_eq!(output.status.code(), Some(1), "{}", stdout(&output));
    let value: serde_json::Value =
        serde_json::from_str(&stdout(&output)).expect("verdict JSON parses");
    assert_eq!(value["verdict"]["allowed"], serde_json::Value::Bool(false));
    let codes = value["verdict"]["blocking"]
        .as_array()
        .expect("blocking findings")
        .iter()
        .map(|finding| finding["code"].as_str().unwrap_or_default().to_string())
        .collect::<Vec<_>>();
    assert!(
        codes.contains(&"AUTHORITY_CONFLICT".to_string()),
        "{codes:?}"
    );
    assert_eq!(value["conflicts"].as_array().map(Vec::len), Some(2));
}

/// No source that exists is unusable input, not a clean bill of health: an
/// empty spec set would read as "nothing is stale".
#[test]
fn missing_spec_source_is_an_input_error() {
    let h = Harness::new("dispatch-authority-missing");
    let absent = h.temp.join("nowhere");
    let output = h.gate_sources(&[&absent.to_string_lossy()], &[]);
    assert_eq!(output.status.code(), Some(2), "{}", combined(&output));
    assert!(
        stderr(&output).contains("does not exist"),
        "{}",
        stderr(&output)
    );
}

/// The subcommand is discoverable from `dispatch --help`, which is the only
/// place an operator looks once a dispatch refuses.
#[test]
fn dispatch_help_lists_the_authority_gate() {
    let output = run(&["dispatch".to_string(), "--help".to_string()]);
    assert_eq!(output.status.code(), Some(0));
    let out = stdout(&output);
    assert!(out.contains("authority"), "{out}");
    assert!(out.contains("SUBCOMMANDS:"), "{out}");
}

/// The gate's own help names its flags, so the exit codes are documented at
/// the point of use.
#[test]
fn authority_help_documents_its_flags() {
    let output = run(&[
        "dispatch".to_string(),
        "authority".to_string(),
        "--help".to_string(),
    ]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let out = stdout(&output);
    for flag in [
        "--spec-dir",
        "--spec-file",
        "--component",
        "--tasks",
        "--json",
    ] {
        assert!(out.contains(flag), "missing {flag} in:\n{out}");
    }
    assert!(out.contains("SUBCOMMANDS:"), "{out}");
}
