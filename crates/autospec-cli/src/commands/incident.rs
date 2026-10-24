use autospec_core::incident::{parse_agent_failure_event, IncidentIssueProjection};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs::{self, create_dir_all, remove_file, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::process::Command;
use std::thread::sleep;
use std::time::Duration;

use super::CommandFailure;

const MAX_INCIDENT_EVENT_BYTES: u64 = 64 * 1024;

pub fn run(args: &[String]) -> Result<(), CommandFailure> {
    if args.first().map(String::as_str) != Some("ingest") {
        return Err(CommandFailure::diagnostic(
            "autospec incident requires the ingest subcommand",
        ));
    }
    let options = parse_options(&args[1..])?;
    let event = read_event(&options)?;
    let projection = IncidentIssueProjection::from_event(&event);
    if options.dry_run {
        println!("{}", render_dry_run(&event, &projection));
        return Ok(());
    }

    let _lock = IncidentIssueLock::acquire(&options.repo)?;
    let bindings = IncidentBindingStore::open(&options.repo)?;
    let bound = bindings.find(&event.incident_id, &event.fingerprint)?;
    let existing = bound.or(find_existing_issue(&options.repo, &projection.marker())?);
    let (number, deduplicated) = match existing {
        Some(number) => (number, true),
        None => (create_issue(&options.repo, &projection)?, false),
    };
    bindings.record(&event.incident_id, &event.fingerprint, number)?;
    println!(
        "{}",
        render_result(&event, &projection, number, deduplicated)
    );
    Ok(())
}

fn read_event(
    options: &IncidentOptions,
) -> Result<autospec_core::incident::AgentFailureEvent, CommandFailure> {
    let size = fs::metadata(&options.event_file)
        .map_err(|error| {
            CommandFailure::diagnostic(format!("could not stat incident event: {error}"))
        })?
        .len();
    if size > MAX_INCIDENT_EVENT_BYTES {
        return Err(CommandFailure::diagnostic(
            "incident event exceeds 65536 bytes",
        ));
    }
    let document = fs::read_to_string(&options.event_file).map_err(|error| {
        CommandFailure::diagnostic(format!("could not read incident event: {error}"))
    })?;
    let event = parse_agent_failure_event(&document).map_err(CommandFailure::diagnostic)?;
    if event.repo != options.repo {
        return Err(CommandFailure::diagnostic(format!(
            "incident repository {} does not match --repo {}",
            event.repo, options.repo
        )));
    }
    Ok(event)
}

fn render_dry_run(
    event: &autospec_core::incident::AgentFailureEvent,
    projection: &IncidentIssueProjection,
) -> String {
    serde_json::to_string_pretty(&json!({
        "ok": true,
        "dry_run": true,
        "fingerprint": event.fingerprint,
        "deduplicated": false,
        "issue_title": projection.title,
        "issue_body": projection.body,
        "labels": projection.labels,
        "debugger_dispatch": debugger_json(projection),
    }))
    .expect("incident projection is JSON serializable")
}

fn render_result(
    event: &autospec_core::incident::AgentFailureEvent,
    projection: &IncidentIssueProjection,
    number: u64,
    deduplicated: bool,
) -> String {
    serde_json::to_string(&json!({
        "ok": true,
        "dry_run": false,
        "fingerprint": event.fingerprint,
        "deduplicated": deduplicated,
        "issue": number,
        "debugger_dispatch": debugger_json(projection),
    }))
    .expect("incident result is JSON serializable")
}

fn debugger_json(projection: &IncidentIssueProjection) -> serde_json::Value {
    json!({
        "role": projection.debugger_dispatch.role,
        "dry_run": projection.debugger_dispatch.dry_run,
        "prompt": projection.debugger_dispatch.prompt,
    })
}

#[derive(Debug, Default)]
struct RawIncidentOptions {
    repo: Option<String>,
    event_file: Option<String>,
    dry_run: bool,
}

struct IncidentOptions {
    repo: String,
    event_file: String,
    dry_run: bool,
}

fn parse_options(args: &[String]) -> Result<IncidentOptions, CommandFailure> {
    let mut options = RawIncidentOptions::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--repo" => {
                index += 1;
                options.repo = Some(required_value(args, index, "--repo")?);
            }
            "--event-file" => {
                index += 1;
                options.event_file = Some(required_value(args, index, "--event-file")?);
            }
            "--dry-run" | "--json" => {}
            "--help" | "-h" => {
                print_help();
                return Err(CommandFailure::status("", 0));
            }
            option => {
                return Err(CommandFailure::diagnostic(format!(
                    "unknown incident option: {option}"
                )))
            }
        }
        index += 1;
    }
    Ok(IncidentOptions {
        repo: options
            .repo
            .ok_or_else(|| CommandFailure::diagnostic("--repo is required"))?,
        event_file: options
            .event_file
            .ok_or_else(|| CommandFailure::diagnostic("--event-file is required"))?,
        dry_run: options.dry_run || args.iter().any(|arg| arg == "--dry-run"),
    })
}

