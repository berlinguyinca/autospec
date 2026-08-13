// claim tests: shared fixtures.
//
// Split out of tests.rs; see the note in that file. These are the helpers more
// than one module builds on, so they are `pub(super)` rather than private.

use super::super::{ClaimRefAdvance, advance_claim_ref_in, lifecycle_claim_evidence_from_record};
use autospec_core::autonomous_lifecycle::{ClaimBranch, ClaimEvidence, IssueNumber, RepositoryScope, WorkerId};
use autospec_core::claim::RunStateRecord;
#[cfg(unix)]
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Barrier, Mutex};
use crate::commands::claim;

pub(super) static BRIDGE_TRANSITION_ENV: Mutex<()> = Mutex::new(());

pub(super) static STARTUP_HEARTBEAT_ENV: Mutex<()> = Mutex::new(());

pub(super) fn startup_heartbeat_fixture(label: &str) -> (PathBuf, PathBuf) {
    let directory = std::env::temp_dir().join(format!(
        "autospec-startup-heartbeat-{label}-{}-{}",
        std::process::id(),
        claim::UNIQUE_ID_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&directory).expect("startup heartbeat fixture");
    #[cfg(unix)]
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))
        .expect("private startup heartbeat fixture");
    let path = directory.join("42.json");
    (directory, path)
}

#[cfg(unix)]
pub(super) fn assert_fifo_reader_nonblocking(
    fifo: &Path,
    reader: impl FnOnce() -> std::io::Result<claim::RegularFileSnapshot> + Send + 'static,
) {
    let (send, receive) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        let _ = send.send(reader());
    });
    match receive.recv_timeout(std::time::Duration::from_secs(2)) {
        Ok(result) => assert!(result.is_err(), "FIFO was accepted as a regular file"),
        Err(error) => {
            if let Ok(writer) = nix::fcntl::open(
                fifo,
                nix::fcntl::OFlag::O_WRONLY | nix::fcntl::OFlag::O_NONBLOCK,
                nix::sys::stat::Mode::empty(),
            ) {
                drop(writer);
            }
            if receive
                .recv_timeout(std::time::Duration::from_secs(2))
                .is_ok()
            {
                let _ = reader.join();
            }
            panic!("heartbeat FIFO reader blocked: {error}");
        }
    }
    reader.join().unwrap();
}

#[cfg(target_os = "linux")]
pub(super) fn anchored_startup_heartbeat_fixture(
    label: &str,
) -> (
    PathBuf,
    PathBuf,
    PathBuf,
    std::fs::File,
    Box<claim::StartupHeartbeatSnapshot>,
) {
    use nix::fcntl::{open, OFlag};
    use nix::sys::stat::Mode;

    let (parent_path, _) = startup_heartbeat_fixture(label);
    let repo_path = parent_path.join("repo");
    std::fs::create_dir(&repo_path).unwrap();
    std::fs::set_permissions(&repo_path, std::fs::Permissions::from_mode(0o700)).unwrap();
    let source = repo_path.join("42.json");
    std::fs::write(
        &source,
        startup_heartbeat_document("host:user:rust:4242:nonce-a", 0),
    )
    .unwrap();
    std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o600)).unwrap();
    let snapshot = expired_heartbeat_snapshot(&source);
    let parent = std::fs::File::from(
        open(
            &parent_path,
            OFlag::O_PATH | OFlag::O_DIRECTORY,
            Mode::empty(),
        )
        .unwrap(),
    );
    let repo = claim::open_heartbeat_directory_beneath(&parent, Path::new("repo")).unwrap();
    (parent_path, repo_path, source, repo, snapshot)
}

pub(super) fn startup_heartbeat_document(worker: &str, pid: u32) -> String {
    let nonce = claim::startup_heartbeat_nonce("owner/repo", 42, "claim-a");
    format!(
        r#"{{"repo":"owner/repo","issue":"42","worker_id":"{worker}","branch":"feat/worker","pr":"","claim_id":"claim-a","step":"claimed","ts":100,"ttl_seconds":10,"pid":{pid},"nonce":"{nonce}","host":"host-a","boot_id":"boot-a","process_start":"1"}}"#
    )
}

