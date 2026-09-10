//! Client timeouts and failure attribution for calls into bounded server
//! operations (issue #3637).
//!
//! The invariant this module exists to enforce:
//!
//! > A client timeout must exceed the server-side bound of the operation it
//! > invokes.
//!
//! When it does not, a *correct* server produces a client-side error with no
//! server-side trace. `curl` reports that as status `000`, which reads as "the
//! service is unreachable", and the failure is attributed to the component that
//! had done nothing wrong. The observed case: worker registration ran a
//! server-side admission probe bounded at 30s (`defaultProbeTimeout`) and the
//! sweep called it with `curl --max-time 20`. A busy worker legitimately needed
//! 23.8s, so every sweep recorded an outage the gateway never saw.
//!
//! The second half of the same lesson: a transport-level failure (`000`,
//! connection reset) and an HTTP error status have **opposite** causes — the
//! first means the server never completed the request, the second means it did
//! respond — and must never be logged identically.

use std::error::Error;
use std::fmt;
use std::time::Duration;

/// What bounds the duration of a server-side operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationClass {
    /// Bounded by request handling: validation, metadata lookups, writes. A
    /// short client timeout is usually safe, but still derive it from a number
    /// rather than from intuition.
    ControlPlane,
    /// Duration depends on **GPU work**: admission probes, warmups, one-token
    /// completions. The server bound for these is set by the queue behind the
    /// device, not by the request, and is measured in tens of seconds. Never
    /// pick a client timeout for a `GpuWork` call by intuition.
    GpuWork,
}

impl OperationClass {
    pub fn as_str(self) -> &'static str {
        match self {
            OperationClass::ControlPlane => "control-plane",
            OperationClass::GpuWork => "gpu-work",
        }
    }
}

/// A server-side bound for one operation: the longest the server will work on
/// it before it answers (or returns its own error).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerBound {
    /// Operation identity as documented on the server, e.g. `POST /v1/workers`.
    pub operation: &'static str,
    /// The server's own limit for that operation.
    pub limit: Duration,
    pub class: OperationClass,
}

impl ServerBound {
    pub const fn new(
        operation: &'static str,
        limit: Duration,
        class: OperationClass,
    ) -> ServerBound {
        ServerBound {
            operation,
            limit,
            class,
        }
    }

    /// Slack between the server bound and the smallest acceptable client bound.
    /// Half the server bound, at least 5s so short control-plane calls still get
    /// room for connection setup and TLS handshake.
    const fn margin(&self) -> Duration {
        let half = Duration::from_millis(self.limit.as_millis() as u64 / 2);
        let floor = Duration::from_secs(5);
        if half.as_millis() < floor.as_millis() {
            floor
        } else {
            half
        }
    }

    /// The smallest client bound that cannot hang up before the server answers.
    pub const fn minimum_client_timeout(&self) -> Duration {
        Duration::from_millis(self.limit.as_millis() as u64 + self.margin().as_millis() as u64)
    }
}

/// `POST /v1/workers` runs a server-side admission probe that includes a
/// one-token completion. `defaultProbeTimeout = 30s` on the gateway. A client
/// bound below this — the old 20s sweep limit — reports a healthy worker as
/// unreachable, so it is a configuration error, not a tuning choice.
pub const ADMISSION_PROBE: ServerBound = ServerBound::new(
    "POST /v1/workers admission probe",
    Duration::from_secs(30),
    OperationClass::GpuWork,
);

/// The client timeout to use for a bounded server operation: the server bound
/// plus [`ServerBound::margin`]. For [`ADMISSION_PROBE`] this is 45s, the value
/// that fixed the silent registration failures in issue #3637 (the worker
/// answered `201` in 23.8s — over the old 20s client limit, under the server's
/// 30s).
#[must_use]
pub fn client_timeout_for(bound: &ServerBound) -> Duration {
    bound.minimum_client_timeout()
}

/// A client bound that cannot outlast the server's own bound for the operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientTimeoutViolation {
    pub operation: &'static str,
    pub client: Duration,
    pub server_limit: Duration,
    pub required_minimum: Duration,
}

impl fmt::Display for ClientTimeoutViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "client timeout {}s is below the server bound for {}: the server allows {}s \
             and the client must allow at least {}s. A shorter client bound hangs up \
             mid-operation and records a failure the server never sees (curl status 000, \
             no server-side trace).",
            self.client.as_secs(),
            self.operation,
            self.server_limit.as_secs(),
            self.required_minimum.as_secs(),
        )
    }
}

impl Error for ClientTimeoutViolation {}

/// Reject a configured client bound that is shorter than the server bound of the
/// call it makes. Callers must surface the error, not silently clamp: a timeout
/// that "works most of the time" is exactly the failure mode in issue #3637.
pub fn validate_client_timeout(
    client: Duration,
    bound: &ServerBound,
) -> Result<Duration, ClientTimeoutViolation> {
    let required = bound.minimum_client_timeout();
    if client < required {
        return Err(ClientTimeoutViolation {
            operation: bound.operation,
            client,
            server_limit: bound.limit,
            required_minimum: required,
        });
    }
    Ok(client)
}

