//! The issue-tracker interactions a stalled release needs.
//!
//! A trait, deliberately: the release path must work against GitHub, a local
//! markdown tracker, or a test double, so nothing below this boundary assumes
//! `gh`, labels by GitHub name, or a hosted service at all.

use std::fmt;

use super::note::SpecRepairReport;
use super::partial_work::Artifact;

/// Which issue to act on. `project` is tracker-scoped: a GitHub `owner/name`, a
/// Jira project key, or a local directory name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueRef {
    pub project: String,
    pub number: u64,
}

impl IssueRef {
    pub fn new(project: impl Into<String>, number: u64) -> Self {
        Self {
            project: project.into(),
            number,
        }
    }
}

/// A tracker call that did not go through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackerError(pub String);

impl fmt::Display for TrackerError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for TrackerError {}

impl TrackerError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

/// What the release path asks of an issue tracker.
pub trait IssueTracker {
    /// Put the issue back on the queue, dropping the in-progress marker, with
    /// the stall note attached.
    fn release_to_queue(&mut self, issue: &IssueRef, note: &str) -> Result<(), TrackerError>;

    /// Increment and return the attempt counter for this issue.
    fn bump_attempt_counter(&mut self, issue: &IssueRef) -> Result<u32, TrackerError>;

    /// Attach captured partial work so the next attempt can read it.
    fn attach(&mut self, issue: &IssueRef, artifact: &Artifact) -> Result<(), TrackerError>;

    /// Apply a label, e.g. marking that attempts are exhausted.
    fn add_label(&mut self, issue: &IssueRef, label: &str) -> Result<(), TrackerError>;

    /// Hand the issue to the spec-repair path (#3541) with its attempt history.
    fn escalate_to_spec_repair(
        &mut self,
        issue: &IssueRef,
        report: &SpecRepairReport,
    ) -> Result<(), TrackerError>;
}
