//! Verification coverage for a finished agent patch (#3786).
//!
//! A run converted a 15k-line patch and stamped it `VERIFIED`. The tests that
//! guard the property it changed need a database and the machine had none, so
//! the gate that should have caught the defect was *silently treated as
//! passing*: the status recorded one word and could not distinguish "ran and
//! passed" from "did not run".
//!
//! Two rules, each encoded here as a pure, testable primitive:
//!
//! 1. **A status enumerates or it does not exist.** [`VerificationRecord`]
//!    never accepts a verdict; the verdict is *derived* from the per-check
//!    outcomes ([`VerificationRecord::verdict`], [`VerificationRecord::status`])
//!    and the rendered status always carries the `ran=` and `skipped=`
//!    sections. A bare `VERIFIED` is rejected at parse
//!    ([`VerificationStatus::parse`]) with `BARE_STATUS_NOT_PERMITTED`.
//! 2. **Skipped is never passed.** A check that did not run is a
//!    [`Gap`], and any gap on a check the patch *requires* keeps the record
//!    out of [`VerificationVerdict::Complete`]. Consumers ask
//!    [`VerificationStatus::counts_as_verified_evidence`] instead of matching
//!    a status word, so ordering and admission cannot read a skip as
//!    evidence.
//!
//! Required checks are derived from what the patch touches
//! ([`required_checks`]): a patch under `migrations/`, a `*.sql` file, or a
//! `schema` path requires the database integration check with it. The
//! fixture is not an excuse — a database is a container the run can start, so
//! a skip citing an unavailable provisionable fixture is reported as
//! `fixture_not_provisioned` and the record itself names the command that
//! would have provided it ([`VerificationRecord::provision_missing_fixtures`],
//! [`FixtureProvision`]). Only a genuinely external fixture
//! ([`FixtureClass::External`]) may be skipped as merely unavailable.

use std::collections::BTreeSet;
use std::fmt;

/// The maximum length of a check name, fixture name, or token in a status.
const MAX_TOKEN_LENGTH: usize = 128;

/// A dependency a check needs to actually measure something.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FixtureClass {
    /// A PostgreSQL instance (migrations, SQL, relational schemas).
    Postgres,
    /// A MySQL/MariaDB instance.
    Mysql,
    /// A Redis instance (cache-backed checks).
    Redis,
    /// A fixture the run cannot start: a third-party service or sandbox. The
    /// name is the service identifier, e.g. `payments`.
    External(String),
}

impl FixtureClass {
    /// The stable wire name, e.g. `postgres` or `external:payments`.
    pub fn as_str(&self) -> String {
        match self {
            Self::Postgres => "postgres".to_string(),
            Self::Mysql => "mysql".to_string(),
            Self::Redis => "redis".to_string(),
            Self::External(name) => format!("external:{name}"),
        }
    }

    /// Whether the run can provide this fixture itself. Every container-backed
    /// class is provisionable: a database is a container the run can start, so
    /// "no database on this machine" is not a reason to call its tests passed.
    pub fn provisionable(&self) -> bool {
        !matches!(self, Self::External(_))
    }

    /// The check this fixture gates, by name: a patch requiring Postgres is
    /// not verified until `db_integration` ran.
    pub fn check_name(&self) -> String {
        match self {
            Self::Postgres | Self::Mysql => "db_integration".to_string(),
            Self::Redis => "cache_integration".to_string(),
            Self::External(name) => format!("integration_{}", slug(name)),
        }
    }

    /// The container image that provides this fixture, if it is a container.
    pub fn container_image(&self) -> Option<&'static str> {
        match self {
            Self::Postgres => Some("postgres:16-alpine"),
            Self::Mysql => Some("mysql:8.4"),
            Self::Redis => Some("redis:7-alpine"),
            Self::External(_) => None,
        }
    }

    /// The container name used for this fixture's provisioned instance.
    pub fn container_name(&self) -> String {
        format!("autospec-fixture-{}", slug(&self.as_str()))
    }
}

/// What a run must start to make a skipped check measurable (#3786).
///
/// The record names the fix alongside the gap: "no database" is an action, not
/// a verdict. A non-provisionable fixture yields no plan
/// ([`FixtureClass::provisionable`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixtureProvision {
    /// The fixture to provide.
    pub class: FixtureClass,
    /// The container name to start it under.
    pub container: String,
    /// The command that starts it.
    pub command: String,
    /// The command that reports it ready, run before the dependent check.
    pub ready_probe: String,
    /// The environment the dependent check needs to reach it.
    pub env: Vec<(String, String)>,
}

impl FixtureProvision {
    /// The provision plan for `class`, or `None` when the fixture cannot be
    /// started by the run.
    pub fn plan(class: &FixtureClass) -> Option<Self> {
        let image = class.container_image()?;
        let container = class.container_name();
        let (env, ready_probe) = match class {
            FixtureClass::Postgres => (
                vec![
                    ("DATABASE_URL".to_string(), postgres_url(&container)),
                    ("PGPASSWORD".to_string(), FIXTURE_PASSWORD.to_string()),
                ],
                format!("docker exec {container} pg_isready -U postgres"),
            ),
            FixtureClass::Mysql => (
                vec![
                    ("DATABASE_URL".to_string(), mysql_url(&container)),
                    ("MYSQL_PWD".to_string(), FIXTURE_PASSWORD.to_string()),
                ],
                format!("docker exec {container} mysqladmin ping -uroot"),
            ),
            FixtureClass::Redis => (
                vec![(
                    "REDIS_URL".to_string(),
                    format!("redis://{container}:6379/0"),
                )],
                format!("docker exec {container} redis-cli ping"),
            ),
            FixtureClass::External(_) => return None,
        };
        Some(Self {
            class: class.clone(),
            command: format!(
                "docker run -d --rm --name {container} --network autospec-fixture {env} {image}",
                env = fixture_env_args(class).trim(),
            ),
            ready_probe,
            env,
            container,
        })
    }
}

