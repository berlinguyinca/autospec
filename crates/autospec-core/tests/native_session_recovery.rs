//! Recovery tests for the provider-neutral native session runtime.
//!
//! Deterministic in-memory fixture; no real harnesses, no secret values, no
//! database mocks. Exercises crash-after-create, stale attach, duplicate
//! retry, harness fallback, and lease-epoch fencing.

use std::collections::BTreeMap;

use autospec_core::agent::{
    AgentResult, AgentTask, CodingAgentRuntime, CreationIntent, NativeSessionV1,
    SessionCapabilities, SessionEvent, SessionFenceError, SessionHarness, SessionLineage,
    SessionState, NATIVE_SESSION_SCHEMA,
};

fn lineage(work_item: &str) -> SessionLineage {
    SessionLineage {
        work_item: work_item.to_string(),
        stage: "implement".to_string(),
        role: "implementer".to_string(),
        worktree: "/scratch/wt/feat-native".to_string(),
        branch: "feat/native-session-extension".to_string(),
        pull_request: Some("1234".to_string()),
        model: "qwen3-38b".to_string(),
        provider: "local:rtx4090".to_string(),
    }
}

/// Deterministic fixture runtime.
///
/// `ledger` is the durable create-record (idempotency key -> scoped ID); it
/// survives a simulated client crash, which is what makes crash-after-create
/// retryable. `sessions` is the volatile session mirror.
struct FixtureRuntime {
    ledger: BTreeMap<String, String>,
    sessions: BTreeMap<String, NativeSessionV1>,
    next_id: u32,
    crash_next_create: bool,
    degrade_harnesses: Vec<SessionHarness>,
    fallback_harness: SessionHarness,
}

impl Default for FixtureRuntime {
    fn default() -> Self {
        Self {
            ledger: BTreeMap::new(),
            sessions: BTreeMap::new(),
            next_id: 0,
            crash_next_create: false,
            degrade_harnesses: Vec::new(),
            fallback_harness: SessionHarness::OpenCode,
        }
    }
}

impl CodingAgentRuntime for FixtureRuntime {
    fn execute_once(&self, task: &AgentTask) -> Result<AgentResult, String> {
        Ok(AgentResult::new(
            format!("ran {}", task.spec_id),
            Vec::new(),
            task.validation_command.clone(),
            Vec::new(),
            "done",
        ))
    }

    fn create_session(
        &mut self,
        harness: SessionHarness,
        intent: CreationIntent,
        idempotency_key: &str,
        lineage: SessionLineage,
        capabilities: SessionCapabilities,
    ) -> Result<NativeSessionV1, String> {
        if let Some(scoped_id) = self.ledger.get(idempotency_key) {
            // Idempotent retry: crash-after-create or duplicate call returns
            // the already-created session, never a second one.
            return self
                .sessions
                .get(scoped_id)
                .cloned()
                .ok_or_else(|| format!("ledger references missing session {scoped_id}"));
        }

        let (effective, effective_caps, fallback) = if self.degrade_harnesses.contains(&harness) {
            (
                self.fallback_harness,
                capabilities.degraded(),
                Some((harness, self.fallback_harness)),
            )
        } else {
            (harness, capabilities, None)
        };

        self.next_id += 1;
        let native_id = format!("native-{}", self.next_id);
        let mut session = NativeSessionV1::new(
            effective,
            native_id,
            intent,
            idempotency_key,
            lineage,
            effective_caps,
            "runtime",
        )?;
        if let Some((from, to)) = fallback {
            // The runtime's own ledger records the fallback even when the
            // degraded harness does not stream events.
            session
                .apply("runtime", 1, SessionEvent::Fallback { from, to })
                .expect("fresh session accepts its own runtime lease");
        }
        let scoped_id = session.scoped_id();
        self.ledger
            .insert(idempotency_key.to_string(), scoped_id.clone());
        self.sessions.insert(scoped_id, session.clone());

        if self.crash_next_create {
            // Simulated crash after create: the record is durable, the
            // client lost the return value.
            self.crash_next_create = false;
            return Err("simulated crash after create".to_string());
        }
        Ok(session)
    }