pub(super) fn expected_startup_heartbeat<'a>(
    worker_id: &'a str,
) -> claim::StartupHeartbeatExpectation<'a> {
    claim::StartupHeartbeatExpectation {
        repo: "owner/repo",
        issue: 42,
        worker_id,
        branch: "feat/worker",
        pull_request: "",
        claim_id: "claim-a",
        step: "claimed",
    }
}

pub(super) fn inject_heartbeat_boundary(
    observed: &str,
    target: &str,
    message: &str,
) -> Result<(), claim::CommandFailure> {
    if observed == target {
        Err(claim::CommandFailure::diagnostic(message))
    } else {
        Ok(())
    }
}

#[cfg(unix)]
pub(super) fn expired_heartbeat_snapshot(path: &Path) -> Box<claim::StartupHeartbeatSnapshot> {
    let worker = "host:user:rust:4242:nonce-a";
    std::fs::write(path, startup_heartbeat_document(worker, 4242))
        .expect("write expired heartbeat");
    let classified = claim::classify_startup_heartbeat(
        path,
        expected_startup_heartbeat(worker),
        200,
        |_, _, _, _, _| claim::StartupPidLiveness::Dead,
    );
    let claim::StartupHeartbeatClassification::ExpiredDead(snapshot) = classified else {
        panic!("fixture heartbeat was not expired and dead");
    };
    snapshot
}

#[cfg(unix)]
pub(super) fn heartbeat_copy_path(root: &Path) -> PathBuf {
    let nonce = claim::startup_heartbeat_nonce("owner/repo", 42, "claim-a");
    root.join(format!(
        "quarantine/startup-heartbeats/42-{}.json",
        claim::heartbeat_session_key(&nonce)
    ))
}

#[cfg(unix)]
pub(super) fn heartbeat_handoff_count(root: &Path) -> usize {
    std::fs::read_dir(root.join("quarantine/startup-heartbeat-handoffs"))
        .expect("handoff directory")
        .count()
}

#[cfg(unix)]
pub(super) fn write_new_heartbeat_at(directory: &impl std::os::fd::AsFd, document: &[u8]) {
    let fd = nix::fcntl::openat(
        directory,
        "42.json",
        nix::fcntl::OFlag::O_WRONLY | nix::fcntl::OFlag::O_CREAT | nix::fcntl::OFlag::O_EXCL,
        nix::sys::stat::Mode::from_bits_truncate(0o600),
    )
    .expect("publish live replacement");
    std::fs::File::from(fd)
        .write_all(document)
        .expect("write replacement");
}

#[cfg(unix)]
pub(super) fn drift_heartbeat_at(directory: &impl std::os::fd::AsFd, name: &str) {
    let fd = nix::fcntl::openat(
        directory,
        name,
        nix::fcntl::OFlag::O_WRONLY | nix::fcntl::OFlag::O_TRUNC,
        nix::sys::stat::Mode::empty(),
    )
    .expect("open moved heartbeat");
    std::fs::File::from(fd).write_all(b"drift").unwrap();
}

#[cfg(target_os = "linux")]
pub(super) fn mutate_retained(path: &Path, source: &Path, mutation: &str) {
    match mutation.trim_start_matches("cleanup-") {
        "content" => std::fs::write(path, b"drift"),
        "mode" => std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o640)),
        "binding" => std::fs::rename(path, path.with_extension("moved")),
        "source" => std::fs::write(source, b"foreign"),
        _ => unreachable!(),
    }
    .unwrap();
}

#[cfg(unix)]
pub(super) fn assert_mode(path: &Path, expected: u32) {
    let permissions = std::fs::metadata(path)
        .expect("private path metadata")
        .permissions();
    assert_eq!(permissions.mode() & 0o777, expected);
}

pub(super) struct ClaimRefFixture {
    pub(super) root: PathBuf,
    pub(super) remote: PathBuf,
    pub(super) clients: [PathBuf; 2],
}

