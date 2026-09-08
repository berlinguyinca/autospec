//! Idle audit-remediation loop: when no executable issues remain, the ledger
//! records repeatable repository audits, classifies actionable findings,
//! schedules remediation through the normal Autospec lifecycle, re-audits
//! until the quality gate passes, and only then reports that feature
//! discovery may proceed. Credential-gated or unsafe findings remain tracked
//! instead of being silently skipped, and the quality-balance policy plus the
//! durable ledger stop feature throughput from starving quality work.

mod codec;
mod policy;

use std::collections::BTreeSet;

use crate::autonomous::waterfall::sha256_hex;

pub use policy::parse_policy;
pub use policy::QualityBalancePolicy;

pub const QUALITY_LEDGER_SCHEMA: u64 = 1;
pub const QUALITY_SHARE_BPS_TOTAL: u64 = 10_000;
pub const MAX_EVIDENCE_CHARS: usize = 4096;
pub const MAX_AFFECTED_PATHS: usize = 32;
pub const REVISION_HEX_LENGTH: usize = 40;
pub const FINDING_FINGERPRINT_HEX_LENGTH: usize = 64;

/// The ten dimensions a repeatable repository audit covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditDimension {
    Security,
    Correctness,
    Usability,
    DocumentationDrift,
    Maintainability,
    Duplication,
    Performance,
    Tests,
    Configuration,
    DeploymentSafety,
}

impl AuditDimension {
    pub const ALL: [Self; 10] = [
        Self::Security,
        Self::Correctness,
        Self::Usability,
        Self::DocumentationDrift,
        Self::Maintainability,
        Self::Duplication,
        Self::Performance,
        Self::Tests,
        Self::Configuration,
        Self::DeploymentSafety,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Security => "security",
            Self::Correctness => "correctness",
            Self::Usability => "usability",
            Self::DocumentationDrift => "documentation_drift",
            Self::Maintainability => "maintainability",
            Self::Duplication => "duplication",
            Self::Performance => "performance",
            Self::Tests => "tests",
            Self::Configuration => "configuration",
            Self::DeploymentSafety => "deployment_safety",
        }
    }

    pub fn parse(value: &str) -> Result<Self, String> {
        Self::ALL
            .iter()
            .find(|dimension| dimension.as_str() == value)
            .copied()
            .ok_or_else(|| format!("unknown audit dimension: {value}"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FindingSeverity {
    Critical,
    High,
    Medium,
    Low,
}

impl FindingSeverity {
    pub const ALL: [Self; 4] = [Self::Critical, Self::High, Self::Medium, Self::Low];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Critical => "critical",
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
        }
    }

    /// Higher rank means more severe. Used for plan ordering.
    pub fn rank(self) -> u8 {
        match self {
            Self::Critical => 3,
            Self::High => 2,
            Self::Medium => 1,
            Self::Low => 0,
        }
    }

    pub fn parse(value: &str) -> Result<Self, String> {
        Self::ALL
            .iter()
            .find(|severity| severity.as_str() == value)
            .copied()
            .ok_or_else(|| format!("unknown finding severity: {value}"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindingConfidence {
    High,
    Medium,
    Low,
}

impl FindingConfidence {
    pub const ALL: [Self; 3] = [Self::High, Self::Medium, Self::Low];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
        }
    }

    /// Higher rank means the observed evidence is stronger. Findings are
    /// ranked by observed confidence separately from their inferred
    /// severity.
    pub fn rank(self) -> u8 {
        match self {
            Self::High => 2,
            Self::Medium => 1,
            Self::Low => 0,
        }
    }

    pub fn parse(value: &str) -> Result<Self, String> {
        Self::ALL
            .iter()
            .find(|confidence| confidence.as_str() == value)
            .copied()
            .ok_or_else(|| format!("unknown finding confidence: {value}"))
    }
}

/// One observed audit finding. The fingerprint is derived, so a finding that
/// reappears with identical dimension, evidence, and affected paths dedups
/// against both ledger entries and pre-existing issues.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditFinding {
    pub dimension: AuditDimension,
    pub severity: FindingSeverity,
    pub confidence: FindingConfidence,
    pub evidence: String,
    pub affected_paths: Vec<String>,
    pub regression_test_required: bool,
    pub credential_gated: bool,
    pub safe_to_autofix: bool,
    /// When the finding is already tracked by an issue, deduplication must
    /// not file a second remediation issue.
    pub existing_issue: Option<u64>,
}

impl AuditFinding {
    pub fn fingerprint(&self) -> String {
        let mut paths = self.affected_paths.clone();
        paths.sort();
        paths.dedup();
        let identity = format!(
            "autospec-quality-finding-v1\n{}\n{}\n{}",
            self.dimension.as_str(),
            self.evidence.trim(),
            paths.join("\u{1f}")
        );
        sha256_hex(identity.as_bytes())
    }

    pub fn validate(&self) -> Result<(), String> {
        let evidence = self.evidence.trim();
        if evidence.is_empty() {
            return Err("quality finding evidence must not be empty".to_string());
        }
        if evidence.len() > MAX_EVIDENCE_CHARS {
            return Err(format!(
                "quality finding evidence must be at most {MAX_EVIDENCE_CHARS} characters"
            ));
        }
        if self.affected_paths.is_empty() {
            return Err("quality finding must name at least one affected path".to_string());
        }
        if self.affected_paths.len() > MAX_AFFECTED_PATHS {
            return Err(format!(
                "quality finding must name at most {MAX_AFFECTED_PATHS} affected paths"
            ));
        }
        for path in &self.affected_paths {
            if !is_valid_relative_path(path) {
                return Err(format!(
                    "quality finding affected path `{path}` must be a repository-relative path"
                ));
            }
        }
        if let Some(issue) = self.existing_issue {
            if issue == 0 {
                return Err("quality finding existing issue must be positive".to_string());
            }
        }
        Ok(())
    }
}

fn is_valid_relative_path(path: &str) -> bool {
    if path.is_empty()
        || path.len() > 256
        || path.contains(['?', '#', '\\', '\0'])
        || path.starts_with('/')
        || path.starts_with("./")
    {
        return false;
    }
    path.split('/')
        .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}

/// One sealed pass of the repeatable repository audit against an exact
/// revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditPass {
    pub repo: String,
    pub pass_id: u64,
    pub revision: String,
    pub findings: Vec<AuditFinding>,
}

impl AuditPass {
    pub fn validate(&self) -> Result<(), String> {
        if self.repo.trim().is_empty() {
            return Err("audit pass repository must not be empty".to_string());
        }
        if self.pass_id == 0 {
            return Err("audit pass id must be positive".to_string());
        }
        if !is_revision(&self.revision) {
            return Err(format!(
                "audit pass revision must be {REVISION_HEX_LENGTH} lower-case hexadecimal characters"
            ));
        }
        let mut seen = BTreeSet::new();
        for finding in &self.findings {
            finding.validate()?;
            let fingerprint = finding.fingerprint();
            if !seen.insert(fingerprint) {
                return Err("audit pass contains duplicate finding fingerprints".to_string());
            }
        }
        Ok(())
    }

    pub fn fingerprints(&self) -> Vec<String> {
        self.findings
            .iter()
            .map(AuditFinding::fingerprint)
            .collect()
    }

    /// Sealed digest of the whole pass: repeatable audits can be compared
    /// without re-running them.
    pub fn digest(&self) -> String {
        let mut identity = format!(
            "autospec-quality-audit-v1\n{}\n{}\n{}\n",
            self.repo, self.pass_id, self.revision
        );
        for fingerprint in self.fingerprints() {
            identity.push_str(&fingerprint);
            identity.push('\n');
        }
        sha256_hex(identity.as_bytes())
    }
}

fn is_revision(value: &str) -> bool {
    value.len() == REVISION_HEX_LENGTH
        && value.bytes().all(|byte| {
            byte.is_ascii_digit() || (byte.is_ascii_lowercase() && byte.is_ascii_hexdigit())
        })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindingStatus {
    /// Actionable and awaiting remediation through the normal lifecycle.
    Open,
    /// Credential-gated or unsafe to autofix: tracked, never silently skipped.
    Tracked,
    /// A safe verified fix landed.
    Remediated,
    /// Re-audits confirmed this finding is not a real defect.
    FalsePositive,
}

impl FindingStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Tracked => "tracked",
            Self::Remediated => "remediated",
            Self::FalsePositive => "false_positive",
        }
    }

    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "open" => Ok(Self::Open),
            "tracked" => Ok(Self::Tracked),
            "remediated" => Ok(Self::Remediated),
            "false_positive" => Ok(Self::FalsePositive),
            _ => Err(format!("unknown finding status: {value}")),
        }
    }
}