/// Password used by provisioned fixture containers. Fixtures hold no real
/// data, so the value is a placeholder that satisfies the image's requirement
/// for a non-empty password.
const FIXTURE_PASSWORD: &str = "autospec-fixture";

fn fixture_env_args(class: &FixtureClass) -> String {
    match class {
        FixtureClass::Postgres => format!("-e POSTGRES_PASSWORD={FIXTURE_PASSWORD}"),
        FixtureClass::Mysql => format!("-e MYSQL_ROOT_PASSWORD={FIXTURE_PASSWORD}"),
        FixtureClass::Redis | FixtureClass::External(_) => String::new(),
    }
}

fn postgres_url(container: &str) -> String {
    format!("postgres://postgres:{FIXTURE_PASSWORD}@{container}:5432/autospec")
}

fn mysql_url(container: &str) -> String {
    format!("mysql://root:{FIXTURE_PASSWORD}@{container}:3306/autospec")
}

/// Why a check did not run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    /// The fixture the check needs was not present. When the fixture is
    /// provisionable ([`FixtureClass::provisionable`]) the record reports the
    /// gap as `fixture_not_provisioned` and names the container to start.
    FixtureUnavailable(FixtureClass),
    /// The check was started and did not finish in its budget.
    TimedOut,
    /// The check does not apply to this patch (declared, not inferred).
    NotApplicable,
}

impl SkipReason {
    /// The wire token, e.g. `fixture_unavailable:postgres`.
    pub fn as_str(&self) -> String {
        match self {
            Self::FixtureUnavailable(class) => format!("fixture_unavailable:{}", class.as_str()),
            Self::TimedOut => "timed_out".to_string(),
            Self::NotApplicable => "not_applicable".to_string(),
        }
    }
}

/// What happened to one check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckState {
    /// The check ran and passed.
    Passed,
    /// The check ran and failed.
    Failed,
    /// The check did not produce a measurement.
    Skipped(SkipReason),
}

impl CheckState {
    /// Whether this state is evidence that the patch is correct. A skip is
    /// never evidence: it says nothing about the patch (#3786).
    pub fn counts_as_evidence(&self) -> bool {
        matches!(self, Self::Passed)
    }
}

/// One named check and its outcome, as reported by the run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckOutcome {
    /// The check name as configured for the project, e.g. `cargo_clippy`.
    pub name: String,
    /// What happened.
    pub state: CheckState,
}

impl CheckOutcome {
    /// A check that ran and passed.
    pub fn passed(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            state: CheckState::Passed,
        }
    }

    /// A check that ran and failed.
    pub fn failed(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            state: CheckState::Failed,
        }
    }

    /// A check that did not run, with the reason it did not run.
    pub fn skipped(name: impl Into<String>, reason: SkipReason) -> Self {
        Self {
            name: name.into(),
            state: CheckState::Skipped(reason),
        }
    }
}

/// A check the patch must pass for its verification to be complete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequiredCheck {
    /// The check name.
    pub name: String,
    /// The fixture the check needs, if any.
    pub fixture: Option<FixtureClass>,
}

/// The checks every patch needs, whatever it touches.
const BASE_CHECKS: [&str; 3] = ["cargo_fmt", "cargo_clippy", "cargo_test"];

/// Derive the checks a patch requires from the paths it touches.
///
/// Every patch owes the base checks; a patch that touches migrations, SQL, or
/// a relational schema additionally owes the database integration check,
/// because the property it changes is only observable against a real database.
/// Detection is path-shaped and deliberately conservative — it never infers a
/// fixture from file contents.
pub fn required_checks(touched_paths: &[String]) -> Vec<RequiredCheck> {
    let mut required: Vec<RequiredCheck> = BASE_CHECKS
        .iter()
        .map(|name| RequiredCheck {
            name: (*name).to_string(),
            fixture: None,
        })
        .collect();
    for class in fixture_classes_required(touched_paths) {
        required.push(RequiredCheck {
            name: class.check_name(),
            fixture: Some(class),
        });
    }
    required
}

/// The fixture classes the touched paths require.
pub fn fixture_classes_required(touched_paths: &[String]) -> Vec<FixtureClass> {
    let mut classes: BTreeSet<FixtureClass> = BTreeSet::new();
    for path in touched_paths {
        for class in fixture_classes_for_path(path) {
            classes.insert(class);
        }
    }
    // A patch naming MySQL is tested against MySQL, not also against
    // Postgres: the marker is the target, not another fixture to satisfy.
    if classes.contains(&FixtureClass::Mysql) {
        classes.remove(&FixtureClass::Postgres);
    }
    classes.into_iter().collect()
}

