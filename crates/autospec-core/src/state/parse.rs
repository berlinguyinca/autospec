use std::collections::{BTreeMap, BTreeSet};

use super::json::{JsonParser, JsonValue};
use super::{
    ChildIssueRecord, ParentIssueRecord, SpecLifecycle, SpecRunState, SpecStateStore,
    STATE_SCHEMA_VERSION,
};
use crate::spec::is_valid_spec_id;

pub(super) fn parse_store(document: &str) -> Result<SpecStateStore, String> {
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

pub(super) fn parse_lifecycle(value: JsonValue) -> Result<SpecLifecycle, String> {
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

pub(super) fn parse_parent_issue_record(value: JsonValue) -> Result<ParentIssueRecord, String> {
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

pub(super) fn parse_child_issue_record(value: JsonValue) -> Result<ChildIssueRecord, String> {
    let mut record = value.into_object("child issue record")?;
    require_only_keys(&record, &["issue", "terminal"], "child issue record")?;
    let issue = take_required(&mut record, "issue", "child issue record")?.into_number("issue")?;
    let terminal =
        take_required(&mut record, "terminal", "child issue record")?.into_bool("terminal")?;
    Ok(ChildIssueRecord { issue, terminal })
}

pub(super) fn take_required(
    object: &mut BTreeMap<String, JsonValue>,
    key: &str,
    context: &str,
) -> Result<JsonValue, String> {
    object
        .remove(key)
        .ok_or_else(|| format!("missing {key} in {context}"))
}

pub(super) fn require_only_keys(
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

pub(super) fn validate_parent_records(
    records: &BTreeMap<u64, ParentIssueRecord>,
) -> Result<(), String> {
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

pub(super) fn validate_parent_record(record: &ParentIssueRecord) -> Result<(), String> {
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

pub(super) fn decomposition_comment(
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

pub(super) fn extension_decomposition_comment(
    parent_issue: u64,
    child_issues: &[u64],
    quarantined_parent: bool,
) -> String {
    decomposition_comment(parent_issue, child_issues, quarantined_parent).replace(
        "\n<!-- autospec-parent-decomposition:end -->",
        "\nState: `append-only-parent-extension`.\n<!-- autospec-parent-decomposition:end -->",
    )
}

pub(super) fn completion_summary(parent_issue: u64, child_issues: &[u64]) -> String {
    let children = format_bullet_issue_list(child_issues);
    format!(
        "<!-- autospec-parent-complete:begin -->
All child implementation issues for parent #{parent_issue} reached a terminal state:
{children}

Closing parent issue automatically.
<!-- autospec-parent-complete:end -->"
    )
}

pub(super) fn format_bullet_issue_list(child_issues: &[u64]) -> String {
    format!("- {}", issue_list(child_issues, "\n- "))
}

pub(super) fn issue_list(child_issues: &[u64], separator: &str) -> String {
    child_issues
        .iter()
        .map(|issue| format!("#{issue}"))
        .collect::<Vec<_>>()
        .join(separator)
}

pub(super) fn validate_records(records: &BTreeMap<String, SpecLifecycle>) -> Result<(), String> {
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
