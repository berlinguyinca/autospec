// executor_bridge tests: shared fixtures (3 of 3).
//
// Split out of tests.rs; see the note in that file. These are the helpers more than
// one test module builds on, so they are `pub(super)` rather than private.

use crate::commands::autonomous::executor_bridge as bridge;
use super::super::{BridgePhase, PersistedInvocation};
use super::support_base::{git, git_stdout, write_executable, GitFixture};
use super::support_invocation::{
    commit_implementation, implementation_proof_fixture, persisted_invocation,
};
use std::collections::BTreeMap;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[cfg(unix)]
pub(super) fn draft_pr_adapter_fixture(
    fixture: &GitFixture,
    state_path: &Path,
    created_pull_request: &str,
) -> bridge::DraftPrAdapter {
    let gh = fixture.root.join("gh");
    let pull_requests = fixture.root.join("pull-requests.json");
    let created = fixture.root.join("created-pull-request.json");
    let closed = fixture.root.join("closed-pull-request.json");
    let calls = fixture.root.join("gh-calls");
    fs::write(&pull_requests, "[]").expect("empty PR snapshot");
    fs::write(&created, created_pull_request).expect("created PR fixture");
    let closed_document = serde_json::from_str::<serde_json::Value>(created_pull_request)
        .ok()
        .and_then(|value| value.as_array().and_then(|items| items.first()).cloned())
        .map(|pull_request| {
            serde_json::json!({
                "number": pull_request["number"],
                "state": "CLOSED",
                "mergedAt": null,
                "headRefName": pull_request["headRefName"],
                "headRefOid": pull_request["headRefOid"],
                "baseRefName": pull_request["baseRefName"],
                "body": pull_request["body"],
            })
        })
        .unwrap_or_else(|| serde_json::json!({}));
    fs::write(&closed, closed_document.to_string()).expect("closed PR fixture");
    fs::write(
        &gh,
        "#!/bin/sh\n\
         set -eu\n\
         printf '%s\\n' \"$*\" >> \"$GH_CALLS\"\n\
         if [ \"$1 $2\" = \"pr list\" ]; then cat \"$GH_PR_STATE\"; exit 0; fi\n\
         if [ \"$1 $2\" = \"pr view\" ]; then cat \"$GH_CLOSED_PR\"; exit 0; fi\n\
         if [ \"$1 $2\" = \"pr ready\" ]; then cp \"$GH_READY_PR\" \"$GH_PR_STATE\"; exit 0; fi\n\
         if [ \"$1 $2\" = \"pr close\" ]; then printf '%s\\n' '[]' > \"$GH_PR_STATE\"; exit 0; fi\n\
         if [ \"$1 $2\" = \"pr create\" ]; then\n\
           grep -q '\"phase\":\"draft_creating\"' \"$BRIDGE_STATE\"\n\
           if [ -n \"${GH_REQUIRE_DURABLE_RELEASE:-}\" ]; then\n\
             grep -q '\"draft_process\":{' \"$BRIDGE_STATE\"\n\
             test -s \"$AUTOSPEC_DRAFT_RELEASE_RECEIPT\"\n\
           fi\n\
           if [ -n \"${GH_CREATE_DELAY:-}\" ]; then\n\
             if ! mkdir \"$GH_INFLIGHT\" 2>/dev/null; then exit 65; fi\n\
             touch \"$GH_CREATE_STARTED\"\n\
             while [ -e \"$GH_CREATE_DELAY\" ]; do sleep 0.02; done\n\
             rmdir \"$GH_INFLIGHT\"\n\
           fi\n\
           if [ -n \"${GH_MUTATE_REF:-}\" ]; then git --git-dir \"$GH_REMOTE\" update-ref refs/tags/during-create \"$GH_MUTATE_REF\"; fi\n\
           if [ -n \"${GH_MUTATE_PULL_REF:-}\" ]; then git --git-dir \"$GH_REMOTE\" update-ref refs/pull/17/head \"$GH_MUTATE_PULL_REF\"; fi\n\
           cp \"$GH_CREATED_PR\" \"$GH_PR_STATE\"\n\
           printf 'https://example.invalid/pull/17\\n'\n\
           exit 0\n\
         fi\n\
         exit 64\n",
    )
    .expect("gh fixture");
    fs::set_permissions(&gh, fs::Permissions::from_mode(0o755)).expect("executable gh");
    bridge::DraftPrAdapter {
        gh,
        environment: BTreeMap::from([
            ("GH_CALLS".into(), calls.into_os_string()),
            ("GH_PR_STATE".into(), pull_requests.into_os_string()),
            ("GH_CREATED_PR".into(), created.into_os_string()),
            ("GH_CLOSED_PR".into(), closed.into_os_string()),
            ("BRIDGE_STATE".into(), state_path.as_os_str().to_os_string()),
            (
                "GH_REMOTE".into(),
                fixture.root.join("remote.git").into_os_string(),
            ),
        ]),
    }
}