fn required_value(args: &[String], index: usize, option: &str) -> Result<String, CommandFailure> {
    args.get(index)
        .filter(|value| !value.starts_with('-'))
        .cloned()
        .ok_or_else(|| CommandFailure::diagnostic(format!("{option} requires a value")))
}

fn print_help() {
    println!("autospec incident ingest --repo OWNER/REPO --event-file PATH [--dry-run] [--json]");
}

fn find_existing_issue(repo: &str, marker: &str) -> Result<Option<u64>, CommandFailure> {
    let endpoint = format!("repos/{repo}/issues?state=all&labels=origin:pi-incident&per_page=100");
    let output = Command::new("gh")
        .args(["api", "--paginate", "--slurp", &endpoint])
        .output()
        .map_err(|error| {
            CommandFailure::transient(format!("could not execute gh issue lookup: {error}"))
        })?;
    if !output.status.success() {
        return Err(CommandFailure::transient(format!(
            "gh issue lookup failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let issues: serde_json::Value = serde_json::from_slice(&output.stdout).map_err(|error| {
        CommandFailure::transient(format!("could not parse gh issue lookup: {error}"))
    })?;
    let pages = issues.as_array().ok_or_else(|| {
        CommandFailure::transient("gh issue lookup returned a non-array response")
    })?;
    Ok(pages
        .iter()
        .flat_map(|page| page.as_array().into_iter().flatten())
        .find_map(|issue| {
            let body = issue.get("body")?.as_str()?;
            body.lines()
                .any(|line| line.trim() == marker)
                .then(|| issue.get("number")?.as_u64())
                .flatten()
        }))
}

fn create_issue(repo: &str, projection: &IncidentIssueProjection) -> Result<u64, CommandFailure> {
    let output = Command::new("gh")
        .args(create_issue_arguments(repo, projection))
        .output()
        .map_err(|error| {
            CommandFailure::transient(format!("could not execute gh issue create: {error}"))
        })?;
    if !output.status.success() {
        return Err(CommandFailure::transient(format!(
            "gh issue create failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<u64>()
        .ok()
        .filter(|number| *number > 0)
        .ok_or_else(|| {
            CommandFailure::transient("gh issue create returned an invalid issue number")
        })
}

fn create_issue_arguments(repo: &str, projection: &IncidentIssueProjection) -> Vec<String> {
    let mut args = vec![
        "api".to_string(),
        "--method".to_string(),
        "POST".to_string(),
        format!("repos/{repo}/issues"),
        "-f".to_string(),
        format!("title={}", projection.title),
        "-f".to_string(),
        format!("body={}", projection.body),
    ];
    for label in &projection.labels {
        args.push("-f".to_string());
        args.push(format!("labels[]={label}"));
    }
    args.extend(["--jq".to_string(), ".number".to_string()]);
    args
}

struct IncidentIssueLock {
    path: std::path::PathBuf,
}

#[derive(Debug, Deserialize, Serialize)]
struct IncidentBinding {
    incident_id: String,
    fingerprint: String,
    issue: u64,
}

struct IncidentBindingStore {
    path: std::path::PathBuf,
    bindings: Vec<IncidentBinding>,
}

impl IncidentBindingStore {
    fn open(repo: &str) -> Result<Self, CommandFailure> {
        let state_root = std::env::var_os("AUTOSPEC_INCIDENT_STATE_DIR")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(|home| std::path::PathBuf::from(home).join(".autospec/incident-bindings"))
            })
            .ok_or_else(|| {
                CommandFailure::transient("cannot determine AutoSpec incident state directory")
            })?;
        create_dir_all(&state_root).map_err(|error| {
            CommandFailure::transient(format!(
                "could not create incident state directory: {error}"
            ))
        })?;
        let mut permissions = fs::metadata(&state_root)
            .map_err(|error| {
                CommandFailure::transient(format!(
                    "could not stat incident state directory: {error}"
                ))
            })?
            .permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            permissions.set_mode(0o700);
            fs::set_permissions(&state_root, permissions).map_err(|error| {
                CommandFailure::transient(format!(
                    "could not protect incident state directory: {error}"
                ))
            })?;
        }
        let path = state_root.join(format!("{}.jsonl", repo.replace('/', "__")));
        let bindings = match File::open(&path) {
            Ok(file) => BufReader::new(file)
                .lines()
                .map(|line| {
                    let line = line.map_err(|error| error.to_string())?;
                    serde_json::from_str::<IncidentBinding>(&line)
                        .map_err(|error| error.to_string())
                })
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| {
                    CommandFailure::transient(format!("invalid incident binding journal: {error}"))
                })?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => {
                return Err(CommandFailure::transient(format!(
                    "could not open incident binding journal: {error}"
                )))
            }
        };
        Ok(Self { path, bindings })
    }

    fn find(&self, incident_id: &str, fingerprint: &str) -> Result<Option<u64>, CommandFailure> {
        Ok(self
            .bindings
            .iter()
            .find(|binding| {
                binding.incident_id == incident_id || binding.fingerprint == fingerprint
            })
            .map(|binding| binding.issue))
    }

    fn record(
        &self,
        incident_id: &str,
        fingerprint: &str,
        issue: u64,
    ) -> Result<(), CommandFailure> {
        if self.bindings.iter().any(|binding| {
            binding.incident_id == incident_id
                && binding.fingerprint == fingerprint
                && binding.issue == issue
        }) {
            return Ok(());
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|error| {
                CommandFailure::transient(format!("could not append incident binding: {error}"))
            })?;
        let line = serde_json::to_string(&IncidentBinding {
            incident_id: incident_id.to_string(),
            fingerprint: fingerprint.to_string(),
            issue,
        })
        .map_err(|error| {
            CommandFailure::transient(format!("could not encode incident binding: {error}"))
        })?;
        writeln!(file, "{line}").map_err(|error| {
            CommandFailure::transient(format!("could not write incident binding: {error}"))
        })?;
        file.sync_data().map_err(|error| {
            CommandFailure::transient(format!("could not sync incident binding: {error}"))
        })?;
        Ok(())
    }
}

impl IncidentIssueLock {
    fn acquire(repo: &str) -> Result<Self, CommandFailure> {
        let safe_repo = repo.replace('/', "__");
        let path = std::env::temp_dir().join(format!("autospec-pi-incident-{safe_repo}.lock"));
        for _ in 0..50 {
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(_) => return Ok(Self { path }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    sleep(Duration::from_millis(100));
                }
                Err(error) => {
                    return Err(CommandFailure::transient(format!(
                        "could not acquire incident projection lock: {error}"
                    )))
                }
            }
        }
        Err(CommandFailure::transient(
            "timed out waiting for incident projection lock",
        ))
    }
}

impl Drop for IncidentIssueLock {
    fn drop(&mut self) {
        let _ = remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::create_issue_arguments;
    use autospec_core::incident::{DebuggerDispatch, IncidentIssueProjection};

    #[test]
    fn create_issue_arguments_send_labels_as_repeated_array_fields() {
        let projection = IncidentIssueProjection {
            title: "title".to_string(),
            body: "body".to_string(),
            labels: vec![
                "auto-implement".to_string(),
                "origin:pi-incident".to_string(),
            ],
            debugger_dispatch: DebuggerDispatch {
                role: "debugger".to_string(),
                dry_run: true,
                prompt: "prompt".to_string(),
            },
        };
        let args = create_issue_arguments("owner/repo", &projection);
        assert!(args.contains(&"labels[]=auto-implement".to_string()));
        assert!(args.contains(&"labels[]=origin:pi-incident".to_string()));
        assert!(!args.contains(&"labels=auto-implement,origin:pi-incident".to_string()));
    }

    #[test]
    fn binding_store_matches_incident_id_or_fingerprint() {
        let store = super::IncidentBindingStore {
            path: std::path::PathBuf::from("/tmp/unused-incident-bindings.jsonl"),
            bindings: vec![super::IncidentBinding {
                incident_id: "inc-1".to_string(),
                fingerprint: "deadbeefdeadbeef".to_string(),
                issue: 42,
            }],
        };
        assert_eq!(store.find("inc-1", "other"), Ok(Some(42)));
        assert_eq!(store.find("other", "deadbeefdeadbeef"), Ok(Some(42)));
        assert_eq!(store.find("other", "other"), Ok(None));
    }
}