impl ClaimRefFixture {
    pub(super) fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "autospec-claim-ref-{label}-{}-{}",
            std::process::id(),
            claim::UNIQUE_ID_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let remote = root.join("remote.git");
        std::fs::create_dir_all(&root).expect("claim ref fixture root");
        git(&root, &["init", "--bare", remote.to_str().unwrap()]);
        let clients = [root.join("client-a"), root.join("client-b")];
        for client in &clients {
            git(&root, &["init", client.to_str().unwrap()]);
        }
        Self {
            root,
            remote,
            clients,
        }
    }
}

impl Drop for ClaimRefFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

pub(super) fn git(directory: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(directory)
        .output()
        .expect("run git fixture command");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

pub(super) fn git_stdout(directory: &Path, args: &[&str]) -> Vec<u8> {
    let output = Command::new("git")
        .args(args)
        .current_dir(directory)
        .output()
        .expect("run git fixture command");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

pub(super) fn source_function<'a>(source: &'a str, name: &str) -> &'a str {
    let marker = format!("fn {name}(");
    let start = source.find(&marker).expect("source function");
    let tail = &source[start..];
    let end = tail[marker.len()..]
        .find("\nfn ")
        .map(|end| marker.len() + end)
        .unwrap_or(tail.len());
    &tail[..end]
}

pub(super) fn claim_record(worker: &str, claim_id: &str, state: &str) -> RunStateRecord {
    RunStateRecord::new(
        "owner/repo",
        42,
        worker,
        state,
        format!("feat/{worker}"),
        "",
        state,
        Vec::new(),
        "2026-07-25T00:00:00Z",
        "2026-07-25T00:00:00Z",
        1,
    )
    .with_claim_id(claim_id)
}

pub(super) fn lifecycle_evidence(record: &RunStateRecord) -> Result<ClaimEvidence, claim::CommandFailure> {
    lifecycle_claim_evidence_from_record(
        RepositoryScope::try_from("owner/repo").expect("repository scope"),
        IssueNumber::new(42).expect("issue"),
        WorkerId::try_from("worker-requested").expect("requested worker"),
        ClaimBranch::try_from("feat/requested").expect("requested branch"),
        record,
    )
}