#[cfg(unix)]
pub(super) struct PreparedDraftTransaction {
    pub(super) fixture: GitFixture,
    pub(super) state: PersistedInvocation,
    pub(super) proof: bridge::ImplementationProof,
    pub(super) adapter: bridge::DraftPrAdapter,
    pub(super) state_path: PathBuf,
}

#[cfg(unix)]
pub(super) const DRAFT_ISSUE_BODY: &str =
    "## Implementation outline\n\n- implementation.txt\n- .autospec/executor-closeout.md\n";

#[cfg(unix)]
impl PreparedDraftTransaction {
    pub(super) fn bind_continuation(&mut self) {
        self.state.umbrella = Some(42);
        self.state.current_child = Some(101);
        bridge::write_invocation_atomic(&self.state_path, &self.state).unwrap();
        let path = adapter_path(&self.adapter, "GH_CREATED_PR");
        let created = fs::read_to_string(&path)
            .unwrap()
            .replace("Closes #42", "Part of #42\\n\\nCloses #101");
        fs::write(path, created).unwrap();
    }

    pub(super) fn push_exact_at_intent(&mut self) {
        self.state.phase = BridgePhase::BranchPushing;
        bridge::write_invocation_atomic(&self.state_path, &self.state).expect("push intent");
        git(
            &self.state.identity.worktree,
            &[
                "push",
                "origin",
                &format!(
                    "{}:refs/heads/{}",
                    self.proof.head_oid, self.state.identity.branch
                ),
            ],
        );
    }

    pub(super) fn publish(&mut self) -> Result<u64, String> {
        bridge::push_and_create_draft(
            &self.state_path,
            &mut self.state,
            &self.proof,
            "Implement issue",
            DRAFT_ISSUE_BODY,
            &self.adapter,
        )
        .map_err(|error| error.to_string())
    }
}

#[cfg(unix)]
pub(super) fn prepared_draft_transaction(label: &str) -> PreparedDraftTransaction {
    let (fixture, mut state, snapshot, closeout) = implementation_proof_fixture(label);
    let state_path = fixture.root.join("state/invocation.json");
    let empty_adapter = draft_pr_adapter_fixture(&fixture, &state_path, "[]");
    state.phase = BridgePhase::Pending;
    bridge::write_invocation_atomic(&state_path, &state).expect("pending invocation");
    bridge::RemoteMutationSnapshot::capture_and_persist(&state_path, &mut state, &empty_adapter)
        .expect("prelaunch remote");
    state.phase = BridgePhase::ImplementationComplete;
    let snapshot_path = state_path.with_extension("prelaunch-remote.json");
    assert_eq!(
        fs::metadata(&snapshot_path)
            .expect("remote snapshot metadata")
            .permissions()
            .mode()
            & 0o777,
        0o600,
        "remote snapshot must publish privately"
    );
    assert!(
        fs::read_dir(snapshot_path.parent().expect("snapshot parent"))
            .expect("snapshot directory")
            .all(|entry| !entry
                .expect("snapshot directory entry")
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp")),
        "atomic snapshot publication must not strand a temporary file"
    );
    commit_implementation(&state);
    let proof = bridge::prove_implementation(&state_path, &mut state, &snapshot, &closeout)
        .expect("prove implementation");
    let body = format!("Closes #42\n\n{}", proof.closeout_body);
    let created = format!(
        "[{{\"number\":17,\"body\":{},\"headRefName\":\"{}\",\"headRefOid\":\"{}\",\"isDraft\":true,\"baseRefName\":\"main\"}}]",
        serde_json::to_string(&body).unwrap(),
        state.identity.branch,
        proof.head_oid,
    );
    let adapter = draft_pr_adapter_fixture(&fixture, &state_path, &created);
    PreparedDraftTransaction {
        fixture,
        state,
        proof,
        adapter,
        state_path,
    }
}

#[cfg(unix)]
pub(super) fn adapter_path(adapter: &bridge::DraftPrAdapter, key: &str) -> PathBuf {
    PathBuf::from(
        adapter
            .environment
            .get(std::ffi::OsStr::new(key))
            .expect("adapter path"),
    )
}

pub(super) fn direct_launch_supervisor_pid(path: &Path) -> u32 {
    serde_json::from_str::<serde_json::Value>(
        &fs::read_to_string(path).expect("durable direct launch identity"),
    )
    .expect("direct launch JSON")["supervisor"]["pid"]
        .as_u64()
        .and_then(|pid| u32::try_from(pid).ok())
        .expect("direct launch supervisor PID")
}

pub(super) fn automatic_review_command(
    executable: &Path,
    capture_root: &Path,
    identity_digest: &str,
) -> bridge::DirectCommand {
    fs::create_dir_all(capture_root).expect("review capture root");
    fs::set_permissions(capture_root, fs::Permissions::from_mode(0o700))
        .expect("private review capture root");
    let artifacts =
        ["inner.stdout", "inner.stderr", "result.txt"].map(|name| capture_root.join(name));
    for path in &artifacts {
        fs::write(path, b"").expect("review capture artifact");
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .expect("private review capture artifact");
    }
    bridge::DirectCommand {
        argv: vec![executable.display().to_string()],
        accepted_exit_codes: vec![0],
        identity_digest: Some(identity_digest.to_string()),
        review_capture: Some(bridge::ReviewerCapturePolicy { artifacts }),
    }
}

pub(super) fn direct_failure_archive_count(root: &Path) -> usize {
    fs::read_dir(root)
        .expect("direct evidence root")
        .flatten()
        .filter(|entry| {
            entry.file_name().to_string_lossy().contains(".archive-")
                && entry.path().join("complete").is_file()
        })
        .count()
}

pub(super) fn rewrite_direct_terminal_as_signal(evidence: &Path, signal: i32) {
    let record = evidence.join("command-000.json");
    let mut value: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&record).expect("direct failure record"))
            .expect("direct failure record JSON");
    value["terminal"] = serde_json::json!({"kind": "signaled", "signal": signal});
    fs::write(&record, value.to_string()).expect("rewrite direct terminal");
}

