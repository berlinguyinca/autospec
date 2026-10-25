//! The merge gate's side of the head-bound review quorum: may `promote-pr` trust
//! `review-quorum.json`?
//!
//! Who decides that the quorum is REQUIRED must not be the PR being promoted, so the
//! requirement comes only from TRUSTED sources: the operator's own config
//! (`~/.autospec/autospec.yml`, or `--operator-config`) and the default branch's copy of
//! `.autospec/autospec.yml` (`git show refs/remotes/origin/HEAD:...`, a ref the PR cannot
//! move). The checkout under review may only ADD a requirement, never remove one. A trusted
//! source that exists but cannot be read or parsed means "required" (fail closed).

use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;

use autospec_core::autonomous::review_quorum::{
    canonical_commit, canonical_target, hmac_sha256_hex, report_attestation_message,
};
use yaml_edit::Document;

use super::herdr;

pub const REPORT_SOURCE: &str = "head-bound-review-quorum";
const DEFAULT_MAX_AGE_SECONDS: u64 = 900;
const MAX_FUTURE_SKEW_SECONDS: u64 = 60;

/// What one YAML source says about the quorum.
#[derive(Debug, Default, PartialEq)]
pub struct Opinion {
    pub required: bool,
    pub key_file: Option<String>,
    pub max_age: Option<u64>,
    pub herdr_enabled: bool,
}

fn plain_scalar(node: &yaml_edit::YamlNode) -> Option<String> {
    let scalar = node.as_scalar()?;
    (!scalar.is_quoted()).then(|| scalar.value().trim().to_string())
}

fn is_explicit_false(node: &yaml_edit::YamlNode) -> bool {
    matches!(
        plain_scalar(node).as_deref(),
        Some("false" | "False" | "FALSE")
    )
}

/// Read `review_quorum:` and `herdr:` out of a YAML document with a real YAML parser.
/// Anything under `review_quorum` other than an explicit `required: false` (or a bare
/// `review_quorum: false`) makes the quorum required: `"true"`, `yes`, `True`, an inline map,
/// a missing `required`, a duplicate key, a null.
pub fn read_opinion(text: &str) -> Result<Opinion, String> {
    let blank = text.lines().all(|l| {
        let t = l.trim();
        t.is_empty() || t.starts_with('#') || t == "---"
    });
    if blank {
        return Ok(Opinion::default());
    }
    let parsed = yaml_edit::YamlFile::parse(text);
    if !parsed.errors().is_empty() {
        return Err(format!("not valid YAML: {}", parsed.errors().join("; ")));
    }
    let doc: Document = text.parse().map_err(|e| format!("not valid YAML: {e:?}"))?;
    let map = doc
        .as_mapping()
        .ok_or("the top level of the config is not a mapping")?;
    let mut op = Opinion::default();
    let entries: Vec<_> = map.find_all_entries_by_key("review_quorum").collect();
    if entries.len() > 1 {
        op.required = true;
    }
    if let Some(entry) = entries.first() {
        match entry.value_node() {
            None => op.required = true,
            Some(node) if node.as_mapping().is_none() => op.required |= !is_explicit_false(&node),
            Some(node) => {
                let block = node.as_mapping().ok_or("review_quorum is not a mapping")?;
                let flags: Vec<_> = block.find_all_entries_by_key("required").collect();
                op.required |= !(flags.len() == 1
                    && flags[0].value_node().is_some_and(|n| is_explicit_false(&n)));
                let single = |key: &str| -> Result<Option<String>, String> {
                    let found: Vec<_> = block.find_all_entries_by_key(key).collect();
                    match found.as_slice() {
                        [] => Ok(None),
                        [one] => one
                            .value_node()
                            .and_then(|n| n.as_scalar().map(|s| s.as_string()))
                            .map(Some)
                            .ok_or_else(|| format!("review_quorum.{key} must be a scalar")),
                        _ => Err(format!("review_quorum.{key} is duplicated")),
                    }
                };
                op.key_file = single("attestation_key_file")?.filter(|s| !s.is_empty());
                op.max_age = single("max_report_age_seconds")?
                    .map(|v| {
                        v.trim()
                            .parse::<u64>()
                            .ok()
                            .filter(|n| (1..=604_800).contains(n))
                            .ok_or("review_quorum.max_report_age_seconds must be 1..604800")
                    })
                    .transpose()?;
            }
        }
    }
    // A lenient parser can read garbage as "no such key": if the name is in the text but not
    // a key we found, assume the worst.
    if entries.is_empty() && text.contains("review_quorum") {
        op.required = true;
    }
    let herdr_blocks: Vec<_> = map.find_all_entries_by_key("herdr").collect();
    if herdr_blocks.len() > 1 {
        op.herdr_enabled = true;
    }
    if let Some(entry) = herdr_blocks.first() {
        if let Some(node) = entry.value_node() {
            if let Some(block) = node.as_mapping() {
                op.herdr_enabled = block.find_all_entries_by_key("enabled").any(|e| {
                    e.value_node().and_then(|n| plain_scalar(&n)).as_deref() == Some("true")
                }) || op.herdr_enabled;
            }
        }
    }
    // Live re-evaluation against Herdr is part of the requirement once the operator enabled it.
    op.required = op.required || op.herdr_enabled;
    Ok(op)
}

