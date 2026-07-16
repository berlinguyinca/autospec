use std::collections::{BTreeMap, BTreeSet};

use crate::autonomous::waterfall::sha256_hex;
use crate::coordination::{dependency_numbers, RemoteIssue};

use super::model::{
    Tier15Classification, Tier15Decision, Tier15HoldReason, Tier15Input, Tier15Observation,
    Tier15QuarantineReason, Tier15Route, Tier15RouteReason, Tier15SkipReason,
};

pub fn observe_tier15(input: Tier15Input) -> Result<Tier15Observation, String> {
    let open_observed = input.open.len();
    let open = deduplicate_open(input.open)?;
    validate_closed(&input.closed)?;
    let closed_fingerprints = input
        .closed
        .iter()
        .map(fingerprint)
        .collect::<BTreeSet<_>>();
    let known = known_issues(&open, &input.closed);
    let mut decisions = Vec::with_capacity(open.len());
    let mut budget_used = 0usize;

    for issue in open.values() {
        let classification = classification(issue);
        let decision = if has_label(issue, "security:quarantined") {
            Tier15Decision::Quarantined {
                number: issue.number,
                classification,
                reason: Tier15QuarantineReason::ExistingSecurity,
            }
        } else if is_excluded(issue) {
            Tier15Decision::Skipped {
                number: issue.number,
                classification,
                reason: Tier15SkipReason::ExcludedLabel,
            }
        } else if closed_fingerprints.contains(&fingerprint(issue)) {
            Tier15Decision::Skipped {
                number: issue.number,
                classification,
                reason: Tier15SkipReason::ClosedFingerprint,
            }
        } else if has_label(issue, "groom:proposed") || has_label(issue, "groom:rejected") {
            Tier15Decision::Skipped {
                number: issue.number,
                classification,
                reason: Tier15SkipReason::AlreadyGroomed,
            }
        } else if is_epic(issue) {
            Tier15Decision::Routed {
                number: issue.number,
                classification,
                route: Tier15Route::Split,
                reason: Tier15RouteReason::Epic,
            }
        } else if budget_used >= input.budget {
            Tier15Decision::Skipped {
                number: issue.number,
                classification,
                reason: Tier15SkipReason::BudgetExhausted,
            }
        } else {
            budget_used += 1;
            classify_candidate(issue, classification, &known)
        };
        decisions.push(decision);
    }

    Ok(Tier15Observation {
        open_observed,
        open_deduplicated: open.len(),
        closed_observed: input.closed.len(),
        budget: input.budget,
        decisions,
    })
}

fn deduplicate_open(open: Vec<RemoteIssue>) -> Result<BTreeMap<u64, RemoteIssue>, String> {
    let mut deduplicated = BTreeMap::new();
    for issue in open {
        if issue.closed {
            return Err(format!(
                "open snapshot contains closed issue {}",
                issue.number
            ));
        }
        match deduplicated.get(&issue.number) {
            Some(existing) if existing != &issue => {
                return Err(format!("conflicting open issue {}", issue.number));
            }
            Some(_) => {}
            None => {
                deduplicated.insert(issue.number, issue);
            }
        }
    }
    Ok(deduplicated)
}

fn validate_closed(closed: &[RemoteIssue]) -> Result<(), String> {
    if let Some(issue) = closed.iter().find(|issue| !issue.closed) {
        Err(format!(
            "closed snapshot contains open issue {}",
            issue.number
        ))
    } else {
        Ok(())
    }
}

fn known_issues(
    open: &BTreeMap<u64, RemoteIssue>,
    closed: &[RemoteIssue],
) -> BTreeMap<u64, RemoteIssue> {
    let mut known = open.clone();
    for issue in closed {
        known.entry(issue.number).or_insert_with(|| issue.clone());
    }
    known
}

fn classify_candidate(
    issue: &RemoteIssue,
    classification: Tier15Classification,
    known: &BTreeMap<u64, RemoteIssue>,
) -> Tier15Decision {
    if dependency_numbers(&issue.body)
        .into_iter()
        .any(|number| !known.contains_key(&number))
    {
        return Tier15Decision::Held {
            number: issue.number,
            classification,
            reason: Tier15HoldReason::Dependency,
        };
    }
    if issue.body.chars().count() < 40 {
        return Tier15Decision::Held {
            number: issue.number,
            classification,
            reason: Tier15HoldReason::ThinIntent,
        };
    }
    if classification == Tier15Classification::NeedsTemplate {
        return route_template(
            issue.number,
            classification,
            Tier15RouteReason::TemplateRequired,
        );
    }
    if has_structured_intent(issue) {
        return route_template(
            issue.number,
            classification,
            Tier15RouteReason::StructuredIntent,
        );
    }
    if !has_clear_intent(issue) {
        return Tier15Decision::Held {
            number: issue.number,
            classification,
            reason: Tier15HoldReason::AmbiguousIntent,
        };
    }
    if file_path_count(&issue.body) > 3 {
        return route_template(issue.number, classification, Tier15RouteReason::BroadScope);
    }
    Tier15Decision::Produced {
        number: issue.number,
        classification,
    }
}