pub(super) fn completed_generation_bundle(
    fixture: &GitFixture,
    lane: &bridge::PremergeLaneIdentity,
    lane_root: &Path,
    generation: u64,
    execution_count: &Path,
) -> bridge::ObservedEvidenceBundle {
    let input_digest = autospec_core::autonomous::waterfall::sha256_hex(
        format!("test-session-generation-{generation}").as_bytes(),
    );
    let attempt_root = lane_root.join("attempts").join(&input_digest[..24]);
    bridge::ensure_private_directory(&attempt_root).expect("attempt root");
    let base_oid = git_stdout(&fixture.repo, &["rev-parse", "HEAD"]);
    let intent_body = serde_json::json!({
        "schema": 2,
        "lane_digest": lane.lane_digest(),
        "base_oid": base_oid,
        "semantic_input_digest": "test-semantic-input",
        "input_digest": input_digest,
        "run_id": format!("test-run-{generation}"),
        "completed_at": 1_800_000_000 + generation,
        "runtime_session_id": serde_json::Value::Null,
        "runtime_environment_dir": serde_json::Value::Null,
    })
    .to_string();
    bridge::write_private_create_once(
        &attempt_root.join("intent.json"),
        intent_body.as_bytes(),
        "test evidence intent",
    )
    .expect("intent");
    let intent = bridge::EvidenceIntent {
        completed_at: 1_800_000_000 + generation,
        digest: autospec_core::autonomous::waterfall::sha256_hex(intent_body.as_bytes()),
        base_oid: base_oid.clone(),
        runtime_session_id: None,
        runtime_environment_dir: None,
        attempt_root: attempt_root.clone(),
    };
    let qa_plan = bridge::parse_direct_command_plan(&format!(
        "/usr/bin/python3 -c 'from pathlib import Path; p=Path(\"{}\"); p.write_text(str(int(p.read_text())+1) if p.exists() else \"1\")'",
        execution_count.display()
    ))
    .expect("QA plan");
    let qa = bridge::execute_direct_plan(
        &fixture.repo,
        &qa_plan,
        &attempt_root.join("qa"),
        None,
        Duration::from_secs(5),
    )
    .expect("live QA observation");

    let scanner_root = fixture.root.join("scanner-binaries");
    fs::create_dir_all(&scanner_root).expect("scanner bin root");
    let scanner_source = "#!/usr/bin/python3\nimport os,sys\nname=os.path.basename(sys.argv[0])\nif name == 'gitleaks':\n p=sys.argv[sys.argv.index('--report-path')+1]; open(p,'w').write('[]')\nelif name == 'semgrep': print('{\"results\":[],\"errors\":[],\"paths\":{\"scanned\":[\"feature.js\"],\"skipped\":[]}}')\nelif name == 'trivy': print('{\"Results\":[{\"Target\":\".\"}]}')\nelse: print('{\"fixture@1.0.0\":{\"licenses\":\"MIT\"}}')\n";
    let mut scanner_paths = BTreeMap::new();
    for scanner in ["gitleaks", "semgrep", "trivy", "license-checker"] {
        let path = scanner_root.join(scanner);
        if !path.exists() {
            fs::write(&path, scanner_source).expect("scanner executable");
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
                .expect("scanner executable mode");
        }
        scanner_paths.insert(scanner.to_string(), path);
    }
    let scanner_executables = bridge::ScannerExecutables {
        paths: scanner_paths,
    };
    let scanners = bridge::run_required_scanners(
        &fixture.repo,
        &base_oid,
        &attempt_root.join("security"),
        &scanner_executables,
        None,
        Duration::from_secs(5),
    )
    .expect("live scanner observations");
    let (qa_evidence, security_evidence) = bridge::typed_evidence_from_observed(
        &fixture.repo,
        &base_oid,
        lane,
        Ok(&qa),
        Ok(&scanners),
        Some("PASS"),
        intent.completed_at,
    );
    let mut bundle = bridge::observed_evidence_bundle(
        &fixture.repo,
        &base_oid,
        lane_root,
        &intent,
        qa_evidence,
        security_evidence,
        &qa_plan,
        &qa,
        &scanner_executables,
        &scanners,
        None,
    )
    .expect("observed bundle");
    bundle
        .mark_cleanup_verified()
        .expect("verified no-runtime cleanup");
    bundle
}

