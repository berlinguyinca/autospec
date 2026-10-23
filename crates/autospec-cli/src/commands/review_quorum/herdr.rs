//! Producer and consumer for the Herdr side of the head-bound review quorum (T06).
//!
//! * `publish` turns one independent-review verdict into the two lineage envelopes Herdr
//!   ingests (`review:*` node + `reviewed` edge to the PR node) and POSTs them with the
//!   REVIEWER's OWN ingest key. Off unless `herdr.enabled: true`.
//! * `request` asks Herdr whether the PR still needs review at its current head and, if so,
//!   runs the configured dispatch command exactly once per (PR, head).
//!
//! Configuration is YAML only (no environment variables). The file holds a key, so it must
//! not be group/other accessible. The key reaches `curl` through a 0600 header file, never argv.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;

use autospec_core::autonomous::review_quorum::{
    canonical_commit, canonical_target, hmac_sha256_hex, receipt_binding, sha256_hex,
};
use nix::fcntl::{Flock, FlockArg};
use serde_json::{json, Value};

#[derive(Debug)]
pub struct HerdrConfig {
    pub enabled: bool,
    pub url: String,
    pub ingest_key: String,
    pub curl: String,
    pub dispatch_command: Option<String>,
    pub dispatch_timeout_seconds: u64,
}

/// Remove a trailing ` # comment` from a scalar. A quoted scalar ends at its closing quote,
/// so a `#` inside quotes is data; anything after it must be blank or a comment.
fn scalar_value(raw: &str, line_no: usize) -> Result<String, String> {
    let raw = raw.trim();
    for quote in ['"', '\''] {
        if let Some(rest) = raw.strip_prefix(quote) {
            let end = rest
                .find(quote)
                .ok_or_else(|| format!("config line {line_no}: unterminated quote"))?;
            let tail = rest[end + 1..].trim();
            if !tail.is_empty() && !tail.starts_with('#') {
                return Err(format!("config line {line_no}: text after a quoted value"));
            }
            return Ok(rest[..end].to_string());
        }
    }
    let cut = raw
        .match_indices('#')
        .find(|(i, _)| *i == 0 || raw[..*i].ends_with([' ', '\t']))
        .map_or(raw.len(), |(i, _)| i);
    Ok(raw[..cut].trim().to_string())
}

/// Strict reader for the one block we use: a top-level `herdr:` mapping of scalar keys.
///
/// Accepted: comments (whole-line and trailing), blank lines, a leading `---`, and any OTHER
/// top-level blocks (skipped unread, so the file can be shared with other tools' settings).
/// Rejected inside `herdr:`: tabs, nested values, inconsistent indentation, duplicate keys,
/// unknown keys, and a second `herdr:` block. Lists, anchors and multi-line scalars are not
/// supported anywhere in `herdr:` and fail as unknown/odd keys rather than being half-read.
pub fn parse_config(text: &str) -> Result<HerdrConfig, String> {
    let mut values: BTreeMap<String, String> = BTreeMap::new();
    let mut in_block = false;
    let mut seen_block = false;
    let mut indent_of_keys: Option<usize> = None;
    for (n, line) in text.lines().enumerate() {
        let n = n + 1;
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed == "---" || trimmed == "..." {
            continue;
        }
        let indent = line.len() - line.trim_start_matches(' ').len();
        if line[indent..].starts_with('\t') || (in_block && line.contains('\t') && indent > 0) {
            return Err(format!("config line {n}: tabs are not allowed"));
        }
        if indent == 0 {
            let (name, rest) = trimmed.split_once(':').unwrap_or((trimmed, "x"));
            in_block = name.trim() == "herdr";
            if in_block {
                if !scalar_value(rest, n)?.is_empty() {
                    return Err(format!("config line {n}: `herdr:` must open a mapping"));
                }
                if std::mem::replace(&mut seen_block, true) {
                    return Err(format!("config line {n}: second `herdr:` block"));
                }
            }
            continue;
        }
        if !in_block {
            continue; // someone else's block
        }
        match indent_of_keys {
            None => indent_of_keys = Some(indent),
            Some(i) if i != indent => {
                return Err(format!(
                    "config line {n}: nested or misaligned value inside `herdr:`"
                ));
            }
            Some(_) => {}
        }
        let (key, value) = trimmed
            .split_once(':')
            .ok_or_else(|| format!("config line {n}: expected `key: value`"))?;
        if values
            .insert(key.trim().to_string(), scalar_value(value, n)?)
            .is_some()
        {
            return Err(format!("config line {n}: duplicate key"));
        }
    }
    let known = [
        "enabled",
        "url",
        "ingest_key",
        "curl",
        "dispatch_command",
        "dispatch_timeout_seconds",
    ];
    if let Some(unknown) = values.keys().find(|k| !known.contains(&k.as_str())) {
        return Err(format!("config: unknown key `{unknown}`"));
    }
    let enabled = match values.get("enabled").map(String::as_str) {
        None | Some("false") => false,
        Some("true") => true,
        Some(_) => return Err("config: herdr.enabled must be true or false".to_string()),
    };
    let dispatch_timeout_seconds = match values.get("dispatch_timeout_seconds") {
        None => 900,
        Some(v) => v
            .parse::<u64>()
            .ok()
            .filter(|s| (1..=86_400).contains(s))
            .ok_or("config: herdr.dispatch_timeout_seconds must be 1..86400")?,
    };
    Ok(HerdrConfig {
        enabled,
        url: values
            .get("url")
            .cloned()
            .unwrap_or_default()
            .trim_end_matches('/')
            .to_string(),
        ingest_key: values.get("ingest_key").cloned().unwrap_or_default(),
        curl: values
            .get("curl")
            .cloned()
            .unwrap_or_else(|| "curl".to_string()),
        dispatch_command: values
            .get("dispatch_command")
            .cloned()
            .filter(|c| !c.is_empty()),
        dispatch_timeout_seconds,
    })
}

