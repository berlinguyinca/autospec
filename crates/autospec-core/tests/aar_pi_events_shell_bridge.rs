//! Issue #3319: the Rust normalizer and the shell ledger validator agree.
//!
//! `scripts/routing-ledger.sh` reads and writes the one ledger file in
//! production; `aar::pi_events` produces the event rows that go into it. Two
//! languages touching one file drift silently, so these tests hand the
//! normalizer's own output to the shell validator and compare the two
//! vocabularies word for word.

use autospec_core::aar::pi_events::EventKind;
use autospec_core::aar::{normalize_jsonl, to_ledger_lines, SessionIdentity};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const FIXTURE: &str = include_str!("fixtures/pi-events/session.jsonl");

/// A dispatch row in the exact shape `scripts/routing-ledger.sh` requires.
/// The shell validator is stricter than `audit_ledger`: every dispatch key
/// must be present, so the bridge test sends a full row.
const DISPATCH_ROW: &str = r#"{"dispatch_id":"d-3319","ts":"2026-08-21T09:00:00Z","dispatch_kind":"implementer","profile":"qwen38","model":"qwen3.8-27b","harness":"pi","issue":"3319","cell_ctx":"64k","cell_reasoning":"medium","input_tokens":20480,"output_tokens":1024,"cached_tokens":18944,"wall_clock_ms":41200,"retries":0,"escalated":false,"outcome":"merged_clean","reason":"within budget"}"#;

/// A scratch directory holding one ledger, removed when the test ends.
struct TempLedger {
    dir: PathBuf,
    path: PathBuf,
}

impl TempLedger {
    fn new(name: &str, body: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "autospec-3319-bridge-{}-{}",
            std::process::id(),
            name
        ));
        fs::create_dir_all(&dir).expect("temp ledger dir created");
        let path = dir.join("ledger.jsonl");
        fs::write(&path, body).expect("ledger written");
        Self { dir, path }
    }

    /// Runs the ledger script with `--ledger <path>` prepended to `args`.
    fn run(&self, args: &[&str]) -> (i32, String) {
        let script = repo_root().join("scripts").join("routing-ledger.sh");
        let mut argv: Vec<String> = vec![script.to_string_lossy().into_owned()];
        argv.push("--ledger".to_string());
        argv.push(self.path.to_string_lossy().into_owned());
        argv.extend(args.iter().map(|a| (*a).to_string()));
        let output = Command::new("bash")
            .args(&argv)
            .output()
            .expect("bash runs the ledger script");
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        (output.status.code().unwrap_or(-1), text)
    }

    fn validate(&self) -> (i32, String) {
        self.run(&["--validate"])
    }

    fn read(&self) -> String {
        fs::read_to_string(&self.path).expect("ledger readable")
    }
}

impl Drop for TempLedger {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|crates| crates.parent())
        .expect("workspace root above the crate")
        .to_path_buf()
}

/// The fixture replayed through the normalizer, preceded by one dispatch row,
/// which is the mixed file the ledger has to accept.
fn replayed_ledger() -> String {
    let identity = SessionIdentity::new("pi-7f3a", "3319", "implementer");
    let records = normalize_jsonl(FIXTURE, &identity).expect("fixture normalizes");
    let events = to_ledger_lines(&records).expect("rows rendered");
    format!("{}\n{}", DISPATCH_ROW, events)
}