#[cfg(unix)]
pub(super) fn assert_bridge_transition_projection(
    label: &str,
    disposition: claim::BridgeClaimDisposition,
    expected_state: &str,
    expected_prepared_step: &str,
    expected_edit: &str,
    expected_comments: usize,
    interrupt_after_preparation: bool,
) {
    let fixture = ClaimRefFixture::new(label);
    let bin = fixture.root.join("bin");
    let gh = bin.join("gh");
    let calls = fixture.root.join("gh-calls");
    let comments = fixture.root.join("comments.json");
    let label_claims = fixture.root.join("label-claims");
    let first_label_failed = fixture.root.join("first-label-failed");
    std::fs::create_dir(&bin).expect("bin");
    std::fs::write(&comments, "[]").expect("comments");
    std::fs::write(
        &gh,
        "#!/bin/sh\n\
         set -eu\n\
         printf '%s\\n' \"$*\" >> \"$GH_CALLS\"\n\
         if [ \"$1\" = api ]; then cat \"$GH_COMMENTS\"; exit 0; fi\n\
         if [ \"$1 $2\" = 'issue comment' ]; then\n\
           body=''; while [ \"$#\" -gt 0 ]; do if [ \"$1\" = --body ]; then shift; body=$1; fi; shift || true; done\n\
           jq --arg body \"$body\" '. + [{id:(length + 1),body:$body,updated_at:\"2026-07-26T00:00:00Z\"}]' \"$GH_COMMENTS\" > \"$GH_COMMENTS.tmp\"\n\
           mv \"$GH_COMMENTS.tmp\" \"$GH_COMMENTS\"; exit 0\n\
         fi\n\
         if [ \"$1 $2\" = 'issue edit' ]; then\n\
           current=$(git -C \"$GH_REMOTE\" rev-parse refs/autospec/claims/issue-42)\n\
           printf '%s\\n' \"$current\" >> \"$GH_LABEL_CLAIMS\"\n\
           if [ \"${GH_FAIL_FIRST_LABEL:-0}\" = 1 ] && [ ! -e \"$GH_FIRST_LABEL_FAILED\" ]; then\n\
             : > \"$GH_FIRST_LABEL_FAILED\"\n\
             exit 23\n\
           fi\n\
           exit 0\n\
         fi\n\
         exit 64\n",
    )
    .expect("gh");
    std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).expect("gh mode");
    let old_path = std::env::var_os("PATH");
    let old_remote = std::env::var_os("AUTOSPEC_CLAIM_GIT_REMOTE");
    let old_state = std::env::var_os("AUTOSPEC_CLAIM_GIT_STATE_DIR");
    let old_calls = std::env::var_os("GH_CALLS");
    let old_comments = std::env::var_os("GH_COMMENTS");
    let old_gh_remote = std::env::var_os("GH_REMOTE");
    let old_label_claims = std::env::var_os("GH_LABEL_CLAIMS");
    let old_fail_first_label = std::env::var_os("GH_FAIL_FIRST_LABEL");
    let old_first_label_failed = std::env::var_os("GH_FIRST_LABEL_FAILED");
    let old_retries = std::env::var_os("AUTOSPEC_GH_API_RETRIES");
    let old_heartbeat = std::env::var_os("AUTOSPEC_HEARTBEAT_DIR");
    std::env::set_var(
        "PATH",
        format!(
            "{}:{}",
            bin.display(),
            old_path.as_deref().unwrap_or_default().to_string_lossy()
        ),
    );
    std::env::set_var("AUTOSPEC_CLAIM_GIT_REMOTE", &fixture.remote);
    std::env::set_var(
        "AUTOSPEC_CLAIM_GIT_STATE_DIR",
        fixture.root.join("claim-state"),
    );
    std::env::set_var("GH_CALLS", &calls);
    std::env::set_var("GH_COMMENTS", &comments);
    std::env::set_var("GH_REMOTE", &fixture.remote);
    std::env::set_var("GH_LABEL_CLAIMS", &label_claims);
    std::env::set_var(
        "GH_FAIL_FIRST_LABEL",
        if interrupt_after_preparation {
            "1"
        } else {
            "0"
        },
    );
    std::env::set_var("GH_FIRST_LABEL_FAILED", &first_label_failed);
    std::env::set_var("AUTOSPEC_GH_API_RETRIES", "1");
    std::env::set_var("AUTOSPEC_HEARTBEAT_DIR", fixture.root.join("heartbeats"));

    let claimed = claim_record("worker-a", "claim-a", "claimed");
    assert!(matches!(
        advance_claim_ref_in(
            Path::new("git"),
            &fixture.clients[0],
            fixture.remote.to_str().unwrap(),
            "owner/repo",
            42,
            None,
            &claimed,
        )
        .expect("seed authoritative claim"),
        ClaimRefAdvance::Won(_)
    ));
    let identity = claim::ClaimMutationIdentity {
        repo: "owner/repo",
        issue: 42,
        worker_id: "worker-a",
        branch: "feat/worker-a",
        claim_id: "claim-a",
    };
    if disposition == claim::BridgeClaimDisposition::Retryable {
        claim::write_startup_heartbeat(
            identity.repo,
            identity.issue,
            identity.worker_id,
            identity.branch,
            identity.claim_id,
            None,
        )
        .expect("retryable heartbeat");
    }
    let pr = (disposition == claim::BridgeClaimDisposition::Merged).then_some(17);
    if interrupt_after_preparation {
        claim::transition_bridge_claim(identity, pr, disposition)
            .expect_err("first label projection must interrupt after preparation");
        let prepared = claim::read_claim_ref("owner/repo", 42)
            .expect("read prepared claim")
            .expect("prepared claim head");
        assert_eq!(prepared.record.state, "claimed");
        assert_eq!(prepared.record.step, expected_prepared_step);
    }
    assert_eq!(
        claim::transition_bridge_claim(identity, pr, disposition)
            .expect("transition or prepared restart"),
        claim::BridgeClaimTransition::Transitioned
    );
    assert_eq!(
        claim::transition_bridge_claim(identity, pr, disposition).expect("resume projection"),
        claim::BridgeClaimTransition::Transitioned
    );
    let head = claim::read_claim_ref("owner/repo", 42)
        .expect("read claim")
        .expect("claim head");
    assert_eq!(head.record.state, expected_state);
    assert_ne!(head.record.state, "claimed");
    let call_log = std::fs::read_to_string(calls).expect("calls");
    assert!(call_log.contains(expected_edit), "{call_log}");
    assert_eq!(
        call_log.matches(expected_edit).count(),
        if interrupt_after_preparation { 2 } else { 1 },
        "terminal restart must not reapply labels: {call_log}"
    );
    let label_claim_oid =
        std::fs::read_to_string(&label_claims).expect("claim observed during label projection");
    let label_claim_oid = label_claim_oid.lines().next().expect("label claim oid");
    let label_claim = String::from_utf8(git_stdout(
        &fixture.root,
        &[
            "-C",
            fixture.remote.to_str().expect("remote path"),
            "cat-file",
            "commit",
            label_claim_oid,
        ],
    ))
    .expect("claim commit utf8");
    let (_, label_claim) = label_claim
        .split_once("\n\n")
        .expect("claim commit message");
    let prepared =
        claim::parse_claim_ref_message("a".repeat(40), label_claim, "owner/repo", 42)
            .expect("prepared terminal claim");
    assert_eq!(prepared.record.state, "claimed");
    assert_eq!(prepared.record.step, expected_prepared_step);
    let comment_value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(comments).expect("comments JSON"))
            .expect("comments");
    assert_eq!(
        comment_value.as_array().expect("comment array").len(),
        expected_comments,
        "restart duplicated a projection"
    );

    for (key, value) in [
        ("PATH", old_path),
        ("AUTOSPEC_CLAIM_GIT_REMOTE", old_remote),
        ("AUTOSPEC_CLAIM_GIT_STATE_DIR", old_state),
        ("GH_CALLS", old_calls),
        ("GH_COMMENTS", old_comments),
        ("GH_REMOTE", old_gh_remote),
        ("GH_LABEL_CLAIMS", old_label_claims),
        ("GH_FAIL_FIRST_LABEL", old_fail_first_label),
        ("GH_FIRST_LABEL_FAILED", old_first_label_failed),
        ("AUTOSPEC_GH_API_RETRIES", old_retries),
        ("AUTOSPEC_HEARTBEAT_DIR", old_heartbeat),
    ] {
        match value {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }
}