/// The outcome of one remediation round for a finding, verified through the
/// normal Autospec lifecycle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemediationOutcome {
    /// The safe verified fix landed and the full suite passed.
    Verified,
    /// The remediation attempt failed; the finding stays open.
    Failed { reason: String },
    /// Validation or CI checks blocked the remediation; the finding stays open.
    Blocked { reason: String },
}

impl RemediationOutcome {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Verified => Ok(()),
            Self::Failed { reason } | Self::Blocked { reason } if reason.trim().is_empty() => {
                Err("remediation outcome reason must not be empty".to_string())
            }
            _ => Ok(()),
        }
    }
}

/// Lifetime record for one finding fingerprint inside the durable ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerEntry {
    pub finding: AuditFinding,
    pub status: FindingStatus,
    pub first_seen_pass: u64,
    pub last_seen_pass: u64,
    /// The issue this finding is bound to (pre-existing or filed during a
    /// remediation round). Deduplication never files a second issue.
    pub issue: Option<u64>,
    pub attempts: u64,
    pub last_reason: Option<String>,
}

impl LedgerEntry {
    pub fn fingerprint(&self) -> String {
        self.finding.fingerprint()
    }
}

fn is_actionable(finding: &AuditFinding) -> bool {
    finding.safe_to_autofix && !finding.credential_gated
}

/// Evaluation of the defined quality gate over the latest audit pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualityGateResult {
    pub passed: bool,
    /// Open findings whose severity and confidence meet the blocking policy.
    pub blocking: Vec<String>,
    /// Credential-gated or unsafe findings: reported so they are never
    /// silently skipped, but they do not block the gate.
    pub tracked: Vec<String>,
    pub false_positives: Vec<String>,
}

/// What the idle loop does next after an audit or a remediation round.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdleCycleDecision {
    /// Quality gate passed and quality work is not starved: feature discovery
    /// may proceed. Tracked findings are carried so they stay visible.
    ProceedToDiscovery { tracked: Vec<String> },
    /// Run a remediation round for the plan through the normal lifecycle,
    /// record outcomes, then re-audit.
    Remediate { plan: Vec<String> },
    /// Gate passed but the quality share is below the balance floor: quality
    /// remediation is forced before discovery.
    Starved { plan: Vec<String> },
    /// Loop/budget termination: the gate has not passed but re-audit or
    /// remediation budget is spent (or no finding is remediable). Everything
    /// stays tracked rather than dropped.
    BudgetExhausted {
        open: Vec<String>,
        tracked: Vec<String>,
    },
}

impl IdleCycleDecision {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ProceedToDiscovery { .. } => "proceed_to_discovery",
            Self::Remediate { .. } => "remediate",
            Self::Starved { .. } => "starved",
            Self::BudgetExhausted { .. } => "budget_exhausted",
        }
    }
}

/// Reconciliation of one audit pass against the ledger.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AuditReconciliation {
    pub new: Vec<String>,
    pub retained: Vec<String>,
    /// Findings that were remediated (or reclassified to actionable) but
    /// reappeared in a repeat audit: the fix did not hold, so they reopen.
    pub regressed: Vec<String>,
}

/// Durable ledger for the idle audit-remediation loop. State is persisted as
/// JSON so feature throughput across many cycles cannot starve quality work:
/// every verified quality fix and every unit of feature work is counted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualityLedger {
    repo: String,
    pass_id: u64,
    revision: String,
    reaudits_this_cycle: u64,
    remediation_rounds_this_cycle: u64,
    pending_outcomes: u64,
    quality_work: u64,
    feature_work: u64,
    entries: Vec<LedgerEntry>,
    latest_fingerprints: Vec<String>,
}