/// The fixture classes one path requires.
pub fn fixture_classes_for_path(path: &str) -> Vec<FixtureClass> {
    let normalized = path.replace('\\', "/").to_ascii_lowercase();
    let segments: Vec<&str> = normalized.split('/').filter(|s| !s.is_empty()).collect();
    let Some(leaf) = segments.last() else {
        return Vec::new();
    };
    let mut classes = Vec::new();
    let joined = segments.join("/");
    if joined.contains("mysql") || joined.contains("mariadb") {
        classes.push(FixtureClass::Mysql);
    } else if normalized.ends_with(".sql")
        || segments.iter().any(|s| {
            matches!(
                *s,
                "migrations" | "migration" | "migrate" | "flyway" | "alembic" | "db"
            )
        })
        || stem(leaf) == "schema"
    {
        classes.push(FixtureClass::Postgres);
    }
    if segments
        .iter()
        .any(|s| s.contains("redis") || matches!(*s, "cache" | "caches"))
    {
        classes.push(FixtureClass::Redis);
    }
    classes
}

fn stem(file: &str) -> &str {
    file.split('.').next().unwrap_or(file)
}

/// A required check the patch was verified against, and what happened to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gap {
    /// The check that was not measured.
    pub check: String,
    /// Why it was not measured.
    pub cause: GapCause,
}

impl Gap {
    /// The status token: `not_run:postgres`, `fixture_not_provisioned:postgres`,
    /// `timed_out`, …
    pub fn token(&self) -> String {
        self.cause.token()
    }
}

/// Why a required check carries no measurement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GapCause {
    /// The run never reported the check at all.
    NotRun { fixture: Option<FixtureClass> },
    /// The run reported the check as skipped.
    Skipped(SkipReason),
}

impl GapCause {
    fn token(&self) -> String {
        match self {
            Self::NotRun { fixture: None } => "not_run".to_string(),
            Self::NotRun {
                fixture: Some(class),
            } => format!("not_run:{}", class.as_str()),
            // A fixture the run could have started is a missing action, not a
            // limitation: the token says so, and the record names the command.
            Self::Skipped(SkipReason::FixtureUnavailable(class)) if class.provisionable() => {
                format!("fixture_not_provisioned:{}", class.as_str())
            }
            Self::Skipped(reason) => reason.as_str(),
        }
    }
}

/// The outcome of one verification pass, derived from the checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerificationVerdict {
    /// Every check the patch requires ran and passed.
    Complete,
    /// Nothing failed, but some required check carries no measurement.
    Incomplete { gaps: Vec<Gap> },
    /// At least one check failed.
    Failed { failed: Vec<String>, gaps: Vec<Gap> },
}

impl VerificationVerdict {
    /// The gaps carried by this verdict: empty when it is `Complete`.
    pub fn gaps(&self) -> &[Gap] {
        match self {
            Self::Complete => &[],
            Self::Incomplete { gaps } | Self::Failed { gaps, .. } => gaps,
        }
    }

    /// The names of the checks that failed: empty unless this verdict is
    /// `Failed`.
    pub fn failed(&self) -> &[String] {
        match self {
            Self::Failed { failed, .. } => failed,
            _ => &[],
        }
    }

    /// The status head token this verdict renders as.
    pub fn head(&self) -> StatusOutcome {
        match self {
            Self::Complete => StatusOutcome::Passed,
            Self::Incomplete { .. } => StatusOutcome::Incomplete,
            Self::Failed { .. } => StatusOutcome::Failed,
        }
    }
}

/// The permitted status head tokens. `VERIFIED` is deliberately absent: it is
/// the word that carried no information, and it stays a rejected legacy alias
/// rather than an accepted synonym for `PASSED`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusOutcome {
    /// Every required check ran and passed.
    Passed,
    /// Nothing failed; something was not measured.
    Incomplete,
    /// Something ran and failed.
    Failed,
}

impl StatusOutcome {
    /// The wire token.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Passed => "PASSED",
            Self::Incomplete => "INCOMPLETE",
            Self::Failed => "FAILED",
        }
    }

    /// Resolve a head token, rejecting the legacy bare `VERIFIED`.
    pub fn parse(token: &str) -> Result<Self, String> {
        match token {
            "VERIFIED" => Err(
                "BARE_STATUS_NOT_PERMITTED: VERIFIED says nothing about what ran; record \
                 PASSED[ran=...;skipped=none] or INCOMPLETE[ran=...;skipped=<check>(<reason>)]"
                    .to_string(),
            ),
            other => match other {
                "PASSED" => Ok(Self::Passed),
                "INCOMPLETE" => Ok(Self::Incomplete),
                "FAILED" => Ok(Self::Failed),
                unknown => Err(format!("UNKNOWN_STATUS_OUTCOME: {unknown}")),
            },
        }
    }
}

/// A recorded status: the head outcome plus the enumeration that makes it
/// meaningful. Rendered as
/// `PASSED[ran=a,b;skipped=none]` or
/// `INCOMPLETE[ran=a;skipped=db_integration(fixture_unavailable:postgres);gap=db_integration(fixture_not_provisioned:postgres)]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationStatus {
    rendered: String,
    outcome: StatusOutcome,
}

impl VerificationStatus {
    /// The full status line.
    pub fn as_str(&self) -> &str {
        &self.rendered
    }

    /// The head outcome.
    pub fn outcome(&self) -> StatusOutcome {
        self.outcome
    }

    /// Whether a consumer may treat this status as evidence the patch is
    /// verified. Only a status whose every required check actually ran counts;
    /// `INCOMPLETE` and `FAILED` never do, whatever words appear in them.
    pub fn counts_as_verified_evidence(&self) -> bool {
        self.outcome == StatusOutcome::Passed
    }

