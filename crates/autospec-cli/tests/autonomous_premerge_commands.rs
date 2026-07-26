use autospec_core::autonomous::premerge::{
    evaluate_premerge, EvidenceAvailability, EvidenceVerdict, PremergeDecision,
    PremergeLaneIdentity, QaEvidence, SecurityAuditEvidence,
};
use autospec_core::claim::RunStateRecord;
use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::os::unix::fs::symlink;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[path = "autonomous_premerge_authority.rs"]
mod authority;
static FIXTURE_SEQUENCE: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    root: PathBuf,
    repo_dir: PathBuf,
    claim_remote: PathBuf,
    state_root: PathBuf,
    bin_dir: PathBuf,
    poison_log: PathBuf,
    lane: PremergeLaneIdentity,
}
impl Fixture {
    fn new(branch: &str, issue: u64, worker_id: &str, claim_id: &str) -> Self {
        let sequence = FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "autospec-premerge-{}-{nanos}-{sequence}",
            std::process::id()
        ));
        let repo_dir = root.join("repo");
        let claim_remote = root.join("claim-remote.git");
        let state_root = root.join("state");
        let bin_dir = root.join("bin");
        let poison_log = root.join("poison.log");
        fs::create_dir_all(&repo_dir).expect("repo fixture directory");
        fs::create_dir_all(&state_root).expect("state fixture directory");
        fs::create_dir_all(&bin_dir).expect("bin fixture directory");
        git(&root, &["init", "--bare", claim_remote.to_str().unwrap()]);
        git(&repo_dir, &["init", "-b", branch]);
        git(&repo_dir, &["config", "user.name", "Autospec Test"]);
        git(
            &repo_dir,
            &["config", "user.email", "autospec@example.invalid"],
        );
        fs::write(repo_dir.join("tracked.txt"), "baseline\n").expect("tracked fixture");
        git(&repo_dir, &["add", "tracked.txt"]);
        git(&repo_dir, &["commit", "-m", "test fixture"]);
        let commit = git_stdout(&repo_dir, &["rev-parse", "HEAD"]);
        let lane =
            PremergeLaneIdentity::new("test/repo", issue, worker_id, claim_id, branch, commit)
                .expect("valid fixture lane");
        let claim = RunStateRecord::new(
            "test/repo",
            issue,
            worker_id,
            "claimed",
            branch,
            "",
            "claimed",
            Vec::new(),
            "2026-07-20T00:00:00Z",
            "2026-07-20T00:00:00Z",
            u64::MAX,
        )
        .with_claim_id(claim_id);
        let comments = json!([{
            "id": 100,
            "updated_at": "2026-07-20T00:00:00Z",
            "body": claim.to_marked_comment(),
        }]);
        write_executable(
            &bin_dir.join("gh"),
            b"#!/bin/sh\nprintf '%s\\n' \"$AUTOSPEC_TEST_CLAIM_COMMENTS\"\n",
        );
        for command in [
            "bash",
            "sh",
            "omx",
            "autospec-run",
            "autonomous-premerge-gate.sh",
        ] {
            write_executable(
                &bin_dir.join(command),
                format!(
                    "#!/bin/sh\nprintf '%s\\n' '{command}' >> \"$AUTOSPEC_TEST_POISON_LOG\"\nexit 91\n"
                )
                .as_bytes(),
            );
        }
        let fixture = Self {
            root,
            repo_dir,
            claim_remote,
            state_root,
            bin_dir,
            poison_log,
            lane,
        };
        fixture.write_claim_comments(comments.to_string());
        fixture.write_claim_ref(&claim);
        fixture
    }
    fn write_claim_comments(&self, comments: String) {
        fs::write(self.root.join("comments.json"), comments).expect("claim comments fixture");
    }
    fn comments(&self) -> String {
        fs::read_to_string(self.root.join("comments.json")).expect("claim comments fixture")
    }
    fn evidence_dir(&self) -> PathBuf {
        self.repo_dir
            .join(".autospec/evidence/premerge")
            .join(self.lane.lane_digest())
    }
    fn write_evidence(&self, qa: &QaEvidence, security: &SecurityAuditEvidence) {
        let directory = self.evidence_dir();
        fs::create_dir_all(&directory).expect("evidence directory");
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
            .expect("private evidence directory");
        let qa_path = directory.join("qa.json");
        let security_path = directory.join("security.json");
        fs::write(&qa_path, format!("{}\n", qa.to_json())).expect("QA evidence");
        fs::write(&security_path, format!("{}\n", security.to_json())).expect("security evidence");
        fs::set_permissions(qa_path, fs::Permissions::from_mode(0o600))
            .expect("private QA evidence");
        fs::set_permissions(security_path, fs::Permissions::from_mode(0o600))
            .expect("private security evidence");
    }
    fn write_claim_ref(&self, claim: &RunStateRecord) {
        let reference = format!("refs/autospec/claims/issue-{}", self.lane.issue);
        let current = git_stdout(
            &self.repo_dir,
            &[
                "ls-remote",
                "--refs",
                self.claim_remote.to_str().expect("UTF-8 claim remote"),
                &reference,
            ],
        )
        .split_whitespace()
        .next()
        .map(str::to_string);
        if let Some(parent) = &current {
            git(
                &self.repo_dir,
                &[
                    "fetch",
                    "--no-tags",
                    self.claim_remote.to_str().expect("UTF-8 claim remote"),
                    &reference,
                ],
            );
            assert_eq!(
                git_stdout(&self.repo_dir, &["rev-parse", "FETCH_HEAD"]),
                *parent
            );
        }
        let tree = git_stdout(&self.repo_dir, &["mktree"]);
        let mut command = Command::new("git");
        command
            .arg("commit-tree")
            .arg(tree)
            .current_dir(&self.repo_dir)
            .env("GIT_AUTHOR_NAME", "Autospec Premerge Test")
            .env("GIT_AUTHOR_EMAIL", "autospec-premerge-test@example.invalid")
            .env("GIT_COMMITTER_NAME", "Autospec Premerge Test")
            .env(
                "GIT_COMMITTER_EMAIL",
                "autospec-premerge-test@example.invalid",
            )
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped());
        if let Some(parent) = &current {
            command.args(["-p", parent]);
        }
        let mut child = command.spawn().expect("create claim ref commit");
        write!(
            child.stdin.take().expect("claim ref commit stdin"),
            "autospec-claim-ledger-v1\ngeneration=premerge-fixture\n\n{}\n",
            claim.to_marked_comment()
        )
        .expect("write claim ref commit");
        let output = child.wait_with_output().expect("finish claim ref commit");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let oid = String::from_utf8_lossy(&output.stdout).trim().to_string();
        git(
            &self.repo_dir,
            &[
                "push",
                self.claim_remote.to_str().expect("UTF-8 claim remote"),
                &format!("{oid}:{reference}"),
            ],
        );
    }
    fn run(&self) -> Output {
        Command::new(env!("CARGO_BIN_EXE_autospec"))
            .args([
                "autonomous",
                "premerge",
                "evaluate",
                "--repo",
                self.lane.repo.as_str(),
                "--repo-dir",
                self.repo_dir.to_str().expect("UTF-8 repo path"),
                "--issue",
                &self.lane.issue.to_string(),
                "--worker-id",
                self.lane.worker_id.as_str(),
                "--claim-id",
                self.lane.claim_id.as_str(),
                "--json",
            ])
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    self.bin_dir.display(),
                    std::env::var("PATH").expect("test PATH")
                ),
            )
            .env("AUTOSPEC_TEST_CLAIM_COMMENTS", self.comments())
            .env("AUTOSPEC_TEST_POISON_LOG", &self.poison_log)
            .env("AUTOSPEC_CLAIM_GIT_REMOTE", &self.claim_remote)
            .env(
                "AUTOSPEC_CLAIM_GIT_STATE_DIR",
                self.root.join("claim-git-state"),
            )
            .env("AUTOSPEC_CLAIM_LEASE_SECONDS", u64::MAX.to_string())
            .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &self.state_root)
            .output()
            .expect("premerge evaluate starts")
    }
    fn lane_state_dir(&self) -> PathBuf {
        self.state_root
            .join("test_repo/premerge/lanes")
            .join(self.lane.lane_digest())
    }
}
fn git(repo_dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_dir)
        .args(args)
        .output()
        .expect("git starts");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