/// Replace the first occurrence of `needle` in the nth line of `ledger`.
fn substitute(ledger: &str, line_index: usize, needle: &str, replacement: &str) -> String {
    ledger
        .lines()
        .enumerate()
        .map(|(index, line)| {
            if index == line_index {
                line.replacen(needle, replacement, 1)
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

/// The shell validator must accept every row the normalizer emits, mixed
/// with a dispatch row, with no findings.
#[test]
fn the_shell_validator_accepts_every_replayed_row() {
    let ledger = TempLedger::new("accept", &replayed_ledger());
    let (code, output) = ledger.validate();
    assert_eq!(
        code, 0,
        "shell validator rejected a normalized ledger: {}",
        output
    );
}

/// A blank identity field is rejected by the shell with the field named, so a
/// row the Rust validator refuses cannot slip through the shell path.
#[test]
fn the_shell_validator_names_the_identity_field_it_rejects() {
    let broken = substitute(
        &replayed_ledger(),
        2,
        r#""agent_role":"implementer""#,
        r#""agent_role":"""#,
    );
    assert!(
        broken.contains(r#""agent_role":"""#),
        "substitution did not hit an event row"
    );
    let ledger = TempLedger::new("blank-role", &broken);
    let (code, output) = ledger.validate();
    assert_ne!(code, 0, "blank agent_role passed the shell validator");
    assert!(
        output.contains("agent_role"),
        "rejection did not name agent_role: {}",
        output
    );
}

/// The two sides share one event vocabulary: a kind the Rust enum does not
/// know is rejected by the shell too, naming the same unknown token.
#[test]
fn the_shell_validator_rejects_a_kind_rust_does_not_know() {
    assert!(
        EventKind::from_wire("quantum_leap").is_none(),
        "quantum_leap must not be a canonical kind"
    );
    let broken = substitute(
        &replayed_ledger(),
        2,
        r#""event":"model_request""#,
        r#""event":"quantum_leap""#,
    );
    let ledger = TempLedger::new("bad-kind", &broken);
    let (code, output) = ledger.validate();
    assert_ne!(code, 0, "unknown event kind passed the shell validator");
    assert!(
        output.contains("quantum_leap"),
        "rejection did not name the unknown kind: {}",
        output
    );
}

/// The canonical kind list in Rust and `ALLOWED_EVENT_KINDS` in the shell
/// must be the same set, in the same words.
#[test]
fn the_canonical_kinds_match_the_shell_vocabulary() {
    let script = fs::read_to_string(repo_root().join("scripts").join("routing-ledger.sh"))
        .expect("ledger script readable");
    let prefix = "ALLOWED_EVENT_KINDS=\"";
    let line = script
        .lines()
        .find(|line| line.starts_with(prefix))
        .expect("ALLOWED_EVENT_KINDS declared in the ledger script");
    let shell_kinds: Vec<&str> = line
        .trim_start_matches(prefix)
        .trim_end_matches('"')
        .split_whitespace()
        .collect();

    let mut rust_kinds: Vec<&'static str> = EventKind::ALL.iter().map(|k| k.as_str()).collect();
    rust_kinds.sort_unstable();
    let mut shell_sorted = shell_kinds.clone();
    shell_sorted.sort_unstable();

    assert_eq!(
        rust_kinds, shell_sorted,
        "Rust EventKind and shell ALLOWED_EVENT_KINDS drifted apart"
    );
    assert_eq!(
        shell_kinds.len(),
        EventKind::ALL.len(),
        "duplicate kind in one list but not the other"
    );
}

/// `--append` must store an event row exactly as given: the dispatch
/// normalizer would otherwise inject 21 telemetry keys into every event.
#[test]
fn the_shell_appends_an_event_row_without_injecting_dispatch_keys() {
    let identity = SessionIdentity::new("pi-7f3a", "3319", "implementer");
    let records = normalize_jsonl(FIXTURE, &identity).expect("fixture normalizes");
    let first_event = to_ledger_lines(&records)
        .expect("rows rendered")
        .lines()
        .next()
        .expect("at least one event row")
        .to_string();

    let ledger = TempLedger::new("append-event", "");
    let (code, output) = ledger.run(&["--append", first_event.as_str()]);
    assert_eq!(code, 0, "--append failed: {}", output);

    let stored = ledger.read();
    assert_eq!(
        stored.lines().count(),
        1,
        "append wrote more than one row: {}",
        stored
    );
    assert_eq!(stored.trim_end(), first_event, "append rewrote the row");
    assert!(
        !stored.contains("dispatch_kind"),
        "dispatch normalization leaked into an event row: {}",
        stored
    );

    let (code, output) = ledger.validate();
    assert_eq!(code, 0, "appended event row does not validate: {}", output);
}