    /// Parse a status line, rejecting a bare outcome word.
    ///
    /// The grammar is `HEAD[ran=<checks>;failed=<checks>;skipped=<entries>;gap=<entries>]`
    /// with `ran=` and `skipped=` mandatory and the sections in that order.
    /// `PASSED` must carry `skipped=none` and no gap; `INCOMPLETE` must name at
    /// least one skip or gap; `FAILED` must name the failed checks. A status
    /// that could not have been produced by [`VerificationRecord::status`] is
    /// malformed, which is what keeps a hand-written `VERIFIED` out.
    pub fn parse(status: &str) -> Result<Self, String> {
        let status = status.trim();
        let (head, body) = status.split_once('[').ok_or_else(|| {
            format!(
                "BARE_STATUS_NOT_PERMITTED: {status} must enumerate ran= and skipped=; \
                 a bare outcome word is not a status"
            )
        })?;
        let outcome = StatusOutcome::parse(head)?;
        let body = body.strip_suffix(']').ok_or_else(|| {
            format!("MALFORMED_STATUS: {status} must end with ']' closing the ran= section")
        })?;
        let mut sections: Vec<(&str, &str)> = Vec::new();
        for entry in split_top_level(body) {
            let (key, value) = entry.split_once('=').ok_or_else(|| {
                format!("MALFORMED_STATUS: section '{entry}' must be written key=value")
            })?;
            sections.push((key, value));
        }
        let order: Vec<&str> = sections.iter().map(|(key, _)| *key).collect();
        let expected = ["ran", "failed", "skipped", "gap"];
        let filtered: Vec<&str> = expected
            .iter()
            .copied()
            .filter(|k| order.contains(k))
            .collect();
        if filtered != order {
            return Err(format!(
                "MALFORMED_STATUS: sections must appear in order ran,failed,skipped,gap (got {:?})",
                order
            ));
        }
        let get = |key: &str| -> Option<&str> {
            sections.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
        };
        let ran = get("ran").ok_or_else(|| {
            "MALFORMED_STATUS: a status must enumerate the checks that ran under ran=".to_string()
        })?;
        let skipped = get("skipped").ok_or_else(|| {
            "MALFORMED_STATUS: a status must enumerate the checks that were skipped under skipped= (use skipped=none)"
                .to_string()
        })?;
        let failed = get("failed").unwrap_or("none");
        let gap = get("gap").unwrap_or("none");
        for list in [ran, failed, skipped, gap] {
            for entry in split_entries(list) {
                validate_entry(entry)?;
            }
        }
        match outcome {
            StatusOutcome::Passed => {
                if failed != "none" || skipped != "none" || gap != "none" {
                    return Err(
                        "INCONSISTENT_STATUS: PASSED must carry failed=none, skipped=none and gap=none"
                            .to_string(),
                    );
                }
            }
            StatusOutcome::Incomplete => {
                if failed != "none" {
                    return Err(
                        "INCONSISTENT_STATUS: INCOMPLETE must not name failed checks; use FAILED"
                            .to_string(),
                    );
                }
                if skipped == "none" && gap == "none" {
                    return Err(
                        "INCONSISTENT_STATUS: INCOMPLETE must name at least one skipped check or gap"
                            .to_string(),
                    );
                }
            }
            StatusOutcome::Failed => {
                if failed == "none" {
                    return Err(
                        "INCONSISTENT_STATUS: FAILED must name at least one failed check"
                            .to_string(),
                    );
                }
            }
        }
        if ran == "none" && outcome == StatusOutcome::Passed {
            return Err(
                "INCONSISTENT_STATUS: PASSED requires at least one check under ran=".to_string(),
            );
        }
        Ok(Self {
            rendered: status.to_string(),
            outcome,
        })
    }
}

impl fmt::Display for VerificationStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.rendered)
    }
}

/// What one run recorded about one finished patch.
///
/// The record holds observations only — which checks ran, which did not, and
/// why. The verdict is computed from them, so an agent cannot assert a
/// verification the checks do not support (#3786).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationRecord {
    /// Stable identity of the finished patch (patch file name or content hash).
    pub patch_identity: String,
    /// Paths the patch touches; they decide which checks are required.
    pub touched_paths: Vec<String>,
    /// Every check the run reported, passed, failed or skipped.
    pub checks: Vec<CheckOutcome>,
}

impl VerificationRecord {
    /// Record a run's checks for one patch.
    ///
    /// Rejects an empty identity, an empty check list (a run that measured
    /// nothing verified nothing), duplicate check names, and tokens that could
    /// not survive the status round-trip.
    pub fn new(
        patch_identity: impl Into<String>,
        touched_paths: Vec<String>,
        checks: Vec<CheckOutcome>,
    ) -> Result<Self, String> {
        let patch_identity = patch_identity.into();
        if patch_identity.trim().is_empty() {
            return Err("verification record must name the patch it verifies".to_string());
        }
        if checks.is_empty() {
            return Err(format!(
                "VERIFICATION_WITHOUT_CHECKS: {patch_identity} records no checks; \
                 a status must enumerate what ran and what was skipped"
            ));
        }
        for check in &checks {
            validate_token(&check.name)?;
            if let CheckState::Skipped(SkipReason::FixtureUnavailable(FixtureClass::External(
                service,
            ))) = &check.state
            {
                validate_token(service)?;
            }
        }
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        for check in &checks {
            if !seen.insert(check.name.as_str()) {
                return Err(format!("DUPLICATE_CHECK: {} reported twice", check.name));
            }
        }
        for path in &touched_paths {
            if path.trim().is_empty() {
                return Err("touched paths must not contain empty entries".to_string());
            }
        }
        Ok(Self {
            patch_identity,
            touched_paths,
            checks,
        })
    }