pub fn load_config(path: &Path) -> Result<HerdrConfig, String> {
    let meta = fs::metadata(path).map_err(|e| format!("read herdr config: {e}"))?;
    if meta.permissions().mode() & 0o077 != 0 {
        return Err("herdr config holds an ingest key and must be chmod 600".to_string());
    }
    parse_config(&fs::read_to_string(path).map_err(|e| format!("read herdr config: {e}"))?)
}

/// Parse `herdr.url` with a real URL parser and return the NORMALISED form that is used for
/// every request (so curl and this validator cannot read a different host out of one string).
/// https always; plain http only for the exact loopback hosts `127.0.0.1`, `::1`, `localhost`.
/// Credentials, query and fragment are refused.
pub fn validate_url(raw: &str) -> Result<String, String> {
    let url = url::Url::parse(raw.trim()).map_err(|e| format!("herdr.url is not a URL: {e}"))?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err("herdr.url must not carry credentials".to_string());
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err("herdr.url must not carry a query or fragment".to_string());
    }
    let loopback_http = url.scheme() == "http"
        && match url.host() {
            Some(url::Host::Ipv4(ip)) => ip == Ipv4Addr::LOCALHOST,
            Some(url::Host::Ipv6(ip)) => ip == Ipv6Addr::LOCALHOST,
            Some(url::Host::Domain(name)) => name == "localhost",
            None => false,
        };
    let https = url.scheme() == "https" && url.host().is_some();
    if !(https || loopback_http) {
        return Err("herdr.url must be https (http only for 127.0.0.1, ::1 or localhost)".into());
    }
    Ok(url.as_str().trim_end_matches('/').to_string())
}

fn require_ready(cfg: &HerdrConfig) -> Result<String, String> {
    let base = validate_url(&cfg.url)?;
    if cfg.ingest_key.is_empty() {
        return Err("herdr.ingest_key is required".to_string());
    }
    Ok(base)
}

/// A PR node id is interpolated into a URL path and handed to `dispatch_command`, so it is
/// held to a strict shape: `pr:` + printable ASCII, no leading `-` after the prefix, bounded.
pub fn validate_pr_node(raw: &str) -> Result<String, String> {
    let node = canonical_target(raw).ok_or("pr node must be printable ASCII")?;
    let body = node.strip_prefix("pr:").unwrap_or("");
    if body.is_empty()
        || node.len() > 400
        || body.starts_with('-')
        || !body.contains('#')
        || body.contains(['?', '\\', '"', '\'', '`', '$', ';', '&', '|', '<', '>', '*'])
    {
        return Err("pr node must look like pr:<host>/<owner>/<repo>#<n>".to_string());
    }
    Ok(node)
}

