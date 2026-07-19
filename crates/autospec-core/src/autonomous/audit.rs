#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureClass {
    GuidelineViolation,
    NoWork,
    Stuck,
    FalsePositive,
    ScopeDrift,
    ValidationBlocked,
    Runaway,
    Other,
}

impl FailureClass {
    pub fn as_str(&self) -> &'static str {
        match self {
            FailureClass::GuidelineViolation => "guideline-violation",
            FailureClass::NoWork => "no-work",
            FailureClass::Stuck => "stuck",
            FailureClass::FalsePositive => "false-positive",
            FailureClass::ScopeDrift => "scope-drift",
            FailureClass::ValidationBlocked => "validation-blocked",
            FailureClass::Runaway => "runaway",
            FailureClass::Other => "other",
        }
    }
}

pub fn classify_failure(text: &str) -> FailureClass {
    let text = text.to_ascii_lowercase();

    if contains_any(
        &text,
        &[
            "worktree",
            "direct-main",
            "direct main",
            "primary checkout",
            "wrong checkout",
            "main branch",
            "dry-run silently runs live",
            "dry run silently runs live",
            "mutates github",
            "pushed directly to main",
            "pushing directly to main",
            "committed directly on main",
            "committed directly to main",
            "committing directly on main",
            "committing directly to main",
        ],
    ) {
        FailureClass::GuidelineViolation
    } else if contains_any(
        &text,
        &[
            "no-op",
            "no op",
            "noop",
            "no useful work",
            "filed=0",
            "filed zero",
            "empty queue",
            "zero proposals",
            "structurally dry",
            "dry promotion",
            "idle-rescan",
            "convergence-park",
        ],
    ) {
        FailureClass::NoWork
    } else if contains_any(
        &text,
        &[
            "stuck",
            "hang",
            "hung",
            "stall",
            "stalled",
            "stalling",
            "watchdog",
            "reclaim",
            "reclaimed",
            "heartbeat",
            "lock",
            "liveness",
            "crash",
            "crashed",
        ],
    ) {
        FailureClass::Stuck
    } else if contains_any(
        &text,
        &[
            "gitleaks",
            "false-positive",
            "false positive",
            "generated",
            "cache",
            "node_modules",
            ".next",
            "dist/",
            "out/",
            "build cache",
            "build caches",
        ],
    ) {
        FailureClass::FalsePositive
    } else if contains_any(
        &text,
        &[
            "scope drift",
            "scope:",
            "quarantine",
            "unrelated",
            "off-scope",
            "off scope",
        ],
    ) {
        FailureClass::ScopeDrift
    } else if contains_any(
        &text,
        &[
            "validation-blocked",
            "validation blocked",
            "blocked by validation",
            "failed validation",
            "validation failed",
            "ci failed",
            "ci failure",
            "check failed",
            "check failure",
            "failing check",
            "failing checks",
            "pre-existing fail",
            "main-health pending",
        ],
    ) {
        FailureClass::ValidationBlocked
    } else {
        FailureClass::Other
    }
}

fn contains_any(text: &str, patterns: &[&str]) -> bool {
    patterns.iter().any(|pattern| text.contains(pattern))
}