    /// The checks this patch is verified against, derived from its paths.
    pub fn required_checks(&self) -> Vec<RequiredCheck> {
        required_checks(&self.touched_paths)
    }

    /// The required checks carrying no measurement, in required order.
    pub fn gaps(&self) -> Vec<Gap> {
        self.required_checks()
            .iter()
            .filter_map(|required| {
                match self.state_of(&required.name) {
                    Some(CheckState::Passed) => None,
                    Some(CheckState::Skipped(reason)) => Some(Gap {
                        check: required.name.clone(),
                        cause: GapCause::Skipped(reason.clone()),
                    }),
                    // A failed check is reported under `failed=`, not as a gap:
                    // it ran, and its verdict is its own.
                    Some(CheckState::Failed) => None,
                    None => Some(Gap {
                        check: required.name.clone(),
                        cause: GapCause::NotRun {
                            fixture: required.fixture.clone(),
                        },
                    }),
                }
            })
            .collect()
    }

    fn state_of(&self, name: &str) -> Option<&CheckState> {
        self.checks
            .iter()
            .find(|check| check.name == name)
            .map(|check| &check.state)
    }

    /// Checks that were reported but the patch does not require, and their
    /// non-passing states (a skipped extra check is still enumerated).
    fn skipped_checks(&self) -> Vec<&CheckOutcome> {
        self.checks
            .iter()
            .filter(|check| matches!(check.state, CheckState::Skipped(_)))
            .collect()
    }

    fn failed_checks(&self) -> Vec<&CheckOutcome> {
        self.checks
            .iter()
            .filter(|check| matches!(check.state, CheckState::Failed))
            .collect()
    }

    /// The derived verdict: `Complete` only when every check the patch
    /// requires ran and passed, and nothing at all was skipped.
    pub fn verdict(&self) -> VerificationVerdict {
        let failed: Vec<String> = self
            .failed_checks()
            .iter()
            .map(|check| check.name.clone())
            .collect();
        let mut gaps = self.gaps();
        // A skipped check the patch does not require is still a gap in the
        // evidence: nothing skipped may read as passed (#3786).
        for check in self.skipped_checks() {
            if let CheckState::Skipped(reason) = &check.state {
                if !gaps.iter().any(|gap| gap.check == check.name) {
                    gaps.push(Gap {
                        check: check.name.clone(),
                        cause: GapCause::Skipped(reason.clone()),
                    });
                }
            }
        }
        if !failed.is_empty() {
            return VerificationVerdict::Failed { failed, gaps };
        }
        if gaps.is_empty() {
            return VerificationVerdict::Complete;
        }
        VerificationVerdict::Incomplete { gaps }
    }

    /// The status line for this record: the head outcome plus the full
    /// enumeration. Never a bare word.
    pub fn status(&self) -> VerificationStatus {
        let verdict = self.verdict();
        let ran: Vec<&str> = self
            .checks
            .iter()
            .filter(|check| matches!(check.state, CheckState::Passed | CheckState::Failed))
            .map(|check| check.name.as_str())
            .collect();
        let failed: Vec<&str> = self
            .failed_checks()
            .iter()
            .map(|check| check.name.as_str())
            .collect();
        let skipped: Vec<String> = self
            .skipped_checks()
            .iter()
            .map(|check| match &check.state {
                CheckState::Skipped(reason) => format!("{}({})", check.name, reason.as_str()),
                _ => unreachable!("skipped_checks yields skips only"),
            })
            .collect();
        let gaps: Vec<String> = verdict
            .gaps()
            .iter()
            .map(|gap| format!("{}({})", gap.check, gap.token()))
            .collect();
        let mut sections = vec![format!("ran={}", join_or_none(&ran))];
        if !failed.is_empty() {
            sections.push(format!("failed={}", join_or_none(&failed)));
        }
        sections.push(format!("skipped={}", join_or_none(&skipped)));
        if !gaps.is_empty() {
            sections.push(format!("gap={}", join_or_none(&gaps)));
        }
        let rendered = format!("{}[{}]", verdict.head().as_str(), sections.join(";"));
        // The renderer is the only producer of a valid status; if it ever
        // emits something its own parser rejects, that is a bug, not a
        // runtime error to swallow.
        VerificationStatus::parse(&rendered)
            .unwrap_or_else(|error| panic!("renderer produced an invalid status: {error}"))
    }

    /// The rendered status line as text — the exact string a run records.
    pub fn status_line(&self) -> String {
        self.status().as_str().to_string()
    }

    /// Whether this record is evidence that the patch is verified.
    pub fn fully_verified(&self) -> bool {
        self.verdict() == VerificationVerdict::Complete
    }

    /// Whether an admission decision may admit this patch as verified. Same
    /// question as [`VerificationRecord::fully_verified`], named for the
    /// consumer so a skipped check can never be counted as evidence by
    /// accident.
    pub fn admits_as_verified(&self) -> bool {
        self.fully_verified()
    }

    /// The fixtures this run should have started and did not (#3786).
    ///
    /// "No PostgreSQL on this machine" is an action item, not a limitation:
    /// a database is a container the run can start. Each entry names the
    /// command that provides it and the environment the check needs.
    pub fn provision_missing_fixtures(&self) -> Vec<FixtureProvision> {
        let mut plans: Vec<FixtureProvision> = Vec::new();
        for gap in self.verdict().gaps() {
            let class = match &gap.cause {
                GapCause::NotRun {
                    fixture: Some(class),
                } => Some(class.clone()),
                GapCause::Skipped(SkipReason::FixtureUnavailable(class)) => Some(class.clone()),
                _ => None,
            };
            let Some(class) = class else { continue };
            if !class.provisionable() {
                continue;
            }
            if plans.iter().any(|plan| plan.class == class) {
                continue;
            }
            if let Some(plan) = FixtureProvision::plan(&class) {
                plans.push(plan);
            }
        }
        plans
    }
}