pub(super) fn run_process_generation_producer(
    repo: &Path,
    execution_count: &Path,
    scanner_root: &Path,
) -> Result<bridge::DeterministicEvidenceOutcome, String> {
    let repo = repo.canonicalize().expect("canonical generation repo");
    let commit = git_stdout(&repo, &["rev-parse", "HEAD"]);
    let mut state = persisted_invocation();
    state.identity.repository = "test/repo".to_string();
    state.identity.issue = 42;
    state.identity.worker_id = "worker".to_string();
    state.identity.claim_id = "claim".to_string();
    state.identity.repository_path = repo.clone();
    state.identity.worktree = repo.clone();
    state.identity.branch = "main".to_string();
    state.identity.base_ref = "origin/main".to_string();
    state.identity.base_oid = git_stdout(&repo, &["rev-parse", "origin/main"]);
    state.phase = bridge::BridgePhase::DraftCreated;
    state.head_oid = Some(commit.clone());
    let proof = bridge::ImplementationProof {
        head_oid: commit.clone(),
        closeout_body: String::new(),
    };
    let lane = bridge::PremergeLaneIdentity::new(
        state.identity.repository.clone(),
        state.identity.issue,
        state.identity.worker_id.clone(),
        state.identity.claim_id.clone(),
        state.identity.branch.clone(),
        commit,
    )?;
    let artifact_root = repo
        .join(".autospec/evidence/premerge")
        .join(lane.lane_digest());
    fs::create_dir_all(scanner_root).map_err(|error| format!("create scanner root: {error}"))?;
    let scanner_source = "#!/usr/bin/python3\nimport os,sys\nname=os.path.basename(sys.argv[0])\nif name == 'gitleaks':\n p=sys.argv[sys.argv.index('--report-path')+1]; open(p,'w').write('[]')\nelif name == 'semgrep': print('{\"results\":[],\"errors\":[],\"paths\":{\"scanned\":[\"feature.js\"],\"skipped\":[]}}')\nelif name == 'trivy': print('{\"Results\":[{\"Target\":\".\"}]}')\nelse: print('{\"fixture@1.0.0\":{\"licenses\":\"MIT\"}}')\n";
    let mut scanner_paths = BTreeMap::new();
    for scanner in ["gitleaks", "semgrep", "trivy", "license-checker"] {
        let path = scanner_root.join(scanner);
        if !path.exists() {
            write_executable(&path, scanner_source);
        }
        scanner_paths.insert(scanner.to_string(), path);
    }
    let scanners = bridge::ScannerExecutables {
        paths: scanner_paths,
    };
    let issue_body = format!(
        "## Goal\n\nRun exact local evidence.\n\n## Files to read first\n\n- `README.md`\n\n## Implementation outline\n\n- `.gitignore`\n- `tests/smoke/generation.sh`\n\n## Tests required\n\n- smoke\n\n### Primary smoke test (inner loop)\n\n```bash\n/usr/bin/python3 -c 'from pathlib import Path; p=Path(\"{}\"); p.write_text(str(int(p.read_text())+1) if p.exists() else \"1\")'\n```\n\n### Operator/full verification\n\n```bash\n/usr/bin/true\n```\n",
        execution_count.display()
    );
    bridge::produce_deterministic_premerge_evidence(bridge::DeterministicEvidenceRequest {
        state: &state,
        proof: &proof,
        review_requirements: autospec_core::autonomous::review_policy::classify_review_requirements(
            &autospec_core::autonomous::review_policy::ReviewPolicyInput::default(),
        ),
        issue_body: &issue_body,
        spec_documents: &[],
        env: &BTreeMap::new(),
        scanners: &scanners,
        artifact_root: &artifact_root,
        runtime: None,
        model_output: Some("PASS"),
        stall_timeout: Duration::from_secs(5),
    })
    .map_err(|error| error.to_string())
}