fn git_stdout(repo_dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_dir)
        .args(args)
        .output()
        .expect("git starts");
    assert!(output.status.success(), "git {args:?}");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}
fn write_executable(path: &Path, contents: &[u8]) {
    fs::write(path, contents).expect("executable fixture");
    let mut permissions = fs::metadata(path).expect("fixture metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("fixture permissions");
}
fn qa(lane: &PremergeLaneIdentity, verdict: EvidenceVerdict) -> QaEvidence {
    QaEvidence {
        lane: lane.clone(),
        run_id: "qa-run-1".into(),
        completed_at: 1_800_000_000,
        verdict,
    }
}
fn security(lane: &PremergeLaneIdentity, verdict: EvidenceVerdict) -> SecurityAuditEvidence {
    SecurityAuditEvidence {
        lane: lane.clone(),
        run_id: "security-run-1".into(),
        completed_at: 1_800_000_001,
        verdict,
    }
}
fn decision_digest(
    lane: &PremergeLaneIdentity,
    qa: EvidenceAvailability<QaEvidence>,
    security: EvidenceAvailability<SecurityAuditEvidence>,
) -> String {
    match evaluate_premerge(lane, qa, security) {
        PremergeDecision::Pass {
            evidence_digest, ..
        }
        | PremergeDecision::Blocked {
            evidence_digest, ..
        }
        | PremergeDecision::Failed {
            evidence_digest, ..
        } => evidence_digest,
    }
}
fn json_output(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("command emits JSON")
}
#[test]
fn pass_uses_git_identity_fixed_evidence_and_closed_authority() {
    let fixture = Fixture::new("feat/lane-a", 42, "worker-42", "claim-42");
    let qa = qa(&fixture.lane, EvidenceVerdict::Pass);
    let security = security(&fixture.lane, EvidenceVerdict::Pass);
    fixture.write_evidence(&qa, &security);
    fs::create_dir_all(fixture.repo_dir.join(".autospec/evidence/elsewhere"))
        .expect("unrelated evidence directory");
    fs::write(
        fixture
            .repo_dir
            .join(".autospec/evidence/elsewhere/qa.json"),
        "not JSON",
    )
    .expect("unrelated poisoned evidence");
    let first = fixture.run();
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let body = json_output(&first);
    assert_eq!(body["schema"], 1);
    assert_eq!(body["decision"], "pass");
    assert_eq!(body["repo"], "test/repo");
    assert_eq!(body["branch"], fixture.lane.branch);
    assert_eq!(body["commit"], fixture.lane.commit);
    assert_eq!(body["lane_digest"], fixture.lane.lane_digest());
    assert_eq!(body["finding_codes"], json!([]));
    let digest = decision_digest(
        &fixture.lane,
        EvidenceAvailability::Present(qa),
        EvidenceAvailability::Present(security),
    );
    let lane_state = fixture.lane_state_dir();
    let decision = lane_state.join("decisions").join(format!("{digest}.json"));
    assert!(decision.is_file());
    assert!(lane_state.join("latest.json").is_file());
    assert!(!lane_state.join("quarantine.json").exists());
    let original = fs::read(&decision).expect("immutable decision");
    let second = fixture.run();
    assert!(second.status.success());
    assert_eq!(fs::read(&decision).expect("immutable decision"), original);
    assert_eq!(
        fs::read_dir(lane_state.join("decisions"))
            .expect("decision directory")
            .count(),
        1
    );
    assert!(!fixture.poison_log.exists());
}
#[test]
fn tracked_dirt_and_detached_head_fail_before_receipt_creation() {
    let dirty = Fixture::new("feat/dirty", 42, "worker-42", "claim-dirty");
    dirty.write_evidence(
        &qa(&dirty.lane, EvidenceVerdict::Pass),
        &security(&dirty.lane, EvidenceVerdict::Pass),
    );
    fs::write(dirty.repo_dir.join("tracked.txt"), "dirty\n").expect("tracked dirt");
    let output = dirty.run();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("dirty"));
    assert!(!dirty.lane_state_dir().exists());
    let staged = Fixture::new("feat/staged", 43, "worker-43", "claim-staged");
    fs::write(staged.repo_dir.join("staged.txt"), "staged only\n").expect("staged fixture");
    git(&staged.repo_dir, &["add", "staged.txt"]);
    let output = staged.run();
    assert_eq!(output.status.code(), Some(2));
    assert!(!staged.lane_state_dir().exists());
    let detached = Fixture::new("feat/detached", 54, "worker-54", "claim-detached");
    detached.write_evidence(
        &qa(&detached.lane, EvidenceVerdict::Pass),
        &security(&detached.lane, EvidenceVerdict::Pass),
    );
    git(&detached.repo_dir, &["checkout", "--detach"]);
    let output = detached.run();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("detached"));
    assert!(!detached.lane_state_dir().exists());
}
#[test]
fn missing_malformed_foreign_and_stale_claim_evidence_fail_without_quarantine() {
    let missing = Fixture::new("feat/missing", 44, "worker-44", "claim-missing");
    let output = missing.run();
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(json_output(&output)["decision"], "failed");
    assert!(!missing.lane_state_dir().join("quarantine.json").exists());
    let malformed = Fixture::new("feat/malformed", 45, "worker-45", "claim-malformed");
    fs::create_dir_all(malformed.evidence_dir()).expect("evidence directory");
    fs::write(malformed.evidence_dir().join("qa.json"), [0xff, 0xfe]).expect("malformed UTF-8");
    fs::write(
        malformed.evidence_dir().join("security.json"),
        security(&malformed.lane, EvidenceVerdict::Pass).to_json(),
    )
    .expect("security evidence");
    let output = malformed.run();
    assert_eq!(output.status.code(), Some(2));
    assert!(json_output(&output)["reason"]
        .as_str()
        .expect("failure reason")
        .contains("malformed"));
    assert!(!malformed.lane_state_dir().join("quarantine.json").exists());
    let foreign = Fixture::new("feat/foreign", 46, "worker-46", "claim-foreign");
    let other = PremergeLaneIdentity::new(
        "test/repo",
        46,
        "other-worker",
        "claim-foreign",
        "feat/foreign",
        &foreign.lane.commit,
    )
    .expect("foreign lane");
    foreign.write_evidence(
        &qa(&other, EvidenceVerdict::Pass),
        &security(&other, EvidenceVerdict::Pass),
    );
    let output = foreign.run();
    assert_eq!(output.status.code(), Some(2));
    assert!(json_output(&output)["reason"]
        .as_str()
        .expect("failure reason")
        .contains("mismatch"));
    assert!(!foreign.lane_state_dir().join("quarantine.json").exists());
    for (repo, issue, worker, claim_id, branch) in [
        ("other/repo", 47, "worker-47", "claim-47", "feat/claim"),
        ("test/repo", 99, "worker-47", "claim-47", "feat/claim"),
        ("test/repo", 47, "other-worker", "claim-47", "feat/claim"),
        ("test/repo", 47, "worker-47", "other-claim", "feat/claim"),
        ("test/repo", 47, "worker-47", "claim-47", "feat/other"),
    ] {
        let fixture = Fixture::new("feat/claim", 47, "worker-47", "claim-47");
        let claim = RunStateRecord::new(
            repo,
            issue,
            worker,
            "claimed",
            branch,
            "",
            "claimed",
            Vec::new(),
            "2026-07-20T00:00:00Z",
            "2026-07-20T00:00:00Z",
            u64::MAX,
        )
        .with_claim_id(claim_id);
        fixture.write_claim_comments(
            json!([{"id":100,"updated_at":"2026-07-20T00:00:00Z",
                "body":claim.to_marked_comment()}])
            .to_string(),
        );
        fixture.write_claim_ref(&claim);
        let output = fixture.run();
        assert_eq!(output.status.code(), Some(2));
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("active claim") || stderr.contains("claim ref record does not match"),
            "claim tuple ({repo}, {issue}, {worker}, {claim_id}, {branch}): {}",
            stderr
        );
        assert!(!fixture.lane_state_dir().exists());
    }
}
#[test]
fn fixed_evidence_symlinks_never_escape_the_canonical_repository() {
    for (offset, filename) in ["qa.json", "security.json"].into_iter().enumerate() {
        let fixture = Fixture::new(
            "feat/symlink",
            60 + offset as u64,
            "worker-link",
            "claim-link",
        );
        fixture.write_evidence(
            &qa(&fixture.lane, EvidenceVerdict::Pass),
            &security(&fixture.lane, EvidenceVerdict::Pass),
        );
        let evidence = fixture.evidence_dir().join(filename);
        let outside = fixture.root.join(filename);
        fs::rename(&evidence, &outside).expect("move evidence outside lane");
        symlink(&outside, &evidence).expect("outside evidence symlink");
        let output = fixture.run();
        assert_eq!(output.status.code(), Some(2));
        assert!(!fixture.lane_state_dir().exists());
    }
}
#[test]
fn blocking_quarantine_is_lane_scoped_and_passing_lane_is_unaffected() {
    let blocked = Fixture::new("feat/lane-blocked", 48, "worker-48", "claim-48");
    blocked.write_evidence(
        &qa(
            &blocked.lane,
            EvidenceVerdict::Blocked {
                finding_codes: vec!["QA-RED".into()],
            },
        ),
        &security(&blocked.lane, EvidenceVerdict::Pass),
    );
    let output = blocked.run();
    assert_eq!(output.status.code(), Some(20));
    assert_eq!(json_output(&output)["decision"], "blocked");
    let quarantine = blocked.lane_state_dir().join("quarantine.json");
    assert!(quarantine.is_file());
    let passing = Fixture::new("feat/lane-passing", 49, "worker-49", "claim-49");
    passing.write_evidence(
        &qa(&passing.lane, EvidenceVerdict::Pass),
        &security(&passing.lane, EvidenceVerdict::Pass),
    );
    let output = passing.run();
    assert!(output.status.success());
    assert_eq!(json_output(&output)["decision"], "pass");
    assert!(!passing.lane_state_dir().join("quarantine.json").exists());
    assert_eq!(
        json_output(&blocked.run())["finding_codes"],
        json!(["QA-RED"])
    );
    assert!(quarantine.is_file());
}
#[test]
fn an_existing_decision_with_different_contents_is_never_replaced() {
    let fixture = Fixture::new("feat/immutable", 50, "worker-50", "claim-50");
    let qa = qa(&fixture.lane, EvidenceVerdict::Pass);
    let security = security(&fixture.lane, EvidenceVerdict::Pass);
    fixture.write_evidence(&qa, &security);
    let digest = decision_digest(
        &fixture.lane,
        EvidenceAvailability::Present(qa),
        EvidenceAvailability::Present(security),
    );
    let decisions = fixture.lane_state_dir().join("decisions");
    fs::create_dir_all(&decisions).expect("decision directory");
    let decision = decisions.join(format!("{digest}.json"));
    fs::write(&decision, "poisoned immutable receipt\n").expect("poisoned receipt");
    let output = fixture.run();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("immutable"));
    assert_eq!(
        fs::read_to_string(&decision).expect("poisoned receipt retained"),
        "poisoned immutable receipt\n"
    );
    assert!(!fixture.lane_state_dir().join("latest.json").exists());
}
#[test]
fn evaluate_rejects_unknown_duplicate_and_extra_arguments() {
    let fixture = Fixture::new("feat/args", 51, "worker-51", "claim-51");
    let base = [
        "autonomous",
        "premerge",
        "evaluate",
        "--repo",
        "test/repo",
        "--repo-dir",
        fixture.repo_dir.to_str().expect("UTF-8 repo path"),
        "--issue",
        "51",
        "--worker-id",
        "worker-51",
        "--claim-id",
        "claim-51",
    ];
    for suffix in [
        vec!["--repo", "test/repo"],
        vec!["--repo-dir", fixture.repo_dir.to_str().unwrap()],
        vec!["--issue", "51"],
        vec!["--worker-id", "worker-51"],
        vec!["--claim-id", "claim-51"],
        vec!["--json", "--json"],
        vec!["--producer-command", "bash"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_autospec"))
            .args(base)
            .args(suffix)
            .output()
            .expect("premerge evaluate starts");
        assert_eq!(output.status.code(), Some(2));
    }
}