/// The reviewer id Herdr derives from an API key (`key:` + first 12 hex of its sha256).
pub fn principal_id(ingest_key: &str) -> String {
    format!("key:{}", &sha256_hex(ingest_key.as_bytes())[..12])
}

pub struct Publication {
    pub bodies: Vec<Value>,
    pub review_node: String,
}

struct Identity {
    pr_node: String,
    commit: String,
    verdict: String,
    digest: String,
    review_node: String,
    source: String,
}

fn identify(
    pr_node: &str,
    commit: &str,
    verdict: &str,
    verdict_bytes: &[u8],
    reviewer: &str,
) -> Result<Identity, String> {
    let commit =
        canonical_commit(commit).ok_or("commit must be a full 40 or 64 hex character head")?;
    let verdict = verdict.trim().to_ascii_lowercase();
    if verdict != "lgtm" && verdict != "blocked" {
        return Err("verdict must be lgtm or blocked".to_string());
    }
    let pr_node = validate_pr_node(pr_node)?;
    let digest = sha256_hex(verdict_bytes);
    let id = sha256_hex(format!("{pr_node}\0{commit}\0{reviewer}\0{verdict}\0{digest}").as_bytes());
    Ok(Identity {
        review_node: format!("review:{}", &id[..24]),
        source: format!("ar-{}", &id[..12]),
        pr_node,
        commit,
        verdict,
        digest,
    })
}

fn envelope(i: &Identity, seq: u64, event_type: &str, payload: Value) -> Value {
    json!({"schema_version": 2, "event_id": format!("{}-{seq}", i.source), "source": i.source,
           "source_sequence": seq, "entity_type": "lineage", "entity_id": i.review_node,
           "event_type": event_type, "payload": payload, "visibility_scope": "org"})
}

/// Build the two ingest bodies for one verdict. Deterministic: re-publishing the same verdict
/// yields the same event ids, so Herdr answers `duplicate` instead of storing it twice.
pub fn build_publication(
    pr_node: &str,
    commit: &str,
    verdict: &str,
    verdict_bytes: &[u8],
    reviewer: &str,
) -> Result<Publication, String> {
    let i = identify(pr_node, commit, verdict, verdict_bytes, reviewer)?;
    let binding = receipt_binding(&i.pr_node, &i.commit, reviewer, &i.digest);
    Ok(Publication {
        bodies: vec![
            envelope(
                &i,
                0,
                "node.upsert",
                json!({"node_type": "review", "node_id": i.review_node, "label": format!("review of {}", i.commit)}),
            ),
            envelope(
                &i,
                1,
                "edge.add",
                json!({
                "edge_type": "reviewed", "from_id": i.review_node, "to_id": i.pr_node,
                "attrs": {"commit": i.commit, "verdict": i.verdict, "receipt_digest": i.digest, "receipt_binding": binding}}),
            ),
        ],
        review_node: i.review_node,
    })
}

/// The owner-only retraction of ONE earlier verdict: an `edge.remove` on the same
/// `review:* -> pr:*` edge, sent with the same key, at the same commit. Herdr honours a
/// remove only from the edge's owner, so nobody else can retract (or be retracted by) this.
/// It follows the publication on the same source, so it is ordered after the add.
pub fn build_retraction(
    pr_node: &str,
    commit: &str,
    verdict: &str,
    verdict_bytes: &[u8],
    reviewer: &str,
) -> Result<Publication, String> {
    let i = identify(pr_node, commit, verdict, verdict_bytes, reviewer)?;
    Ok(Publication {
        bodies: vec![envelope(
            &i,
            2,
            "edge.remove",
            json!({"edge_type": "reviewed", "from_id": i.review_node, "to_id": i.pr_node,
                   "attrs": {"commit": i.commit}, "reason": "retracted by the reviewer"}),
        )],
        review_node: i.review_node,
    })
}

/// A private (0700) scratch directory for the key-bearing files of ONE curl call. Dropping it
/// blanks and removes everything inside, on every path: success, curl failing to spawn, or an
/// early return. (A SIGKILL cannot run destructors; the directory is then 0700 and holds only
/// the call's own header file.)
struct SecretDir {
    path: PathBuf,
}