impl QualityLedger {
    pub fn new(repo: impl Into<String>) -> Result<Self, String> {
        let repo = repo.into();
        if repo.trim().is_empty() {
            return Err("quality ledger repository must not be empty".to_string());
        }
        Ok(Self {
            repo,
            pass_id: 0,
            revision: String::new(),
            reaudits_this_cycle: 0,
            remediation_rounds_this_cycle: 0,
            pending_outcomes: 0,
            quality_work: 0,
            feature_work: 0,
            entries: Vec::new(),
            latest_fingerprints: Vec::new(),
        })
    }

    pub fn repo(&self) -> &str {
        &self.repo
    }

    pub fn pass_id(&self) -> u64 {
        self.pass_id
    }

    pub fn revision(&self) -> &str {
        &self.revision
    }

    pub fn reaudits_this_cycle(&self) -> u64 {
        self.reaudits_this_cycle
    }

    pub fn remediation_rounds_this_cycle(&self) -> u64 {
        self.remediation_rounds_this_cycle
    }

    pub fn quality_work(&self) -> u64 {
        self.quality_work
    }

    pub fn feature_work(&self) -> u64 {
        self.feature_work
    }

    pub fn entries(&self) -> &[LedgerEntry] {
        &self.entries
    }

    pub fn latest_fingerprints(&self) -> &[String] {
        &self.latest_fingerprints
    }

    /// Record one pass of the repeatable audit. Passes must arrive exactly in
    /// sequence, mirroring the no-work pass discipline.
    pub fn record_audit(&mut self, audit: &AuditPass) -> Result<AuditReconciliation, String> {
        audit.validate()?;
        if audit.repo != self.repo {
            return Err("audit pass repository does not match the ledger".to_string());
        }
        let expected = self
            .pass_id
            .checked_add(1)
            .ok_or_else(|| "quality ledger pass sequence overflow".to_string())?;
        if audit.pass_id != expected {
            return Err("audit pass must be exactly next in sequence".to_string());
        }

        self.pending_outcomes = 0;
        let mut reconciliation = AuditReconciliation::default();
        for finding in &audit.findings {
            let fingerprint = finding.fingerprint();
            match self
                .entries
                .iter_mut()
                .find(|entry| entry.fingerprint() == fingerprint)
            {
                Some(entry) => {
                    entry.finding = finding.clone();
                    entry.last_seen_pass = audit.pass_id;
                    match entry.status {
                        FindingStatus::Remediated if audit.pass_id > entry.first_seen_pass => {
                            entry.status = FindingStatus::Open;
                            reconciliation.regressed.push(fingerprint);
                        }
                        FindingStatus::Tracked if is_actionable(&entry.finding) => {
                            entry.status = FindingStatus::Open;
                            reconciliation.regressed.push(fingerprint);
                        }
                        _ => reconciliation.retained.push(fingerprint),
                    }
                }
                None => {
                    let status = if is_actionable(finding) {
                        FindingStatus::Open
                    } else {
                        FindingStatus::Tracked
                    };
                    self.entries.push(LedgerEntry {
                        finding: finding.clone(),
                        status,
                        first_seen_pass: audit.pass_id,
                        last_seen_pass: audit.pass_id,
                        issue: finding.existing_issue,
                        attempts: 0,
                        last_reason: None,
                    });
                    reconciliation.new.push(fingerprint);
                }
            }
        }

        self.pass_id = audit.pass_id;
        self.revision = audit.revision.clone();
        self.latest_fingerprints = audit.fingerprints();
        self.reaudits_this_cycle = self
            .reaudits_this_cycle
            .checked_add(1)
            .ok_or_else(|| "quality ledger re-audit counter overflow".to_string())?;
        self.validate_invariants()?;
        Ok(reconciliation)
    }

    /// Reset the loop/budget counters when an idle cycle ends (discovery
    /// started or budget was exhausted). Finding state is durable and
    /// survives the reset.
    pub fn end_cycle(&mut self) {
        self.reaudits_this_cycle = 0;
        self.remediation_rounds_this_cycle = 0;
        self.pending_outcomes = 0;
    }

    /// Findings in the latest pass that are open, safe, not credential-gated,
    /// not already tracked by a pre-existing issue, and within the per-finding
    /// attempt budget. Ordered by severity, then dimension, then fingerprint
    /// for a deterministic plan.
    pub fn remediation_plan(&self, policy: &QualityBalancePolicy) -> Vec<String> {
        let mut plan: Vec<(u8, usize, String)> = self
            .latest_fingerprints
            .iter()
            .filter_map(|fingerprint| {
                let entry = self
                    .entries()
                    .iter()
                    .find(|entry| &entry.fingerprint() == fingerprint)?;
                let actionable = entry.status == FindingStatus::Open
                    && is_actionable(&entry.finding)
                    && entry.finding.existing_issue.is_none()
                    && entry.attempts < policy.max_attempts_per_finding;
                actionable.then(|| {
                    (
                        entry.finding.severity.rank(),
                        AuditDimension::ALL
                            .iter()
                            .position(|dimension| *dimension == entry.finding.dimension)
                            .expect("closed dimension has a position"),
                        fingerprint.clone(),
                    )
                })
            })
            .collect();
        plan.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
        plan.into_iter()
            .map(|(_, _, fingerprint)| fingerprint)
            .collect()
    }

    /// Record the outcome of one remediation round for one finding. The issue
    /// binds the finding to the normal lifecycle so a later round dedups
    /// instead of filing a second issue.
    pub fn record_remediation(
        &mut self,
        fingerprint: &str,
        issue: u64,
        outcome: &RemediationOutcome,
    ) -> Result<(), String> {
        outcome.validate()?;
        if issue == 0 {
            return Err("remediation issue must be positive".to_string());
        }
        if !self.latest_fingerprints.iter().any(|f| f == fingerprint) {
            return Err("remediation finding is not present in the latest audit pass".to_string());
        }
        let entry = self
            .entries
            .iter_mut()
            .find(|entry| entry.fingerprint() == fingerprint)
            .ok_or_else(|| format!("unknown quality finding: {fingerprint}"))?;
        if entry.status != FindingStatus::Open {
            return Err(format!(
                "finding is not open for remediation (status: {})",
                entry.status.as_str()
            ));
        }
        if let Some(bound) = entry.issue {
            if bound != issue {
                return Err(format!(
                    "finding is already bound to issue {bound}, refusing duplicate issue {issue}"
                ));
            }
        }
        entry.issue = Some(issue);
        entry.attempts = entry
            .attempts
            .checked_add(1)
            .ok_or_else(|| "quality ledger attempt counter overflow".to_string())?;
        self.pending_outcomes += 1;
        match outcome {
            RemediationOutcome::Verified => {
                entry.status = FindingStatus::Remediated;
                entry.last_reason = None;
                self.quality_work = self
                    .quality_work
                    .checked_add(1)
                    .ok_or_else(|| "quality ledger quality-work counter overflow".to_string())?;
            }
            RemediationOutcome::Failed { reason } | RemediationOutcome::Blocked { reason } => {
                entry.last_reason = Some(reason.clone());
            }
        }
        Ok(())
    }

