//! `autospec dispatch stage|freshness` — the staged-spec gate (issue #3864).
//!
//! Staging is done by a host that can reach GitHub — usually the merge host,
//! because the cluster's `gh` is unauthenticated — and the cluster dispatches
//! from the staged file. That makes the staged file the single point where a
//! stale read silently becomes wrong work: the queue lists issue 50, the staged
//! file holds the discussion as of this morning, and the maintainer's
//! clarification from ten minutes ago is in neither.
//!
//! So the file carries its own revision — the source issue's `updatedAt` and the
//! moment it was staged — and dispatch asks `freshness` before starting. The
//! verdict is three-way and the third arm matters most: when the live revision
//! cannot be read (no `gh`, no token, no network) the dispatch is *refused*, not
//! waved through. A quiet refusal names the issue and the last time it was
//! staged, because the operator's next question is exactly that.
//!
//! Staging also records the host it was staged on — container runtime, database,
//! registry reachability — so a run that starts without a runtime it assumed can
//! be explained from the spec instead of from the node's history.
//!
//! Staging also names the gate set the patch will be graded against, when the
//! caller knows it: each `--gate NAME=COMMAND` flag becomes an acceptance
//! criterion in the staged spec (issue #3925). The grade then enforces exactly
//! the commands the spec carries — a run graded against a weaker set is not a
//! run this spec asked for — and a gate that was never run is named in the
//! verdict instead of counted as clean.
//!
//! Input problems (unreadable JSON, an issue payload with no `updatedAt`, a
//! malformed `--repo`) are the staging host's fault and exit 2 as a diagnostic.
//! Verdicts that hold or refuse a dispatch exit 1.

use std::env;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;

use autospec_core::grading::{Gate, GateSet};
use autospec_core::staged_spec::{
    authorize, format_timestamp, issue_endpoint, parse_timestamp, EnvironmentProbe, IssueComment,
    IssueSnapshot, ProbeState, KEY_ABSENT, KEY_CONTAINER_RUNTIME, KEY_DATABASE, KEY_REGISTRY,
};

use super::dispatch::{
    autospec_home, now_epoch, opt_string, validate_issue_number, verdict_exit, write_atomic,
};
use super::{is_json, CommandFailure};

/// Container runtimes the staging host is checked for, in preference order.
const CONTAINER_RUNTIMES: &[&str] = &["apptainer", "singularity", "docker", "podman", "nerdctl"];

/// The tool that reads the live revision when the caller does not supply it.
const GH: &str = "gh";

/// `autospec dispatch stage --issue N [--issue-json F] [--comments-json F] [--out F]
/// [--gate NAME=COMMAND]...`
///
/// Writes `<n>.md`: revision headers, the execution-environment block, the
/// discussion since the last body edit, the verbatim body, and — when `--gate`
/// flags were passed — the gate set the patch is graded against (#3925).
pub fn stage(args: &[String]) -> Result<(), CommandFailure> {
    let issue = issue_arg(args, "stage")?;
    let out = staged_path(args, "--out", issue)?;
    let mut snapshot = snapshot_from_args(args, issue)?;
    snapshot.gates = parse_gates(opt_strings(args, "--gate")?)?;
    let environment = probe_environment(args)?;
    let staged_at = match opt_string(args, "--staged-at")? {
        Some(raw) => parse_timestamp(&raw).ok_or_else(|| {
            CommandFailure::diagnostic(format!(
                "dispatch stage: --staged-at {raw:?} is neither epoch seconds nor RFC 3339"
            ))
        })?,
        None => now_epoch()?,
    };

    let included = snapshot.comments_since_body_edit().len();
    let text = snapshot.stage(&environment, staged_at);
    let parent = out
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    fs::create_dir_all(&parent).map_err(|error| {
        CommandFailure::diagnostic(format!(
            "dispatch stage: cannot create {}: {error}",
            parent.display()
        ))
    })?;
    write_atomic(&out, &text)?;

    if is_json(args) {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "ok": true,
                "issue": issue,
                "path": out.display().to_string(),
                "staged_at": staged_at,
                "source_updated_at": snapshot.source_updated_at,
                "comments_included": included,
                "comments_total": snapshot.comments.len(),
                "gates": snapshot.gates.len(),
                "bytes": text.len(),
            }))
            .map_err(|error| CommandFailure::diagnostic(format!("serialise: {error}")))?
        );
    } else {
        println!(
            "STAGED issue {}: {} of {} comment(s), source updatedAt {} -> {} ({} bytes)",
            issue,
            included,
            snapshot.comments.len(),
            format_timestamp(snapshot.source_updated_at),
            out.display(),
            text.len()
        );
    }
    Ok(())
}

