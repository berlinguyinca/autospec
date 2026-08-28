use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use autospec_core::autonomous::no_work::NoWorkTier;
use autospec_core::autonomous::waterfall::{sha256_hex, TierReceipt, TierStatus};
use autospec_core::coordination::RemoteIssue;
use autospec_core::lint::{
    lint_issue_body, lint_issue_implementation_contract, DiffFile, ImplementationLintSeverity,
    UnifiedDiff,
};
use serde_json::Value;

use super::gh_read::run_gh_read_with_retry;
use super::resilience::{with_current_lifecycle_lease, ConductorLease};
use super::tier15;
use super::tier2_receipts::{
    acknowledge_tier2_publication_with_lease, Tier2Progress, PRODUCER_VERSION,
};
use super::waterfall::{StoreAcquisition, WaterfallStore, WaterfallStoreError};

const MARKER_PREFIX: &str = "<!-- autospec:tier2-publication:v1:";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PublicationDraft {
    pub stable_key: String,
    pub title: String,
    pub body: String,
    pub labels: Vec<&'static str>,
}

#[derive(Debug)]
struct ProposalDraft {
    stable_key: String,
    title: String,
    target_path: String,
}

pub(super) fn publish_tier2_with_lease(
    state_root: &Path,
    repo: &str,
    repo_dir: &Path,
    lease: &ConductorLease,
) -> Result<Tier2Progress, String> {
    let existing = with_current_lifecycle_lease(lease, || tier15::repository_issues(repo))?;
    let drafts =
        with_current_lifecycle_lease(lease, || publication_plan(state_root, repo, &existing))?;
    if !drafts.is_empty() {
        for label in required_labels(&drafts) {
            with_current_lifecycle_lease(lease, || ensure_label(repo, label))?;
        }
        for draft in drafts {
            with_current_lifecycle_lease(lease, || create_issue(repo_dir, repo, &draft))?;
        }
    }
    let confirmed = with_current_lifecycle_lease(lease, || tier15::repository_issues(repo))?;
    confirm_and_acknowledge(state_root, repo, lease, &confirmed)
}

pub(super) fn confirm_and_acknowledge(
    state_root: &Path,
    repo: &str,
    lease: &ConductorLease,
    confirmed: &[RemoteIssue],
) -> Result<Tier2Progress, String> {
    let remaining =
        with_current_lifecycle_lease(lease, || publication_plan(state_root, repo, confirmed))?;
    if !remaining.is_empty() {
        return Err(format!(
            "Tier 2 publication could not confirm {} stable-key markers",
            remaining.len()
        ));
    }
    acknowledge_tier2_publication_with_lease(state_root, repo, lease)
}

pub(super) fn publication_plan(
    state_root: &Path,
    repo: &str,
    existing: &[RemoteIssue],
) -> Result<Vec<PublicationDraft>, String> {
    let (receipt, collector, ranked) = load_verified_documents(state_root, repo)?;
    if !matches!(receipt.status(), TierStatus::Produced { count } if *count > 0) {
        return Err("Tier 2 publisher requires a produced receipt".to_string());
    }
    let test_path = test_path_template(&collector)?;
    let test_command = test_command(&collector);
    let proposals = parse_ranked(&ranked)?;
    if proposals.len() as u64 != receipt.funnel().ranked {
        return Err("Tier 2 publication proposal count does not match receipt".to_string());
    }
    let mut drafts = Vec::new();
    for proposal in proposals {
        let marker = marker(repo, &proposal.stable_key);
        let marked = existing
            .iter()
            .filter(|issue| issue.body.lines().any(|line| line == marker))
            .collect::<Vec<_>>();
        match marked.as_slice() {
            [] => drafts.push(render_draft(
                proposal,
                marker,
                &receipt,
                &test_path,
                &test_command,
            )?),
            [issue] => {
                let required_labels = if issue.closed {
                    &["origin:self"][..]
                } else {
                    &["origin:self", "auto-implement"][..]
                };
                for required in required_labels {
                    if !issue.labels.iter().any(|label| label == required) {
                        return Err(format!(
                            "Tier 2 publication marker on issue {} is missing {required}",
                            issue.number
                        ));
                    }
                }
            }
            marked => {
                return Err(format!(
                    "Tier 2 publication marker occurs on {} issues; refusing ambiguous replay",
                    marked.len()
                ));
            }
        }
    }
    Ok(drafts)
}

