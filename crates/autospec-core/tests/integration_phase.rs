//! Issue #3565 — the integration phase, tested end to end against a fake
//! in-memory VCS backend (the VCS interface is the seam the issue asks for)
//! and against real git (see `integration_phase_git.rs`).
//!
//! Scenarios covered here (issue acceptance criteria):
//! - a batch of parallel branches lands in dependency order;
//! - an additive conflict (both sides adding different fields) is resolved
//!   as the union, keeping both sides' symbols;
//! - a resolution that silently drops one side is rejected with the dropped
//!   symbol named — even though the tree would build and pass tests;
//! - a genuine semantic conflict halts that branch, reports the hunk, and
//!   leaves the rest of the batch landable;
//! - a post-rebase verification failure names the branch;
//! - the VCS interface offers no skip operation (the git backend's
//!   rebase argv is asserted in unit tests in `src/integration/git.rs`).

use autospec_core::integration::{
    BranchOutcome, ConflictHunk, ConflictedFile, GitVcs, IntegrationPhase, RebaseOutcome,
    Resolution, ResolveError, ResolvedFile, Resolver, UnionResolver, Vcs, VcsError, Verifier,
    VerifyOutcome,
};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

fn lines(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

/// The additive conflict from the issue: trunk added `telemetry`, the branch
/// added `regToken` and `endpoints`, to the same struct.
fn additive_store_files() -> Vec<ConflictedFile> {
    vec![ConflictedFile {
        path: "store.go".to_string(),
        base: "type Store struct {\n\tcore        *coreClient\n\ttelemetry   *telemetryStore\n}\n"
            .to_string(),
        hunks: vec![ConflictHunk {
            start: 1,
            ancestor: lines(&["\tcore        *coreClient"]),
            ours: lines(&["\tcore        *coreClient", "\ttelemetry   *telemetryStore"]),
            theirs: lines(&[
                "\tcore        *coreClient",
                "\tregToken    string",
                "\tendpoints   endpointPolicy",
            ]),
        }],
    }]
}

/// The semantic conflict from the issue: both sides rewrote the same line.
fn semantic_store_files() -> Vec<ConflictedFile> {
    vec![ConflictedFile {
        path: "store.go".to_string(),
        base: "type Store struct {\n\tcore        *coreClient\n}\n".to_string(),
        hunks: vec![ConflictHunk {
            start: 1,
            ancestor: lines(&["\tcore        *coreClient"]),
            ours: lines(&["\tcore        *RenamedTrunkCore"]),
            theirs: lines(&["\tcore        *BranchCore"]),
        }],
    }]
}

/// In-memory VCS backend recording every operation the phase performs.
#[derive(Default)]
struct State {
    rebased: Vec<String>,
    applied: Vec<(String, Vec<ResolvedFile>)>,
    landed: Vec<String>,
    settled: bool,
}

struct FakeVcs {
    branches: Vec<String>,
    conflicts: HashMap<String, Vec<ConflictedFile>>,
    rebase_failures: HashSet<String>,
    state: Arc<Mutex<State>>,
}

impl FakeVcs {
    fn state(&self) -> Arc<Mutex<State>> {
        self.state.clone()
    }
}

impl Vcs for FakeVcs {
    fn batch(&self) -> Result<Vec<String>, VcsError> {
        Ok(self.branches.clone())
    }

    fn rebase(&mut self, branch: &str) -> Result<RebaseOutcome, VcsError> {
        if self.rebase_failures.contains(branch) {
            return Err(VcsError(format!("rebase of {branch} exploded")));
        }
        self.state.lock().unwrap().rebased.push(branch.to_string());
        match self.conflicts.get(branch) {
            None => Ok(RebaseOutcome::Clean),
            Some(files) => Ok(RebaseOutcome::Conflict {
                files: files.clone(),
            }),
        }
    }

    fn apply_resolution(&mut self, branch: &str, files: &[ResolvedFile]) -> Result<(), VcsError> {
        let conflicted = self
            .conflicts
            .get(branch)
            .expect("apply_resolution on a branch without a conflict");
        for conflicted in conflicted {
            if !files.iter().any(|f| f.path == conflicted.path) {
                return Err(VcsError(format!(
                    "resolution for {branch} does not cover conflicted file {}",
                    conflicted.path
                )));
            }
        }
        self.state
            .lock()
            .unwrap()
            .applied
            .push((branch.to_string(), files.to_vec()));
        Ok(())
    }

    fn land(&mut self, branch: &str) -> Result<(), VcsError> {
        self.state.lock().unwrap().landed.push(branch.to_string());
        Ok(())
    }

    fn settle(&mut self) -> Result<(), VcsError> {
        self.state.lock().unwrap().settled = true;
        Ok(())
    }
}

/// Resolver that keeps only the trunk side (the classic silent drop).
#[derive(Default)]
struct TakeTrunkResolver;

impl Resolver for TakeTrunkResolver {
    fn resolve(&self, files: &[ConflictedFile]) -> Result<Resolution, ResolveError> {
        Ok(Resolution {
            files: files
                .iter()
                .map(|f| ResolvedFile {
                    path: f.path.clone(),
                    content: f.base.clone(),
                })
                .collect(),
        })
    }
}

/// Resolver that keeps only the branch side.
#[derive(Default)]
struct TakeBranchResolver;

impl Resolver for TakeBranchResolver {
    fn resolve(&self, files: &[ConflictedFile]) -> Result<Resolution, ResolveError> {
        Ok(Resolution {
            files: files
                .iter()
                .map(|f| {
                    let mut content = String::new();
                    for (i, base_line) in f.base.lines().enumerate() {
                        let mut line = base_line.to_string();
                        for hunk in &f.hunks {
                            if hunk.start <= i && i < hunk.start + hunk.ours.len() {
                                if let Some(t) = hunk.theirs.get(i - hunk.start) {
                                    line = t.clone();
                                }
                            }
                        }
                        content.push_str(&line);
                        if i + 1 < f.base.lines().count() {
                            content.push('\n');
                        }
                    }
                    if f.base.ends_with('\n') {
                        content.push('\n');
                    }
                    ResolvedFile {
                        path: f.path.clone(),
                        content,
                    }
                })
                .collect(),
        })
    }
}

/// Verifier that fails for named branches and records the call order.
#[derive(Default)]
struct RecordingVerifier {
    failing: HashSet<String>,
    calls: Arc<Mutex<Vec<String>>>,
}

impl Verifier for RecordingVerifier {
    fn verify(&self, branch: &str) -> VerifyOutcome {
        self.calls.lock().unwrap().push(branch.to_string());
        if self.failing.contains(branch) {
            VerifyOutcome::Failed {
                reason: "build failed: 2 test failures".to_string(),
            }
        } else {
            VerifyOutcome::Passed
        }
    }
}

fn phase_for(
    conflicts: Vec<(&str, Vec<ConflictedFile>)>,
) -> (
    IntegrationPhase<FakeVcs, UnionResolver, RecordingVerifier>,
    Arc<Mutex<State>>,
    Arc<Mutex<Vec<String>>>,
) {
    let state = Arc::new(Mutex::new(State::default()));
    let calls = Arc::new(Mutex::new(Vec::new()));
    let verifier = RecordingVerifier {
        failing: HashSet::new(),
        calls: calls.clone(),
    };
    let fake = FakeVcs {
        branches: vec![
            "feat/alpha".to_string(),
            "feat/beta".to_string(),
            "feat/gamma".to_string(),
        ],
        conflicts: conflicts
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
        rebase_failures: HashSet::new(),
        state: state.clone(),
    };
    let phase = IntegrationPhase::new(fake, UnionResolver, verifier);
    (phase, state, calls)
}

#[test]
fn lands_batch_in_dependency_order() {
    let (phase, state, calls) = phase_for(vec![("feat/beta", additive_store_files())]);
    let report = phase.run();
    assert!(report.ok(), "{report:?}");
    assert_eq!(
        report.landed(),
        vec!["feat/alpha", "feat/beta", "feat/gamma"]
    );
    let state = state.lock().unwrap();
    assert_eq!(
        state.rebased,
        vec!["feat/alpha", "feat/beta", "feat/gamma"],
        "rebase order is the batch (dependency) order"
    );
    assert_eq!(
        state.landed,
        vec!["feat/alpha", "feat/beta", "feat/gamma"],
        "land order is the batch (dependency) order"
    );
    drop(state);
    assert_eq!(
        *calls.lock().unwrap(),
        vec!["feat/alpha", "feat/beta", "feat/gamma"],
        "verifier runs after each rebase, in order"
    );
}

#[test]
fn additive_conflict_union_keeps_both_sides() {
    let (phase, state, _calls) = phase_for(vec![("feat/beta", additive_store_files())]);
    let report = phase.run();
    assert!(report.ok(), "{report:?}");
    let state = state.lock().unwrap();
    let (branch, files) = state
        .applied
        .iter()
        .find(|(b, _)| b == "feat/beta")
        .expect("beta was resolved");
    assert_eq!(branch, "feat/beta");
    let content = &files[0].content;
    assert!(content.contains("telemetryStore"), "{content}");
    assert!(content.contains("regToken"), "{content}");
    assert!(content.contains("endpoints"), "{content}");
    assert!(content.contains("endpointPolicy"), "{content}");
    // Both fields coexist, trunk order preserved.
    assert!(
        content.find("telemetryStore").unwrap() < content.find("regToken").unwrap(),
        "{content}"
    );
}

#[test]
fn resolution_dropping_the_branch_side_is_rejected_with_named_symbols() {
    // The TakeTrunk "resolution" is exactly what a human types as
    // "take ours": it compiles and passes tests. The gate must catch it.
    let state = Arc::new(Mutex::new(State::default()));
    let fake = FakeVcs {
        branches: vec![
            "feat/alpha".to_string(),
            "feat/beta".to_string(),
            "feat/gamma".to_string(),
        ],
        conflicts: HashMap::from([("feat/beta".to_string(), additive_store_files())]),
        rebase_failures: HashSet::new(),
        state: state.clone(),
    };
    let phase = IntegrationPhase::new(fake, TakeTrunkResolver, RecordingVerifier::default());
    let report = phase.run();

    let beta = &report.results[1];
    assert_eq!(beta.branch, "feat/beta");
    match &beta.outcome {
        BranchOutcome::RejectedSymbolLoss { missing } => {
            let names: Vec<&str> = missing.iter().map(|m| m.symbol.as_str()).collect();
            assert!(names.contains(&"regToken"), "{names:?}");
            assert!(names.contains(&"endpoints"), "{names:?}");
            assert!(names.contains(&"endpointPolicy"), "{names:?}");
            assert!(!names.iter().any(|n| *n == "telemetryStore"), "{names:?}");
            for m in missing {
                assert_eq!(m.file, "store.go");
            }
        }
        other => panic!("expected RejectedSymbolLoss, got {other:?}"),
    }
    let state = state.lock().unwrap();
    assert!(
        !state.landed.iter().any(|b| b == "feat/beta"),
        "rejected branch must not land"
    );
    assert!(
        !state.applied.iter().any(|(b, _)| b == "feat/beta"),
        "rejected resolution must not be applied"
    );
    // The rest of the batch stays landable.
    assert!(state.landed.iter().any(|b| b == "feat/gamma"));
    assert!(state.landed.iter().any(|b| b == "feat/alpha"));
}

#[test]
fn resolution_dropping_the_trunk_side_is_rejected_with_named_symbols() {
    let state = Arc::new(Mutex::new(State::default()));
    let fake = FakeVcs {
        branches: vec![
            "feat/alpha".to_string(),
            "feat/beta".to_string(),
            "feat/gamma".to_string(),
        ],
        conflicts: HashMap::from([("feat/beta".to_string(), additive_store_files())]),
        rebase_failures: HashSet::new(),
        state: state.clone(),
    };
    let phase = IntegrationPhase::new(fake, TakeBranchResolver, RecordingVerifier::default());
    let report = phase.run();

    let beta = &report.results[1];
    match &beta.outcome {
        BranchOutcome::RejectedSymbolLoss { missing } => {
            let names: Vec<&str> = missing.iter().map(|m| m.symbol.as_str()).collect();
            assert!(names.contains(&"telemetry"), "{names:?}");
            assert!(names.contains(&"telemetryStore"), "{names:?}");
            assert!(!names.iter().any(|n| *n == "regToken"), "{names:?}");
        }
        other => panic!("expected RejectedSymbolLoss, got {other:?}"),
    }
    let state = state.lock().unwrap();
    assert!(!state.landed.iter().any(|b| b == "feat/beta"));
}

#[test]
fn semantic_conflict_halts_branch_reports_hunk_rest_of_batch_lands() {
    let state = Arc::new(Mutex::new(State::default()));
    let fake = FakeVcs {
        branches: vec![
            "feat/alpha".to_string(),
            "feat/beta".to_string(),
            "feat/gamma".to_string(),
        ],
        conflicts: HashMap::from([("feat/beta".to_string(), semantic_store_files())]),
        rebase_failures: HashSet::new(),
        state: state.clone(),
    };
    let phase = IntegrationPhase::new(fake, UnionResolver, RecordingVerifier::default());
    let report = phase.run();

    let beta = &report.results[1];
    match &beta.outcome {
        BranchOutcome::HaltedSemantic { conflicts } => {
            assert_eq!(conflicts.len(), 1);
            assert_eq!(conflicts[0].file, "store.go");
            assert_eq!(conflicts[0].start, 1);
            assert!(
                conflicts[0].reason.contains("trunk side"),
                "{}",
                conflicts[0].reason
            );
            assert!(
                conflicts[0].reason.contains("branch side"),
                "{}",
                conflicts[0].reason
            );
            // The hunk is reported with both sides visible.
            assert_eq!(
                conflicts[0].ours,
                lines(&["\tcore        *RenamedTrunkCore"])
            );
            assert_eq!(conflicts[0].theirs, lines(&["\tcore        *BranchCore"]));
        }
        other => panic!("expected HaltedSemantic, got {other:?}"),
    }
    assert!(beta.outcome.describe().contains("store.go"));
    assert!(beta.outcome.describe().contains("line 2"));
    let state = state.lock().unwrap();
    assert!(!state.landed.iter().any(|b| b == "feat/beta"));
    assert!(!state.applied.iter().any(|(b, _)| b == "feat/beta"));
    // The rest of the batch stays landable.
    assert_eq!(
        state.landed,
        vec!["feat/alpha", "feat/gamma"],
        "a halted branch must not block the others"
    );
}

#[test]
fn verification_failure_names_branch_and_blocks_landing() {
    let verifier = RecordingVerifier {
        failing: HashSet::from(["feat/beta".to_string()]),
        calls: Arc::new(Mutex::new(Vec::new())),
    };
    let state = Arc::new(Mutex::new(State::default()));
    let fake = FakeVcs {
        branches: vec![
            "feat/alpha".to_string(),
            "feat/beta".to_string(),
            "feat/gamma".to_string(),
        ],
        conflicts: HashMap::from([("feat/beta".to_string(), additive_store_files())]),
        rebase_failures: HashSet::new(),
        state: state.clone(),
    };
    let phase = IntegrationPhase::new(fake, UnionResolver, verifier);
    let report = phase.run();

    let beta = &report.results[1];
    match &beta.outcome {
        BranchOutcome::VerificationFailed { reason } => {
            assert!(reason.contains("2 test failures"), "{reason}");
        }
        other => panic!("expected VerificationFailed, got {other:?}"),
    }
    let state = state.lock().unwrap();
    assert!(!state.landed.iter().any(|b| b == "feat/beta"));
    // gamma still lands: verification gates per-branch.
    assert!(state.landed.iter().any(|b| b == "feat/gamma"));
}

#[test]
fn rebase_error_is_reported_and_batch_continues() {
    let state = Arc::new(Mutex::new(State::default()));
    let fake = FakeVcs {
        branches: vec![
            "feat/alpha".to_string(),
            "feat/beta".to_string(),
            "feat/gamma".to_string(),
        ],
        conflicts: HashMap::new(),
        rebase_failures: HashSet::from(["feat/beta".to_string()]),
        state: state.clone(),
    };
    let phase = IntegrationPhase::new(fake, UnionResolver, RecordingVerifier::default());
    let report = phase.run();

    let beta = &report.results[1];
    match &beta.outcome {
        BranchOutcome::VcsError { message } => {
            assert!(message.contains("feat/beta"), "{message}");
        }
        other => panic!("expected VcsError, got {other:?}"),
    }
    let state = state.lock().unwrap();
    assert_eq!(
        state.landed,
        vec!["feat/alpha", "feat/gamma"],
        "a rebase error on one branch must not stop the batch"
    );
}

#[test]
fn git_backend_rebase_invocation_never_passes_skip() {
    // The automated path must not be able to skip a commit. The only way
    // the git backend enters a rebase is this exact argv.
    let args = GitVcs::rebase_invocation("main");
    assert!(
        !args.iter().any(|a| a.contains("skip")),
        "rebase argv must not contain skip: {args:?}"
    );
    assert!(
        !args.iter().any(|a| a == "--edit"),
        "rebase argv must not contain --edit: {args:?}"
    );
}
