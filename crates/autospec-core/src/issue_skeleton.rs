//! Render a structured YAML issue-skeleton into a team-lensed issue body.
//!
//! Rust port of `scripts/gen-issue-skeleton.sh` (issue #4440). The shell
//! version parsed YAML with `awk`/`sed` regexes over raw text; this module
//! parses with `yaml-edit` (a real parser), which removes the class of
//! silent-wrong-answer bugs that pattern-matching over YAML invited.
//!
//! The rendered body is linted **in-process** with [`lint_issue_body`], so the
//! generator and the `autospec lint issue` gate share one rule engine instead
//! of the generator shelling out to a separate shell lint with a slightly
//! different rule set.
//!
//! Acceptance-criteria compatibility is pinned by the golden fixtures under
//! `tests/fixtures/gen-issue-skeleton/`. The render is byte-for-byte stable
//! for inputs that omit the optional [`IssueSkeleton::implementation_surface`]
//! field (added by #4439); providing that field appends one section.

use std::str::FromStr;

use yaml_edit::{Document, Mapping};

use crate::lint::{lint_issue_body, IssueLintFinding};

/// A fully parsed and validated issue-skeleton input.
///
/// All fields that the shell required are required here (fail-closed). The
/// optional fields are `operator_full` (falls back to `primary_smoke` at
/// render time) and `implementation_surface` (rendered only when present).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueSkeleton {
    pub issue_id: String,
    pub spec_path: String,
    pub spec_url: String,
    pub goal_sentence: String,
    pub branch_name: String,
    pub feature_profile: Option<String>,
    pub primary_smoke: String,
    pub operator_full: Option<String>,
    pub team_personality: Vec<String>,
    pub review_counter_team: Vec<String>,
    pub files_to_read: Vec<String>,
    pub files_touched: Vec<String>,
    pub local_llm_notes: Vec<String>,
    pub dependencies: Vec<String>,
    pub implementation_scope: Vec<String>,
    pub out_of_scope: Vec<String>,
    pub implementation_outline: Vec<String>,
    pub tests_required: Vec<String>,
    pub acceptance_criteria: Vec<String>,
    // Security profile — required only when `feature_profile == "security_database"`.
    pub evidence_consumed: Vec<String>,
    pub controls_covered: Vec<String>,
    pub prerequisites: Vec<String>,
    /// #4439 implementation-surface field: the crate and module the work
    /// belongs in. Optional for backward compatibility; when present it is
    /// rendered as its own section so an issue cannot be answered in whatever
    /// language the neighbouring file happens to be.
    pub implementation_surface: Option<String>,
}

fn missing(key: &str) -> String {
    format!("MISSING_FIELD:{key}")
}