fn load_verified_documents(
    state_root: &Path,
    repo: &str,
) -> Result<(TierReceipt, String, String), String> {
    let store =
        match WaterfallStore::acquire(state_root.join("waterfall"), repo).map_err(store_error)? {
            StoreAcquisition::Acquired(store) => store,
            StoreAcquisition::Held => return Err("Tier 2 publication store is busy".to_string()),
        };
    let state = store
        .load_state()
        .map_err(store_error)?
        .ok_or_else(|| "Tier 2 publication state is missing".to_string())?;
    if state.current_tier() != NoWorkTier::Tier2 {
        return Err("Tier 2 publication cursor is not at Tier 2".to_string());
    }
    let receipt = store
        .load_receipt(state.next_pass_id(), NoWorkTier::Tier2)
        .map_err(store_error)?
        .ok_or_else(|| "Tier 2 publication receipt is missing".to_string())?;
    store
        .verify_tier2_evidence(state.next_pass_id(), &receipt)
        .map_err(store_error)?;
    if receipt.producer_version() != PRODUCER_VERSION {
        return Err("Tier 2 publisher rejected an unknown producer".to_string());
    }
    let evidence_root = state_root.join("waterfall");
    let collector = read_evidence(&evidence_root, &receipt, "collector.json")?;
    let ranked = read_evidence(&evidence_root, &receipt, "roi-rank.json")?;
    Ok((receipt, collector, ranked))
}

fn read_evidence(root: &Path, receipt: &TierReceipt, suffix: &str) -> Result<String, String> {
    let evidence = receipt
        .evidence()
        .iter()
        .find(|evidence| evidence.reference.ends_with(suffix))
        .ok_or_else(|| format!("Tier 2 receipt is missing {suffix}"))?;
    fs::read_to_string(root.join(&evidence.reference))
        .map_err(|error| format!("cannot read Tier 2 {suffix}: {error}"))
}

fn parse_ranked(document: &str) -> Result<Vec<ProposalDraft>, String> {
    let root: Value = serde_json::from_str(document)
        .map_err(|error| format!("invalid Tier 2 rank JSON: {error}"))?;
    let ranked = root
        .get("ranked")
        .and_then(Value::as_array)
        .ok_or_else(|| "Tier 2 rank document is missing ranked proposals".to_string())?;
    let mut proposals = Vec::with_capacity(ranked.len());
    for value in ranked {
        let proposal = value
            .get("proposal")
            .and_then(Value::as_object)
            .ok_or_else(|| "Tier 2 ranked proposal is missing its proposal".to_string())?;
        let stable_key = required_line(proposal.get("stable_key"), "stable_key")?;
        if !valid_stable_key(&stable_key) {
            return Err("Tier 2 proposal stable key is not kebab-case".to_string());
        }
        let title = required_line(proposal.get("title"), "title")?;
        let target_path = proposal
            .get("evidence")
            .and_then(Value::as_array)
            .and_then(|evidence| evidence.first())
            .and_then(|evidence| evidence.get("file"))
            .and_then(Value::as_str)
            .ok_or_else(|| "Tier 2 proposal is missing target evidence".to_string())?
            .to_string();
        if !safe_issue_path(&target_path) {
            return Err("Tier 2 proposal target path is not safe to publish".to_string());
        }
        proposals.push(ProposalDraft {
            stable_key,
            title,
            target_path,
        });
    }
    Ok(proposals)
}

fn required_line(value: Option<&Value>, field: &str) -> Result<String, String> {
    let value = value
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Tier 2 proposal is missing {field}"))?;
    if value.is_empty() || value.chars().any(char::is_control) {
        return Err(format!("Tier 2 proposal {field} is not one line"));
    }
    Ok(value.to_string())
}

fn valid_stable_key(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 60
        && value.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
}

fn safe_issue_path(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 240
        && !value.starts_with('/')
        && !value.contains(['`', '\\'])
        && value
            .split('/')
            .all(|component| !component.is_empty() && !matches!(component, "." | ".."))
}