/// `autospec dispatch freshness --issue N [--staged F] [--live-updated-at T]`
///
/// Exit 0 dispatches; exit 1 holds — either the staged spec is behind the live
/// issue (re-stage) or the live revision cannot be verified (refuse).
pub fn freshness(args: &[String]) -> Result<(), CommandFailure> {
    let issue = issue_arg(args, "freshness")?;
    let path = staged_path(args, "--staged", issue)?;
    let staged = match fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(error) if error.kind() == ErrorKind::NotFound => None,
        Err(error) => {
            return Err(CommandFailure::diagnostic(format!(
                "dispatch freshness: cannot read {}: {error}",
                path.display()
            )))
        }
    };

    let live = live_updated_at(args, issue)?;
    let verdict = authorize(staged.as_deref(), live.value);
    let rendered = path.display().to_string();

    if is_json(args) {
        println!("{}", verdict.to_json(issue, &rendered));
    } else {
        println!("{}", verdict.line(issue, &rendered));
        if verdict.needs_restage() {
            println!(
                "re-stage with: autospec dispatch stage --issue {issue} --out {}",
                path.display()
            );
        }
    }
    // The reason a live read failed belongs on stderr: stdout stays one verdict
    // line for wrappers that parse it.
    if let Some(detail) = live.detail {
        eprintln!("dispatch freshness: live revision unknown — {detail}");
    }
    // The verdict line above already names the issue and the last staging time,
    // so the held status carries no message and nothing prints twice.
    verdict_exit(verdict.held())
}

/// The live revision to compare against: an explicit value, an explicit JSON
/// payload, or a read through `gh`. Only the first two can be a caller error;
/// every failure of the third becomes "unknown, because", which the core turns
/// into a refusal.
struct LiveRead {
    value: Option<u64>,
    detail: Option<String>,
}

impl LiveRead {
    fn known(value: u64) -> Self {
        Self {
            value: Some(value),
            detail: None,
        }
    }

    fn unknown(detail: impl Into<String>) -> Self {
        Self {
            value: None,
            detail: Some(detail.into()),
        }
    }
}

fn live_updated_at(args: &[String], issue: u64) -> Result<LiveRead, CommandFailure> {
    if let Some(raw) = opt_string(args, "--live-updated-at")? {
        let parsed = parse_timestamp(&raw).ok_or_else(|| {
            CommandFailure::diagnostic(format!(
                "dispatch freshness: --live-updated-at {raw:?} is neither epoch seconds nor RFC 3339"
            ))
        })?;
        return Ok(LiveRead::known(parsed));
    }
    if let Some(path) = opt_string(args, "--live-json")? {
        let payload = read_json(&path, "--live-json")?;
        let updated = json_timestamp(&payload, "updated_at").ok_or_else(|| {
            CommandFailure::diagnostic(format!(
                "dispatch freshness: {path} carries no updated_at; the live revision must be \
                 read, not assumed"
            ))
        })?;
        return Ok(LiveRead::known(updated));
    }
    let Some(repo) = source_repo(args)? else {
        return Ok(LiveRead::unknown(
            "no repository to query: pass --repo owner/name, set AUTOSPEC_REPO, or \
             pass --live-updated-at",
        ));
    };
    Ok(read_updated_at_from_github(&repo, issue))
}

/// The repository to read live: `--repo` when given, else a non-empty
/// `$AUTOSPEC_REPO`. Shared by staging and freshness so both resolve the
/// source the same way.
fn source_repo(args: &[String]) -> Result<Option<String>, CommandFailure> {
    if let Some(repo) = opt_string(args, "--repo")? {
        return Ok(Some(repo));
    }
    Ok(env::var("AUTOSPEC_REPO")
        .ok()
        .filter(|repo| !repo.trim().is_empty()))
}