    fn resume(&mut self, session_id: &str, holder: &str) -> Result<NativeSessionV1, String> {
        let session = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| format!("no session {session_id}"))?;
        session.take_lease(holder).map_err(|err| err.to_string())?;
        Ok(session.clone())
    }

    fn inspect(&self, session_id: &str) -> Option<NativeSessionV1> {
        self.sessions.get(session_id).cloned()
    }

    fn attach(
        &self,
        session_id: &str,
        holder: &str,
        lease_epoch: u64,
    ) -> Result<NativeSessionV1, String> {
        let session = self
            .sessions
            .get(session_id)
            .ok_or_else(|| format!("no session {session_id}"))?;
        session
            .fence(holder, lease_epoch)
            .map_err(|err| err.to_string())?;
        Ok(session.clone())
    }

    fn heartbeat(
        &mut self,
        session_id: &str,
        holder: &str,
        lease_epoch: u64,
    ) -> Result<(), String> {
        let session = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| format!("no session {session_id}"))?;
        session
            .heartbeat(holder, lease_epoch)
            .map_err(|err| err.to_string())
    }

    fn reconcile(&self, work_item: &str) -> Vec<NativeSessionV1> {
        self.sessions
            .values()
            .filter(|session| session.lineage.work_item == work_item)
            .cloned()
            .collect()
    }

    fn events(&self, session_id: &str) -> Option<Vec<SessionEvent>> {
        self.sessions
            .get(session_id)
            .map(|session| session.events.clone())
    }
}

/// Fenced helper for the tests: apply an event to the stored session the way
/// the runtime's own loop would, or fail closed.
fn note(
    runtime: &mut FixtureRuntime,
    session_id: &str,
    holder: &str,
    epoch: u64,
    event: SessionEvent,
) -> Result<(), String> {
    let session = runtime
        .sessions
        .get_mut(session_id)
        .ok_or_else(|| format!("no session {session_id}"))?;
    session
        .apply(holder, epoch, event)
        .map_err(|err| err.to_string())
}

#[test]
fn execute_once_preserves_the_one_shot_contract() {
    let runtime = FixtureRuntime::default();
    let task = AgentTask::new(
        "WI-1",
        "implement the change",
        "cargo test -p autospec-core",
    );

    let result = runtime.execute_once(&task).expect("one-shot run succeeds");

    assert_eq!(result.result, "ran WI-1");
    assert_eq!(result.validation, "cargo test -p autospec-core");
    assert!(result.files_changed.is_empty());
    assert!(result.blockers.is_empty());
}

#[test]
fn crash_after_create_retry_returns_one_session_with_full_lineage() {
    let mut runtime = FixtureRuntime::default();
    runtime.crash_next_create = true;
    let lineage = lineage("WI-1");

    // First create crashes after the harness record is durable.
    let crashed = runtime.create_session(
        SessionHarness::Pi,
        CreationIntent::Fresh,
        "key-A",
        lineage.clone(),
        SessionCapabilities::full(),
    );
    assert!(crashed.is_err(), "the first create must crash");

    // Crash retry with the same idempotency key recovers the one session.
    let session = runtime
        .create_session(
            SessionHarness::Pi,
            CreationIntent::RetryAfterCrash,
            "key-A",
            lineage.clone(),
            SessionCapabilities::full(),
        )
        .expect("crash retry recovers the created session");

    let reconciled = runtime.reconcile("WI-1");
    assert_eq!(
        reconciled.len(),
        1,
        "crash retry must not create a second session"
    );
    assert_eq!(reconciled[0].scoped_id(), session.scoped_id());
    assert_eq!(session.schema_version, NATIVE_SESSION_SCHEMA);

    // Work, role, worktree, PR, model, and provider lineage all survive.
    assert_eq!(session.lineage, lineage);
    assert_eq!(session.lineage.work_item, "WI-1");
    assert_eq!(session.lineage.stage, "implement");
    assert_eq!(session.lineage.role, "implementer");
    assert_eq!(session.lineage.worktree, "/scratch/wt/feat-native");
    assert_eq!(session.lineage.branch, "feat/native-session-extension");
    assert_eq!(session.lineage.pull_request.as_deref(), Some("1234"));
    assert_eq!(session.lineage.model, "qwen3-38b");
    assert_eq!(session.lineage.provider, "local:rtx4090");

    // Exactly one Created event, from the original fresh create.
    let events = runtime.events(&session.scoped_id()).expect("event stream");
    let created = events
        .iter()
        .filter(|event| matches!(event, SessionEvent::Created { .. }))
        .count();
    assert_eq!(created, 1);
    assert_eq!(
        events.first(),
        Some(&SessionEvent::Created {
            intent: CreationIntent::Fresh
        })
    );
}