fn test_path_template(collector: &str) -> Result<(PathBuf, String), String> {
    let root: Value = serde_json::from_str(collector)
        .map_err(|error| format!("invalid Tier 2 collector JSON: {error}"))?;
    if let Some(path) = domain_file(&root, "test-surface") {
        let path = Path::new(path);
        let parent = path.parent().unwrap_or_else(|| Path::new("tests"));
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("test");
        return Ok((parent.to_path_buf(), extension.to_string()));
    }
    if domain_file(&root, "project-manifests") == Some("Cargo.toml")
        || any_domain_file(&root, "Cargo.toml")
    {
        return Ok((PathBuf::from("tests"), "rs".to_string()));
    }
    Ok((PathBuf::from("tests"), "test".to_string()))
}

fn test_command(collector: &str) -> String {
    let Ok(root) = serde_json::from_str::<Value>(collector) else {
        return "git diff --check".to_string();
    };
    for (manifest, command) in [
        ("package.json", "npm test"),
        ("Cargo.toml", "cargo test"),
        ("pom.xml", "mvn test"),
        ("build.gradle", "./gradlew test"),
        ("build.gradle.kts", "./gradlew test"),
    ] {
        if any_domain_file(&root, manifest) {
            return command.to_string();
        }
    }
    "git diff --check".to_string()
}

fn domain_file<'a>(root: &'a Value, domain_name: &str) -> Option<&'a str> {
    root.get("domains")?
        .as_array()?
        .iter()
        .find(|domain| domain.get("name").and_then(Value::as_str) == Some(domain_name))?
        .get("evidence")?
        .as_array()?
        .first()?
        .get("file")?
        .as_str()
}