impl SecretDir {
    fn create(parent: &Path) -> Result<Self, String> {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        fs::create_dir_all(parent).map_err(|e| format!("create scratch dir: {e}"))?;
        for _ in 0..8 {
            let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let nonce = sha256_hex(
                format!(
                    "{:?}{}{n}",
                    std::time::SystemTime::now(),
                    std::process::id()
                )
                .as_bytes(),
            );
            let path = parent.join(format!("rq-{}", &nonce[..16]));
            match std::os::unix::fs::DirBuilderExt::mode(&mut fs::DirBuilder::new(), 0o700)
                .create(&path)
            {
                Ok(()) => return Ok(Self { path }),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(format!("create private scratch dir: {e}")),
            }
        }
        Err("could not create a private scratch dir".to_string())
    }

    fn file(&self, name: &str, content: &str) -> Result<PathBuf, String> {
        let path = self.path.join(name);
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .map_err(|e| format!("create {name}: {e}"))?;
        file.write_all(content.as_bytes())
            .map_err(|e| format!("write {name}: {e}"))?;
        Ok(path)
    }
}

impl Drop for SecretDir {
    fn drop(&mut self) {
        if let Ok(entries) = fs::read_dir(&self.path) {
            for entry in entries.flatten() {
                let _ = fs::write(entry.path(), "");
                let _ = fs::remove_file(entry.path());
            }
        }
        let _ = fs::remove_dir(&self.path);
    }
}