#[test]
fn duplicate_retry_without_crash_returns_the_same_session() {
    let mut runtime = FixtureRuntime::default();
    let lineage = lineage("WI-2");

    let first = runtime
        .create_session(
            SessionHarness::Codex,
            CreationIntent::Fresh,
            "key-B",
            lineage.clone(),
            SessionCapabilities::full(),
        )
        .expect("first create");
    let second = runtime
        .create_session(
            SessionHarness::Codex,
            CreationIntent::Fresh,
            "key-B",
            lineage.clone(),
            SessionCapabilities::full(),
        )
        .expect("duplicate retry");

    assert_eq!(first.scoped_id(), second.scoped_id());
    assert_eq!(first.native_id, second.native_id);
    assert_eq!(runtime.reconcile("WI-2").len(), 1);
}

#[test]
fn stale_lease_epoch_cannot_mutate_the_session() {
    let mut runtime = FixtureRuntime::default();
    let session = runtime
        .create_session(
            SessionHarness::Pi,
            CreationIntent::Fresh,
            "key-C",
            lineage("WI-3"),
            SessionCapabilities::full(),
        )
        .expect("create");
    let session_id = session.scoped_id();
    assert_eq!(session.lease.epoch, 1);

    // Resume hands the lease to the monitor at epoch 2.
    let resumed = runtime
        .resume(&session_id, "monitor")
        .expect("resume under runtime authority");
    assert_eq!(resumed.lease.epoch, 2);
    assert_eq!(resumed.lease.holder, "monitor");
    assert_eq!(resumed.lineage, session.lineage, "resume preserves lineage");

    // The stale epoch (same holder, old epoch) cannot heartbeat or attach.
    let mut stale_view = runtime.inspect(&session_id).expect("inspect");
    assert_eq!(
        stale_view.heartbeat("monitor", 1),
        Err(SessionFenceError::StaleEpoch {
            provided: 1,
            current: 2
        })
    );
    assert!(
        runtime.attach(&session_id, "monitor", 1).is_err(),
        "stale attach must be rejected"
    );

    // Stale apply on the record itself fails and leaves no trace.
    let mut stale_view = runtime.inspect(&session_id).expect("inspect");
    let events_before = stale_view.events.len();
    assert_eq!(
        stale_view.apply(
            "monitor",
            1,
            SessionEvent::Work {
                artifact: "stale".to_string()
            }
        ),
        Err(SessionFenceError::StaleEpoch {
            provided: 1,
            current: 2
        })
    );
    assert_eq!(stale_view.events.len(), events_before);
    assert_eq!(
        runtime.inspect(&session_id).unwrap().events.len(),
        events_before
    );

    // No heartbeat or work event was ever recorded.
    let events = runtime.events(&session_id).expect("event stream");
    assert!(!events
        .iter()
        .any(|event| matches!(event, SessionEvent::Heartbeat { .. })));
    assert!(!events
        .iter()
        .any(|event| matches!(event, SessionEvent::Work { .. })));

    // A foreign holder at the current epoch is also rejected.
    assert_eq!(
        runtime
            .attach(&session_id, "runtime", 2)
            .unwrap_err()
            .as_str(),
        "unknown lease holder \"runtime\", session held by \"monitor\""
    );
}

