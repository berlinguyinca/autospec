use super::{sanitize_text, AccountabilityEvent, EventKind, EventRecord, LaunchDescriptor};
use std::collections::BTreeMap;

pub(super) const MAX_MARKDOWN_BYTES: usize = 48 * 1024;
const MAX_WORK_NODES: usize = 25;
const MAX_TIMELINE_EVENTS: usize = 25;

pub(super) fn markdown(launch: &LaunchDescriptor, events: &[EventRecord]) -> String {
    let state = current_state(events);
    let mut body = format!(
        "## Overview\n\n{}\n\n{}\n\nCurrent state: **{}**.\n\n",
        sentence(&launch.outcome),
        sentence(&launch.why),
        state
    );
    body.push_str(&build_flow(events));
    body.push_str(run_state());
    body.push_str(&timeline(events));
    body.push_str(&deliverables(events));
    body.push_str(&risks(events));
    debug_assert!(body.len() <= MAX_MARKDOWN_BYTES);
    body
}

fn build_flow(events: &[EventRecord]) -> String {
    let work: Vec<_> = events
        .iter()
        .filter(|record| {
            matches!(
                record.kind,
                EventKind::WorkSelected { .. }
                    | EventKind::ClaimStarted { .. }
                    | EventKind::IssueClaimed { .. }
                    | EventKind::HeartbeatPublicationDeferred { .. }
                    | EventKind::StartupClaimRecovered { .. }
                    | EventKind::ImplementationStarted { .. }
                    | EventKind::PullRequestOpened { .. }
                    | EventKind::ReviewStarted { .. }
                    | EventKind::PullRequestVerified { .. }
                    | EventKind::Merged { .. }
                    | EventKind::Quarantined { .. }
                    | EventKind::Failed
                    | EventKind::Blocked
            )
        })
        .collect();
    let aggregate = work.len() > MAX_WORK_NODES;
    let visible = if aggregate {
        MAX_WORK_NODES - 1
    } else {
        work.len()
    };
    let start = work.len().saturating_sub(visible);
    let mut graph = String::from("## Build flow\n\n```mermaid\nflowchart TD\n    goal[Run goal]\n");
    if aggregate {
        graph.push_str(&format!(
            "    goal --> work_aggregate[{} older work items]\n",
            work.len() - visible
        ));
    }
    for (index, record) in work[start..].iter().enumerate() {
        let number = index + usize::from(aggregate);
        let label = sanitize_text(&record.event.what, 80);
        graph.push_str(&format!("    goal --> work_{number}[{label}]\n"));
    }
    let mut recovery = BTreeMap::new();
    for record in events {
        match &record.kind {
            EventKind::HeartbeatPublicationDeferred { issue, .. } => {
                recovery.entry(*issue).or_insert((None, None)).0 = Some(record);
            }
            EventKind::StartupClaimRecovered { issue, .. } => {
                recovery.entry(*issue).or_insert((None, None)).1 = Some(record);
            }
            _ => {}
        }
    }
    for (issue, (deferred, recovered)) in recovery {
        if let Some(record) = deferred {
            graph.push_str(&format!(
                "    deferred_{issue}[{}]\n",
                sanitize_text(&record.event.what, 80)
            ));
        }
        if let Some(record) = recovered {
            graph.push_str(&format!(
                "    recovered_{issue}[{}]\n",
                sanitize_text(&record.event.what, 80)
            ));
        }
        if deferred.is_some() && recovered.is_some() {
            graph.push_str(&format!("    deferred_{issue} --> recovered_{issue}\n"));
        }
    }
    graph.push_str("```\n\n");
    graph
}

fn run_state() -> &'static str {
    "## Run state\n\n```mermaid\nstateDiagram-v2\n    [*] --> Initializing\n    Initializing --> Active\n    Active --> DegradedProjection\n    DegradedProjection --> Active\n    Active --> Parked\n    Active --> Blocked\n    Active --> Failed\n    Active --> Completed\n    Parked --> Active\n    Blocked --> Active\n    Completed --> [*]\n    Failed --> [*]\n```\n\n"
}

fn timeline(events: &[EventRecord]) -> String {
    let mut section = String::from("## Decision timeline\n\n");
    let omitted = events.len().saturating_sub(MAX_TIMELINE_EVENTS);
    if omitted > 0 {
        section.push_str(&format!(
            "{omitted} older events remain in the private journal and are summarized here.\n\n"
        ));
    }
    for record in events.iter().skip(omitted) {
        section.push_str(&format!(
            "### Event {}\n\n**What:** {}\n\n**Why:** {}\n\n**Evidence:** {}\n\n",
            record.seq,
            sentence(&record.event.what),
            sentence(&record.event.why),
            sentence(&evidence(&record.event))
        ));
    }
    section
}

fn deliverables(events: &[EventRecord]) -> String {
    let mut values = Vec::new();
    let deliverable_count = events
        .iter()
        .filter(|record| {
            matches!(
                record.kind,
                EventKind::IssueClaimed { .. }
                    | EventKind::PullRequestOpened { .. }
                    | EventKind::Merged { .. }
            )
        })
        .count();
    let omitted = deliverable_count.saturating_sub(MAX_WORK_NODES);
    if omitted > 0 {
        values.push(format!(
            "- {omitted} older deliverables remain in the private journal"
        ));
    }
    for record in events
        .iter()
        .rev()
        .filter(|record| {
            matches!(
                record.kind,
                EventKind::IssueClaimed { .. }
                    | EventKind::PullRequestOpened { .. }
                    | EventKind::Merged { .. }
            )
        })
        .take(MAX_WORK_NODES)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        match record.kind {
            EventKind::IssueClaimed { issue } => values.push(format!("- Issue #{issue}: claimed")),
            EventKind::PullRequestOpened { pull_request } => {
                values.push(format!("- PR #{pull_request}: opened, not yet implemented"));
            }
            EventKind::Merged { pull_request } => {
                values.push(format!(
                    "- PR #{pull_request}: merged into the target branch"
                ));
            }
            _ => {}
        }
    }
    if values.is_empty() {
        values.push("- No linked deliverables recorded yet".to_owned());
    }
    format!("## Deliverables\n\n{}\n\n", values.join("\n"))
}

fn risks(events: &[EventRecord]) -> String {
    let likely = events
        .iter()
        .rev()
        .find(|record| matches!(record.kind, EventKind::Blocked | EventKind::Failed))
        .map_or(
            "Projection or implementation may fail after the latest verified event".to_owned(),
            |record| sanitize_text(&record.event.what, 240),
        );
    format!(
        "## Verification and remaining risk\n\nEvidence is listed with each decision above.\n\nMost likely hidden failure: {}.\n",
        likely.trim_end_matches(['.', '!', '?'])
    )
}

fn current_state(events: &[EventRecord]) -> &'static str {
    events
        .last()
        .map_or("initializing", |record| match record.kind {
            EventKind::Completed => "completed",
            EventKind::Failed => "failed",
            EventKind::Blocked => "blocked",
            EventKind::Parked => "parked",
            EventKind::Stopped => "stopped",
            _ => "active",
        })
}

fn evidence(event: &AccountabilityEvent) -> String {
    event
        .evidence
        .iter()
        .map(|item| item.display())
        .collect::<Vec<_>>()
        .join("; ")
}

fn sentence(value: &str) -> String {
    let value = sanitize_text(value, 320);
    if value.ends_with(['.', '!', '?']) {
        value
    } else {
        format!("{value}.")
    }
}