/// Run curl with the key in a 0600 header file inside a private temp dir.
/// Returns (http status, body).
fn curl(
    cfg: &HerdrConfig,
    scratch: &Path,
    method_post_body: Option<&str>,
    url: &str,
) -> Result<(u16, String), String> {
    let dir = SecretDir::create(scratch)?;
    let headers = dir.file(
        "headers",
        &format!(
            "X-API-Key: {}\nContent-Type: application/json\n",
            cfg.ingest_key
        ),
    )?;
    let mut cmd = Command::new(&cfg.curl);
    cmd.args(["-sS", "--max-time", "20", "-H"])
        .arg(format!("@{}", headers.display()));
    if let Some(body) = method_post_body {
        let p = dir.file("body", body)?;
        cmd.args(["-X", "POST", "--data-binary"])
            .arg(format!("@{}", p.display()));
    }
    cmd.args(["-w", "\n%{http_code}"]).arg(url);
    let out = cmd.output().map_err(|e| format!("run curl: {e}"))?;
    drop(dir);
    if !out.status.success() {
        return Err(format!(
            "curl failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let (body, code) = text.rsplit_once('\n').ok_or("curl produced no status")?;
    Ok((
        code.trim().parse().map_err(|_| "curl status unreadable")?,
        body.to_string(),
    ))
}

pub fn publish(
    cfg: &HerdrConfig,
    scratch: &Path,
    pubn: &Publication,
) -> Result<Vec<Value>, String> {
    let base = require_ready(cfg)?;
    let mut acks = Vec::new();
    for body in &pubn.bodies {
        let (status, text) = curl(
            cfg,
            scratch,
            Some(&body.to_string()),
            &format!("{base}/api/v1/ingest"),
        )?;
        if !(200..300).contains(&status) {
            return Err(format!("herdr ingest answered HTTP {status}"));
        }
        acks.push(serde_json::from_str(&text).unwrap_or(Value::Null));
    }
    Ok(acks)
}

fn encode_segment(text: &str) -> String {
    text.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

pub fn fetch_quorum(cfg: &HerdrConfig, scratch: &Path, pr_node: &str) -> Result<Value, String> {
    let base = require_ready(cfg)?;
    let pr_node = validate_pr_node(pr_node)?;
    let url = format!(
        "{base}/api/v1/graph/nodes/{}/review-quorum",
        encode_segment(&pr_node)
    );
    let (status, text) = curl(cfg, scratch, None, &url)?;
    if status != 200 {
        return Err(format!("herdr review-quorum answered HTTP {status}"));
    }
    serde_json::from_str(&text).map_err(|e| format!("herdr answered invalid JSON: {e}"))
}

// ---------------------------------------------------------------- dispatch dedupe

/// Create (0700) or validate a private directory: not a symlink, owned by us, not writable by
/// group/other. Existing directories are accepted as they are; only new ones are tightened.
pub fn private_dir(path: &Path) -> Result<PathBuf, String> {
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            if !meta.file_type().is_dir() {
                return Err(format!(
                    "{} must be a real directory (not a symlink)",
                    path.display()
                ));
            }
            if meta.uid() != nix::unistd::Uid::current().as_raw() || meta.mode() & 0o022 != 0 {
                return Err(format!(
                    "{} must be owned by the current user and not group/other writable",
                    path.display()
                ));
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            std::os::unix::fs::DirBuilderExt::mode(fs::DirBuilder::new().recursive(true), 0o700)
                .create(path)
                .map_err(|e| format!("create {}: {e}", path.display()))?;
        }
        Err(e) => return Err(format!("{}: {e}", path.display())),
    }
    Ok(path.to_path_buf())
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn pid_alive(pid: u32) -> bool {
    use nix::errno::Errno;
    use nix::sys::signal::kill;
    match i32::try_from(pid) {
        Ok(raw) if raw > 0 => match kill(nix::unistd::Pid::from_raw(raw), None) {
            Ok(()) => true,
            Err(e) => e != Errno::ESRCH,
        },
        _ => false,
    }
}

/// What the marker for one (PR, head) says.
#[derive(Debug, PartialEq)]
enum Marker {
    Absent,
    Dispatched,
    /// A dispatch failed `failures` times; the next try is allowed at `next_try_at`.
    Failed {
        failures: u32,
        next_try_at: u64,
    },
    /// Claimed by `pid` at `started`; `token` identifies the claim.
    Pending {
        pid: u32,
        started: u64,
        token: String,
        failures: u32,
    },
    /// Present but not a marker this tool wrote (truncated, forged, wrong owner/mode, link).
    /// Treated as a pending claim whose age is the file's mtime, never as "not dispatched".
    Unreadable {
        started: u64,
    },
}

/// The per-install secret that authenticates markers: a random nonce file in the state dir.
fn marker_key(state_dir: &Path) -> Result<Vec<u8>, String> {
    let path = state_dir.join(".nonce");
    if fs::symlink_metadata(&path).is_err() {
        let mut bytes = [0u8; 32];
        fs::File::open("/dev/urandom")
            .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut bytes))
            .map_err(|e| format!("read /dev/urandom: {e}"))?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .map_err(|e| format!("create nonce: {e}"))?;
        file.write_all(sha256_hex(&bytes).as_bytes())
            .map_err(|e| format!("write nonce: {e}"))?;
    }
    let meta = fs::symlink_metadata(&path).map_err(|e| format!("nonce: {e}"))?;
    if !meta.file_type().is_file()
        || meta.uid() != nix::unistd::Uid::current().as_raw()
        || meta.mode() & 0o077 != 0
    {
        return Err(
            "state nonce must be a regular 0600 file owned by the current user".to_string(),
        );
    }
    fs::read(&path).map_err(|e| format!("nonce: {e}"))
}

fn marker_mac(key: &[u8], body: &Value) -> String {
    let field = |name: &str| body[name].to_string();
    let message = [
        "pr",
        "head",
        "state",
        "pid",
        "started",
        "token",
        "failures",
        "next_try_at",
        "detail",
    ]
    .iter()
    .map(|n| field(n))
    .collect::<Vec<_>>()
    .join("\u{1f}");
    hmac_sha256_hex(key, message.as_bytes())
}

fn file_mtime(path: &Path) -> u64 {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs())
}

fn read_marker(path: &Path, key: &[u8]) -> Marker {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return Marker::Absent;
    };
    let unreadable = || Marker::Unreadable {
        started: file_mtime(path),
    };
    if !meta.file_type().is_file()
        || meta.uid() != nix::unistd::Uid::current().as_raw()
        || meta.mode() & 0o077 != 0
    {
        return unreadable();
    }
    let Some(value) = fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
    else {
        return unreadable();
    };
    if value["mac"].as_str() != Some(marker_mac(key, &value).as_str()) {
        return unreadable();
    }
    let num = |name: &str| value[name].as_u64().unwrap_or(0);
    match value["state"].as_str() {
        Some("dispatched") => Marker::Dispatched,
        Some("failed") => Marker::Failed {
            failures: u32::try_from(num("failures")).unwrap_or(u32::MAX),
            next_try_at: num("next_try_at"),
        },
        Some("pending") => Marker::Pending {
            pid: u32::try_from(num("pid")).unwrap_or(0),
            started: num("started"),
            token: value["token"].as_str().unwrap_or("").to_string(),
            failures: u32::try_from(num("failures")).unwrap_or(0),
        },
        _ => unreadable(),
    }
}