pub(super) fn seed_claim(fixture: &ClaimRefFixture) -> claim::ClaimRefHead {
    let initial = claim_record("worker-a", "claim-a", "claimed");
    match advance_claim_ref_in(
        Path::new("git"),
        &fixture.clients[0],
        fixture.remote.to_str().unwrap(),
        "owner/repo",
        42,
        None,
        &initial,
    )
    .expect("seed claim")
    {
        ClaimRefAdvance::Won(head) => *head,
        ClaimRefAdvance::Lost => panic!("seed claim lost"),
    }
}

pub(super) fn race_claim_ref_transitions(
    fixture: &ClaimRefFixture,
    parent: &claim::ClaimRefHead,
    records: [RunStateRecord; 2],
) -> Vec<ClaimRefAdvance> {
    let barrier = Arc::new(Barrier::new(3));
    let handles = fixture
        .clients
        .clone()
        .into_iter()
        .zip(records)
        .map(|(client, record)| {
            let barrier = Arc::clone(&barrier);
            let remote = fixture.remote.clone();
            let parent = parent.clone();
            std::thread::spawn(move || {
                barrier.wait();
                advance_claim_ref_in(
                    Path::new("git"),
                    &client,
                    remote.to_str().unwrap(),
                    "owner/repo",
                    42,
                    Some(&parent),
                    &record,
                )
            })
        })
        .collect::<Vec<_>>();
    barrier.wait();
    handles
        .into_iter()
        .map(|handle| {
            handle
                .join()
                .expect("claim publisher")
                .expect("claim result")
        })
        .collect()
}