/// Read `updatedAt` live from GitHub. Any failure — no `gh`, no token, a
/// rate-limited or unparseable response — is reported as unknown with the
/// trimmed tool output, never as "unchanged".
fn read_updated_at_from_github(repo: &str, issue: u64) -> LiveRead {
    let endpoint = match issue_endpoint(repo, issue) {
        Some(endpoint) => endpoint,
        None => {
            return LiveRead::unknown(format!(
                "repository {repo:?} is not a plain owner/name pair"
            ))
        }
    };
    match gh_api_text(&["--jq", ".updated_at"], &endpoint) {
        Err(detail) => LiveRead::unknown(detail),
        Ok(text) => {
            let text = text.trim().to_string();
            match parse_timestamp(&text) {
                Some(parsed) => LiveRead::known(parsed),
                None => LiveRead::unknown(format!(
                    "{GH} returned an updatedAt that does not parse: {:?}",
                    truncate(&text, 80)
                )),
            }
        }
    }
}

/// `gh api --method GET <endpoint>` with stdout as text. Both callers report the
/// same failure in their own vocabulary: staging turns it into an exit-2
/// diagnostic, freshness into an unknown live revision.
fn gh_api_text(extra: &[&str], endpoint: &str) -> Result<String, String> {
    let mut command = Command::new(GH);
    command.args(["api", "--method", "GET"]).arg(endpoint);
    command.args(extra);
    let output = command
        .output()
        .map_err(|error| format!("{GH} is not runnable here: {error}"))?;
    if !output.status.success() {
        let code = output
            .status
            .code()
            .map(|code| code.to_string())
            .unwrap_or_else(|| "by signal".to_string());
        return Err(format!(
            "{GH} api {endpoint} exited {code}: {}",
            first_line(&String::from_utf8_lossy(&output.stderr))
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// The issue number, validated the way every other dispatch subcommand
/// validates it: it names a file and a GitHub API path segment.
fn issue_arg(args: &[String], sub: &str) -> Result<u64, CommandFailure> {
    let raw = opt_string(args, "--issue")?.ok_or_else(|| {
        CommandFailure::diagnostic(format!(
            "dispatch {sub}: --issue <n> is required (a staged spec is identified by its issue)"
        ))
    })?;
    validate_issue_number(&raw)
}

/// Where the staged spec for an issue lives: the explicit flag when given,
/// otherwise the default per-issue path under the dispatch home.
fn staged_path(args: &[String], flag: &str, issue: u64) -> Result<PathBuf, CommandFailure> {
    if let Some(raw) = opt_string(args, flag)? {
        return Ok(PathBuf::from(raw));
    }
    Ok(autospec_home()?
        .join("dispatch")
        .join("specs")
        .join(format!("{issue}.md")))
}

/// Assemble the issue snapshot, from whichever source the caller has: a
/// recorded `gh api` payload, a live read of the repository (the merge host),
/// or plain flags for a wrapper that already holds the body in a file.
fn snapshot_from_args(args: &[String], issue: u64) -> Result<IssueSnapshot, CommandFailure> {
    // A wrapper that passes the body or the revision itself has already read
    // the issue; `$AUTOSPEC_REPO` alone must not turn that into a network read.
    let declared = opt_string(args, "--body-file")?.is_some()
        || opt_string(args, "--source-updated-at")?.is_some();
    let fetched_repo = if declared { None } else { source_repo(args)? };

    let mut snapshot = match opt_string(args, "--issue-json")? {
        Some(path) => {
            let payload = read_json(&path, "--issue-json")?;
            snapshot_from_payload(&payload, issue, &path)?
        }
        None => match fetched_repo {
            Some(repo) => {
                snapshot_from_github(&repo, issue, opt_string(args, "--comments-json")?.is_none())?
            }
            None => {
                let body = match opt_string(args, "--body-file")? {
                    Some(path) => fs::read_to_string(&path).map_err(|error| {
                        CommandFailure::diagnostic(format!(
                            "dispatch stage: cannot read --body-file {path}: {error}"
                        ))
                    })?,
                    None => String::new(),
                };
                IssueSnapshot {
                    number: issue,
                    title: opt_string(args, "--title")?.unwrap_or_default(),
                    body,
                    source_updated_at: timestamp_flag(args, "--source-updated-at")?.ok_or_else(
                        || {
                            CommandFailure::diagnostic(
                                "dispatch stage: --issue-json, --repo owner/name, or \
                         --source-updated-at is required; a spec staged without the source \
                         revision can never be freshness-checked",
                            )
                        },
                    )?,
                    body_updated_at: timestamp_flag(args, "--body-updated-at")?,
                    comments: Vec::new(),
                    // The caller's `--gate` flags are merged in by `stage`.
                    gates: Vec::new(),
                }
            }
        },
    };

    // A separate comments payload is the usual shape — `gh api .../comments` is
    // its own endpoint — and is merged with anything the issue payload embedded.
    if let Some(path) = opt_string(args, "--comments-json")? {
        let payload = read_json(&path, "--comments-json")?;
        snapshot
            .comments
            .extend(comments_from_value(&payload, &path)?);
    }
    Ok(snapshot)
}

/// Build the snapshot from a payload shaped like `gh api repos/OWNER/REPO/issues/N`.
/// The source label names the payload in diagnostics so a recorded file and a
/// live read blame the right thing.
fn snapshot_from_payload(
    payload: &serde_json::Value,
    issue: u64,
    source: &str,
) -> Result<IssueSnapshot, CommandFailure> {
    if let Some(number) = payload.get("number").and_then(serde_json::Value::as_u64) {
        if number != issue {
            return Err(CommandFailure::diagnostic(format!(
                "dispatch stage: --issue {issue} but {source} declares number {number}; the \
                 staged spec would be filed under the wrong issue"
            )));
        }
    }
    let source_updated_at = json_timestamp(payload, "updated_at").ok_or_else(|| {
        CommandFailure::diagnostic(format!(
            "dispatch stage: {source} carries no updated_at; a spec staged without the source \
             revision can never be freshness-checked"
        ))
    })?;
    Ok(IssueSnapshot {
        number: issue,
        title: payload
            .get("title")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string(),
        body: payload
            .get("body")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string(),
        source_updated_at,
        body_updated_at: json_timestamp(payload, "body_updated_at"),
        comments: comments_from_value(payload, source)?,
        // The caller's `--gate` flags are merged in by `stage`.
        gates: Vec::new(),
    })
}

/// Read the issue and its discussion from GitHub. This is the merge host's
/// path, and every failure is a staging diagnostic (exit 2) rather than a
/// partially staged spec: a body staged without its discussion is precisely
/// the gap this command exists to close.
fn snapshot_from_github(
    repo: &str,
    issue: u64,
    fetch_comments: bool,
) -> Result<IssueSnapshot, CommandFailure> {
    let endpoint = issue_endpoint(repo, issue).ok_or_else(|| {
        CommandFailure::diagnostic(format!(
            "dispatch stage: repository {repo:?} is not a plain owner/name pair"
        ))
    })?;
    let source = format!("{GH} api {endpoint}");
    let payload = gh_api_json(&endpoint)?;
    let mut snapshot = snapshot_from_payload(&payload, issue, &source)?;
    if !fetch_comments {
        // The caller supplied `--comments-json`; that file is merged by the caller.
        return Ok(snapshot);
    }
    let comments_endpoint = format!("{endpoint}/comments");
    let comments = gh_api_json(&comments_endpoint)?;
    snapshot
        .comments
        .extend(comments_from_value(&comments, &comments_endpoint)?);
    Ok(snapshot)
}

/// `gh api <endpoint>` parsed as JSON, with the tool's own output in the
/// failure text. The staging host is the one with the token, so a failure here
/// is that host's fault and exits 2.
fn gh_api_json(endpoint: &str) -> Result<serde_json::Value, CommandFailure> {
    let text = gh_api_text(&[], endpoint).map_err(CommandFailure::diagnostic)?;
    serde_json::from_str(&text).map_err(|error| {
        CommandFailure::diagnostic(format!("{GH} api {endpoint} is not JSON: {error}"))
    })
}

/// Every value of a repeatable flag, in the order the flags appeared.
/// `opt_string` stops at the first occurrence; `--gate` may appear once per
/// gate in the set the patch is graded against.
fn opt_strings(args: &[String], flag: &str) -> Result<Vec<String>, CommandFailure> {
    let mut values = Vec::new();
    let mut index = 0;
    while let Some(position) = args[index..].iter().position(|arg| arg == flag) {
        let flag_index = index + position;
        let value = args
            .get(flag_index + 1)
            .ok_or_else(|| {
                CommandFailure::diagnostic(format!("dispatch stage: {flag} needs a value"))
            })?
            .clone();
        values.push(value);
        index = flag_index + 2;
    }
    Ok(values)
}

/// The `--gate NAME=COMMAND` flags, validated as one gate set. The set is data
/// the grade enforces exactly (#3925): a malformed or duplicated entry is a
/// staging-host fault (exit 2), not a silently weaker set.
fn parse_gates(raw: Vec<String>) -> Result<Vec<Gate>, CommandFailure> {
    // No `--gate` flags: the spec names no gates and renders no gate section.
    // (An empty set is a `GateSet` error, because grading against nothing
    // passes everything; staging without gates is a different thing.)
    if raw.is_empty() {
        return Ok(Vec::new());
    }
    let mut gates = Vec::with_capacity(raw.len());
    for value in &raw {
        let (name, command) = value.split_once('=').ok_or_else(|| {
            CommandFailure::diagnostic(format!(
                "dispatch stage: --gate {value:?} is not NAME=COMMAND; the staged spec names \
                 the gate set the patch is graded against"
            ))
        })?;
        gates.push(Gate::new(name.to_string(), command.to_string()));
    }
    GateSet::new(gates)
        .map(|set| set.gates().to_vec())
        .map_err(CommandFailure::diagnostic)
}

/// A timestamp flag that accepts epoch seconds or RFC 3339, distinguishing
/// "absent" from "present but unreadable".
fn timestamp_flag(args: &[String], flag: &str) -> Result<Option<u64>, CommandFailure> {
    let Some(raw) = opt_string(args, flag)? else {
        return Ok(None);
    };
    parse_timestamp(&raw).map(Some).ok_or_else(|| {
        CommandFailure::diagnostic(format!(
            "dispatch stage: {flag} {raw:?} is neither epoch seconds nor RFC 3339"
        ))
    })
}

fn read_json(path: &str, flag: &str) -> Result<serde_json::Value, CommandFailure> {
    let text = fs::read_to_string(path).map_err(|error| {
        CommandFailure::diagnostic(format!("cannot read {flag} {path}: {error}"))
    })?;
    serde_json::from_str(&text)
        .map_err(|error| CommandFailure::diagnostic(format!("{flag} {path} is not JSON: {error}")))
}

/// Pull `comments` out of a payload — an array, or an object wrapping one — and
/// parse each entry. An entry nobody can date is a diagnostic: an undated
/// comment cannot be placed after the last body edit, and silently dropping it
/// is the exact failure this staging step exists to prevent. An entry with no
/// body is skipped; a bare timeline event carries nothing to stage.
fn comments_from_value(
    payload: &serde_json::Value,
    source: &str,
) -> Result<Vec<IssueComment>, CommandFailure> {
    let items: Vec<serde_json::Value> = match payload {
        serde_json::Value::Array(items) => items.clone(),
        serde_json::Value::Object(map) => ["comments", "items", "nodes"]
            .iter()
            .find_map(|key| map.get(*key).and_then(serde_json::Value::as_array))
            .cloned()
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    let mut comments = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let Some(created_at) = json_timestamp(item, "created_at") else {
            return Err(CommandFailure::diagnostic(format!(
                "{source} comments[{index}] has no created_at; a comment that cannot be dated \
                 cannot be placed after the last body edit, so it is not staged"
            )));
        };
        if let Some(body) = item
            .get("body")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
        {
            comments.push(IssueComment {
                author: author_of(item),
                created_at,
                body: body.to_string(),
            });
        }
    }
    Ok(comments)
}

/// The comment author, from the shapes `gh api` and recorded fixtures use.
fn author_of(item: &serde_json::Value) -> String {
    ["user", "author", "login"]
        .iter()
        .find_map(|key| {
            item.get(*key).and_then(|value| {
                value
                    .get("login")
                    .and_then(serde_json::Value::as_str)
                    .or_else(|| value.as_str())
            })
        })
        .unwrap_or("unknown")
        .to_string()
}

/// Read a timestamp field GitHub sends as an RFC 3339 string and that recorded
/// fixtures sometimes send as epoch seconds.
fn json_timestamp(payload: &serde_json::Value, key: &str) -> Option<u64> {
    match payload.get(key)? {
        serde_json::Value::String(text) => parse_timestamp(text),
        serde_json::Value::Number(number) => number.as_u64(),
        _ => None,
    }
}

/// Build the execution-environment block. Explicit flags win; otherwise the
/// staging host is probed. Registry reachability stays `not probed` unless asked
/// for, because a staging step that opens sockets nobody requested is a side
/// effect on the dispatch path.
fn probe_environment(args: &[String]) -> Result<EnvironmentProbe, CommandFailure> {
    let mut probe = EnvironmentProbe::new();
    if args.iter().any(|arg| arg == "--no-probe") {
        let how = "not probed: --no-probe".to_string();
        return Ok(EnvironmentProbe::new()
            .with(
                KEY_CONTAINER_RUNTIME,
                ProbeState::NotProbed,
                Some(how.clone()),
            )
            .with(KEY_DATABASE, ProbeState::NotProbed, Some(how.clone()))
            .with(KEY_REGISTRY, ProbeState::NotProbed, Some(how)));
    }

    match opt_string(args, "--container-runtime")? {
        Some(explicit) => {
            probe = probe.with(
                KEY_CONTAINER_RUNTIME,
                state_from_word(&explicit),
                Some("reported by --container-runtime".to_string()),
            );
        }
        None => match first_runtime() {
            Some((name, path)) => {
                probe = probe.with(
                    KEY_CONTAINER_RUNTIME,
                    ProbeState::Present { value: path },
                    Some(format!("$PATH lookup: {name}")),
                );
            }
            None => {
                probe = probe.with(
                    KEY_CONTAINER_RUNTIME,
                    ProbeState::Absent,
                    Some(format!("no {} on $PATH", CONTAINER_RUNTIMES.join(", "))),
                );
            }
        },
    }

    let missing: Vec<&str> = CONTAINER_RUNTIMES
        .iter()
        .copied()
        .filter(|name| which(name).is_none())
        .collect();
    if !missing.is_empty() {
        probe = probe.with(
            KEY_ABSENT,
            ProbeState::Present {
                value: missing.join(", "),
            },
            Some("$PATH lookup".to_string()),
        );
    }

    match opt_string(args, "--database")? {
        Some(explicit) => {
            probe = probe.with(
                KEY_DATABASE,
                state_from_word(&explicit),
                Some("reported by --database".to_string()),
            );
        }
        None if env::var_os("DATABASE_URL").is_some() => {
            probe = probe.with(
                KEY_DATABASE,
                ProbeState::Present {
                    value: "$DATABASE_URL is set".to_string(),
                },
                Some("$DATABASE_URL".to_string()),
            );
        }
        None => match which("pg_isready") {
            Some(client) => {
                let listening = Command::new(&client)
                    .arg("-q")
                    .output()
                    .map(|output| output.status.success())
                    .unwrap_or(false);
                probe = probe.with(
                    KEY_DATABASE,
                    if listening {
                        ProbeState::Present {
                            value: "pg_isready: accepting connections".to_string(),
                        }
                    } else {
                        ProbeState::Absent
                    },
                    Some(format!("{} -q", client.display())),
                );
            }
            None => {
                probe = probe.with(
                    KEY_DATABASE,
                    ProbeState::NotProbed,
                    Some(
                        "no pg_isready and no $DATABASE_URL; pass --database <uri|absent>"
                            .to_string(),
                    ),
                );
            }
        },
    }

    match opt_string(args, "--registry")? {
        Some(explicit) => {
            probe = probe.with(
                KEY_REGISTRY,
                state_from_word(&explicit),
                Some("reported by --registry".to_string()),
            );
        }
        None => {
            probe = probe.with(
                KEY_REGISTRY,
                ProbeState::NotProbed,
                Some(
                    "reachability is never probed from the staging path; pass --registry <value>"
                        .to_string(),
                ),
            );
        }
    }
    Ok(probe)
}

/// A flag value that names a state instead of a value: `absent`, `not probed`.
fn state_from_word(word: &str) -> ProbeState {
    match word.trim().to_ascii_lowercase().as_str() {
        "absent" | "none" => ProbeState::Absent,
        "not probed" | "not-probed" | "unknown" => ProbeState::NotProbed,
        _ => ProbeState::Present {
            value: word.trim().to_string(),
        },
    }
}

fn first_runtime() -> Option<(String, String)> {
    CONTAINER_RUNTIMES
        .iter()
        .copied()
        .find_map(|name| which(name).map(|path| (name.to_string(), path.display().to_string())))
}

/// The first `name` on `$PATH`. This checks for a regular file rather than the
/// executable bit, which is enough here: the answer only decorates the
/// environment block, and a non-executable runtime is a host fault the run
/// reports for itself.
fn which(name: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    env::split_paths(&path).find_map(|dir| {
        let candidate = dir.join(name);
        fs::metadata(&candidate)
            .map(|meta| meta.is_file())
            .unwrap_or(false)
            .then_some(candidate)
    })
}

fn first_line(text: &str) -> String {
    truncate(text.lines().next().unwrap_or_default(), 160)
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut kept: String = text.chars().take(max).collect();
    kept.push('\u{2026}');
    kept
}
