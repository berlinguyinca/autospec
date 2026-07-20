use autospec_core::runtime_env::{ProcessIdentity, ReleaseDecision, SessionRecord, SessionSet};

fn session_record(session_id: &str, pid: u32, process_start: &str) -> SessionRecord {
    SessionRecord {
        schema_version: 1,
        session_id: session_id.to_string(),
        pid,
        process_start: process_start.to_string(),
        harness: "test".to_string(),
        host: "test-host".to_string(),
        started_at_unix_ms: 1,
        heartbeat_at_unix_ms: 1,
    }
}

#[test]
fn releasing_one_of_two_live_sessions_keeps_the_environment_active() {
    let mut sessions = SessionSet::default();
    sessions.register(session_record("session-a", 100, "start-a"));
    sessions.register(session_record("session-b", 101, "start-b"));

    assert_eq!(sessions.release("session-a"), ReleaseDecision::KeepActive);
    assert_eq!(sessions.release("session-b"), ReleaseDecision::TearDown);
}

#[test]
fn reused_pid_with_a_different_process_start_is_not_live() {
    let recorded = ProcessIdentity {
        pid: 4242,
        process_start: "111".into(),
    };
    let observed = ProcessIdentity {
        pid: 4242,
        process_start: "222".into(),
    };

    assert!(!recorded.matches(&observed));
}