#[test]
fn heartbeat_advances_under_the_current_lease_only() {
    let mut runtime = FixtureRuntime::default();
    let session = runtime
        .create_session(
            SessionHarness::Claude,
            CreationIntent::Fresh,
            "key-D",
            lineage("WI-4"),
            SessionCapabilities::full(),
        )
        .expect("create");
    let session_id = session.scoped_id();

    runtime
        .heartbeat(&session_id, "runtime", 1)
        .expect("fresh heartbeat");
    let after = runtime.inspect(&session_id).expect("inspect");
    assert_eq!(after.heartbeat_tick, 1);
    assert_eq!(
        runtime.events(&session_id).unwrap()[1],
        SessionEvent::Heartbeat { tick: 1 }
    );

    assert!(
        runtime.heartbeat(&session_id, "runtime", 0).is_err(),
        "epoch 0 is stale against lease epoch 1"
    );
}

#[test]
fn event_stream_is_typed_and_ordered() {
    let mut runtime = FixtureRuntime::default();
    let session = runtime
        .create_session(
            SessionHarness::Pi,
            CreationIntent::Fresh,
            "key-E",
            lineage("WI-5"),
            SessionCapabilities::full(),
        )
        .expect("create");
    let session_id = session.scoped_id();

    runtime
        .heartbeat(&session_id, "runtime", 1)
        .expect("heartbeat");
    runtime.resume(&session_id, "monitor").expect("resume");
    note(
        &mut runtime,
        &session_id,
        "monitor",
        2,
        SessionEvent::Work {
            artifact: "native_session_recovery.rs".to_string(),
        },
    )
    .expect("fenced work event");
    note(
        &mut runtime,
        &session_id,
        "monitor",
        2,
        SessionEvent::Finished {
            outcome: "merged".to_string(),
        },
    )
    .expect("fenced finish");

    let events = runtime.events(&session_id).expect("event stream");
    assert_eq!(
        events,
        vec![
            SessionEvent::Created {
                intent: CreationIntent::Fresh
            },
            SessionEvent::Heartbeat { tick: 1 },
            SessionEvent::Resumed { epoch: 2 },
            SessionEvent::Work {
                artifact: "native_session_recovery.rs".to_string()
            },
            SessionEvent::Finished {
                outcome: "merged".to_string()
            },
        ]
    );
    assert_eq!(
        runtime.inspect(&session_id).unwrap().state,
        SessionState::Finished
    );
    assert!(
        runtime.attach(&session_id, "monitor", 2).is_err(),
        "a finished session accepts no attaches"
    );
}

#[test]
fn harness_fallback_reports_degraded_capabilities_without_weakening_policy() {
    let mut runtime = FixtureRuntime::default();
    runtime.degrade_harnesses.push(SessionHarness::Pi);
    let lineage = lineage("WI-6");
    let ceiling = SessionCapabilities::full();

    let session = runtime
        .create_session(
            SessionHarness::Pi,
            CreationIntent::Fresh,
            "key-F",
            lineage.clone(),
            ceiling,
        )
        .expect("fallback create");

    // The session lives on the fallback harness with a shrunken capability
    // set, and the fallback is recorded in the typed stream.
    assert_eq!(session.harness, SessionHarness::OpenCode);
    assert_eq!(session.scoped_id(), "opencode:native-1");
    assert!(session.capabilities.is_degraded());
    assert!(
        session.capabilities.is_subset_of(ceiling),
        "degraded capabilities must be a subset of the ceiling"
    );
    assert!(!session.capabilities.attach);
    assert!(!session.capabilities.event_stream);
    assert!(!session.capabilities.hidden_context);
    assert_eq!(
        session.events,
        vec![
            SessionEvent::Created {
                intent: CreationIntent::Fresh
            },
            SessionEvent::Fallback {
                from: SessionHarness::Pi,
                to: SessionHarness::OpenCode
            },
        ]
    );

    // Policy is unchanged: role, worktree, branch, and work item lineage
    // survive the fallback exactly as declared.
    assert_eq!(session.lineage, lineage);
    assert_eq!(session.lineage.role, "implementer");
    assert_eq!(session.lineage.worktree, "/scratch/wt/feat-native");
    assert_eq!(session.lineage.branch, "feat/native-session-extension");
    assert_eq!(session.lineage.pull_request.as_deref(), Some("1234"));
}