#[allow(clippy::too_many_arguments)]
fn marker_body(
    pr: &str,
    head: &str,
    state: &str,
    pid: u32,
    started: u64,
    token: &str,
    failures: u32,
    next_try_at: u64,
    detail: &str,
) -> Value {
    json!({"pr": pr, "head": head, "state": state, "pid": pid, "started": started, "token": token,
           "failures": failures, "next_try_at": next_try_at, "detail": detail})
}

/// Write a marker atomically: a fresh 0600 temp file in the same directory, then rename.
fn write_marker(path: &Path, key: &[u8], mut body: Value) -> Result<(), String> {
    body["mac"] = Value::String(marker_mac(key, &body));
    let tmp = path.with_extension(format!("tmp-{}-{}", std::process::id(), now_secs()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)
        .map_err(|e| format!("write marker: {e}"))?;
    file.write_all(format!("{body}\n").as_bytes())
        .map_err(|e| format!("write marker: {e}"))?;
    fs::rename(&tmp, path).map_err(|e| format!("commit marker: {e}"))
}

/// Run `f` holding an exclusive flock on the state dir's lock file. The critical sections are
/// short (read, decide, write one small file); the dispatch itself runs outside the lock.
fn locked<T>(state_dir: &Path, f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    let lock = state_dir.join(".lock");
    if fs::symlink_metadata(&lock).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err("state lock must not be a symlink".to_string());
    }
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .open(&lock)
        .map_err(|e| format!("open state lock: {e}"))?;
    let _guard =
        Flock::lock(file, FlockArg::LockExclusive).map_err(|(_, e)| format!("state lock: {e}"))?;
    f()
}

enum Claim {
    Won { token: String, failures: u32 },
    Already,
    InProgress,
    Backoff(u64),
}

/// Decide whether this process may dispatch for the marker, and if so claim it atomically.
/// A pending claim outlives the dispatch timeout by a grace period, so a claim is never
/// re-taken while its (killed-on-timeout) child may still be dying.
fn claim(
    path: &Path,
    key: &[u8],
    pr_node: &str,
    head: &str,
    timeout: u64,
    take: bool,
) -> Result<Claim, String> {
    const GRACE: u64 = 30;
    let now = now_secs();
    let expired = |started: u64| now.saturating_sub(started) > timeout + GRACE;
    let mut failures = 0;
    match read_marker(path, key) {
        Marker::Dispatched => return Ok(Claim::Already),
        Marker::Pending {
            pid,
            started,
            failures: f,
            ..
        } => {
            if pid_alive(pid) && !expired(started) {
                return Ok(Claim::InProgress);
            }
            failures = f;
        }
        Marker::Unreadable { started } if !expired(started) => return Ok(Claim::InProgress),
        Marker::Failed {
            failures: f,
            next_try_at,
        } => {
            if now < next_try_at {
                return Ok(Claim::Backoff(next_try_at));
            }
            failures = f;
        }
        _ => {}
    }
    let token = sha256_hex(format!("{now}{}{pr_node}{head}", std::process::id()).as_bytes());
    if take {
        write_marker(
            path,
            key,
            marker_body(
                pr_node,
                head,
                "pending",
                std::process::id(),
                now,
                &token,
                failures,
                0,
                "",
            ),
        )?;
    }
    Ok(Claim::Won { token, failures })
}

/// Run the dispatch command in its own process group and kill the group when it outlives
/// `timeout`; the child is always reaped before this returns, so the claim is held until it
/// is dead.
fn run_dispatch(
    command: &str,
    pr_node: &str,
    head: &str,
    timeout: u64,
) -> Result<std::process::ExitStatus, String> {
    use std::os::unix::process::CommandExt;
    let mut child = Command::new(command)
        .arg("--")
        .arg(pr_node)
        .arg(head)
        .process_group(0)
        .spawn()
        .map_err(|e| format!("run dispatch_command: {e}"))?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) if std::time::Instant::now() >= deadline => {
                if let Ok(pid) = i32::try_from(child.id()) {
                    let _ = nix::sys::signal::killpg(
                        nix::unistd::Pid::from_raw(pid),
                        nix::sys::signal::Signal::SIGKILL,
                    );
                }
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "dispatch_command exceeded dispatch_timeout_seconds ({timeout}) and was killed"
                ));
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(25)),
            Err(e) => return Err(format!("wait for dispatch_command: {e}")),
        }
    }
}