pub fn home_dir() -> Option<PathBuf> {
    nix::unistd::User::from_uid(nix::unistd::Uid::current())
        .ok()
        .flatten()
        .map(|u| u.dir)
}

pub fn default_operator_config() -> Option<PathBuf> {
    home_dir().map(|h| h.join(".autospec/autospec.yml"))
}

/// The operator's state directory root (outside every repository and worktree).
pub fn default_state_root() -> Result<PathBuf, String> {
    home_dir()
        .map(|h| h.join(".autospec/state"))
        .ok_or_else(|| {
            "cannot find the operator's home directory; pass --state-dir/--scratch-dir".to_string()
        })
}

enum Source {
    /// No such source (or no file in it): it has no opinion.
    Absent,
    Text(String),
    /// It exists but cannot be read: treated as "required".
    Unreadable(String),
}

fn read_operator(path: &Path) -> Source {
    match fs::read_to_string(path) {
        Ok(text) => Source::Text(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Source::Absent,
        Err(e) => Source::Unreadable(format!("operator config {}: {e}", path.display())),
    }
}

fn git(root: &Path, args: &[&str]) -> Option<std::process::Output> {
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .ok()
}

/// The default branch's `.autospec/autospec.yml`, from a local remote-tracking ref (no network).
fn read_base(root: &Path) -> Source {
    if !git(root, &["rev-parse", "--git-dir"]).is_some_and(|o| o.status.success()) {
        return Source::Absent; // not a git checkout: there is no PR branch to be controlled by
    }
    for reference in [
        "refs/remotes/origin/HEAD",
        "refs/remotes/origin/main",
        "refs/remotes/origin/master",
    ] {
        let spec = format!("{reference}:.autospec/autospec.yml");
        let commit = format!("{reference}^{{commit}}");
        if !git(root, &["rev-parse", "--verify", "-q", &commit]).is_some_and(|o| o.status.success())
        {
            continue;
        }
        if !git(root, &["cat-file", "-e", &spec]).is_some_and(|o| o.status.success()) {
            return Source::Absent; // the default branch has no such file
        }
        return match git(root, &["show", &spec]) {
            Some(o) if o.status.success() => {
                Source::Text(String::from_utf8_lossy(&o.stdout).into_owned())
            }
            _ => Source::Unreadable(format!(
                "cannot read .autospec/autospec.yml from {reference}"
            )),
        };
    }
    Source::Absent
}

/// Validate and read an attestation key: absolute, a regular file (not a link), mode 0600 or
/// tighter, owned by us, and outside the repository and every git worktree.
pub fn load_attestation_key(path: &str, repo_root: Option<&Path>) -> Result<Vec<u8>, String> {
    let file = Path::new(path);
    if !file.is_absolute() {
        return Err("attestation_key_file must be an absolute path".to_string());
    }
    let meta = fs::symlink_metadata(file).map_err(|e| format!("attestation key: {e}"))?;
    if !meta.file_type().is_file() {
        return Err("attestation key must be a regular file (not a symlink)".to_string());
    }
    if meta.permissions().mode() & 0o077 != 0 {
        return Err("attestation key must be chmod 600".to_string());
    }
    if meta.uid() != nix::unistd::Uid::current().as_raw() {
        return Err("attestation key must be owned by the current user".to_string());
    }
    let real = fs::canonicalize(file).map_err(|e| format!("attestation key: {e}"))?;
    if let Some(root) = repo_root.and_then(|r| fs::canonicalize(r).ok()) {
        if real.starts_with(&root) {
            return Err("attestation key must live outside the repository".to_string());
        }
    }
    let is_worktree = |dir: &Path| match fs::symlink_metadata(dir.join(".git")) {
        Ok(m) if m.file_type().is_file() => true, // a linked worktree's `.git` pointer file
        Ok(m) if m.file_type().is_dir() => dir.join(".git/HEAD").exists(),
        Ok(_) => true,
        Err(_) => false,
    };
    if real.ancestors().skip(1).any(is_worktree) {
        return Err("attestation key must live outside every git worktree".to_string());
    }
    let raw = fs::read(&real).map_err(|e| format!("attestation key: {e}"))?;
    let key: Vec<u8> = raw
        .iter()
        .copied()
        .skip_while(u8::is_ascii_whitespace)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .skip_while(u8::is_ascii_whitespace)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    if key.len() < 32 || key.len() > 4096 {
        return Err("attestation key must be 32..4096 bytes".to_string());
    }
    Ok(key)
}

/// The trusted view used by BOTH `--write-report` and `verify-report`.
pub struct Trusted {
    pub required: bool,
    pub key_file: Option<String>,
    pub max_age: u64,
    pub herdr_enabled: bool,
    pub problems: Vec<String>,
    pub operator_config: Option<PathBuf>,
}

pub fn trusted(root: &Path, operator: Option<&Path>, include_base: bool) -> Trusted {
    let operator_path = operator
        .map(Path::to_path_buf)
        .or_else(default_operator_config);
    let mut sources: Vec<(&str, Source)> = Vec::new();
    if let Some(path) = &operator_path {
        sources.push(("operator", read_operator(path)));
    }
    if include_base {
        sources.push(("base", read_base(root)));
    }
    let mut out = Trusted {
        required: false,
        key_file: None,
        max_age: DEFAULT_MAX_AGE_SECONDS,
        herdr_enabled: false,
        problems: Vec::new(),
        operator_config: operator_path,
    };
    let mut ages = Vec::new();
    for (name, source) in sources {
        let text = match source {
            Source::Absent => continue,
            Source::Unreadable(why) => {
                out.required = true;
                out.problems.push(why);
                continue;
            }
            Source::Text(t) => t,
        };
        match read_opinion(&text) {
            Ok(op) => {
                out.required |= op.required;
                if name == "operator" {
                    out.herdr_enabled = op.herdr_enabled;
                }
                if out.key_file.is_none() {
                    out.key_file = op.key_file;
                }
                ages.extend(op.max_age);
            }
            Err(why) => {
                out.required = true;
                out.problems.push(format!("{name} config: {why}"));
            }
        }
    }
    if let Some(age) = ages.into_iter().min() {
        out.max_age = age;
    }
    // The checkout under review can only make the gate stricter.
    if let Ok(text) = fs::read_to_string(root.join(".autospec/autospec.yml")) {
        if read_opinion(&text).is_ok_and(|op| op.required) {
            out.required = true;
        }
    }
    out
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

pub fn evaluated_now() -> u64 {
    now_secs()
}

fn live_pr(root: &Path, pr: &str, repo: Option<&str>) -> Result<(String, String), String> {
    let mut cmd = Command::new("gh");
    cmd.current_dir(root)
        .args(["pr", "view", pr, "--json", "headRefOid,url"]);
    if let Some(repo) = repo {
        cmd.args(["--repo", repo]);
    }
    let out = cmd.output().map_err(|e| format!("cannot run gh: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "gh pr view failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let value: serde_json::Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| format!("gh answered invalid JSON: {e}"))?;
    let head = value["headRefOid"]
        .as_str()
        .and_then(canonical_commit)
        .ok_or("gh did not return a full head sha")?;
    let url = url::Url::parse(value["url"].as_str().unwrap_or(""))
        .map_err(|e| format!("gh returned no PR url: {e}"))?;
    let segments: Vec<&str> = url
        .path_segments()
        .map(Iterator::collect)
        .unwrap_or_default();
    let (Some(host), [owner, name, "pull", number, ..]) = (url.host_str(), segments.as_slice())
    else {
        return Err("gh returned an unexpected PR url".to_string());
    };
    Ok((
        head,
        format!("pr:{host}/{owner}/{name}#{number}").to_ascii_lowercase(),
    ))
}

fn constant_time_eq(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

pub struct VerifyArgs {
    pub root: PathBuf,
    pub report: Option<PathBuf>,
    pub pr: String,
    pub repo: Option<String>,
    pub expect_head: Option<String>,
    pub force_required: bool,
    pub no_live: bool,
    pub operator_config: Option<PathBuf>,
    pub scratch: Option<PathBuf>,
}

/// Returns (ok, JSON). Never errors: every problem is a reason.
pub fn verify(args: &VerifyArgs) -> (bool, serde_json::Value) {
    let root = fs::canonicalize(&args.root).unwrap_or_else(|_| args.root.clone());
    let trusted = trusted(&root, args.operator_config.as_deref(), true);
    let required = args.force_required || trusted.required;
    let mut reasons: Vec<String> = trusted.problems.clone();
    let mut warnings: Vec<String> = Vec::new();
    let report_path = args
        .report
        .clone()
        .unwrap_or_else(|| root.join(".autospec/reports/review-quorum.json"));
    let mut origin = "none";
    let mut live = "skipped";
    let mut live_head: Option<String> = None;
    let mut report_head: Option<String> = None;

    let loaded: Option<serde_json::Value> = match fs::symlink_metadata(&report_path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if required {
                reasons.push(
                    "review quorum is required but review-quorum.json is missing".to_string(),
                );
            }
            None
        }
        Err(e) => {
            reasons.push(format!("review-quorum.json is unreadable: {e}"));
            None
        }
        Ok(meta) if !meta.file_type().is_file() => {
            reasons.push(
                "review-quorum.json must be a regular file (not a symlink or directory)"
                    .to_string(),
            );
            None
        }
        Ok(_) => {
            let inside = fs::canonicalize(&report_path)
                .map(|real| real.starts_with(&root))
                .unwrap_or(false);
            if !inside {
                reasons.push("review-quorum.json must resolve inside the repository".to_string());
                None
            } else {
                match fs::read_to_string(&report_path) {
                    Ok(text) => match serde_json::from_str::<serde_json::Value>(&text) {
                        Ok(v) if v.as_object().is_some_and(|o| !o.is_empty()) => Some(v),
                        Ok(_) => {
                            reasons
                                .push("review-quorum.json is empty or not an object".to_string());
                            None
                        }
                        Err(e) => {
                            reasons.push(format!("review-quorum.json is malformed: {e}"));
                            None
                        }
                    },
                    Err(e) => {
                        reasons.push(format!("review-quorum.json is unreadable: {e}"));
                        None
                    }
                }
            }
        }
    };

    if let Some(report) = &loaded {
        let verdict = report["verdict"].as_str().unwrap_or("");
        let ours = report["source"].as_str() == Some(REPORT_SOURCE);
        report_head = report["head"].as_str().and_then(canonical_commit);
        if !matches!(verdict, "pass" | "pass_with_warnings") {
            reasons.push(format!(
                "review quorum verdict is {}",
                if verdict.is_empty() {
                    "missing"
                } else {
                    verdict
                }
            ));
        }
        if report["blocking_findings"]
            .as_array()
            .is_some_and(|b| !b.is_empty())
        {
            reasons.push("blocking specialist finding".to_string());
        }
        if required || ours {
            if !ours {
                reasons.push(
                    "required review quorum report is not from the head-bound evaluator"
                        .to_string(),
                );
            }
            if report["promotion_ready"].as_bool() != Some(true) || verdict != "pass" {
                reasons.push("report is not a head-bound pass".to_string());
            }
            // freshness
            match report["evaluated_at"].as_u64() {
                None => reasons.push("report has no evaluated_at".to_string()),
                Some(at) => {
                    let now = now_secs();
                    if at > now + MAX_FUTURE_SKEW_SECONDS {
                        reasons.push("report evaluated_at is in the future".to_string());
                    } else if now.saturating_sub(at) > trusted.max_age {
                        reasons.push(format!(
                            "report is older than max_report_age_seconds ({})",
                            trusted.max_age
                        ));
                    }
                }
            }
            // origin
            if let Some(key_file) = &trusted.key_file {
                origin = "attested";
                match load_attestation_key(key_file, Some(&root)) {
                    Err(e) => reasons.push(e),
                    Ok(key) => {
                        let expected = report_attestation_message(
                            report["target_id"].as_str().unwrap_or(""),
                            report["head"].as_str().unwrap_or(""),
                            verdict,
                            report["evaluated_at"].as_u64().unwrap_or(0),
                            report["votes_digest"].as_str().unwrap_or(""),
                        );
                        let mac = hmac_sha256_hex(&key, &expected);
                        let got = report["hmac"].as_str().unwrap_or("").to_ascii_lowercase();
                        if !constant_time_eq(&mac, &got) {
                            reasons.push(
                                "report attestation (hmac) is missing or invalid".to_string(),
                            );
                        }
                    }
                }
            } else {
                origin = "unattested";
                warnings.push("origin: unattested (no review_quorum.attestation_key_file in a trusted config); anyone who can write the report file can forge it".to_string());
            }
            // binding to the live PR
            if args.pr.is_empty() {
                reasons.push("no --pr: cannot bind the report to a PR head".to_string());
            } else {
                match live_pr(&root, &args.pr, args.repo.as_deref()) {
                    Err(e) => reasons.push(format!("cannot verify the live PR head: {e}")),
                    Ok((head, target)) => {
                        if report_head.as_deref() != Some(head.as_str()) {
                            reasons.push(format!(
                                "report head {} is not the live PR head {head}",
                                report_head.as_deref().unwrap_or("<none>")
                            ));
                        }
                        if let Some(expect) = &args.expect_head {
                            if canonical_commit(expect).as_deref() != Some(head.as_str()) {
                                reasons.push(format!(
                                    "live PR head {head} is not the expected head {expect}"
                                ));
                            }
                        }
                        let report_target = report["target_id"].as_str().and_then(canonical_target);
                        if report_target.as_deref() != Some(target.as_str()) {
                            reasons.push(format!(
                                "report target {} is not {target}",
                                report_target.as_deref().unwrap_or("<none>")
                            ));
                        }
                        live_head = Some(head);
                        // Herdr, at verify time: a later `blocked` for this head is seen here.
                        if trusted.herdr_enabled && !args.no_live {
                            live = "checked";
                            if let Err(e) = live_herdr(
                                &trusted,
                                &target,
                                live_head.as_deref(),
                                args.scratch.as_deref(),
                            ) {
                                live = "error";
                                reasons.push(e);
                            }
                        }
                    }
                }
            }
        }
    } else if required && trusted.herdr_enabled && !args.no_live {
        live = "skipped";
    }
    let ok = reasons.is_empty();
    (
        ok,
        serde_json::json!({"ok": ok, "required": required, "origin": origin, "live": live,
            "live_head": live_head, "report_head": report_head, "reasons": reasons, "warnings": warnings}),
    )
}

/// Ask Herdr for the quorum NOW and require `satisfied` at the live head.
fn live_herdr(
    trusted: &Trusted,
    target: &str,
    live_head: Option<&str>,
    scratch: Option<&Path>,
) -> Result<(), String> {
    let path = trusted
        .operator_config
        .as_deref()
        .ok_or("live Herdr check: no operator config")?;
    let cfg = herdr::load_config(path).map_err(|e| format!("live Herdr check: {e}"))?;
    if !cfg.enabled {
        return Ok(());
    }
    let scratch = match scratch {
        Some(p) => p.to_path_buf(),
        None => default_state_root()
            .map_err(|e| format!("live Herdr check: {e}"))?
            .join("herdr-scratch"),
    };
    let quorum = herdr::fetch_quorum(&cfg, &scratch, target)
        .map_err(|e| format!("live Herdr check failed: {e}"))?;
    let status = quorum["status"].as_str().unwrap_or("unknown");
    let head = quorum["head"].as_str().and_then(canonical_commit);
    if status != "satisfied" || head.as_deref() != live_head {
        return Err(format!(
            "Herdr says the quorum is {status} at {} (live head {})",
            head.as_deref().unwrap_or("<none>"),
            live_head.unwrap_or("<none>")
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn required(text: &str) -> bool {
        read_opinion(text).map_or(true, |o| o.required)
    }

    #[test]
    fn only_an_explicit_false_opts_out() {
        for text in [
            "review_quorum:\n  required: true\n",
            "review_quorum:\n  required: \"true\"\n",
            "review_quorum:\n  required: 'true'\n",
            "review_quorum:\n  required: True\n",
            "review_quorum:\n  required: yes\n",
            "review_quorum:\n  required:   true   # why\n",
            "review_quorum: {required: true}\n",
            "review_quorum: {required: \"false\"}\n",
            "review_quorum:\n  required: false\n  required: true\n",
            "review_quorum:\n  required: true\n  required: false\n",
            "review_quorum:\n  other: 1\n",
            "review_quorum:\n",
            "review_quorum: yes\n",
            "review_quorum: \"false\"\n",
            "review_quorum:\r\n  required: true\r\n",
            "review_quorum:\n  required: false\nreview_quorum:\n  required: false\n",
            "review_quorum: false\nherdr:\n  enabled: true\n",
            "herdr: {enabled: true}\n",
            "x: [\nreview_quorum: false\n",
            "- a\n- b\n",
        ] {
            assert!(required(text), "must be required: {text:?}");
        }
        for text in [
            "",
            "# nothing\n---\n",
            "version: 1\n",
            "review_quorum:\n  required: false\n",
            "review_quorum:\n  required: False\n",
            "review_quorum: {required: false}\n",
            "review_quorum: false\n",
            "review_quorum:\r\n  required: false\r\n",
            "review_quorum:\n  required: false # opted out\nother:\n  required: true\n",
            "herdr:\n  enabled: false\nreview_quorum:\n  required: false\n",
        ] {
            assert!(
                !required(text),
                "explicit opt-out must not be required: {text:?}"
            );
        }
    }

    #[test]
    fn settings_are_read_from_the_block_and_validated() {
        let op = read_opinion(
            "review_quorum:\n  required: true\n  attestation_key_file: /etc/k\n  max_report_age_seconds: 60\n",
        )
        .unwrap();
        assert_eq!(op.key_file.as_deref(), Some("/etc/k"));
        assert_eq!(op.max_age, Some(60));
        for bad in [
            "review_quorum:\n  max_report_age_seconds: 0\n",
            "review_quorum:\n  max_report_age_seconds: soon\n",
            "review_quorum:\n  attestation_key_file: a\n  attestation_key_file: b\n",
            "review_quorum:\n  attestation_key_file: [a]\n",
        ] {
            assert!(read_opinion(bad).is_err(), "{bad:?}");
        }
    }
}