#[test]
fn reconcile_is_scoped_to_one_work_item() {
    let mut runtime = FixtureRuntime::default();
    runtime
        .create_session(
            SessionHarness::Pi,
            CreationIntent::Fresh,
            "key-G1",
            lineage("WI-A"),
            SessionCapabilities::full(),
        )
        .expect("create A");
    runtime
        .create_session(
            SessionHarness::Codex,
            CreationIntent::Fresh,
            "key-G2",
            lineage("WI-B"),
            SessionCapabilities::full(),
        )
        .expect("create B");

    let a = runtime.reconcile("WI-A");
    let b = runtime.reconcile("WI-B");
    assert_eq!(a.len(), 1);
    assert_eq!(b.len(), 1);
    assert_ne!(a[0].scoped_id(), b[0].scoped_id());
    assert_eq!(a[0].lineage.work_item, "WI-A");
    assert_eq!(b[0].lineage.work_item, "WI-B");
}

#[test]
fn lineage_and_id_validation_fail_closed() {
    let bad = SessionLineage {
        work_item: "WI-7".to_string(),
        stage: String::new(),
        role: "implementer".to_string(),
        worktree: "/scratch/wt/x".to_string(),
        branch: "feat/x".to_string(),
        pull_request: None,
        model: "qwen3-38b".to_string(),
        provider: "local:rtx4090".to_string(),
    };
    assert_eq!(
        bad.validate().unwrap_err().as_str(),
        "lineage stage must be non-empty"
    );

    assert!(
        NativeSessionV1::new(
            SessionHarness::Pi,
            "",
            CreationIntent::Fresh,
            "key",
            lineage("WI-7"),
            SessionCapabilities::full(),
            "runtime",
        )
        .is_err(),
        "empty native_id is rejected"
    );
    assert!(
        NativeSessionV1::new(
            SessionHarness::Pi,
            "native-1",
            CreationIntent::Fresh,
            "",
            lineage("WI-7"),
            SessionCapabilities::full(),
            "runtime",
        )
        .is_err(),
        "empty idempotency key is rejected"
    );
    assert!(
        SessionHarness::parse("aider").is_err(),
        "unknown harness is an error, not a fallback"
    );
    assert_eq!(SessionHarness::parse("pi"), Ok(SessionHarness::Pi));
}

#[test]
fn scoped_ids_never_collide_across_harnesses() {
    let pi = NativeSessionV1::new(
        SessionHarness::Pi,
        "native-1",
        CreationIntent::Fresh,
        "key-H1",
        lineage("WI-8"),
        SessionCapabilities::full(),
        "runtime",
    )
    .expect("create");
    let codex = NativeSessionV1::new(
        SessionHarness::Codex,
        "native-1",
        CreationIntent::Fresh,
        "key-H2",
        lineage("WI-8"),
        SessionCapabilities::full(),
        "runtime",
    )
    .expect("create");

    assert_eq!(pi.scoped_id(), "pi:native-1");
    assert_eq!(codex.scoped_id(), "codex:native-1");
    assert_ne!(pi.scoped_id(), codex.scoped_id());
}
