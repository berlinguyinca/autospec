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
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;

use autospec_core::autonomous::review_quorum::{canonical_commit, receipt_binding, sha256_hex};
use serde_json::{json, Value};

#[derive(Debug, Default)]
pub struct HerdrConfig {
    pub enabled: bool,
    pub url: String,
    pub ingest_key: String,
    pub curl: String,
    pub dispatch_command: Option<String>,
}

/// Strict reader for the one shape we accept: a top-level `herdr:` mapping of scalar keys.
pub fn parse_config(text: &str) -> Result<HerdrConfig, String> {
    let mut values: BTreeMap<String, String> = BTreeMap::new();
    let mut in_block = false;
    for (n, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if !line.starts_with(' ') {
            if trimmed != "herdr:" {
                return Err(format!(
                    "config line {}: only a top-level `herdr:` block is accepted",
                    n + 1
                ));
            }
            in_block = true;
            continue;
        }
        if !in_block {
            return Err(format!("config line {}: indented outside `herdr:`", n + 1));
        }
        let (key, value) = trimmed
            .split_once(':')
            .ok_or_else(|| format!("config line {}: expected `key: value`", n + 1))?;
        let value = value
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .to_string();
        if values.insert(key.trim().to_string(), value).is_some() {
            return Err(format!("config line {}: duplicate key", n + 1));
        }
    }
    let known = ["enabled", "url", "ingest_key", "curl", "dispatch_command"];
    if let Some(unknown) = values.keys().find(|k| !known.contains(&k.as_str())) {
        return Err(format!("config: unknown key `{unknown}`"));
    }
    let enabled = match values.get("enabled").map(String::as_str) {
        None | Some("false") => false,
        Some("true") => true,
        Some(_) => return Err("config: herdr.enabled must be true or false".to_string()),
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
    })
}

pub fn load_config(path: &Path) -> Result<HerdrConfig, String> {
    let meta = fs::metadata(path).map_err(|e| format!("read herdr config: {e}"))?;
    if meta.permissions().mode() & 0o077 != 0 {
        return Err("herdr config holds an ingest key and must be chmod 600".to_string());
    }
    parse_config(&fs::read_to_string(path).map_err(|e| format!("read herdr config: {e}"))?)
}

fn require_ready(cfg: &HerdrConfig) -> Result<(), String> {
    if !(cfg.url.starts_with("https://")
        || cfg.url.starts_with("http://127.0.0.1")
        || cfg.url.starts_with("http://localhost"))
    {
        return Err("herdr.url must be https (or loopback http)".to_string());
    }
    if cfg.ingest_key.is_empty() {
        return Err("herdr.ingest_key is required".to_string());
    }
    Ok(())
}

/// The reviewer id Herdr derives from an API key (`key:` + first 12 hex of its sha256).
pub fn principal_id(ingest_key: &str) -> String {
    format!("key:{}", &sha256_hex(ingest_key.as_bytes())[..12])
}

pub struct Publication {
    pub bodies: Vec<Value>,
    pub review_node: String,
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
    let commit =
        canonical_commit(commit).ok_or("commit must be a full 40 or 64 hex character head")?;
    let verdict = verdict.trim().to_ascii_lowercase();
    if verdict != "lgtm" && verdict != "blocked" {
        return Err("verdict must be lgtm or blocked".to_string());
    }
    if !pr_node.starts_with("pr:")
        || pr_node.len() > 400
        || pr_node.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err("pr node must look like pr:<host>/<owner>/<repo>#<n>".to_string());
    }
    let pr_node = pr_node.to_ascii_lowercase();
    let digest = sha256_hex(verdict_bytes);
    let binding = receipt_binding(&pr_node, &commit, reviewer, &digest);
    let id = sha256_hex(format!("{pr_node}\0{commit}\0{reviewer}\0{verdict}\0{digest}").as_bytes());
    let review_node = format!("review:{}", &id[..24]);
    let source = format!("ar-{}", &id[..12]);
    let base = |seq: u64, entity: &str, event_type: &str, payload: Value| {
        json!({"schema_version": 2, "event_id": format!("{source}-{seq}"), "source": source,
               "source_sequence": seq, "entity_type": "lineage", "entity_id": entity,
               "event_type": event_type, "payload": payload, "visibility_scope": "org"})
    };
    Ok(Publication {
        bodies: vec![
            base(
                0,
                &review_node,
                "node.upsert",
                json!({"node_type": "review", "node_id": review_node, "label": format!("review of {commit}")}),
            ),
            base(
                1,
                &review_node,
                "edge.add",
                json!({
                "edge_type": "reviewed", "from_id": review_node, "to_id": pr_node,
                "attrs": {"commit": commit, "verdict": verdict, "receipt_digest": digest, "receipt_binding": binding}}),
            ),
        ],
        review_node,
    })
}

