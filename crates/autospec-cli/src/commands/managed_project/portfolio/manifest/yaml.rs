//! Deterministic YAML rendering of a frozen plan.
//!
//! No YAML library is used to *produce* the canonical document: the rendering is a plain
//! string writer with fixed key order and quoted scalars, so the bytes — and therefore
//! the digest — cannot drift with a dependency's formatting choices. Every scalar is
//! double-quoted, which makes `[]`, an empty string, and a value that looks like a number
//! or boolean unambiguous on re-read.

use super::{PortfolioPlan, PORTFOLIO_PLAN_SCHEMA};

/// Indent of one nesting level, in spaces.
const INDENT: &str = "  ";

/// Render the plan. `include_digest` adds the `plan_digest` key; the digest itself is
/// computed over the rendering *without* it.
pub(super) fn render_document(plan: &PortfolioPlan, include_digest: bool) -> String {
    let mut out = String::new();
    out.push_str(&format!("schema: {}\n", yaml_scalar(PORTFOLIO_PLAN_SCHEMA)));
    out.push_str(&format!(
        "portfolio_id: {}\n",
        yaml_scalar(plan.portfolio_id().as_str())
    ));
    out.push_str(&format!(
        "project_owner: {}\n",
        yaml_scalar(plan.project_owner())
    ));
    out.push_str(&format!(
        "source_spec: {}\n",
        yaml_scalar(&plan.source_spec().to_string())
    ));
    let selector = plan
        .primary_scope_selector()
        .map(|selector| selector.as_str());
    out.push_str(&format!(
        "primary_scope: {}\n",
        optional_scalar(selector.as_deref())
    ));
    render_repositories(&mut out, plan);
    render_items(&mut out, plan);
    if include_digest {
        out.push_str(&format!(
            "plan_digest: {}\n",
            yaml_scalar(plan.plan_digest())
        ));
    }
    out
}

fn render_repositories(out: &mut String, plan: &PortfolioPlan) {
    out.push_str("repositories:\n");
    for facts in plan.repositories() {
        out.push_str(&format!(
            "{INDENT}- id: {}\n",
            yaml_scalar(facts.repository())
        ));
        out.push_str(&format!(
            "{INDENT}{INDENT}capability: {}\n",
            yaml_scalar(facts.capability().as_str())
        ));
        out.push_str(&format!(
            "{INDENT}{INDENT}observed_revision: {}\n",
            optional_scalar(facts.observed_revision())
        ));
    }
}

fn render_items(out: &mut String, plan: &PortfolioPlan) {
    out.push_str("items:\n");
    for item in plan.items() {
        out.push_str(&format!(
            "{INDENT}- key: {}\n",
            yaml_scalar(&item.item_key().to_string())
        ));
        let inner = format!("{INDENT}{INDENT}");
        out.push_str(&format!(
            "{inner}role: {}\n",
            yaml_scalar(item.role().as_str())
        ));
        out.push_str(&format!(
            "{inner}repository: {}\n",
            yaml_scalar(item.repository())
        ));
        out.push_str(&format!(
            "{inner}completion_policy: {}\n",
            yaml_scalar(item.completion_policy().as_str())
        ));
        render_key_list(out, "depends_on", item.depends_on());
        render_key_list(out, "local_parents", item.local_parents());
    }
}

/// A list of item keys: inline `[]` when empty, one quoted entry per line otherwise.
fn render_key_list(out: &mut String, key: &str, keys: &[autospec_core::managed_project::ItemKey]) {
    let inner = format!("{INDENT}{INDENT}");
    if keys.is_empty() {
        out.push_str(&format!("{inner}{key}: []\n"));
        return;
    }
    out.push_str(&format!("{inner}{key}:\n"));
    for item_key in keys {
        out.push_str(&format!(
            "{inner}{INDENT}- {}\n",
            yaml_scalar(&item_key.to_string())
        ));
    }
}

/// An optional scalar: `null` when absent (derive the scope), quoted text when the plan
/// declares it.
fn optional_scalar(value: Option<&str>) -> String {
    match value {
        Some(value) => yaml_scalar(value),
        None => "null".to_string(),
    }
}

/// Double-quoted scalar with the minimal escapes YAML requires. Backslash and quote are
/// escaped, everything else is literal, so an oid or path renders identically everywhere.
fn yaml_scalar(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\t' => escaped.push_str("\\t"),
            '\r' => escaped.push_str("\\r"),
            other => escaped.push(other),
        }
    }
    escaped.push('"');
    escaped
}