    /// Close one remediation round after all of its outcomes are recorded.
    /// The round budget only advances for rounds that actually ran.
    pub fn complete_remediation_round(&mut self) -> Result<(), String> {
        if self.pending_outcomes == 0 {
            return Err("no remediation outcomes recorded for this round".to_string());
        }
        self.remediation_rounds_this_cycle = self
            .remediation_rounds_this_cycle
            .checked_add(1)
            .ok_or_else(|| "quality ledger remediation round counter overflow".to_string())?;
        self.pending_outcomes = 0;
        Ok(())
    }

    /// Mark an open or tracked finding as a confirmed false positive after a
    /// repeat audit. False positives never re-enter the remediation plan.
    pub fn mark_false_positive(&mut self, fingerprint: &str, reason: &str) -> Result<(), String> {
        if reason.trim().is_empty() {
            return Err("false-positive reason must not be empty".to_string());
        }
        let entry = self
            .entries
            .iter_mut()
            .find(|entry| entry.fingerprint() == fingerprint)
            .ok_or_else(|| format!("unknown quality finding: {fingerprint}"))?;
        match entry.status {
            FindingStatus::Open | FindingStatus::Tracked => {}
            other => {
                return Err(format!(
                    "finding cannot be reclassified to false positive from status {}",
                    other.as_str()
                ))
            }
        }
        entry.status = FindingStatus::FalsePositive;
        entry.last_reason = Some(reason.to_string());
        Ok(())
    }

    /// Count one unit of feature work against the balance floor. Called by the
    /// lifecycle when a feature issue completes so quality work can be seen
    /// to be starving.
    pub fn record_feature_work(&mut self, count: u64) {
        self.feature_work = self.feature_work.saturating_add(count);
    }

    /// Quality share of recorded work in basis points, or `None` when nothing
    /// has been recorded yet.
    pub fn quality_share_bps(&self) -> Option<u64> {
        let total = self.quality_work.saturating_add(self.feature_work);
        (total > 0).then(|| self.quality_work.saturating_mul(QUALITY_SHARE_BPS_TOTAL) / total)
    }

    /// Sealed digest of the latest recorded pass, so a persisted ledger can
    /// be re-verified against its own audit trail.
    pub fn digest(&self) -> String {
        let findings = self
            .latest_fingerprints
            .iter()
            .filter_map(|fingerprint| {
                self.entries
                    .iter()
                    .find(|entry| &entry.fingerprint() == fingerprint)
                    .map(|entry| entry.finding.clone())
            })
            .collect::<Vec<_>>();
        AuditPass {
            repo: self.repo.clone(),
            pass_id: self.pass_id,
            revision: self.revision.clone(),
            findings,
        }
        .digest()
    }

    /// True when feature throughput has pushed the quality share below the
    /// balance floor.
    pub fn is_quality_starved(&self, policy: &QualityBalancePolicy) -> bool {
        self.quality_share_bps()
            .is_some_and(|share| share < policy.min_quality_share_bps)
    }

    pub fn gate(&self, policy: &QualityBalancePolicy) -> QualityGateResult {
        let mut blocking = Vec::new();
        let mut tracked = Vec::new();
        let mut false_positives = Vec::new();
        for fingerprint in &self.latest_fingerprints {
            let Some(entry) = self
                .entries
                .iter()
                .find(|entry| &entry.fingerprint() == fingerprint)
            else {
                continue;
            };
            match entry.status {
                FindingStatus::Open
                    if policy.blocking_severities.contains(&entry.finding.severity)
                        && entry.finding.confidence.rank()
                            >= policy.blocking_min_confidence.rank() =>
                {
                    blocking.push(fingerprint.clone());
                }
                FindingStatus::Tracked => tracked.push(fingerprint.clone()),
                FindingStatus::FalsePositive => false_positives.push(fingerprint.clone()),
                _ => {}
            }
        }
        QualityGateResult {
            passed: blocking.is_empty(),
            blocking,
            tracked,
            false_positives,
        }
    }

    /// The next step of the idle audit-remediation loop.
    pub fn decision(&self, policy: &QualityBalancePolicy) -> IdleCycleDecision {
        let gate = self.gate(policy);
        if gate.passed {
            if self.is_quality_starved(policy) {
                let plan = self.remediation_plan(policy);
                if !plan.is_empty() {
                    return IdleCycleDecision::Starved { plan };
                }
            }
            return IdleCycleDecision::ProceedToDiscovery {
                tracked: gate.tracked,
            };
        }
        let budget_exhausted = self.reaudits_this_cycle >= policy.max_reaudits
            || self.remediation_rounds_this_cycle >= policy.max_remediation_rounds;
        if budget_exhausted {
            return IdleCycleDecision::BudgetExhausted {
                open: gate.blocking,
                tracked: gate.tracked,
            };
        }
        let plan = self.remediation_plan(policy);
        if plan.is_empty() {
            IdleCycleDecision::BudgetExhausted {
                open: gate.blocking,
                tracked: gate.tracked,
            }
        } else {
            IdleCycleDecision::Remediate { plan }
        }
    }

    pub fn to_json(&self) -> String {
        codec::ledger_json(self)
    }

    pub fn parse_json(input: &str) -> Result<Self, String> {
        codec::parse_ledger(input)
    }

