//! Service-address resolution from the authoritative record at use time
//! (issue #3776).
//!
//! A service address is a fact with an expiry. The address of a gateway is
//! decided by whatever placed it — a scheduler, a port allocator, an operator
//! moving it to another node — and it changes there, not in the argument list
//! of a process that started before the change.
//!
//! The incident: every GPU worker registers itself with the gateway at startup,
//! against an address passed down as a positional argument that defaults to
//! empty. Nothing read `state/gateway-url` — the record the gateway itself
//! writes when it starts, which was correct the whole time. The gateway was
//! rescheduled onto a different node and port; every worker launched afterwards
//! registered against the address that was correct *before* the move. The
//! worker logs record the arc: `401` (reachable, auth not right), `201`
//! (registered, working), `000` (connection refused; the gateway has moved).
//!
//! Why it stayed invisible for a day: a failed registration is not a failed
//! worker. It publishes its endpoint file, agents dispatch to it directly,
//! tests pass, GPUs are busy. The only thing lost is the gateway's *knowledge*
//! that the worker exists, so the fleet looks healthy from every angle anyone
//! checks, and the pool drains one preemption at a time. Meanwhile the
//! reconciler logged "one gateway up … nothing to do" every ten minutes —
//! correct and useless, because it had verified that Slurm believes the *job*
//! is running, which was always true.
//!
//! The rules this module makes checkable:
//!
//! 1. **Resolve at use time.** [`AddressResolver::resolve`] reads the
//!    authoritative record ([`read_record`]). A launch-time argument is a
//!    fallback used only when the record cannot be read, and the resolution
//!    says so ([`AddressOrigin::LaunchArgument`]) — a captured value is never
//!    silently preferred to a readable record.
//! 2. **A cache is invalidated on failure.** [`AddressResolver::record_failure`]
//!    drops the cached value so the next use re-reads; a long-lived process
//!    therefore converges on a relocated service without a restart
//!    ([`AddressResolver::register`]).
//! 3. **Registration failure is a distinct unhealthy state.**
//!    [`component_health`] reports [`ComponentHealth::DegradedNotInPool`] for a
//!    worker that serves but cannot join the pool it was created for.
//! 4. **A health check asserts a response, never a scheduler's belief.**
//!    [`service_health`] returns [`ServiceHealth::AssertedWrongThing`] for
//!    [`HealthEvidence::SchedulerJobState`]: container liveness is not service
//!    health.
//! 5. **Slow drain is visible before it reaches zero.** [`PoolMonitor`] keeps
//!    the pool size per pass and flags a sustained decline, and the reconciler
//!    line it produces ([`PoolMonitor::reconcile_line`]) never says "nothing to
//!    do" without naming the pool size on the same line.
//! 6. **A merged fix is not a deployed fix** (#4228). The service states the
//!    revision it is running (`branch @ sha`, [`parse_revision`]); the
//!    reconciler compares it against the expected tip ([`drift`]) and says
//!    "nothing to do" only when the two agree ([`decide`]). A redeploy is
//!    refused — naming the unverified preconditions — until restart safety is
//!    tested: the build works, the preflight refuses to bind without auth, and
//!    the reconciler starts a replacement ([`PreconditionLedger`]). The line
//!    it produces ([`reconcile_line`]) never says "nothing to do" while the
//!    running revision is stale or unreported.
//!
//! Everything here is pure except [`read_record`], which reads one file. The
//! caller supplies the transport closure and the pool counts; no clock, no
//! subprocesses.

pub mod deploy;
pub mod health;
pub mod pool;
pub mod record;
pub mod registration;

/// Conventional path of the record a gateway writes for its own address when
/// it starts, relative to the deployment state directory.
pub const GATEWAY_URL_RECORD: &str = "state/gateway-url";

pub use deploy::{
    decide, drift, parse_revision, reconcile_line, DeployAction, Precondition, PreconditionLedger,
    Revision, RevisionDrift, RevisionError,
};
pub use health::{
    component_health, service_health, ComponentHealth, HealthEvidence, ServiceHealth,
};
pub use pool::{PoolMonitor, PoolSample, PoolTrend, DEFAULT_DECLINE_WINDOW};
pub use record::{
    parse_address, read_record, AddressOrigin, AddressResolver, NoAddress, RecordError, Resolution,
};
pub use registration::RegistrationOutcome;