fn route_template(
    number: u64,
    classification: Tier15Classification,
    reason: Tier15RouteReason,
) -> Tier15Decision {
    Tier15Decision::Routed {
        number,
        classification,
        route: Tier15Route::Template,
        reason,
    }
}

fn classification(issue: &RemoteIssue) -> Tier15Classification {
    if has_label(issue, "needs-autospec-template") {
        Tier15Classification::NeedsTemplate
    } else if has_label(issue, "needs-classify") {
        Tier15Classification::NeedsClassify
    } else {
        Tier15Classification::Unlabeled
    }
}

fn has_label(issue: &RemoteIssue, label: &str) -> bool {
    issue
        .labels
        .iter()
        .any(|current| current.eq_ignore_ascii_case(label))
}

fn is_excluded(issue: &RemoteIssue) -> bool {
    issue.labels.iter().any(|label| {
        let label = label.to_ascii_lowercase();
        matches!(
            label.as_str(),
            "auto-implement"
                | "no-auto"
                | "paused-by-user"
                | "autospec:needs-human"
                | "wontfix"
                | "duplicate"
        ) || label.starts_with("hold:")
            || label.starts_with("locked-")
    })
}

fn is_epic(issue: &RemoteIssue) -> bool {
    has_label(issue, "epic")
        || issue.body.to_ascii_lowercase().contains("multi-subsystem")
        || issue.body.to_ascii_lowercase().contains("multi subsystem")
        || issue
            .body
            .split(|character: char| !character.is_ascii_alphanumeric())
            .any(|word| word.eq_ignore_ascii_case("epic"))
}

fn fingerprint(issue: &RemoteIssue) -> String {
    let body = issue.body.chars().take(200).collect::<String>();
    sha256_hex(format!("{}{}", issue.title, body).as_bytes())
}

fn has_clear_intent(issue: &RemoteIssue) -> bool {
    let body = issue.body.to_ascii_lowercase();
    body.starts_with("fix:")
        || body.starts_with("feat:")
        || body.contains(" fix:")
        || body.contains(" feat:")
        || body.contains("repro:")
        || body.contains("steps to reproduce")
        || body.contains("expected:")
}

fn has_structured_intent(issue: &RemoteIssue) -> bool {
    let title = issue.title.to_ascii_lowercase();
    let typed_title = [
        "fix:",
        "feat:",
        "gap:",
        "chore:",
        "refactor:",
        "perf:",
        "docs:",
        "test:",
        "ci:",
        "build:",
        "revert:",
    ]
    .iter()
    .any(|prefix| title.starts_with(prefix));
    typed_title
        || issue.body.lines().any(|line| {
            let trimmed = line.trim_start();
            (trimmed.starts_with("- [ ")
                || trimmed.starts_with("* [ ")
                || trimmed.starts_with("- [x")
                || trimmed.starts_with("* [x")
                || trimmed.starts_with("- [X"))
                || trimmed.starts_with("* [X")
                || structured_heading(trimmed)
        })
}

fn structured_heading(line: &str) -> bool {
    let level = line.bytes().take_while(|byte| *byte == b'#').count();
    if level < 2 || !line[level..].starts_with(char::is_whitespace) {
        return false;
    }
    let heading = line[level..].trim().to_ascii_lowercase();
    [
        "goal",
        "objective",
        "proposed",
        "problem",
        "summary",
        "acceptance",
        "suggested ac",
        "observed",
    ]
    .iter()
    .any(|prefix| heading == *prefix || heading.starts_with(&format!("{prefix} ")))
}

fn file_path_count(body: &str) -> usize {
    body.split(|character: char| !(character.is_ascii_alphanumeric() || "_./-".contains(character)))
        .filter(|token| {
            let Some((_, extension)) = token.rsplit_once('.') else {
                return false;
            };
            (1..=5).contains(&extension.len())
                && extension
                    .chars()
                    .all(|character| character.is_ascii_alphabetic())
        })
        .collect::<BTreeSet<_>>()
        .len()
}