    fn validate_invariants(&self) -> Result<(), String> {
        if self.pass_id == 0 {
            if !self.revision.is_empty() || !self.latest_fingerprints.is_empty() {
                return Err("fresh quality ledger must not carry audit state".to_string());
            }
            if !self.entries.is_empty() {
                return Err("fresh quality ledger must not carry entries".to_string());
            }
            if self.reaudits_this_cycle != 0
                || self.remediation_rounds_this_cycle != 0
                || self.pending_outcomes != 0
            {
                return Err("fresh quality ledger must have zero loop counters".to_string());
            }
        } else if !is_revision(&self.revision) {
            return Err(format!(
                "quality ledger revision must be {REVISION_HEX_LENGTH} lower-case hexadecimal characters"
            ));
        }
        let mut seen = BTreeSet::new();
        for entry in &self.entries {
            entry.finding.validate()?;
            let fingerprint = entry.fingerprint();
            if !seen.insert(fingerprint.clone()) {
                return Err(format!("duplicate quality finding: {fingerprint}"));
            }
            if entry.first_seen_pass == 0
                || entry.last_seen_pass < entry.first_seen_pass
                || entry.first_seen_pass > self.pass_id
                || entry.last_seen_pass > self.pass_id
            {
                return Err(format!(
                    "quality finding pass range is inconsistent: {fingerprint}"
                ));
            }
            if entry.status == FindingStatus::Tracked && is_actionable(&entry.finding) {
                return Err(format!(
                    "tracked finding must not be actionable: {fingerprint}"
                ));
            }
            if let Some(issue) = entry.issue {
                if issue == 0 {
                    return Err(format!(
                        "quality finding issue must be positive: {fingerprint}"
                    ));
                }
            }
        }
        for fingerprint in &self.latest_fingerprints {
            if !seen.contains(fingerprint) {
                return Err(format!(
                    "latest audit references unknown quality finding: {fingerprint}"
                ));
            }
        }
        if self.remediation_rounds_this_cycle > self.reaudits_this_cycle {
            return Err("quality ledger remediation rounds cannot exceed re-audits".to_string());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPO: &str = "berlinguyinca/autospec";
    const REVISION: &str = "0123456789abcdef0123456789abcdef01234567";

    fn policy() -> QualityBalancePolicy {
        QualityBalancePolicy::default()
    }

    fn finding(
        dimension: AuditDimension,
        severity: FindingSeverity,
        confidence: FindingConfidence,
        evidence: &str,
        path: &str,
    ) -> AuditFinding {
        AuditFinding {
            dimension,
            severity,
            confidence,
            evidence: evidence.to_string(),
            affected_paths: vec![path.to_string()],
            regression_test_required: false,
            credential_gated: false,
            safe_to_autofix: true,
            existing_issue: None,
        }
    }

    fn pass(repo: &str, pass_id: u64, findings: Vec<AuditFinding>) -> AuditPass {
        AuditPass {
            repo: repo.to_string(),
            pass_id,
            revision: REVISION.to_string(),
            findings,
        }
    }

    fn fingerprint_of(finding: &AuditFinding) -> String {
        finding.fingerprint()
    }

    #[test]
    fn dimensions_cover_the_audit_surface() {
        assert_eq!(
            AuditDimension::ALL.map(|d| d.as_str()),
            [
                "security",
                "correctness",
                "usability",
                "documentation_drift",
                "maintainability",
                "duplication",
                "performance",
                "tests",
                "configuration",
                "deployment_safety"
            ]
        );
        assert_eq!(
            AuditDimension::parse("deployment_safety"),
            Ok(AuditDimension::DeploymentSafety)
        );
        assert!(AuditDimension::parse("style").is_err());
    }

    #[test]
    fn finding_fingerprint_is_deterministic_and_path_order_independent() {
        let first = finding(
            AuditDimension::Security,
            FindingSeverity::Critical,
            FindingConfidence::High,
            "hardcoded secret in scripts/deploy.sh",
            "scripts/deploy.sh",
        );
        let mut second = first.clone();
        second.evidence = "  hardcoded secret in scripts/deploy.sh  ".to_string();
        assert_eq!(first.fingerprint(), second.fingerprint());

        let mut reordered = first.clone();
        reordered.affected_paths = vec!["b.txt".to_string(), "a.txt".to_string()];
        let mut other = first.clone();
        other.affected_paths = vec!["a.txt".to_string(), "b.txt".to_string()];
        assert_eq!(reordered.fingerprint(), other.fingerprint());

        let different_evidence = finding(
            AuditDimension::Security,
            FindingSeverity::Critical,
            FindingConfidence::High,
            "hardcoded secret in scripts/publish.sh",
            "scripts/deploy.sh",
        );
        assert_ne!(first.fingerprint(), different_evidence.fingerprint());
    }

    #[test]
    fn finding_validation_rejects_unsafe_paths_and_missing_evidence() {
        let mut bad_path = finding(
            AuditDimension::Security,
            FindingSeverity::High,
            FindingConfidence::High,
            "evidence",
            "../outside.txt",
        );
        assert!(bad_path.validate().is_err());
        bad_path.affected_paths = vec!["/absolute.txt".to_string()];
        assert!(bad_path.validate().is_err());
        bad_path.affected_paths = vec!["ok.txt".to_string()];
        bad_path.evidence = "   ".to_string();
        assert!(bad_path.validate().is_err());

        let mut no_paths = finding(
            AuditDimension::Tests,
            FindingSeverity::Low,
            FindingConfidence::Low,
            "evidence",
            "ok.txt",
        );
        no_paths.affected_paths.clear();
        assert!(no_paths.validate().is_err());

        let mut zero_issue = finding(
            AuditDimension::Tests,
            FindingSeverity::Low,
            FindingConfidence::Low,
            "evidence",
            "ok.txt",
        );
        zero_issue.existing_issue = Some(0);
        assert!(zero_issue.validate().is_err());
    }

    #[test]
    fn empty_queue_empty_audit_proceeds_to_discovery() {
        let mut ledger = QualityLedger::new(REPO).unwrap();
        ledger.record_audit(&pass(REPO, 1, vec![])).unwrap();

        let gate = ledger.gate(&policy());
        assert!(gate.passed);
        match ledger.decision(&policy()) {
            IdleCycleDecision::ProceedToDiscovery { tracked } => {
                assert!(tracked.is_empty());
            }
            other => panic!("expected proceed to discovery, got {}", other.as_str()),
        }
        assert_eq!(ledger.quality_share_bps(), None);
    }

    #[test]
    fn critical_finding_blocks_gate_and_remediation_clears_it() {
        let critical = finding(
            AuditDimension::Security,
            FindingSeverity::Critical,
            FindingConfidence::High,
            "credential hardcoded in config/prod.yml",
            "config/prod.yml",
        );
        let fp = fingerprint_of(&critical);
        let mut ledger = QualityLedger::new(REPO).unwrap();
        ledger.record_audit(&pass(REPO, 1, vec![critical])).unwrap();

        assert!(!ledger.gate(&policy()).passed);
        let plan = ledger.remediation_plan(&policy());
        assert_eq!(plan, vec![fp.clone()]);

        ledger
            .record_remediation(&fp, 42, &RemediationOutcome::Verified)
            .unwrap();
        ledger.complete_remediation_round().unwrap();
        assert_eq!(ledger.quality_work(), 1);

        // Re-audit: the finding is gone from the latest pass, so the gate
        // passes even though the entry is durable.
        ledger.record_audit(&pass(REPO, 2, vec![])).unwrap();
        let gate = ledger.gate(&policy());
        assert!(gate.passed);
        assert!(matches!(
            ledger.decision(&policy()),
            IdleCycleDecision::ProceedToDiscovery { .. }
        ));
    }

    #[test]
    fn remediated_finding_reappearing_regresses_to_open() {
        let finding = finding(
            AuditDimension::Duplication,
            FindingSeverity::Critical,
            FindingConfidence::High,
            "copy-pasted retry loop in two commands",
            "crates/example-app/src/commands/queue.rs",
        );
        let fp = fingerprint_of(&finding);
        let mut ledger = QualityLedger::new(REPO).unwrap();
        ledger
            .record_audit(&pass(REPO, 1, vec![finding.clone()]))
            .unwrap();
        ledger
            .record_remediation(&fp, 7, &RemediationOutcome::Verified)
            .unwrap();
        ledger.complete_remediation_round().unwrap();

        let reconciliation = ledger.record_audit(&pass(REPO, 2, vec![finding])).unwrap();
        assert_eq!(reconciliation.regressed, vec![fp.clone()]);
        assert!(ledger.gate(&policy()).blocking.contains(&fp));
    }

    #[test]
    fn false_positives_never_block_or_replan() {
        let original = finding(
            AuditDimension::Security,
            FindingSeverity::Critical,
            FindingConfidence::Low,
            "generated lockfile flagged by scanner",
            "Cargo.lock",
        );
        let fp = fingerprint_of(&original);
        let mut ledger = QualityLedger::new(REPO).unwrap();
        ledger.record_audit(&pass(REPO, 1, vec![original])).unwrap();
        ledger
            .mark_false_positive(&fp, "scanner flags generated lockfile")
            .unwrap();

        let gate = ledger.gate(&policy());
        assert!(gate.passed);
        assert_eq!(gate.false_positives, vec![fp.clone()]);
        assert!(ledger.remediation_plan(&policy()).is_empty());

        // Repeat audit: the false positive stays a false positive.
        let finding_again = finding(
            AuditDimension::Security,
            FindingSeverity::Critical,
            FindingConfidence::Low,
            "generated lockfile flagged by scanner",
            "Cargo.lock",
        );
        let reconciliation = ledger
            .record_audit(&pass(REPO, 2, vec![finding_again]))
            .unwrap();
        assert_eq!(reconciliation.retained, vec![fp.clone()]);
        assert!(ledger.remediation_plan(&policy()).is_empty());
    }

    #[test]
    fn credential_gated_and_unsafe_findings_stay_tracked() {
        let mut gated = finding(
            AuditDimension::DeploymentSafety,
            FindingSeverity::Critical,
            FindingConfidence::High,
            "deploy key rotation requires operator credentials",
            "scripts/deploy.sh",
        );
        gated.credential_gated = true;
        let mut unsafe_finding = finding(
            AuditDimension::Configuration,
            FindingSeverity::Critical,
            FindingConfidence::High,
            "schema migration is unsafe to run autonomously",
            "scripts/migrate.sh",
        );
        unsafe_finding.safe_to_autofix = false;
        let gated_fp = fingerprint_of(&gated);
        let unsafe_fp = fingerprint_of(&unsafe_finding);

        let mut ledger = QualityLedger::new(REPO).unwrap();
        ledger
            .record_audit(&pass(REPO, 1, vec![gated, unsafe_finding]))
            .unwrap();

        let gate = ledger.gate(&policy());
        assert!(gate.passed, "tracked findings must not block the gate");
        assert_eq!(gate.tracked, vec![gated_fp.clone(), unsafe_fp.clone()]);
        assert!(ledger.remediation_plan(&policy()).is_empty());

        match ledger.decision(&policy()) {
            IdleCycleDecision::ProceedToDiscovery { tracked } => {
                assert_eq!(tracked, vec![gated_fp, unsafe_fp]);
            }
            other => panic!("expected proceed to discovery, got {}", other.as_str()),
        }
    }

    #[test]
    fn dedup_against_existing_issues_never_files_a_second_issue() {
        let mut finding = finding(
            AuditDimension::Maintainability,
            FindingSeverity::Critical,
            FindingConfidence::High,
            "oversized orchestrator module",
            "crates/example-app/src/commands/autonomous.rs",
        );
        finding.existing_issue = Some(111);
        let fp = fingerprint_of(&finding);
        let mut ledger = QualityLedger::new(REPO).unwrap();
        ledger.record_audit(&pass(REPO, 1, vec![finding])).unwrap();

        // Open and blocking, but already tracked: the idle plan must not
        // schedule it and the entry is bound to the pre-existing issue.
        assert!(ledger.remediation_plan(&policy()).is_empty());
        let entry = &ledger.entries()[0];
        assert_eq!(entry.issue, Some(111));

        // A second issue for the same fingerprint is refused.
        assert!(ledger
            .record_remediation(&fp, 222, &RemediationOutcome::Verified)
            .is_err());
        // The bound issue may complete the remediation.
        ledger
            .record_remediation(&fp, 111, &RemediationOutcome::Verified)
            .unwrap();
        ledger.complete_remediation_round().unwrap();
        assert!(ledger.gate(&policy()).passed);
    }

    #[test]
    fn remediation_failure_keeps_finding_open_until_budget_exhausts() {
        let finding = finding(
            AuditDimension::Correctness,
            FindingSeverity::Critical,
            FindingConfidence::High,
            "off-by-one in batch size policy",
            "crates/autospec-core/src/autonomous/config.rs",
        );
        let fp = fingerprint_of(&finding);
        let mut ledger = QualityLedger::new(REPO).unwrap();
        ledger.record_audit(&pass(REPO, 1, vec![finding])).unwrap();

        let policy = QualityBalancePolicy {
            max_remediation_rounds: 2,
            ..QualityBalancePolicy::default()
        };

        // Round 1: blocked by validation.
        assert!(matches!(
            ledger.decision(&policy),
            IdleCycleDecision::Remediate { .. }
        ));
        ledger
            .record_remediation(
                &fp,
                5,
                &RemediationOutcome::Blocked {
                    reason: "main-health pending".to_string(),
                },
            )
            .unwrap();
        ledger.complete_remediation_round().unwrap();

        // Round 2: the same bound issue fails.
        ledger
            .record_remediation(
                &fp,
                5,
                &RemediationOutcome::Failed {
                    reason: "test regression in config parser".to_string(),
                },
            )
            .unwrap();
        ledger.complete_remediation_round().unwrap();

        let decision = ledger.decision(&policy);
        match decision {
            IdleCycleDecision::BudgetExhausted { open, tracked } => {
                assert_eq!(open, vec![fp]);
                assert!(tracked.is_empty());
            }
            other => panic!("expected budget exhausted, got {}", other.as_str()),
        }
        let entry = &ledger.entries()[0];
        assert_eq!(entry.attempts, 2);
        assert_eq!(
            entry.last_reason.as_deref(),
            Some("test regression in config parser")
        );
    }

    #[test]
    fn repeat_audits_beyond_budget_terminate_the_loop() {
        let finding = finding(
            AuditDimension::Performance,
            FindingSeverity::Critical,
            FindingConfidence::High,
            "quadratic scan in claim sweep",
            "crates/autospec-core/src/claim/mod.rs",
        );
        let fp = fingerprint_of(&finding);
        let mut ledger = QualityLedger::new(REPO).unwrap();
        let policy = QualityBalancePolicy {
            max_reaudits: 2,
            max_remediation_rounds: 5,
            ..QualityBalancePolicy::default()
        };

        for pass_id in 1..=3u64 {
            ledger
                .record_audit(&pass(REPO, pass_id, vec![finding.clone()]))
                .unwrap();
        }

        match ledger.decision(&policy) {
            IdleCycleDecision::BudgetExhausted { open, .. } => {
                assert_eq!(open, vec![fp.clone()]);
            }
            other => panic!("expected budget exhausted, got {}", other.as_str()),
        }

        // A new idle cycle resets the loop budget and retries the finding.
        ledger.end_cycle();
        assert_eq!(ledger.reaudits_this_cycle(), 0);
        let plan = ledger.remediation_plan(&policy);
        assert_eq!(plan, vec![fp.clone()]);
        assert!(matches!(
            ledger.decision(&policy),
            IdleCycleDecision::Remediate { .. }
        ));
    }

    #[test]
    fn attempts_budget_exhaustion_terminates_without_reaudits() {
        let finding = finding(
            AuditDimension::Tests,
            FindingSeverity::Critical,
            FindingConfidence::High,
            "missing smoke test for sentinel state",
            "tests/autonomous/test_sentinel.bats",
        );
        let fp = fingerprint_of(&finding);
        let mut ledger = QualityLedger::new(REPO).unwrap();
        let policy = QualityBalancePolicy {
            max_attempts_per_finding: 1,
            ..QualityBalancePolicy::default()
        };
        ledger.record_audit(&pass(REPO, 1, vec![finding])).unwrap();
        ledger
            .record_remediation(
                &fp,
                9,
                &RemediationOutcome::Failed {
                    reason: "flaky in CI".to_string(),
                },
            )
            .unwrap();
        ledger.complete_remediation_round().unwrap();

        match ledger.decision(&policy) {
            IdleCycleDecision::BudgetExhausted { open, .. } => {
                assert_eq!(open, vec![fp]);
            }
            other => panic!("expected budget exhausted, got {}", other.as_str()),
        }
    }

    #[test]
    fn balance_floor_forces_quality_work_before_discovery() {
        let policy = QualityBalancePolicy {
            min_quality_share_bps: 50,
            ..QualityBalancePolicy::default()
        };
        let finding = finding(
            AuditDimension::Usability,
            FindingSeverity::High,
            FindingConfidence::Medium,
            "error message leaks internals",
            "crates/example-app/src/commands/status.rs",
        );
        let fp = fingerprint_of(&finding);
        let mut ledger = QualityLedger::new(REPO).unwrap();
        ledger.record_feature_work(100);
        ledger.record_audit(&pass(REPO, 1, vec![finding])).unwrap();

        // A high-severity finding does not block under the default gate, so
        // the gate passes even though quality work is starved.
        let gate = ledger.gate(&policy);
        assert!(gate.passed);
        assert!(ledger.is_quality_starved(&policy));
        assert_eq!(ledger.quality_share_bps(), Some(0));

        match ledger.decision(&policy) {
            IdleCycleDecision::Starved { plan } => {
                assert_eq!(plan, vec![fp.clone()]);
            }
            other => panic!("expected starved, got {}", other.as_str()),
        }

        // After a verified quality fix the share recovers and discovery
        // proceeds.
        ledger
            .record_remediation(&fp, 3, &RemediationOutcome::Verified)
            .unwrap();
        ledger.complete_remediation_round().unwrap();
        ledger.record_audit(&pass(REPO, 2, vec![])).unwrap();
        assert!(!ledger.is_quality_starved(&policy));
        assert!(matches!(
            ledger.decision(&policy),
            IdleCycleDecision::ProceedToDiscovery { .. }
        ));
    }

    #[test]
    fn low_confidence_findings_do_not_block_the_gate() {
        let finding = finding(
            AuditDimension::Security,
            FindingSeverity::Critical,
            FindingConfidence::Low,
            "inferred secret exposure without reproduction",
            "scripts/deploy.sh",
        );
        let mut ledger = QualityLedger::new(REPO).unwrap();
        ledger.record_audit(&pass(REPO, 1, vec![finding])).unwrap();

        // The default gate requires high confidence: observed evidence is
        // ranked separately from inferred impact.
        assert!(ledger.gate(&policy()).passed);

        let strict = QualityBalancePolicy {
            blocking_min_confidence: FindingConfidence::Low,
            ..QualityBalancePolicy::default()
        };
        assert!(!ledger.gate(&strict).passed);
    }

    #[test]
    fn pass_sequencing_and_repo_identity_are_enforced() {
        let mut ledger = QualityLedger::new(REPO).unwrap();
        assert!(ledger.record_audit(&pass(REPO, 2, vec![])).is_err());
        ledger.record_audit(&pass(REPO, 1, vec![])).unwrap();
        assert!(ledger.record_audit(&pass("other/repo", 2, vec![])).is_err());
        assert!(ledger.record_audit(&pass(REPO, 4, vec![])).is_err());
    }

    #[test]
    fn remediation_requires_an_open_finding_from_the_latest_pass() {
        let finding = finding(
            AuditDimension::Security,
            FindingSeverity::Critical,
            FindingConfidence::High,
            "evidence",
            "scripts/deploy.sh",
        );
        let fp = fingerprint_of(&finding);
        let mut ledger = QualityLedger::new(REPO).unwrap();
        assert!(ledger
            .record_remediation(&fp, 1, &RemediationOutcome::Verified)
            .is_err());

        ledger.record_audit(&pass(REPO, 1, vec![finding])).unwrap();
        assert!(ledger
            .record_remediation(
                &fp,
                1,
                &RemediationOutcome::Failed {
                    reason: "".to_string()
                }
            )
            .is_err());
        assert!(ledger
            .record_remediation(&fp, 1, &RemediationOutcome::Verified)
            .is_ok());
        assert!(ledger
            .record_remediation(&fp, 1, &RemediationOutcome::Verified)
            .is_err());

        // After the latest pass no longer reports the finding it cannot be
        // remediated from that pass.
        ledger.record_audit(&pass(REPO, 2, vec![])).unwrap();
        assert!(ledger
            .record_remediation(&fp, 1, &RemediationOutcome::Verified)
            .is_err());
    }

    #[test]
    fn round_completion_requires_recorded_outcomes() {
        let mut ledger = QualityLedger::new(REPO).unwrap();
        assert!(ledger.complete_remediation_round().is_err());
        ledger.record_audit(&pass(REPO, 1, vec![])).unwrap();
        assert!(ledger.complete_remediation_round().is_err());
    }

    #[test]
    fn empty_round_and_duplicate_issue_are_refused() {
        let finding = finding(
            AuditDimension::Security,
            FindingSeverity::Critical,
            FindingConfidence::High,
            "evidence",
            "scripts/deploy.sh",
        );
        let fp = fingerprint_of(&finding);
        let mut ledger = QualityLedger::new(REPO).unwrap();
        ledger.record_audit(&pass(REPO, 1, vec![finding])).unwrap();
        ledger
            .record_remediation(
                &fp,
                1,
                &RemediationOutcome::Blocked {
                    reason: "check pending".to_string(),
                },
            )
            .unwrap();
        assert!(ledger
            .record_remediation(&fp, 2, &RemediationOutcome::Verified)
            .is_err());
        ledger.complete_remediation_round().unwrap();
        assert_eq!(ledger.remediation_rounds_this_cycle(), 1);
    }

    #[test]
    fn ledger_round_trips_through_json() {
        let finding = finding(
            AuditDimension::Security,
            FindingSeverity::Critical,
            FindingConfidence::High,
            "credential hardcoded in config/prod.yml",
            "config/prod.yml",
        );
        let fp = fingerprint_of(&finding);
        let mut ledger = QualityLedger::new(REPO).unwrap();
        ledger.record_audit(&pass(REPO, 1, vec![finding])).unwrap();
        ledger
            .record_remediation(
                &fp,
                42,
                &RemediationOutcome::Blocked {
                    reason: "main-health pending".to_string(),
                },
            )
            .unwrap();
        ledger.complete_remediation_round().unwrap();
        ledger.record_feature_work(3);

        let restored = QualityLedger::parse_json(&ledger.to_json()).unwrap();
        assert_eq!(restored, ledger);
        assert_eq!(restored.pass_id(), 1);
        assert_eq!(restored.entries()[0].issue, Some(42));
    }

    #[test]
    fn ledger_json_rejects_tampering() {
        let mut ledger = QualityLedger::new(REPO).unwrap();
        ledger.record_audit(&pass(REPO, 1, vec![])).unwrap();
        let json = ledger.to_json();

        // Bumped pass counter without a matching audit.
        let tampered = json.replace("\"pass_id\":1", "\"pass_id\":2");
        assert!(QualityLedger::parse_json(&tampered).is_err());
        // Unknown schema.
        let tampered = json.replace("\"schema\":1", "\"schema\":9");
        assert!(QualityLedger::parse_json(&tampered).is_err());
        // Unknown top-level field.
        let tampered = format!("{},\"injected\":true}}", &json[..json.len() - 1]);
        assert!(QualityLedger::parse_json(&tampered).is_err());
        // Garbage.
        assert!(QualityLedger::parse_json("not json").is_err());
    }

    #[test]
    fn ledger_digest_seals_the_latest_pass() {
        let finding = finding(
            AuditDimension::Security,
            FindingSeverity::Critical,
            FindingConfidence::High,
            "evidence",
            "scripts/deploy.sh",
        );
        let mut ledger = QualityLedger::new(REPO).unwrap();
        ledger
            .record_audit(&pass(REPO, 1, vec![finding.clone()]))
            .unwrap();
        assert_eq!(ledger.digest(), pass(REPO, 1, vec![finding]).digest());
    }

    #[test]
    fn audit_digest_is_sealed_and_repeatable() {
        let finding = finding(
            AuditDimension::Security,
            FindingSeverity::Critical,
            FindingConfidence::High,
            "evidence",
            "scripts/deploy.sh",
        );
        let first = pass(REPO, 1, vec![finding.clone()]);
        let second = pass(REPO, 1, vec![finding.clone()]);
        assert_eq!(first.digest(), second.digest());
        assert_eq!(first.digest().len(), FINDING_FINGERPRINT_HEX_LENGTH);

        let other = pass(REPO, 2, vec![finding]);
        assert_ne!(first.digest(), other.digest());
    }
}