/// Why a call produced no successful response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallFailure {
    /// No HTTP response was received. The request did not complete, so the
    /// server is not the suspect.
    Transport(TransportFailure),
    /// The server answered with this status. It saw the request and made a
    /// decision; only this variant is evidence about the server.
    HttpStatus { status: u16 },
}

/// Transport-level reasons a request never completed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportFailure {
    /// Exit code 0 with status `000`, or a status-less abort: nothing was received.
    NoResponse,
    ConnectionRefused,
    ConnectionReset,
    DnsFailure,
    TlsFailure,
    /// The client hung up at its own bound. The server may well have completed
    /// the operation afterwards; if the bound is below the server bound this is
    /// the misconfiguration described at the top of this module.
    ClientTimeout {
        bound: Duration,
    },
}

impl TransportFailure {
    pub fn as_str(self) -> &'static str {
        match self {
            TransportFailure::NoResponse => "no_response",
            TransportFailure::ConnectionRefused => "connection_refused",
            TransportFailure::ConnectionReset => "connection_reset",
            TransportFailure::DnsFailure => "dns_failure",
            TransportFailure::TlsFailure => "tls_failure",
            TransportFailure::ClientTimeout { .. } => "client_timeout",
        }
    }

    /// Map a `curl` exit code to a transport failure. Codes outside the
    /// transport set (e.g. 22, "HTTP response >= 400 with `-f`") return `None`
    /// so the caller reports the HTTP status instead.
    pub fn from_curl_exit(exit_code: i32, client_bound: Duration) -> Option<TransportFailure> {
        match exit_code {
            0 | 22 => None,
            // 6: could not resolve host.
            6 => Some(TransportFailure::DnsFailure),
            // 7: failed to connect to host.
            7 => Some(TransportFailure::ConnectionRefused),
            // 28: operation timed out at the client's own --max-time.
            28 => Some(TransportFailure::ClientTimeout {
                bound: client_bound,
            }),
            // 35, 51, 58, 59, 60, 66, 77, 80, 81, 82, 83, 90, 91: TLS/SSL.
            35 | 51 | 58 | 59 | 60 | 66 | 77 | 80 | 81 | 82 | 83 | 90 | 91 => {
                Some(TransportFailure::TlsFailure)
            }
            // 52 empty reply, 55 failed sending, 56 failed receiving,
            // 65 partly received: the connection died mid-exchange.
            52 | 55 | 56 | 65 => Some(TransportFailure::ConnectionReset),
            // Anything else still produced no response body worth parsing.
            _ => Some(TransportFailure::NoResponse),
        }
    }
}

impl CallFailure {
    /// Did the server receive and answer the request? Only an HTTP status is
    /// evidence about the server; a transport failure is evidence about the
    /// network, the client bound, or the client's own lifetime.
    #[must_use]
    pub fn server_saw_request(&self) -> bool {
        matches!(self, CallFailure::HttpStatus { .. })
    }

    /// Short machine-readable failure kind for logs and metrics.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            CallFailure::Transport(failure) => failure.as_str(),
            CallFailure::HttpStatus { .. } => "http_status",
        }
    }

    /// One log line per call that keeps the two opposite causes apart: a
    /// transport failure never prints a status, an HTTP failure always does.
    #[must_use]
    pub fn log_line(&self, operation: &str, target: &str) -> String {
        match self {
            CallFailure::Transport(failure) => {
                let mut line = format!(
                    "operation={operation} target={target} transport={} http_status=none \
                     server_saw_request=false",
                    failure.as_str(),
                );
                if let TransportFailure::ClientTimeout { bound } = *failure {
                    line.push_str(&format!(
                        " client_timeout={}s (check the server bound: a shorter bound than \
                         the server's hangs up mid-operation)",
                        bound.as_secs()
                    ));
                }
                line
            }
            CallFailure::HttpStatus { status } => format!(
                "operation={operation} target={target} transport=none http_status={status} \
                 server_saw_request=true",
            ),
        }
    }
}

impl fmt::Display for CallFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CallFailure::Transport(TransportFailure::ClientTimeout { bound }) => write!(
                f,
                "transport failure: client timed out after {}s with no HTTP response",
                bound.as_secs()
            ),
            CallFailure::Transport(failure) => {
                write!(
                    f,
                    "transport failure: {} with no HTTP response",
                    failure.as_str()
                )
            }
            CallFailure::HttpStatus { status } => {
                write!(f, "server returned HTTP {status}")
            }
        }
    }
}

impl Error for CallFailure {}

/// Classify one completed call from its HTTP status code and transport exit
/// code, where `http_status == 0` is curl's `000` (no response). Returns
/// `None` for a 2xx/3xx response that arrived intact.
///
/// The exit code wins over the status: curl reports `000` precisely because no
/// status arrived, and attributing that to the service is the bug this function
/// prevents.
pub fn classify_call(
    http_status: u16,
    curl_exit: i32,
    client_bound: Duration,
) -> Option<CallFailure> {
    if let Some(failure) = TransportFailure::from_curl_exit(curl_exit, client_bound) {
        return Some(CallFailure::Transport(failure));
    }
    if http_status == 0 {
        return Some(CallFailure::Transport(TransportFailure::NoResponse));
    }
    if http_status >= 400 {
        return Some(CallFailure::HttpStatus {
            status: http_status,
        });
    }
    None
}