fn join_or_none<S: AsRef<str>>(items: &[S]) -> String {
    if items.is_empty() {
        "none".to_string()
    } else {
        items
            .iter()
            .map(|item| item.as_ref())
            .collect::<Vec<&str>>()
            .join(",")
    }
}

/// Split a section body on `;`, ignoring separators nested inside `()`.
fn split_top_level(body: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (index, ch) in body.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ';' if depth == 0 => {
                parts.push(&body[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    parts.push(&body[start..]);
    parts
}

/// Split one section's value into entries, treating `none` as empty.
fn split_entries(value: &str) -> Vec<&str> {
    if value == "none" {
        return Vec::new();
    }
    value.split(',').collect()
}

/// Validate one enumerated entry: a bare name, or `name(detail)` where the
/// detail optionally carries a `:<arg>` suffix.
fn validate_entry(entry: &str) -> Result<(), String> {
    match entry.split_once('(') {
        Some((name, rest)) => {
            let detail = rest.strip_suffix(')').ok_or_else(|| {
                format!("MALFORMED_STATUS: entry '{entry}' has an unclosed detail parenthesis")
            })?;
            validate_token(name)?;
            match detail.split_once(':') {
                Some((kind, arg)) => {
                    validate_token(kind)?;
                    validate_token(arg)
                }
                None => validate_token(detail),
            }
        }
        None => validate_token(entry),
    }
}

/// Reject tokens that could not survive a status round-trip: empty, oversized,
/// or carrying a separator the parser would mis-read.
fn validate_token(token: &str) -> Result<(), String> {
    if token.is_empty() {
        return Err("MALFORMED_STATUS: empty token in status enumeration".to_string());
    }
    if token.len() > MAX_TOKEN_LENGTH {
        return Err(format!(
            "TOKEN_TOO_LONG: '{token}' exceeds {MAX_TOKEN_LENGTH} characters"
        ));
    }
    if token.chars().any(|c| {
        matches!(
            c,
            ';' | '[' | ']' | '=' | ' ' | '\t' | '\n' | ',' | '(' | ')'
        )
    }) {
        // Parenthesised entries (`name(reason)`) are built by the renderer, so
        // they are validated on their inner tokens, not here.
        return Err(format!(
            "MALFORMED_STATUS: token '{token}' contains a reserved separator"
        ));
    }
    Ok(())
}

/// Lower-case a name into a slug safe for container names and check names.
fn slug(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('_') {
            out.push('_');
        }
    }
    let trimmed = out.trim_matches('_').to_string();
    if trimmed.is_empty() {
        "fixture".to_string()
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIGRATION: &str = "migrations/20260909000001_engine_correlation_premises.sql";

    fn base_passed() -> Vec<CheckOutcome> {
        vec![
            CheckOutcome::passed("cargo_fmt"),
            CheckOutcome::passed("cargo_clippy"),
            CheckOutcome::passed("cargo_test"),
        ]
    }

    fn record(paths: &[&str], checks: Vec<CheckOutcome>) -> VerificationRecord {
        VerificationRecord::new(
            "iw-424",
            paths.iter().map(|p| (*p).to_string()).collect(),
            checks,
        )
        .expect("record builds")
    }

    #[test]
    fn migration_paths_require_the_database_check() {
        let required = required_checks(&[MIGRATION.to_string()]);
        assert!(required
            .iter()
            .any(|check| check.name == "db_integration"
                && check.fixture == Some(FixtureClass::Postgres)));
    }

    #[test]
    fn schema_and_db_directories_require_the_database_check() {
        for path in [
            "db/schema.rb",
            "engine/schema.rs",
            "crates/app/migrations/0001_init.up.sql",
            "services/alembic/versions/a1b2.py",
        ] {
            let classes = fixture_classes_for_path(path);
            assert!(
                classes.contains(&FixtureClass::Postgres),
                "{path} must require a database fixture, got {classes:?}"
            );
        }
    }

    #[test]
    fn plain_source_paths_require_no_fixture() {
        for path in [
            "src/main.rs",
            "docs/design.md",
            "engine/src/cost_model.rs",
            "crates/app/src/scoring.rs",
        ] {
            assert!(
                fixture_classes_for_path(path).is_empty(),
                "{path} must not require a fixture"
            );
        }
    }

    #[test]
    fn mysql_marker_selects_mysql_rather_than_both_engines() {
        let classes = fixture_classes_required(&[
            "migrations/0001_init.sql".to_string(),
            "db/mysql/lookup.sql".to_string(),
        ]);
        assert_eq!(classes, vec![FixtureClass::Mysql]);
    }

    #[test]
    fn redis_paths_require_the_cache_check() {
        let classes = fixture_classes_for_path("src/cache/session_store.rs");
        assert!(classes.contains(&FixtureClass::Redis));
    }

    // --- AC: a bare VERIFIED is not a permitted value ----------------------

    #[test]
    fn bare_verified_is_rejected() {
        let error = VerificationStatus::parse("VERIFIED").expect_err("bare VERIFIED must fail");
        assert!(
            error.starts_with("BARE_STATUS_NOT_PERMITTED"),
            "unexpected error: {error}"
        );
        assert!(error.contains("VERIFIED"));
    }

    #[test]
    fn every_outcome_word_needs_its_enumeration() {
        for bare in ["PASSED", "INCOMPLETE", "FAILED", "VERIFIED"] {
            let error = VerificationStatus::parse(bare).expect_err("bare word must fail");
            assert!(
                error.starts_with("BARE_STATUS_NOT_PERMITTED"),
                "{bare}: unexpected error: {error}"
            );
        }
    }

    #[test]
    fn status_without_a_skipped_section_is_rejected() {
        let error = VerificationStatus::parse("PASSED[ran=cargo_fmt]")
            .expect_err("a status must enumerate skips, or skipped=none");
        assert!(error.contains("skipped="), "unexpected error: {error}");
    }

    #[test]
    fn inconsistent_status_combinations_are_rejected() {
        for status in [
            "PASSED[ran=cargo_fmt;skipped=db_integration(timed_out)]",
            "PASSED[ran=none;skipped=none]",
            "INCOMPLETE[ran=cargo_fmt;skipped=none]",
            "FAILED[ran=cargo_fmt;skipped=none]",
            "PASSED[gap=x(not_run);ran=cargo_fmt;skipped=none]",
            "MAYBE[ran=cargo_fmt;skipped=none]",
        ] {
            let error = VerificationStatus::parse(status).expect_err("must be rejected");
            assert!(
                error.starts_with("INCONSISTENT_STATUS")
                    || error.starts_with("MALFORMED_STATUS")
                    || error.starts_with("UNKNOWN_STATUS_OUTCOME"),
                "{status}: unexpected error: {error}"
            );
        }
    }

    // --- AC: skipped is never passed ---------------------------------------

    #[test]
    fn a_skip_is_never_evidence() {
        assert!(!CheckState::Skipped(SkipReason::NotApplicable).counts_as_evidence());
        assert!(CheckState::Passed.counts_as_evidence());
        assert!(!CheckState::Failed.counts_as_evidence());
    }

    #[test]
    fn skipped_required_check_leaves_a_gap() {
        let mut checks = base_passed();
        checks.push(CheckOutcome::skipped(
            "db_integration",
            SkipReason::FixtureUnavailable(FixtureClass::Postgres),
        ));
        let record = record(&[MIGRATION], checks);
        let gaps = record.gaps();
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0].check, "db_integration");
        // A fixture the run could have started is named as a missing action.
        assert_eq!(
            gaps[0].token(),
            "fixture_not_provisioned:postgres",
            "got {}",
            gaps[0].token()
        );
        assert!(!record.fully_verified());
    }

    #[test]
    fn external_fixture_is_recorded_as_unavailable_not_not_provisioned() {
        let mut checks = base_passed();
        checks.push(CheckOutcome::skipped(
            "db_integration",
            SkipReason::FixtureUnavailable(FixtureClass::External("payments".to_string())),
        ));
        let record = record(&[MIGRATION], checks);
        let status = record.status_line();
        assert!(
            status.contains("skipped=db_integration(fixture_unavailable:external:payments)"),
            "got {status}"
        );
        // Nothing to provision: the fixture is genuinely outside the run.
        assert!(record.provision_missing_fixtures().is_empty());
    }

    // --- AC: the recorded status enumerates ran and skipped ----------------

    #[test]
    fn status_enumerates_ran_and_skipped() {
        let clean = record(&["src/main.rs"], base_passed());
        assert_eq!(
            clean.status_line(),
            "PASSED[ran=cargo_fmt,cargo_clippy,cargo_test;skipped=none]"
        );

        let mut checks = base_passed();
        checks.push(CheckOutcome::skipped(
            "cache_integration",
            SkipReason::TimedOut,
        ));
        let with_skip = record(&["src/cache/store.rs"], checks);
        let status = with_skip.status_line();
        assert!(
            status.starts_with("INCOMPLETE[ran=cargo_fmt,cargo_clippy,cargo_test;"),
            "got {status}"
        );
        assert!(
            status.contains("skipped=cache_integration(timed_out)"),
            "got {status}"
        );
        assert!(
            status.contains("gap=cache_integration(timed_out)"),
            "got {status}"
        );
    }

    #[test]
    fn status_round_trips_through_the_parser() {
        let mut checks = base_passed();
        checks.push(CheckOutcome::failed("engine_integration"));
        for rec in [
            record(&["src/main.rs"], base_passed()),
            record(&[MIGRATION], base_passed()),
            record(&[MIGRATION], {
                let mut with_db = base_passed();
                with_db.push(CheckOutcome::skipped(
                    "db_integration",
                    SkipReason::FixtureUnavailable(FixtureClass::Postgres),
                ));
                with_db
            }),
            record(&["src/main.rs"], checks),
        ] {
            let status = rec.status();
            let parsed = VerificationStatus::parse(status.as_str())
                .unwrap_or_else(|error| panic!("{} did not parse: {error}", status.as_str()));
            assert_eq!(parsed.outcome(), rec.verdict().head());
            assert_eq!(parsed.counts_as_verified_evidence(), rec.fully_verified());
        }
    }

    #[test]
    fn failed_check_names_the_failure() {
        let mut checks = base_passed();
        checks.push(CheckOutcome::failed("db_integration"));
        let rec = record(&[MIGRATION], checks);
        let status = rec.status_line();
        assert!(status.starts_with("FAILED["), "got {status}");
        assert!(status.contains("failed=db_integration"), "got {status}");
        assert!(!VerificationStatus::parse(&status)
            .expect("parses")
            .counts_as_verified_evidence());
    }

    #[test]
    fn record_rejects_an_empty_check_list() {
        let error = VerificationRecord::new("iw-1", vec!["src/main.rs".to_string()], Vec::new())
            .expect_err("no checks cannot be a verification");
        assert!(error.contains("VERIFICATION_WITHOUT_CHECKS"), "got {error}");
    }

    #[test]
    fn record_rejects_duplicate_and_blank_input() {
        let duplicate = VerificationRecord::new(
            "iw-1",
            vec!["src/main.rs".to_string()],
            vec![
                CheckOutcome::passed("cargo_fmt"),
                CheckOutcome::passed("cargo_fmt"),
            ],
        )
        .expect_err("duplicate check names are a bookkeeping error");
        assert!(duplicate.contains("DUPLICATE_CHECK"), "got {duplicate}");
        assert!(
            VerificationRecord::new(" ", vec!["src/main.rs".to_string()], base_passed()).is_err()
        );
    }

    // --- AC: provision the fixture where it can be provided ----------------

    #[test]
    fn missing_database_names_the_container_to_start() {
        let record = record(&[MIGRATION], base_passed());
        let plans = record.provision_missing_fixtures();
        assert_eq!(plans.len(), 1, "got {plans:?}");
        let plan = &plans[0];
        assert_eq!(plan.class, FixtureClass::Postgres);
        assert!(
            plan.command.starts_with("docker run -d --rm"),
            "got {plan:?}"
        );
        assert!(plan.command.contains("postgres:16-alpine"), "got {plan:?}");
        assert!(plan.ready_probe.contains("pg_isready"), "got {plan:?}");
        assert!(plan.env.iter().any(|(key, _)| key == "DATABASE_URL"));
    }

    #[test]
    fn provision_plan_covers_every_container_backed_fixture() {
        for class in [
            FixtureClass::Postgres,
            FixtureClass::Mysql,
            FixtureClass::Redis,
        ] {
            assert!(class.provisionable());
            let plan = FixtureProvision::plan(&class).expect("container-backed fixture plans");
            assert!(plan.command.contains(class.container_image().unwrap()));
            assert!(!plan.ready_probe.is_empty());
        }
        assert!(!FixtureClass::External("payments".to_string()).provisionable());
        assert!(FixtureProvision::plan(&FixtureClass::External("payments".to_string())).is_none());
    }

    // --- AC5 regression: a migration patch, no database, never verified ----

    #[test]
    fn regression_migration_patch_without_database_is_never_fully_verified() {
        // The 15k-line engine patch: fmt, clippy and the unit suite ran and
        // passed; the database-backed gate ran nothing because the machine had
        // no PostgreSQL.
        let checks = vec![
            CheckOutcome::passed("cargo_fmt"),
            CheckOutcome::passed("cargo_clippy"),
            CheckOutcome::passed("cargo_test"),
            CheckOutcome::skipped(
                "engine_enforces_correlation_premises",
                SkipReason::NotApplicable,
            ),
        ];
        let record = record(
            &[
                MIGRATION,
                "engine/src/correlation.rs",
                "engine/src/optimizer.rs",
            ],
            checks,
        );

        assert!(!record.fully_verified());
        assert!(!record.admits_as_verified());
        let status = record.status();
        assert!(!status.counts_as_verified_evidence());
        let line = status.as_str();
        assert!(line.starts_with("INCOMPLETE["), "got {line}");
        assert!(
            line.contains("gap=db_integration(not_run:postgres)"),
            "the unrun database gate must be named: {line}"
        );
        assert!(
            line.contains("skipped=engine_enforces_correlation_premises(not_applicable)"),
            "the skipped gate must be named with its reason: {line}"
        );
        assert_ne!(line, "VERIFIED");
        assert!(VerificationStatus::parse("VERIFIED").is_err());
    }

    #[test]
    fn same_patch_with_the_database_gate_run_is_verified() {
        let mut checks = base_passed();
        checks.push(CheckOutcome::passed("db_integration"));
        checks.push(CheckOutcome::passed("engine_enforces_correlation_premises"));
        let record = record(&[MIGRATION, "engine/src/correlation.rs"], checks);
        assert!(record.fully_verified());
        assert!(record
            .status_line()
            .starts_with("PASSED[ran=cargo_fmt,cargo_clippy,cargo_test,db_integration"));
        assert!(record.status_line().contains("skipped=none"));
    }

    #[test]
    fn renderer_output_is_always_parseable() {
        // The renderer and the parser must agree, or a hand-written status can
        // slip through as "valid" while carrying no enumeration.
        let mut checks = base_passed();
        checks.push(CheckOutcome::skipped(
            "db_integration",
            SkipReason::FixtureUnavailable(FixtureClass::Postgres),
        ));
        checks.push(CheckOutcome::failed("extra_gate"));
        let status = record(&[MIGRATION], checks).status_line();
        assert!(
            status.contains("gap=db_integration(fixture_not_provisioned:postgres)"),
            "got {status}"
        );
        assert!(!status.contains("ran=none"), "got {status}");
        assert!(VerificationStatus::parse(&status).is_ok(), "got {status}");
    }

    #[test]
    fn slugs_are_container_safe() {
        assert_eq!(slug("Postgres"), "postgres");
        assert_eq!(slug("external:payments"), "external_payments");
        assert_eq!(slug("  --  "), "fixture");
        assert_eq!(
            FixtureClass::Postgres.container_name(),
            "autospec-fixture-postgres"
        );
    }
}