/// Read an optional scalar at `root[key]`.
fn scalar(root: &Mapping, key: &str) -> Option<String> {
    let node = root.get(key)?;
    let value = node.as_scalar()?.as_string().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

/// Read a required scalar at `root[key]`, or `None` when absent/empty.
///
/// A required key present but holding a non-scalar (mapping/sequence) is
/// treated as missing, matching the shell's "treats a scalar-only key as the
/// value" contract.
fn required_scalar(root: &Mapping, key: &str) -> Option<String> {
    scalar(root, key)
}

/// Read a list at `root[key]` as a `Vec<String>`.
///
/// Each sequence item may be a plain scalar, or a mapping with a `path` key
/// (the shell extracted the `path` value from `{path: ...}` entries). Items
/// that are neither are dropped, matching the shell's behaviour.
fn string_list(root: &Mapping, key: &str) -> Option<Vec<String>> {
    let node = root.get(key)?;
    let sequence = node.as_sequence()?;
    let mut out = Vec::new();
    for item in sequence.values() {
        // Plain scalar items.
        if let Some(value) = item.as_scalar() {
            let text = value.as_string().trim().to_string();
            if !text.is_empty() {
                out.push(text);
            }
            continue;
        }
        // `{path: ...}` entries: extract the path value.
        if let Some(mapping) = item.as_mapping() {
            if let Some(path) = mapping.get("path") {
                if let Some(value) = path.as_scalar() {
                    let text = value.as_string().trim().to_string();
                    if !text.is_empty() {
                        out.push(text);
                    }
                }
            }
        }
    }
    Some(out)
}

fn nested_scalar(root: &Mapping, parent: &str, child: &str) -> Option<String> {
    let parent_node = root.get(parent)?;
    let parent_mapping = parent_node.as_mapping()?;
    scalar(parent_mapping, child)
}

/// Parse a YAML issue-skeleton document.
///
/// Fail-closed: the first missing required field (in the shell's validation
/// order) yields `Err("MISSING_FIELD:<key>")`.
pub fn parse(yaml: &str) -> Result<IssueSkeleton, String> {
    let document = Document::from_str(yaml)
        .map_err(|error| format!("YAML_PARSE_ERROR:{error}"))?;
    let root = document
        .as_mapping()
        .ok_or_else(|| missing("issue_id"))?;
    let root = &root;

    let issue_id = required_scalar(root, "issue_id").ok_or_else(|| missing("issue_id"))?;
    let spec_path = required_scalar(root, "spec_path").ok_or_else(|| missing("spec_path"))?;
    let spec_url = required_scalar(root, "spec_url").ok_or_else(|| missing("spec_url"))?;
    let goal_sentence =
        required_scalar(root, "goal_sentence").ok_or_else(|| missing("goal_sentence"))?;
    let branch_name = required_scalar(root, "branch_name").ok_or_else(|| missing("branch_name"))?;
    let feature_profile = scalar(root, "feature_profile");
    let primary_smoke = nested_scalar(root, "verification", "primary_smoke")
        .ok_or_else(|| missing("verification.primary_smoke"))?;
    let operator_full = nested_scalar(root, "verification", "operator_full");

    let team_personality =
        string_list(root, "team_personality").ok_or_else(|| missing("team_personality"))?;
    let review_counter_team = string_list(root, "review_counter_team")
        .ok_or_else(|| missing("review_counter_team"))?;
    let files_to_read = string_list(root, "files_to_read").ok_or_else(|| missing("files_to_read"))?;
    let files_touched =
        string_list(root, "files_touched").ok_or_else(|| missing("files_touched"))?;
    let local_llm_notes =
        string_list(root, "local_llm_notes").ok_or_else(|| missing("local_llm_notes"))?;
    let dependencies = string_list(root, "dependencies").ok_or_else(|| missing("dependencies"))?;
    let implementation_scope = string_list(root, "implementation_scope")
        .ok_or_else(|| missing("implementation_scope"))?;
    let out_of_scope = string_list(root, "out_of_scope").unwrap_or_default();
    let implementation_outline = string_list(root, "implementation_outline_lines")
        .ok_or_else(|| missing("implementation_outline_lines"))?;
    let tests_required =
        string_list(root, "tests_required").ok_or_else(|| missing("tests_required"))?;
    let acceptance_criteria = string_list(root, "acceptance_criteria")
        .ok_or_else(|| missing("acceptance_criteria"))?;

    // Security profile fields are required only when the profile demands them.
    let security = feature_profile.as_deref() == Some("security_database");
    let evidence_consumed = if security {
        string_list(root, "evidence_consumed")
            .ok_or_else(|| missing("evidence_consumed"))?
    } else {
        Vec::new()
    };
    let controls_covered = if security {
        string_list(root, "controls_covered")
            .ok_or_else(|| missing("controls_covered"))?
    } else {
        Vec::new()
    };
    let prerequisites = if security {
        string_list(root, "prerequisites").ok_or_else(|| missing("prerequisites"))?
    } else {
        Vec::new()
    };

    Ok(IssueSkeleton {
        issue_id,
        spec_path,
        spec_url,
        goal_sentence,
        branch_name,
        feature_profile,
        primary_smoke,
        operator_full,
        team_personality,
        review_counter_team,
        files_to_read,
        files_touched,
        local_llm_notes,
        dependencies,
        implementation_scope,
        out_of_scope,
        implementation_outline,
        tests_required,
        acceptance_criteria,
        evidence_consumed,
        controls_covered,
        prerequisites,
        implementation_surface: scalar(root, "implementation_surface"),
    })
}

fn bullets(items: &[String]) -> String {
    items
        .iter()
        .map(|item| format!("- {item}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn numbered(items: &[String]) -> String {
    items
        .iter()
        .enumerate()
        .map(|(index, item)| format!("{}. {item}", index + 1))
        .collect::<Vec<_>>()
        .join("\n")
}

fn checkboxes(items: &[String]) -> String {
    items
        .iter()
        .map(|item| format!("- [ ] {item}"))
        .collect::<Vec<_>>()
        .join("\n")
}

impl IssueSkeleton {
    /// Render the issue body, without a trailing newline.
    ///
    /// Byte-for-byte compatible with the shell renderer for inputs that omit
    /// `implementation_surface`. With a security profile the security sections
    /// are inserted after `## Dependencies`; with `implementation_surface`
    /// set, an `## Implementation surface` section is inserted after
    /// `## Implementation scope`.
    pub fn render(&self) -> String {
        let operator_full = self
            .operator_full
            .clone()
            .unwrap_or_else(|| self.primary_smoke.clone());

        let security_context = if self.feature_profile.as_deref() == Some("security_database") {
            format!(
                "## Evidence consumed\n\n{}\n\n## Controls covered\n\n{}\n\n## Prerequisites\n\n{}",
                bullets(&self.evidence_consumed),
                bullets(&self.controls_covered),
                bullets(&self.prerequisites)
            )
        } else {
            String::new()
        };

        // The `## Implementation scope` … `## Out of scope` span. With an
        // implementation-surface value (#4439) an extra section is inserted
        // between them, blank-line separated like every other section.
        let scope = bullets(&self.implementation_scope);
        let middle = match &self.implementation_surface {
            Some(surface) => format!(
                "## Implementation scope\n\n{scope}\n\n## Implementation surface\n\n{surface}\n\n## Out of scope"
            ),
            None => format!("## Implementation scope\n\n{scope}\n\n## Out of scope"),
        };

        format!(
            "## Goal\n\n\
             {goal}\n\n\
             ## Source spec\n\n\
             `{spec_path}` \u{2014} {spec_url}\n\n\
             ## Team personality\n\n\
             {team}\n\n\
             ## Review counter-team\n\n\
             {review}\n\n\
             ## Files to read first\n\n\
             {files_read}\n\n\
             ## Files touched\n\n\
             {files_touched}\n\n\
             ## Local-LLM execution notes\n\n\
             {notes}\n\n\
             ## Dependencies\n\n\
             {deps}\n\n\
             {security}\n\n\
             {middle}\n\n\
             {oos}\n\n\
             ## Implementation outline\n\n\
             {outline}\n\n\
             ## Tests required\n\n\
             {tests}\n\n\
             ## Acceptance criteria\n\n\
             {criteria}\n\n\
             ## Verification\n\n\
             ### Primary smoke test\n\n\
             ```\n{primary}\n```\n\n\
             ### Operator full\n\n\
             ```\n{operator}\n```\n\n\
             ## Branch name\n\n\
             `{branch}`",
            goal = self.goal_sentence,
            spec_path = self.spec_path,
            spec_url = self.spec_url,
            team = bullets(&self.team_personality),
            review = bullets(&self.review_counter_team),
            files_read = bullets(&self.files_to_read),
            files_touched = bullets(&self.files_touched),
            notes = bullets(&self.local_llm_notes),
            deps = self
                .dependencies
                .join("\n"),
            security = security_context,
            middle = middle,
            oos = bullets(&self.out_of_scope),
            outline = numbered(&self.implementation_outline),
            tests = bullets(&self.tests_required),
            criteria = checkboxes(&self.acceptance_criteria),
            primary = self.primary_smoke,
            operator = operator_full,
            branch = self.branch_name,
        )
    }

    /// All lint findings for the rendered body (blocking and warning).
    pub fn lint_findings(&self) -> Vec<IssueLintFinding> {
        let body = self.render();
        lint_issue_body(&body)
    }

    /// Count of **blocking** findings — the value the shell used as its exit
    /// code for a lint failure. Warnings (e.g. AS-DAG-001) are reported but
    /// never counted, matching `autospec lint issue` and `scripts/lint-issue.sh`.
    pub fn blocking_finding_count(&self) -> usize {
        self.lint_findings()
            .iter()
            .filter(|finding| finding.is_blocking())
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL_YAML: &str = r#"
issue_id: 1234
spec_path: docs/specs/2026-01-01-tooling-design.md
spec_url: https://github.com/acme/specs/blob/main/docs/specs/2026-01-01-tooling-design.md
goal_sentence: Add a deterministic tooling option to the release checklist.
branch_name: feat/tooling-opt-t1-gen-issue-skeleton
team_personality:
  - Platform engineer
review_counter_team:
  - Security reviewer
files_to_read:
  - docs/specs/2026-01-01-tooling-design.md
files_touched:
  - crates/tooling/src/lib.rs
local_llm_notes:
  - Keep the change under 200 lines.
dependencies:
  - none
implementation_scope:
  - crates/tooling/src/lib.rs
out_of_scope:
  - No new external dependencies
implementation_outline_lines:
  - Add the option to the config struct
  - Wire it into the release checklist
tests_required:
  - unit
acceptance_criteria:
  - "cargo test -p tooling --lib passes."
verification:
  primary_smoke: cargo test -p tooling --lib
"#;

    #[test]
    fn parses_minimal_input() {
        let input = parse(MINIMAL_YAML).expect("minimal input parses");
        assert_eq!(input.issue_id, "1234");
        assert_eq!(input.branch_name, "feat/tooling-opt-t1-gen-issue-skeleton");
        assert_eq!(input.dependencies, vec!["none"]);
        assert!(input.implementation_surface.is_none());
    }

    #[test]
    fn missing_required_field_is_fail_closed() {
        let yaml = "issue_id: 1\nspec_path: a\nspec_url: b\n";
        let err = parse(yaml).expect_err("goal_sentence missing");
        assert_eq!(err, "MISSING_FIELD:goal_sentence");
    }

    #[test]
    fn renders_minimal_body_without_trailing_newline() {
        let input = parse(MINIMAL_YAML).expect("minimal input parses");
        let body = input.render();
        assert!(!body.ends_with('\n'));
        assert!(body.starts_with("## Goal\n\nAdd a deterministic tooling option"));
        assert!(body.ends_with("## Branch name\n\n`feat/tooling-opt-t1-gen-issue-skeleton`"));
        // The security context is empty for a non-security profile: three blank
        // lines sit between the dependencies and the implementation scope.
        assert!(body.contains("## Dependencies\n\nnone\n\n\n\n## Implementation scope"));
    }

    #[test]
    fn operator_full_falls_back_to_primary_smoke() {
        let input = parse(MINIMAL_YAML).expect("minimal input parses");
        let body = input.render();
        let operator_section = body
            .split("### Operator full")
            .nth(1)
            .expect("operator section present");
        assert!(operator_section.contains("cargo test -p tooling --lib"));
    }

    #[test]
    fn security_profile_renders_security_sections() {
        let mut yaml = String::from(MINIMAL_YAML);
        yaml.push_str(
            "\nfeature_profile: security_database\nevidence_consumed:\n  - E1\
             \ncontrols_covered:\n  - T1\nprerequisites:\n  - verified",
        );
        let input = parse(&yaml).expect("security input parses");
        let body = input.render();
        assert!(body.contains("## Evidence consumed\n\n- E1"));
        assert!(body.contains("## Controls covered\n\n- T1"));
        assert!(body.contains("## Prerequisites\n\n- verified"));
    }

    #[test]
    fn security_profile_requires_security_fields() {
        let yaml = format!("{MINIMAL_YAML}\nfeature_profile: security_database\n");
        let err = parse(&yaml).expect_err("evidence_consumed missing");
        assert_eq!(err, "MISSING_FIELD:evidence_consumed");
    }

    #[test]
    fn implementation_surface_renders_its_own_section() {
        let mut yaml = String::from(MINIMAL_YAML);
        yaml.push_str("\nimplementation_surface: crates/autospec-core (issue_skeleton module)\n");
        let input = parse(&yaml).expect("input with surface parses");
        let body = input.render();
        assert!(body.contains("## Implementation surface\n\ncrates/autospec-core (issue_skeleton module)"));
        assert!(body.contains("## Implementation scope\n\n- crates/tooling/src/lib.rs\n\n## Implementation surface\n\ncrates/autospec-core (issue_skeleton module)\n\n## Out of scope"));
    }
}
