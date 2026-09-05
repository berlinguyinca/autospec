//! Scope qualifiers for the production-or-infrastructure intent rule.
//!
//! The rule answers one question: does this issue propose touching production
//! or shared infrastructure? Most of its vocabulary is unambiguous — `terraform`
//! and `kms` mean what they say. `migration` does not: a database migration
//! inside a development crate is routine engineering, and only an unqualified or
//! production-facing one is an infrastructure touch.
//!
//! Matching the bare word quarantined InferWeave P0-T05, whose whole subject is
//! a *local* Compose profile and a *disposable* test database. That task was the
//! only one in a 133-task graph with every dependency closed, so the false
//! positive stalled the other 124 issues until a human cleared the label by
//! hand. Nearly every storage task names migrations, so an unqualified match
//! does not separate risky work from ordinary work — it quarantines the
//! category.

use super::contains_any_word;

/// Words that name production or shared infrastructure outright. No amount of
/// surrounding context makes `kms` mean something else, so these stay exact.
const INFRASTRUCTURE: &[&str] = &[
    "production",
    "prod",
    "billing",
    "payments",
    "terraform",
    "iam",
    "kms",
];

/// Qualifiers that place a migration inside development work.
const DEVELOPMENT_SCOPE: &[&str] = &[
    "local",
    "disposable",
    "development",
    "dev ",
    "test",
    "fixture",
    "ephemeral",
    "in-memory",
    "temporary",
    "sandbox",
];

pub(super) fn mentions_production_or_infra_touch(body: &str) -> bool {
    contains_any_word(body, INFRASTRUCTURE) || mentions_unscoped_migration(body)
}

/// Whether any `migration` mention lacks a development-scope qualifier.
///
/// The qualifier is looked for in a one-line window around the mention, because
/// task cards wrap a single sentence across lines and the qualifier routinely
/// lands on a different line from the word it governs.
///
/// Absent a qualifier the finding still fires, so "apply the pending migration
/// to the cluster" is unchanged. This narrows the rule; it does not disarm it.
fn mentions_unscoped_migration(body: &str) -> bool {
    let lines: Vec<&str> = body.lines().collect();
    lines.iter().enumerate().any(|(index, line)| {
        if !contains_any_word(line, &["migration"]) {
            return false;
        }
        let window = [
            index.checked_sub(1).and_then(|i| lines.get(i)),
            Some(line),
            lines.get(index + 1),
        ];
        !window
            .into_iter()
            .flatten()
            .any(|text| DEVELOPMENT_SCOPE.iter().any(|scope| text.contains(scope)))
    })
}
