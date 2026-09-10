//! Issue #3637: a client timeout shorter than the server's bound reports a
//! working service as unreachable.

use std::time::Duration;

use autospec_core::service_timeout::{
    classify_call, client_timeout_for, validate_client_timeout, CallFailure, OperationClass,
    ServerBound, TransportFailure, ADMISSION_PROBE,
};

#[test]
fn admission_probe_declares_its_server_bound_as_gpu_work() {
    assert_eq!(ADMISSION_PROBE.limit, Duration::from_secs(30));
    assert_eq!(ADMISSION_PROBE.class, OperationClass::GpuWork);
}

#[test]
fn derived_client_bound_is_45s_for_the_30s_admission_probe() {
    // 23.8s is the measured registration latency that the old 20s client bound
    // lost; the derived bound must sit above the server bound, not below it.
    assert_eq!(
        client_timeout_for(&ADMISSION_PROBE),
        Duration::from_secs(45)
    );
    assert!(client_timeout_for(&ADMISSION_PROBE) > ADMISSION_PROBE.limit);
}

#[test]
fn the_old_20s_sweep_bound_is_rejected_against_the_admission_probe() {
    let err = validate_client_timeout(Duration::from_secs(20), &ADMISSION_PROBE)
        .expect_err("20s against a 30s server bound must not validate");
    assert_eq!(err.client, Duration::from_secs(20));
    assert_eq!(err.server_limit, Duration::from_secs(30));
    assert_eq!(err.required_minimum, Duration::from_secs(45));
    let message = err.to_string();
    assert!(message.contains("POST /v1/workers admission probe"));
    assert!(message.contains("000"));
}

#[test]
fn a_client_bound_at_or_above_the_minimum_validates() {
    assert_eq!(
        validate_client_timeout(Duration::from_secs(45), &ADMISSION_PROBE),
        Ok(Duration::from_secs(45))
    );
    assert_eq!(
        validate_client_timeout(Duration::from_secs(120), &ADMISSION_PROBE),
        Ok(Duration::from_secs(120))
    );
}

#[test]
fn short_control_plane_bounds_still_get_a_floor_margin() {
    let bound = ServerBound::new(
        "GET /health",
        Duration::from_secs(2),
        OperationClass::ControlPlane,
    );
    // margin floors at 5s so a 2s server bound still leaves room for connect+TLS
    assert_eq!(client_timeout_for(&bound), Duration::from_secs(7));
}

#[test]
fn status_000_is_a_transport_failure_not_an_http_error() {
    // curl: exit 28 (client --max-time), status 000.
    let failure =
        classify_call(0, 28, Duration::from_secs(20)).expect("a timed-out call is a failure");
    assert_eq!(
        failure,
        CallFailure::Transport(TransportFailure::ClientTimeout {
            bound: Duration::from_secs(20)
        })
    );
    assert!(!failure.server_saw_request());
    assert_eq!(failure.kind(), "client_timeout");

    let line = failure.log_line("POST /v1/workers", "worker-7");
    assert!(line.contains("transport=client_timeout"));
    assert!(line.contains("http_status=none"));
    assert!(line.contains("server_saw_request=false"));
    // The line points at the client bound instead of blaming the gateway.
    assert!(line.contains("client_timeout=20s"));
}

#[test]
fn exit_code_wins_over_a_missing_status() {
    assert_eq!(
        classify_call(0, 7, Duration::from_secs(45)),
        Some(CallFailure::Transport(TransportFailure::ConnectionRefused))
    );
    assert_eq!(
        classify_call(0, 0, Duration::from_secs(45)),
        Some(CallFailure::Transport(TransportFailure::NoResponse))
    );
    assert_eq!(
        classify_call(0, 56, Duration::from_secs(45)),
        Some(CallFailure::Transport(TransportFailure::ConnectionReset))
    );
}

#[test]
fn http_error_status_is_attributed_to_the_server() {
    let failure = classify_call(503, 0, Duration::from_secs(45)).expect("503 is a failure");
    assert_eq!(failure, CallFailure::HttpStatus { status: 503 });
    assert!(failure.server_saw_request());
    assert_eq!(failure.kind(), "http_status");

    let line = failure.log_line("POST /v1/workers", "worker-7");
    assert!(line.contains("http_status=503"));
    assert!(line.contains("server_saw_request=true"));
    assert!(line.contains("transport=none"));
}

#[test]
fn transport_and_http_failures_never_log_identically() {
    let transport = classify_call(0, 28, Duration::from_secs(45)).expect("timeout is a failure");
    let http = classify_call(500, 0, Duration::from_secs(45)).expect("500 is a failure");
    assert_ne!(
        transport.log_line("POST /v1/workers", "worker-7"),
        http.log_line("POST /v1/workers", "worker-7")
    );
}

#[test]
fn success_is_not_a_failure() {
    assert_eq!(classify_call(201, 0, Duration::from_secs(45)), None);
    // curl -f turns >=400 into exit 22; the status still decides attribution.
    assert_eq!(
        classify_call(404, 22, Duration::from_secs(45)),
        Some(CallFailure::HttpStatus { status: 404 })
    );
}