/// Statuses where Herdr cannot vouch for the head or the author: an operator must look, and
/// nothing may pass. Surfaced as an alert, never as "not needed".
const ALERT_STATUSES: [&str; 3] = ["head_unverified", "unknown_head", "unknown_author"];

/// Idempotent per (PR, head), serialised by a lock: returns the outcome name. An outcome that
/// starts with `alert:` means a human must act.
pub fn request_rereview(
    cfg: &HerdrConfig,
    scratch: &Path,
    state_dir: &Path,
    pr_node: &str,
    dry_run: bool,
) -> Result<(String, Value), String> {
    let pr_node = validate_pr_node(pr_node)?;
    let quorum = fetch_quorum(cfg, scratch, &pr_node)?;
    let status = quorum
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    if ALERT_STATUSES.contains(&status.as_str())
        || quorum.get("head_unverified").and_then(Value::as_bool) == Some(true)
    {
        return Ok((format!("alert:{status}"), quorum));
    }
    let required = quorum.get("rereview_required").and_then(Value::as_bool) == Some(true);
    let head = quorum
        .get("head")
        .and_then(Value::as_str)
        .and_then(canonical_commit);
    let Some(head) = head.filter(|_| required) else {
        return Ok((format!("not_needed:{status}"), quorum));
    };
    private_dir(state_dir)?;
    let marker = state_dir.join(format!(
        "{}-{}.json",
        &sha256_hex(pr_node.as_bytes())[..16],
        head
    ));
    let timeout = cfg.dispatch_timeout_seconds;
    let mine = locked(state_dir, || {
        let key = marker_key(state_dir)?;
        claim(&marker, &key, &pr_node, &head, timeout, !dry_run)
    })?;
    let (token, failures) = match mine {
        Claim::Already => return Ok(("already_requested".to_string(), quorum)),
        Claim::InProgress => return Ok(("request_in_progress".to_string(), quorum)),
        Claim::Backoff(_) => return Ok(("request_backoff".to_string(), quorum)),
        Claim::Won { .. } if dry_run => return Ok(("would_request".to_string(), quorum)),
        Claim::Won { token, failures } => (token, failures),
    };
    let finish = |state: &str, detail: &str| -> Result<bool, String> {
        locked(state_dir, || {
            let key = marker_key(state_dir)?;
            match read_marker(&marker, &key) {
                Marker::Pending { token: t, .. } if t == token => {}
                _ => return Ok(false), // reclaimed by someone else after our timeout
            }
            let now = now_secs();
            let (n, next) = if state == "failed" {
                let n = failures.saturating_add(1);
                // exponential backoff: 30s, 60s, 120s ... capped at one hour
                (n, now + (30u64 << n.saturating_sub(1).min(7)).min(3600))
            } else {
                (0, 0)
            };
            write_marker(
                &marker,
                &key,
                marker_body(&pr_node, &head, state, 0, now, "", n, next, detail),
            )?;
            Ok(true)
        })
    };
    match &cfg.dispatch_command {
        None => {
            finish("dispatched", "no dispatch_command configured: marker only")?;
            Ok(("requested_marker_only".to_string(), quorum))
        }
        Some(command) => match run_dispatch(command, &pr_node, &head, timeout) {
            Ok(status) if status.success() => {
                finish("dispatched", "")?;
                Ok(("requested".to_string(), quorum))
            }
            Ok(status) => {
                finish("failed", &format!("exit {status}"))?;
                Err(format!(
                    "dispatch_command failed ({status}); will retry after a backoff"
                ))
            }
            Err(e) => {
                finish("failed", &e)?;
                Err(e)
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_accepts_comments_other_blocks_and_a_document_marker() {
        let cfg = parse_config(
            "---\n# shared settings file\nversion: 1\nproject:\n  name: \"x: y # z\"\n  tags:\n    - a\nherdr:   # ours\n  enabled: true # on\n  url: \"https://h.example.test/p#x\" # quoted # data\n  ingest_key: k\n\n  dispatch_command: /bin/true\n  dispatch_timeout_seconds: 60\nother:\n  enabled: banana\n",
        )
        .unwrap();
        assert!(cfg.enabled);
        assert_eq!(cfg.url, "https://h.example.test/p#x");
        assert_eq!(cfg.dispatch_command.as_deref(), Some("/bin/true"));
        assert_eq!(cfg.dispatch_timeout_seconds, 60);
    }

    #[test]
    fn config_stays_strict_inside_the_herdr_block() {
        for (text, why) in [
            ("herdr:\n  enabled: maybe\n", "enabled value"),
            ("herdr:\n  bogus: 1\n", "unknown key"),
            (
                "herdr:\n  enabled: true\n  enabled: false\n",
                "duplicate key",
            ),
            (
                "herdr:\n  enabled: true\nherdr:\n  enabled: false\n",
                "second block",
            ),
            ("herdr:\n  enabled: true\n    url: x\n", "nested value"),
            ("herdr:\n  enabled: true\n url: x\n", "misaligned"),
            ("herdr: yes\n", "scalar herdr"),
            ("herdr:\n\tenabled: true\n", "tab"),
            ("herdr:\n  url: \"x\n", "unterminated quote"),
            ("herdr:\n  url: \"x\" y\n", "text after quote"),
            (
                "herdr:\n  dispatch_timeout_seconds: 0\n",
                "timeout too small",
            ),
            (
                "herdr:\n  dispatch_timeout_seconds: soon\n",
                "timeout not a number",
            ),
            ("herdr:\n  enabled\n", "no colon"),
        ] {
            assert!(
                parse_config(text).is_err(),
                "{why} must be rejected: {text:?}"
            );
        }
        assert!(
            parse_config("").is_ok(),
            "no herdr block means disabled defaults"
        );
        assert!(!parse_config("version: 1\n").unwrap().enabled);
    }

    #[test]
    fn url_validation_uses_a_real_parser() {
        for ok in [
            "https://herdr.example.test",
            "HTTPS://Herdr.Example.test/prefix/",
            "http://127.0.0.1:8080",
            "http://[::1]:8080",
            "http://localhost",
            "http://LOCALHOST:1",
        ] {
            assert!(validate_url(ok).is_ok(), "{ok}");
        }
        for bad in [
            "http://example.test",
            "http://127.0.0.2",
            "http://127.0.0.1.evil.test",
            "http://localhost.evil.test",
            "http://notlocalhost",
            "http://127.0.0.1@evil.test",
            "https://u:p@herdr.example.test",
            "https://u@herdr.example.test",
            "https://herdr.example.test?x=1",
            "https://herdr.example.test#f",
            "ftp://herdr.example.test",
            "file:///etc/passwd",
            "https://",
            "herdr.example.test",
            "",
        ] {
            assert!(validate_url(bad).is_err(), "{bad}");
        }
        // The backslash that curl and the WHATWG parser read differently is normalised away:
        // what is validated is exactly what is requested.
        assert_eq!(
            validate_url("http://127.0.0.1\\@evil.test").unwrap(),
            "http://127.0.0.1/@evil.test"
        );
        assert_eq!(
            validate_url("https://H.test/a/").unwrap(),
            "https://h.test/a"
        );
    }

    #[test]
    fn pr_node_shape_is_strict() {
        assert_eq!(
            validate_pr_node(" PR:GitHub.com/O/R#7 ").unwrap(),
            "pr:github.com/o/r#7"
        );
        for bad in [
            "",
            "pr:",
            "pr:x",
            "pr:-x#1",
            "--evil",
            "pr:a/b#1;x",
            "pr:a b#1",
            "pr:é#1",
            "issue:a#1",
            "pr:a/b#1\u{0}",
        ] {
            assert!(validate_pr_node(bad).is_err(), "{bad:?}");
        }
        assert!(validate_pr_node(&format!("pr:{}#1", "a".repeat(400))).is_err());
        // surrounding ASCII blanks are trimmed, never passed on
        assert_eq!(validate_pr_node("pr:a/b#1\n").unwrap(), "pr:a/b#1");
    }
}