fn any_domain_file(root: &Value, expected: &str) -> bool {
    root.get("domains")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|domain| {
            domain
                .get("evidence")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .any(|evidence| evidence.get("file").and_then(Value::as_str) == Some(expected))
}

fn marker(repo: &str, stable_key: &str) -> String {
    let identity = format!("autospec-tier2-publication-v1\0{repo}\0{stable_key}");
    format!("{MARKER_PREFIX}{} -->", sha256_hex(identity.as_bytes()))
}

fn render_draft(
    proposal: ProposalDraft,
    marker: String,
    receipt: &TierReceipt,
    test_template: &(PathBuf, String),
    test_command: &str,
) -> Result<PublicationDraft, String> {
    let mut test_path = test_template
        .0
        .join(format!("{}.{}", proposal.stable_key, test_template.1))
        .to_string_lossy()
        .into_owned();
    if test_path.len() > 80 || !safe_issue_path(&test_path) {
        test_path = format!("tests/{}.test", proposal.stable_key);
    }
    let receipt_marker = format!(
        "<!-- autospec:tier2-receipt:v1:{}:{} -->",
        receipt.reference(),
        receipt.digest()
    );
    let body = format!(
        "{marker}\n{receipt_marker}\n\n## Goal\n\nImplement `{key}` for `{target}`.\n\n## Acceptance criteria\n\n- [ ] `{key}` has `1` passing behavior scenario.\n- [ ] `{test_path}` covers the verified proposal.\n- [ ] `{test_command}` exits with status `0`.\n\n## Files to read first\n\n- `{target}`\n\n## Implementation outline\n\n- Read `{target}` to preserve its current behavior contract.\n- Add `{test_path}` using the project test conventions.\n\n## Tests required\n\n- project-native behavior\n\n## Test execution\n\n### Primary smoke test (inner loop)\n\n```bash\n{test_command}\n```\n\n## Files touched\n\n- `{target}`\n- `{test_path}`\n\n## Dependencies\n\nnone\n\n## Discovery evidence\n\n- Stable key: `{key}`\n- Source proposal: {title}\n",
        key = proposal.stable_key,
        target = proposal.target_path,
        title = proposal.title,
    );
    let findings = lint_issue_body(&body);
    if !findings.is_empty() {
        let summary = findings
            .iter()
            .map(|finding| format!("{}: {}", finding.rule_id(), finding.message))
            .collect::<Vec<_>>()
            .join("; ");
        return Err(format!(
            "Tier 2 proposal failed the issue-quality contract: {summary}"
        ));
    }
    admit_expected_implementation_contract(&body, &proposal.target_path, &test_path)?;
    Ok(PublicationDraft {
        stable_key: proposal.stable_key,
        title: proposal.title,
        body,
        labels: vec![
            "auto-implement",
            "origin:self",
            "ctx:32k",
            "reasoning:medium",
        ],
    })
}

fn admit_expected_implementation_contract(
    body: &str,
    target_path: &str,
    test_path: &str,
) -> Result<(), String> {
    let paths = [target_path, test_path]
        .into_iter()
        .collect::<BTreeSet<_>>();
    let expected = UnifiedDiff {
        files: paths
            .into_iter()
            .map(|path| DiffFile {
                path: path.to_string(),
                is_new: false,
                is_binary: false,
                hunks: Vec::new(),
            })
            .collect(),
    };
    let result = lint_issue_implementation_contract(&expected, body);
    if result.blocking_count == 0 {
        return Ok(());
    }
    let summary = result
        .findings
        .iter()
        .filter(|finding| finding.severity == ImplementationLintSeverity::Error)
        .map(|finding| {
            format!(
                "{}:{}: {}",
                finding.rule_id(),
                finding.path,
                finding.message
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    Err(format!(
        "Tier 2 proposal failed the implementation contract: {summary}"
    ))
}

fn required_labels(drafts: &[PublicationDraft]) -> BTreeSet<&'static str> {
    let mut labels = BTreeSet::new();
    for draft in drafts {
        labels.extend(draft.labels.iter().copied());
    }
    labels
}

fn ensure_label(repo: &str, label: &str) -> Result<(), String> {
    let endpoint = format!("repos/{repo}/labels/{label}");
    if run_gh_read_with_retry(&["api", "--method", "GET", &endpoint], "read label").is_ok() {
        return Ok(());
    }
    let (color, description) = label_metadata(label);
    let endpoint = format!("repos/{repo}/labels");
    let name = format!("name={label}");
    let color = format!("color={color}");
    let description = format!("description={description}");
    let create = run_gh(&[
        "api",
        "--method",
        "POST",
        &endpoint,
        "-f",
        &name,
        "-f",
        &color,
        "-f",
        &description,
    ])?;
    if create.status.success() {
        return Ok(());
    }
    let confirm_endpoint = format!("repos/{repo}/labels/{label}");
    if run_gh_read_with_retry(
        &["api", "--method", "GET", &confirm_endpoint],
        "confirm label creation",
    )
    .is_ok()
    {
        return Ok(());
    }
    require_success(&create, &format!("create label {label}"))
}

fn label_metadata(label: &str) -> (&'static str, &'static str) {
    match label {
        "auto-implement" => ("8250df", "Autospec autonomous implementation issue"),
        "origin:self" => ("8250df", "Discovered from local autonomous analysis"),
        "ctx:32k" => ("8250df", "Small-context implementation fit"),
        "reasoning:medium" => ("8250df", "Medium reasoning implementation fit"),
        _ => ("8250df", "Managed by Autospec"),
    }
}

pub(super) fn create_issue(
    repo_dir: &Path,
    repo: &str,
    draft: &PublicationDraft,
) -> Result<u64, String> {
    let arguments = create_issue_arguments(repo, draft);
    let output = Command::new("gh")
        .args(&arguments)
        .output()
        .map_err(|error| format!("could not execute gh: {error}"))?;
    require_success(&output, "create Tier 2 issue")?;
    let number = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<u64>()
        .ok()
        .filter(|number| *number > 0)
        .ok_or_else(|| "Tier 2 issue creation returned an invalid number".to_string())?;
    project_sync_issue(repo_dir, repo, number)?;
    Ok(number)
}

fn project_sync_issue(repo_dir: &Path, repo: &str, number: u64) -> Result<(), String> {
    let issue_url = format!("https://github.com/{repo}/issues/{number}");
    let result = Command::new(std::env::var("AUTOSPEC_BIN").unwrap_or_else(|_| "autospec".into()))
        .arg("project")
        .arg("sync")
        .arg("--repo-dir")
        .arg(repo_dir)
        .arg("--issue-url")
        .arg(&issue_url)
        .output();
    let output =
        result.map_err(|error| format!("could not execute managed Project sync: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    let diagnostic = String::from_utf8_lossy(&output.stderr);
    if diagnostic.contains("journaled_projection_pending:") {
        eprintln!(
            "WARNING: managed Project sync failed for {issue_url}; durable projection remains retryable"
        );
        return Ok(());
    }
    Err(format!(
        "managed Project sync failed before durable journaling for {issue_url}: {}",
        diagnostic.trim()
    ))
}

pub(super) fn create_issue_arguments(repo: &str, draft: &PublicationDraft) -> Vec<String> {
    let endpoint = format!("repos/{repo}/issues");
    let title = format!("title={}", draft.title);
    let body = format!("body={}", draft.body);
    let mut arguments = vec![
        "api".to_string(),
        "--method".to_string(),
        "POST".to_string(),
        endpoint,
        "-f".to_string(),
        title,
        "-f".to_string(),
        body,
    ];
    for label in &draft.labels {
        arguments.push("-f".to_string());
        arguments.push(format!("labels[]={label}"));
    }
    arguments.push("--jq".to_string());
    arguments.push(".number".to_string());
    arguments
}

fn run_gh(args: &[&str]) -> Result<Output, String> {
    Command::new("gh")
        .args(args)
        .output()
        .map_err(|error| format!("could not execute gh: {error}"))
}

fn require_success(output: &Output, operation: &str) -> Result<(), String> {
    if output.status.success() {
        return Ok(());
    }
    Err(format!(
        "{operation} failed: {}",
        String::from_utf8_lossy(&output.stderr).trim_end()
    ))
}

fn store_error(error: WaterfallStoreError) -> String {
    match error {
        WaterfallStoreError::Diagnostic(reason)
        | WaterfallStoreError::InvalidReceipt(reason)
        | WaterfallStoreError::InvalidState(reason) => reason,
    }
}

#[cfg(test)]
mod implementation_contract_tests {
    use autospec_core::autonomous::no_work::NoWorkTier;
    use autospec_core::autonomous::waterfall::{
        FunnelCounts, SealedEvidence, TierReceipt, TierStatus,
    };

    use super::{admit_expected_implementation_contract, render_draft, ProposalDraft};

    #[test]
    fn autonomous_discovery_issue_matches_implementation_lint() {
        let target_path = "src/status-panel.rs";
        let regression_path = "scripts/test-autonomous-status-panel.mjs";
        let malformed = format!(
            "## Goal\n\nFix the autonomous status panel.\n\n\
             ## Implementation outline\n\n- Update `{target_path}` for the status panel behavior.\n\n\
             ## Tests required\n\n- smoke\n\n\
             ## Files touched\n\n- `{target_path}`\n- `{regression_path}`\n"
        );

        let error =
            admit_expected_implementation_contract(&malformed, target_path, regression_path)
                .expect_err(
                    "an omitted project-native regression path must fail before publication",
                );
        assert!(error.contains("OUT_OF_SCOPE"), "{error}");
        assert!(
            !error.contains("MISSING_TEST"),
            "project-native regression evidence was misclassified: {error}"
        );

        let corrected = malformed.replace(
            &format!("- Update `{target_path}` for the status panel behavior."),
            &format!(
                "- Update `{target_path}` for the status panel behavior.\n- Verify `{regression_path}`."
            ),
        );
        admit_expected_implementation_contract(&corrected, target_path, regression_path)
            .expect("corrected outline and project-native regression evidence must publish");
    }

    #[test]
    fn render_draft_rejects_contract_failure_before_publication_draft() {
        let receipt = TierReceipt::new(
            "owner/repo",
            1,
            NoWorkTier::Tier2,
            "tier2-test",
            1,
            1,
            TierStatus::Produced { count: 1 },
            FunnelCounts::new(1, 1, 1, 1, 1).expect("valid funnel"),
            vec![
                SealedEvidence::new("evidence/tier2.json", "a".repeat(64)).expect("valid evidence")
            ],
        )
        .expect("valid receipt");
        let proposal = ProposalDraft {
            stable_key: "proposal-key".to_string(),
            title: "Proposal title".to_string(),
            target_path: "status-panel".to_string(),
        };

        let error = render_draft(
            proposal,
            "<!-- marker -->".to_string(),
            &receipt,
            &(std::path::PathBuf::from("tests"), "rs".to_string()),
            "cargo test",
        )
        .expect_err("an out-of-scope target must not produce a PublicationDraft");

        assert!(error.contains("OUT_OF_SCOPE"), "{error}");
    }
}
