use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::Path;

use crate::error::AutospecError;
use crate::spec::is_valid_spec_id;
use json::{JsonParser, JsonValue};
use storage::{FileState, StatePaths};

pub mod json;
mod storage;

const STATE_SCHEMA_VERSION: u64 = 1;
const STATE_FILE: &str = "specs.json";
const TEMP_STATE_FILE: &str = "specs.json.tmp";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpecRunState {
    Planned,
    Ready,
    Running,
    Passed,
    Failed,
    Blocked,
    Deferred,
    Superseded,
}

impl SpecRunState {
    pub fn as_str(&self) -> &'static str {
        match self {
            SpecRunState::Planned => "planned",
            SpecRunState::Ready => "ready",
            SpecRunState::Running => "running",
            SpecRunState::Passed => "passed",
            SpecRunState::Failed => "failed",
            SpecRunState::Blocked => "blocked",
            SpecRunState::Deferred => "deferred",
            SpecRunState::Superseded => "superseded",
        }
    }

    pub fn parse(value: &str) -> Result<Self, AutospecError> {
        match value {
            "planned" => Ok(Self::Planned),
            "ready" => Ok(Self::Ready),
            "running" => Ok(Self::Running),
            "passed" => Ok(Self::Passed),
            "failed" => Ok(Self::Failed),
            "blocked" => Ok(Self::Blocked),
            "deferred" => Ok(Self::Deferred),
            "superseded" => Ok(Self::Superseded),
            _ => Err(AutospecError::parse(
                "spec run state",
                format!("unknown spec run state: {value}"),
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecLifecycle {
    pub spec_id: String,
    pub state: SpecRunState,
    pub deferred_reason: Option<String>,
    pub superseded_by: Option<String>,
}

impl SpecLifecycle {
    pub fn new(spec_id: impl Into<String>) -> Self {
        Self {
            spec_id: spec_id.into(),
            state: SpecRunState::Planned,
            deferred_reason: None,
            superseded_by: None,
        }
    }

    pub fn transition_to(&mut self, next: SpecRunState) -> Result<(), AutospecError> {
        if is_allowed_transition(&self.state, &next) {
            self.state = next;
            Ok(())
        } else {
            Err(AutospecError::state(
                &self.spec_id,
                format!(
                    "invalid transition from {} to {}",
                    self.state.as_str(),
                    next.as_str()
                ),
            ))
        }
    }

    pub fn deferred(mut self, reason: impl Into<String>) -> Result<Self, AutospecError> {
        let reason = reason.into();
        if reason.trim().is_empty() {
            return Err(AutospecError::invariant("deferred reason is required"));
        }
        self.transition_to(SpecRunState::Deferred)?;
        self.deferred_reason = Some(reason);
        Ok(self)
    }

    pub fn superseded_by(mut self, replacement: impl Into<String>) -> Result<Self, AutospecError> {
        let replacement = replacement.into();
        if !is_valid_spec_id(&replacement) {
            return Err(AutospecError::validation(format!(
                "invalid replacement spec id: {replacement}"
            )));
        }
        self.transition_to(SpecRunState::Superseded)?;
        self.superseded_by = Some(replacement);
        Ok(self)
    }

    pub fn to_json(&self) -> String {
        format!(
            "{{\"spec_id\":\"{}\",\"state\":\"{}\",\"deferred_reason\":{},\"superseded_by\":{}}}",
            escape_json_string(&self.spec_id),
            self.state.as_str(),
            optional_json_string(&self.deferred_reason),
            optional_json_string(&self.superseded_by)
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParentIssueStatus {
    PendingChildren,
    QuarantinedParentDecomposed,
    CompleteButStale,
    Closed,
}

impl ParentIssueStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PendingChildren => "children pending",
            Self::QuarantinedParentDecomposed => "quarantined-parent-decomposed",
            Self::CompleteButStale => "complete but stale",
            Self::Closed => "closed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParentIssueUpdate {
    pub parent_issue: u64,
    pub comment_body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParentDecompositionExtension {
    pub parent_issue: u64,
    pub child_issues: Vec<u64>,
    pub added_children: Vec<u64>,
    pub comment_body: String,
    pub changed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParentIssueTerminalAction {
    pub parent_issue: u64,
    pub completion_summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParentIssueRecord {
    parent_issue: u64,
    child_issues: Vec<ChildIssueRecord>,
    quarantined_parent: bool,
    decomposition_comment_posted: bool,
    parent_closed: bool,
}

impl ParentIssueRecord {
    fn status(&self) -> ParentIssueStatus {
        if self.parent_closed {
            ParentIssueStatus::Closed
        } else if self.child_issues.iter().all(|child| child.terminal) {
            ParentIssueStatus::CompleteButStale
        } else if self.quarantined_parent {
            ParentIssueStatus::QuarantinedParentDecomposed
        } else {
            ParentIssueStatus::PendingChildren
        }
    }

    fn child_numbers(&self) -> Vec<u64> {
        self.child_issues.iter().map(|child| child.issue).collect()
    }

    fn to_json(&self) -> String {
        let children = self
            .child_issues
            .iter()
            .map(ChildIssueRecord::to_json)
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "{{\"parent_issue\":{},\"child_issues\":[{}],\"quarantined_parent\":{},\"decomposition_comment_posted\":{},\"parent_closed\":{}}}",
            self.parent_issue,
            children,
            self.quarantined_parent,
            self.decomposition_comment_posted,
            self.parent_closed
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ChildIssueRecord {
    issue: u64,
    terminal: bool,
}

impl ChildIssueRecord {
    fn to_json(&self) -> String {
        format!(
            "{{\"issue\":{},\"terminal\":{}}}",
            self.issue, self.terminal
        )
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParentIssueCounts {
    pub pending_children: usize,
    pub quarantined_parent_decomposed: usize,
    pub complete_but_stale: usize,
    pub closed: usize,
}

/// A validated, deterministic state document for package lifecycle progress.
///
/// The store is intentionally local and non-executing. It owns only
/// `<project-root>/.autospec/state/specs.json`, which later queue and report
/// layers can consume without reinterpreting lifecycle metadata.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpecStateStore {
    records: BTreeMap<String, SpecLifecycle>,
    parent_issues: BTreeMap<u64, ParentIssueRecord>,
}

impl SpecStateStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, lifecycle: SpecLifecycle) -> Result<(), AutospecError> {
        let mut candidate = self.records.clone();
        if candidate.contains_key(&lifecycle.spec_id) {
            return Err(AutospecError::state(
                &lifecycle.spec_id,
                format!("duplicate spec id: {}", lifecycle.spec_id),
            ));
        }
        candidate.insert(lifecycle.spec_id.clone(), lifecycle);
        validate_records(&candidate)?;
        self.records = candidate;
        Ok(())
    }

    pub fn get(&self, spec_id: &str) -> Option<&SpecLifecycle> {
        self.records.get(spec_id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &SpecLifecycle> {
        self.records.values()
    }

    pub fn record_parent_decomposition(
        &mut self,
        parent_issue: u64,
        child_issues: Vec<u64>,
        quarantined_parent: bool,
    ) -> Result<ParentIssueUpdate, AutospecError> {
        if parent_issue == 0 {
        return Err(AutospecError::other("parent issue number must be positive".to_string()));
        }
        if child_issues.is_empty() {
            return Err(AutospecError::other("parent issue decomposition requires at least one child issue".to_string()));
        }
        if self.parent_issues.contains_key(&parent_issue) {
            return Err(AutospecError::state(
                "parent",
                format!(
                    "parent issue #{parent_issue} is already decomposed"
                ),
            ));
        }

        let mut seen = BTreeSet::new();
        let mut children = Vec::new();
        for issue in child_issues {
            if issue == 0 {
                return Err(AutospecError::other("child issue number must be positive".to_string()));
            }
            if issue == parent_issue {
                return Err(AutospecError::invariant(format!(
                    "parent issue #{parent_issue} cannot be its own child"
                )));
            }
            if !seen.insert(issue) {
                return Err(AutospecError::other(format!("duplicate child issue #{issue}")));
            }
            children.push(ChildIssueRecord {
                issue,
                terminal: false,
            });
        }

        let record = ParentIssueRecord {
            parent_issue,
            child_issues: children,
            quarantined_parent,
            decomposition_comment_posted: true,
            parent_closed: false,
        };
        validate_parent_record(&record)?;
        let comment_body = decomposition_comment(
            parent_issue,
            &record.child_numbers(),
            record.quarantined_parent,
        );
        self.parent_issues.insert(parent_issue, record);

        Ok(ParentIssueUpdate {
            parent_issue,
            comment_body,
        })
    }

    /// Replace local parent metadata with the relationship currently recorded
    /// on GitHub. The remote lifecycle comment is authoritative; this cache is
    /// only a recoverable status projection.
    pub fn sync_parent_decomposition(
        &mut self,
        parent_issue: u64,
        child_issues: Vec<u64>,
        quarantined_parent: bool,
    ) -> Result<ParentIssueUpdate, AutospecError> {
        let previous = self.parent_issues.remove(&parent_issue);
        match self.record_parent_decomposition(parent_issue, child_issues, quarantined_parent) {
            Ok(update) => Ok(update),
            Err(error) => {
                if let Some(record) = previous {
                    self.parent_issues.insert(parent_issue, record);
                }
                Err(error)
            }
        }
    }

    pub fn extend_parent_decomposition(
        &mut self,
        parent_issue: u64,
        child_issues: Vec<u64>,
    ) -> Result<ParentDecompositionExtension, AutospecError> {
        let record = self
            .parent_issues
            .get(&parent_issue)
            .ok_or_else(|| format!("parent issue #{parent_issue} is not tracked"))?;
        let existing = record.child_numbers();
        if child_issues.len() < existing.len() || !child_issues.starts_with(&existing) {
            return Err(AutospecError::state(
                "parent",
                format!(
                    "parent issue #{parent_issue} extension must preserve the ordered child prefix"
                ),
            ));
        }
        let mut seen = BTreeSet::new();
        for issue in &child_issues {
            if *issue == 0 {
                return Err(AutospecError::other("child issue number must be positive".to_string()));
            }
            if *issue == parent_issue {
                return Err(AutospecError::invariant(format!(
                    "parent issue #{parent_issue} cannot be its own child"
                )));
            }
            if !seen.insert(*issue) {
                return Err(AutospecError::other(format!("duplicate child issue #{issue}")));
            }
        }
        let added_children = child_issues[existing.len()..].to_vec();
        for issue in &added_children {
            if let Some(owner) = self.parent_issues.values().find(|candidate| {
                candidate.parent_issue != parent_issue
                    && candidate
                        .child_issues
                        .iter()
                        .any(|child| child.issue == *issue)
            }) {
                return Err(AutospecError::state(
                    "child",
                    format!(
                        "child issue #{issue} is already linked to parent #{}",
                        owner.parent_issue
                    ),
                ));
            }
        }
        let changed = !added_children.is_empty();
        if changed {
            let record = self
                .parent_issues
                .get_mut(&parent_issue)
                .expect("validated parent remains tracked");
            record
                .child_issues
                .extend(added_children.iter().map(|issue| ChildIssueRecord {
                    issue: *issue,
                    terminal: false,
                }));
            record.parent_closed = false;
            validate_parent_record(record)?;
        }
        let quarantined = self
            .parent_issues
            .get(&parent_issue)
            .expect("validated parent remains tracked")
            .quarantined_parent;
        Ok(ParentDecompositionExtension {
            parent_issue,
            child_issues: child_issues.clone(),
            added_children,
            comment_body: extension_decomposition_comment(parent_issue, &child_issues, quarantined),
            changed,
        })
    }

    pub fn record_child_terminal(
        &mut self,
        child_issue: u64,
    ) -> Result<Vec<ParentIssueTerminalAction>, AutospecError> {
        if child_issue == 0 {
            return Err(AutospecError::other("child issue number must be positive".to_string()));
        }

        let mut matched = false;
        for record in self.parent_issues.values_mut() {
            for child in &mut record.child_issues {
                if child.issue == child_issue {
                    child.terminal = true;
                    matched = true;
                }
            }
            validate_parent_record(record)?;
        }
        if !matched {
            return Err(AutospecError::state(
                "child",
                format!("child issue #{child_issue} is not linked to a parent issue"),
            ));
        }

        Ok(self.parent_issue_terminal_actions())
    }

    pub fn record_parent_closed(&mut self, parent_issue: u64) -> Result<(), AutospecError> {
        let record = self
            .parent_issues
            .get_mut(&parent_issue)
            .ok_or_else(|| format!("parent issue #{parent_issue} is not tracked"))?;
        if !record.child_issues.iter().all(|child| child.terminal) {
            return Err(AutospecError::state(
                "parent",
                format!(
                    "parent issue #{parent_issue} cannot close while child issues are pending"
                ),
            ));
        }
        record.parent_closed = true;
        validate_parent_record(record).map_err(AutospecError::from)
    }

    pub fn parent_issue_status(&self, parent_issue: u64) -> Option<ParentIssueStatus> {
        self.parent_issues
            .get(&parent_issue)
            .map(ParentIssueRecord::status)
    }

    pub fn parent_issue_children(&self, parent_issue: u64) -> Option<Vec<u64>> {
        self.parent_issues
            .get(&parent_issue)
            .map(ParentIssueRecord::child_numbers)
    }

    pub fn parent_issue_numbers(&self) -> Vec<u64> {
        self.parent_issues.keys().copied().collect()
    }

    pub fn parent_issue_update(&self, parent_issue: u64) -> Option<ParentIssueUpdate> {
        self.parent_issues
            .get(&parent_issue)
            .map(|record| ParentIssueUpdate {
                parent_issue,
                comment_body: decomposition_comment(
                    parent_issue,
                    &record.child_numbers(),
                    record.quarantined_parent,
                ),
            })
    }

    pub fn parent_issue_terminal_action(
        &self,
        parent_issue: u64,
    ) -> Option<ParentIssueTerminalAction> {
        self.parent_issues.get(&parent_issue).and_then(|record| {
            record
                .child_issues
                .iter()
                .all(|child| child.terminal)
                .then(|| ParentIssueTerminalAction {
                    parent_issue,
                    completion_summary: completion_summary(parent_issue, &record.child_numbers()),
                })
        })
    }

    pub fn parent_issue_counts(&self) -> ParentIssueCounts {
        let mut counts = ParentIssueCounts::default();
        for record in self.parent_issues.values() {
            match record.status() {
                ParentIssueStatus::PendingChildren => counts.pending_children += 1,
                ParentIssueStatus::QuarantinedParentDecomposed => {
                    counts.quarantined_parent_decomposed += 1;
                }
                ParentIssueStatus::CompleteButStale => counts.complete_but_stale += 1,
                ParentIssueStatus::Closed => counts.closed += 1,
            }
        }
        counts
    }

    pub fn parent_issue_status_lines(&self) -> Vec<String> {
        self.parent_issues
            .values()
            .map(|record| {
                let child_issues = issue_list(&record.child_numbers(), ", ");
                format!(
                    "#{}: {} ({})",
                    record.parent_issue,
                    record.status().as_str(),
                    child_issues
                )
            })
            .collect()
    }

    fn parent_issue_terminal_actions(&self) -> Vec<ParentIssueTerminalAction> {
        self.parent_issues
            .values()
            .filter(|record| matches!(record.status(), ParentIssueStatus::CompleteButStale))
            .map(|record| ParentIssueTerminalAction {
                parent_issue: record.parent_issue,
                completion_summary: completion_summary(
                    record.parent_issue,
                    &record.child_numbers(),
                ),
            })
            .collect()
    }

    pub fn to_json(&self) -> Result<String, AutospecError> {
        validate_records(&self.records)?;
        validate_parent_records(&self.parent_issues)?;
        let records = self
            .records
            .values()
            .map(SpecLifecycle::to_json)
            .collect::<Vec<_>>()
            .join(",");
        let parent_issues = self
            .parent_issues
            .values()
            .map(ParentIssueRecord::to_json)
            .collect::<Vec<_>>()
            .join(",");
        Ok(format!(
            "{{\"schema\":{STATE_SCHEMA_VERSION},\"specs\":[{records}],\"parent_issues\":[{parent_issues}]}}"
        ))
    }

    pub fn load_or_default(root: impl AsRef<Path>) -> Result<Self, AutospecError> {
        let paths = StatePaths::new(root.as_ref());
        let primary = storage::load_state_file(&paths.primary);

        match primary {
            FileState::Valid(store) => Ok(store),
            primary_state @ (FileState::Missing | FileState::Invalid(_)) => {
                match storage::load_state_file(&paths.temporary) {
                    FileState::Valid(store) => {
                        storage::promote_temporary(&paths)?;
                        Ok(store)
                    }
                    temporary_state @ (FileState::Missing | FileState::Invalid(_)) => {
                        match (primary_state, temporary_state) {
                            (FileState::Missing, FileState::Missing) => Ok(Self::new()),
                            (FileState::Invalid(error), FileState::Missing) => Err(AutospecError::io(
                                "read",
                                paths.primary.display().to_string(),
                                error,
                            )),
                            (FileState::Missing, FileState::Invalid(error)) => Err(AutospecError::io(
                                "read",
                                paths.temporary.display().to_string(),
                                error,
                            )),
                            (FileState::Invalid(primary_error), FileState::Invalid(temporary_error)) => {
                                Err(AutospecError::io(
                                    "read",
                                    paths.primary.display().to_string(),
                                    format!("{primary_error}; also {temporary_error}"),
                                ))
                            }
                            (FileState::Valid(_), _) | (_, FileState::Valid(_)) => {
                                unreachable!("valid file states return before this branch")
                            }
                        }
                    }
                }
            }
        }
    }

    pub fn initialize_if_absent(
        root: impl AsRef<Path>,
        records: impl IntoIterator<Item = SpecLifecycle>,
    ) -> Result<Self, AutospecError> {
        let mut store = Self::new();
        for lifecycle in records {
            store.insert(lifecycle)?;
        }
        let rendered = store.to_json()?;
        let paths = StatePaths::new(root.as_ref());
        if paths.primary.exists() || paths.temporary.exists() {
            return Err(AutospecError::other("autospec init refuses to overwrite existing spec state".to_string()));
        }

        let autospec_was_missing = !paths.autospec_directory.exists();
        let state_was_missing = !paths.directory.exists();
        fs::create_dir_all(&paths.directory).map_err(|error| {
            format!(
                "failed to create spec state directory {}: {error}",
                paths.directory.display()
            )
        })?;
        storage::sync_created_directories(&paths, autospec_was_missing, state_was_missing)?;

        let mut temporary = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&paths.temporary)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err(AutospecError::other("autospec init refuses to overwrite existing spec state".to_string()))
            }
            Err(error) => {
                return Err(AutospecError::io(
                    "create",
                    paths.temporary.display().to_string(),
                    error,
                ))
            }
        };
        temporary.write_all(rendered.as_bytes()).map_err(|error| {
            format!(
                "failed to write initialization state file {}: {error}",
                paths.temporary.display()
            )
        })?;
        temporary.sync_all().map_err(|error| {
            format!(
                "failed to synchronize initialization state file {}: {error}",
                paths.temporary.display()
            )
        })?;
        drop(temporary);

        match fs::hard_link(&paths.temporary, &paths.primary) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let _ = fs::remove_file(&paths.temporary);
                return Err(AutospecError::other("autospec init refuses to overwrite existing spec state".to_string()));
            }
            Err(error) => {
                return Err(AutospecError::io(
                    "atomically initialize",
                    paths.primary.display().to_string(),
                    error,
                ));
            }
        }
        storage::sync_directory(&paths.directory)?;
        fs::remove_file(&paths.temporary).map_err(|error| {
            format!(
                "failed to finalize initialization state file {}: {error}",
                paths.temporary.display()
            )
        })?;
        storage::sync_directory(&paths.directory)?;
        Ok(store)
    }

    pub fn save(&self, root: impl AsRef<Path>) -> Result<(), AutospecError> {
        let rendered = self.to_json()?;
        let paths = StatePaths::new(root.as_ref());
        let autospec_was_missing = !paths.autospec_directory.exists();
        let state_was_missing = !paths.directory.exists();
        fs::create_dir_all(&paths.directory).map_err(|error| {
            format!(
                "failed to create spec state directory {}: {error}",
                paths.directory.display()
            )
        })?;
        storage::sync_created_directories(&paths, autospec_was_missing, state_was_missing)?;

        let mut temporary = File::create(&paths.temporary).map_err(|error| {
            format!(
                "failed to create temporary spec state file {}: {error}",
                paths.temporary.display()
            )
        })?;
        temporary.write_all(rendered.as_bytes()).map_err(|error| {
            format!(
                "failed to write temporary spec state file {}: {error}",
                paths.temporary.display()
            )
        })?;
        temporary.sync_all().map_err(|error| {
            format!(
                "failed to synchronize temporary spec state file {}: {error}",
                paths.temporary.display()
            )
        })?;
        storage::sync_directory(&paths.directory)?;
        drop(temporary);

        storage::promote_temporary(&paths).map_err(AutospecError::from)
    }
}

fn parse_store(document: &str) -> Result<SpecStateStore, String> {
    let value = JsonParser::new(document).parse()?;
    let mut root = value.into_object("spec state document")?;
    require_only_keys(
        &root,
        &["schema", "specs", "parent_issues"],
        "spec state document",
    )?;

    let schema =
        take_required(&mut root, "schema", "spec state document")?.into_number("schema")?;
    if schema != STATE_SCHEMA_VERSION {
        return Err(format!("unsupported spec state schema: {schema}"));
    }

    let records = take_required(&mut root, "specs", "spec state document")?.into_array("specs")?;
    let parent_records = root
        .remove("parent_issues")
        .map(|value| value.into_array("parent_issues"))
        .transpose()?
        .unwrap_or_default();
    let mut store = SpecStateStore::new();
    for record in records {
        let lifecycle = parse_lifecycle(record)?;
        if store.records.contains_key(&lifecycle.spec_id) {
            return Err(format!("duplicate spec id: {}", lifecycle.spec_id));
        }
        store.records.insert(lifecycle.spec_id.clone(), lifecycle);
    }
    for record in parent_records {
        let parent = parse_parent_issue_record(record)?;
        if store.parent_issues.contains_key(&parent.parent_issue) {
            return Err(format!("duplicate parent issue: #{}", parent.parent_issue));
        }
        store.parent_issues.insert(parent.parent_issue, parent);
    }
    validate_records(&store.records)?;
    validate_parent_records(&store.parent_issues)?;
    Ok(store)
}

fn parse_lifecycle(value: JsonValue) -> Result<SpecLifecycle, String> {
    let mut record = value.into_object("spec lifecycle record")?;
    require_only_keys(
        &record,
        &["spec_id", "state", "deferred_reason", "superseded_by"],
        "spec lifecycle record",
    )?;
    let spec_id =
        take_required(&mut record, "spec_id", "spec lifecycle record")?.into_string("spec_id")?;
    let state = SpecRunState::parse(
        &take_required(&mut record, "state", "spec lifecycle record")?.into_string("state")?,
    )?;
    let deferred_reason = take_required(&mut record, "deferred_reason", "spec lifecycle record")?
        .into_optional_string("deferred_reason")?;
    let superseded_by = take_required(&mut record, "superseded_by", "spec lifecycle record")?
        .into_optional_string("superseded_by")?;

    Ok(SpecLifecycle {
        spec_id,
        state,
        deferred_reason,
        superseded_by,
    })
}

fn parse_parent_issue_record(value: JsonValue) -> Result<ParentIssueRecord, String> {
    let mut record = value.into_object("parent issue record")?;
    require_only_keys(
        &record,
        &[
            "parent_issue",
            "child_issues",
            "quarantined_parent",
            "decomposition_comment_posted",
            "parent_closed",
        ],
        "parent issue record",
    )?;
    let parent_issue = take_required(&mut record, "parent_issue", "parent issue record")?
        .into_number("parent_issue")?;
    let child_values = take_required(&mut record, "child_issues", "parent issue record")?
        .into_array("child_issues")?;
    let mut child_issues = Vec::new();
    for child in child_values {
        child_issues.push(parse_child_issue_record(child)?);
    }
    let quarantined_parent =
        take_required(&mut record, "quarantined_parent", "parent issue record")?
            .into_bool("quarantined_parent")?;
    let decomposition_comment_posted = take_required(
        &mut record,
        "decomposition_comment_posted",
        "parent issue record",
    )?
    .into_bool("decomposition_comment_posted")?;
    let parent_closed = take_required(&mut record, "parent_closed", "parent issue record")?
        .into_bool("parent_closed")?;
    Ok(ParentIssueRecord {
        parent_issue,
        child_issues,
        quarantined_parent,
        decomposition_comment_posted,
        parent_closed,
    })
}

fn parse_child_issue_record(value: JsonValue) -> Result<ChildIssueRecord, String> {
    let mut record = value.into_object("child issue record")?;
    require_only_keys(&record, &["issue", "terminal"], "child issue record")?;
    let issue = take_required(&mut record, "issue", "child issue record")?.into_number("issue")?;
    let terminal =
        take_required(&mut record, "terminal", "child issue record")?.into_bool("terminal")?;
    Ok(ChildIssueRecord { issue, terminal })
}

fn take_required(
    object: &mut BTreeMap<String, JsonValue>,
    key: &str,
    context: &str,
) -> Result<JsonValue, String> {
    object
        .remove(key)
        .ok_or_else(|| format!("missing {key} in {context}"))
}

fn require_only_keys(
    object: &BTreeMap<String, JsonValue>,
    expected: &[&str],
    context: &str,
) -> Result<(), String> {
    for key in object.keys() {
        if !expected.contains(&key.as_str()) {
            return Err(format!("unknown key {key} in {context}"));
        }
    }
    Ok(())
}

fn validate_parent_records(records: &BTreeMap<u64, ParentIssueRecord>) -> Result<(), String> {
    for (parent_issue, record) in records {
        if parent_issue != &record.parent_issue {
            return Err(format!(
                "parent issue record key does not match issue number: #{parent_issue}"
            ));
        }
        validate_parent_record(record)?;
    }
    Ok(())
}

fn validate_parent_record(record: &ParentIssueRecord) -> Result<(), String> {
    if record.parent_issue == 0 {
        return Err("parent issue number must be positive".to_string());
    }
    if record.child_issues.is_empty() {
        return Err(format!(
            "parent issue #{} requires at least one child issue",
            record.parent_issue
        ));
    }
    let mut seen = BTreeSet::new();
    for child in &record.child_issues {
        if child.issue == 0 {
            return Err(format!(
                "parent issue #{} has a non-positive child issue",
                record.parent_issue
            ));
        }
        if child.issue == record.parent_issue {
            return Err(format!(
                "parent issue #{} cannot be its own child",
                record.parent_issue
            ));
        }
        if !seen.insert(child.issue) {
            return Err(format!(
                "parent issue #{} has duplicate child issue #{}",
                record.parent_issue, child.issue
            ));
        }
    }
    if record.parent_closed && !record.child_issues.iter().all(|child| child.terminal) {
        return Err(format!(
            "parent issue #{} is closed before all child issues are terminal",
            record.parent_issue
        ));
    }
    Ok(())
}

fn decomposition_comment(
    parent_issue: u64,
    child_issues: &[u64],
    quarantined_parent: bool,
) -> String {
    let children = format_bullet_issue_list(child_issues);
    let state = if quarantined_parent {
        "\nState: `quarantined-parent-decomposed`."
    } else {
        ""
    };
    format!(
        "<!-- autospec-parent-decomposition:begin -->
Parent issue #{parent_issue} was decomposed into child implementation issues:
{children}{state}
<!-- autospec-parent-decomposition:end -->"
    )
}

fn extension_decomposition_comment(
    parent_issue: u64,
    child_issues: &[u64],
    quarantined_parent: bool,
) -> String {
    decomposition_comment(parent_issue, child_issues, quarantined_parent).replace(
        "\n<!-- autospec-parent-decomposition:end -->",
        "\nState: `append-only-parent-extension`.\n<!-- autospec-parent-decomposition:end -->",
    )
}

fn completion_summary(parent_issue: u64, child_issues: &[u64]) -> String {
    let children = format_bullet_issue_list(child_issues);
    format!(
        "<!-- autospec-parent-complete:begin -->
All child implementation issues for parent #{parent_issue} reached a terminal state:
{children}

Closing parent issue automatically.
<!-- autospec-parent-complete:end -->"
    )
}

fn format_bullet_issue_list(child_issues: &[u64]) -> String {
    format!("- {}", issue_list(child_issues, "\n- "))
}

fn issue_list(child_issues: &[u64], separator: &str) -> String {
    child_issues
        .iter()
        .map(|issue| format!("#{issue}"))
        .collect::<Vec<_>>()
        .join(separator)
}

fn validate_records(records: &BTreeMap<String, SpecLifecycle>) -> Result<(), String> {
    for (spec_id, lifecycle) in records {
        if spec_id != &lifecycle.spec_id {
            return Err(format!(
                "state record key does not match spec id: {spec_id}"
            ));
        }
        if !is_valid_spec_id(spec_id) {
            return Err(format!("invalid spec id: {spec_id}"));
        }

        match lifecycle.state {
            SpecRunState::Deferred => {
                if lifecycle
                    .deferred_reason
                    .as_deref()
                    .is_none_or(|reason| reason.trim().is_empty())
                {
                    return Err(format!(
                        "deferred spec {spec_id} requires a deferred reason"
                    ));
                }
                if lifecycle.superseded_by.is_some() {
                    return Err(format!(
                        "deferred spec {spec_id} cannot include a superseded-by reference"
                    ));
                }
            }
            SpecRunState::Superseded => {
                let replacement = lifecycle.superseded_by.as_deref().ok_or_else(|| {
                    format!("superseded spec {spec_id} requires a replacement spec id")
                })?;
                if !is_valid_spec_id(replacement) {
                    return Err(format!(
                        "superseded spec {spec_id} has invalid replacement spec id: {replacement}"
                    ));
                }
                if replacement == spec_id {
                    return Err(format!("superseded spec {spec_id} cannot replace itself"));
                }
                if !records.contains_key(replacement) {
                    return Err(format!(
                        "superseded spec {spec_id} references missing replacement: {replacement}"
                    ));
                }
                if lifecycle.deferred_reason.is_some() {
                    return Err(format!(
                        "superseded spec {spec_id} cannot include a deferred reason"
                    ));
                }
            }
            _ => {
                if lifecycle.deferred_reason.is_some() || lifecycle.superseded_by.is_some() {
                    return Err(format!(
                        "{} spec {spec_id} cannot include deferred or superseded metadata",
                        lifecycle.state.as_str()
                    ));
                }
            }
        }
    }
    Ok(())
}

fn is_allowed_transition(current: &SpecRunState, next: &SpecRunState) -> bool {
    matches!(
        (current, next),
        (SpecRunState::Planned, SpecRunState::Ready)
            | (SpecRunState::Planned, SpecRunState::Deferred)
            | (SpecRunState::Planned, SpecRunState::Superseded)
            | (SpecRunState::Ready, SpecRunState::Running)
            | (SpecRunState::Ready, SpecRunState::Deferred)
            | (SpecRunState::Ready, SpecRunState::Superseded)
            | (SpecRunState::Running, SpecRunState::Passed)
            | (SpecRunState::Running, SpecRunState::Failed)
            | (SpecRunState::Running, SpecRunState::Blocked)
            | (SpecRunState::Failed, SpecRunState::Running)
            | (SpecRunState::Blocked, SpecRunState::Ready)
    )
}

fn optional_json_string(value: &Option<String>) -> String {
    value
        .as_ref()
        .map(|value| format!("\"{}\"", escape_json_string(value)))
        .unwrap_or_else(|| "null".to_string())
}

fn escape_json_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\u{08}' => escaped.push_str("\\b"),
            '\u{0C}' => escaped.push_str("\\f"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            character if character.is_control() => {
                escaped.push_str(&format!("\\u{:04x}", character as u32));
            }
            character => escaped.push(character),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::{ParentIssueStatus, SpecStateStore};

    #[test]
    fn extend_parent_decomposition_preserves_terminal_children_and_is_idempotent() {
        let mut store = SpecStateStore::new();
        store
            .record_parent_decomposition(10, vec![11, 12], false)
            .expect("initial decomposition");
        store.record_child_terminal(11).expect("merged child");
        let extended = store
            .extend_parent_decomposition(10, vec![11, 12, 13])
            .expect("ordered extension");
        assert!(extended.changed);
        assert_eq!(extended.added_children, vec![13]);
        assert!(extended
            .comment_body
            .contains("append-only-parent-extension"));

        let repeated = store
            .extend_parent_decomposition(10, vec![11, 12, 13])
            .expect("same full list");
        assert!(!repeated.changed);
        store
            .record_child_terminal(12)
            .expect("second merged child");
        store
            .record_child_terminal(13)
            .expect("extension merged child");
        assert_eq!(
            store.parent_issue_status(10),
            Some(ParentIssueStatus::CompleteButStale)
        );
    }

    #[test]
    fn extend_parent_decomposition_rejects_non_append_and_owned_children() {
        let mut store = SpecStateStore::new();
        store
            .record_parent_decomposition(10, vec![11, 12], false)
            .expect("initial decomposition");
        store
            .record_parent_decomposition(20, vec![21], false)
            .expect("other decomposition");

        for children in [
            vec![11],
            vec![12, 11, 13],
            vec![11, 12, 10],
            vec![11, 12, 13, 13],
        ] {
            assert!(
                store.extend_parent_decomposition(10, children).is_err(),
                "invalid full list must fail closed"
            );
        }
        assert!(store
            .extend_parent_decomposition(10, vec![11, 12, 21])
            .expect_err("child owned by another parent")
            .to_string().contains("parent #20"));
        assert_eq!(store.parent_issue_children(10), Some(vec![11, 12]));
    }
}