fn private_file(dir: &Path, name: &str, content: &str) -> Result<PathBuf, String> {
    let path = dir.join(name);
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

/// Run curl with the key in a 0600 header file. Returns (http status, body).
fn curl(
    cfg: &HerdrConfig,
    scratch: &Path,
    method_post_body: Option<&str>,
    url: &str,
) -> Result<(u16, String), String> {
    let nonce = sha256_hex(
        format!(
            "{url}{:?}{}",
            std::time::SystemTime::now(),
            std::process::id()
        )
        .as_bytes(),
    );
    let headers = private_file(
        scratch,
        &format!("h-{}", &nonce[..12]),
        &format!(
            "X-API-Key: {}\nContent-Type: application/json\n",
            cfg.ingest_key
        ),
    )?;
    let mut cmd = Command::new(&cfg.curl);
    cmd.args(["-sS", "--max-time", "20", "-H"])
        .arg(format!("@{}", headers.display()));
    let body_path = match method_post_body {
        Some(body) => {
            let p = private_file(scratch, &format!("b-{}", &nonce[..12]), body)?;
            cmd.args(["-X", "POST", "--data-binary"])
                .arg(format!("@{}", p.display()));
            Some(p)
        }
        None => None,
    };
    cmd.args(["-w", "\n%{http_code}"]).arg(url);
    let out = cmd.output().map_err(|e| format!("run curl: {e}"))?;
    // Best effort: blank the secret-bearing files (never deleted: they are truncated).
    for p in [Some(&headers), body_path.as_ref()].into_iter().flatten() {
        let _ = fs::write(p, "");
    }
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
    require_ready(cfg)?;
    let mut acks = Vec::new();
    for body in &pubn.bodies {
        let (status, text) = curl(
            cfg,
            scratch,
            Some(&body.to_string()),
            &format!("{}/api/v1/ingest", cfg.url),
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
    require_ready(cfg)?;
    let url = format!(
        "{}/api/v1/graph/nodes/{}/review-quorum",
        cfg.url,
        encode_segment(pr_node)
    );
    let (status, text) = curl(cfg, scratch, None, &url)?;
    if status != 200 {
        return Err(format!("herdr review-quorum answered HTTP {status}"));
    }
    serde_json::from_str(&text).map_err(|e| format!("herdr answered invalid JSON: {e}"))
}

/// Idempotent per (PR, head): returns the outcome name.
pub fn request_rereview(
    cfg: &HerdrConfig,
    scratch: &Path,
    state_dir: &Path,
    pr_node: &str,
    dry_run: bool,
) -> Result<(String, Value), String> {
    let quorum = fetch_quorum(cfg, scratch, pr_node)?;
    let required = quorum.get("rereview_required").and_then(Value::as_bool) == Some(true);
    let head = quorum
        .get("head")
        .and_then(Value::as_str)
        .and_then(canonical_commit);
    let Some(head) = head.filter(|_| required) else {
        let status = quorum
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        return Ok((format!("not_needed:{status}"), quorum));
    };
    fs::create_dir_all(state_dir).map_err(|e| format!("create state dir: {e}"))?;
    let marker = state_dir.join(format!(
        "{}-{}.json",
        &sha256_hex(pr_node.to_ascii_lowercase().as_bytes())[..16],
        head
    ));
    if let Ok(existing) = fs::read_to_string(&marker) {
        if serde_json::from_str::<Value>(&existing)
            .ok()
            .and_then(|v| v["state"].as_str().map(str::to_string))
            .as_deref()
            == Some("dispatched")
        {
            return Ok(("already_requested".to_string(), quorum));
        }
    }
    if dry_run {
        return Ok(("would_request".to_string(), quorum));
    }
    let write_marker = |state: &str, detail: &str| -> Result<(), String> {
        fs::write(
            &marker,
            format!(
                "{}\n",
                json!({"pr": pr_node, "head": head, "state": state, "detail": detail})
            ),
        )
        .map_err(|e| format!("write marker: {e}"))
    };
    write_marker("pending", "")?;
    match &cfg.dispatch_command {
        None => {
            write_marker("dispatched", "no dispatch_command configured: marker only")?;
            Ok(("requested_marker_only".to_string(), quorum))
        }
        Some(command) => {
            let status = Command::new(command)
                .arg(pr_node)
                .arg(&head)
                .status()
                .map_err(|e| format!("run dispatch_command: {e}"))?;
            if status.success() {
                write_marker("dispatched", "")?;
                Ok(("requested".to_string(), quorum))
            } else {
                write_marker("failed", &format!("exit {status}"))?;
                Err(format!(
                    "dispatch_command failed ({status}); will retry on the next request"
                ))
            }
        }
    }
}
