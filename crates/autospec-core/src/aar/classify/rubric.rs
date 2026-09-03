//! Deterministic keyword rubric behind [`super::classify`] (AAR spec section 3).
//!
//! The tables a maintainer tunes live here; the shape of a classification
//! lives in the parent module.

use std::collections::BTreeSet;

use super::{Capability, ClassificationInput, Complexity, Risk, TaskClass};

pub(super) const CLASS_KEYWORDS: &[(TaskClass, &[&str])] = &[
    (
        TaskClass::Bugfix,
        &[
            "bug",
            "fix",
            "regression",
            "broken",
            "crash",
            "panic",
            "incorrect",
            "defect",
            "stack trace",
            "reproduce",
            "fails",
            "failing",
        ],
    ),
    (
        TaskClass::Feature,
        &[
            "feature",
            "implement",
            "introduce",
            "support for",
            "new command",
            "add a",
            "add an",
            "enable",
            "expose",
        ],
    ),
    (
        TaskClass::Refactor,
        &[
            "refactor",
            "extract",
            "restructure",
            "deduplicate",
            "simplify",
            "clean up",
            "cleanup",
            "rename",
            "move module",
        ],
    ),
    (
        TaskClass::Test,
        &[
            "unit test",
            "integration test",
            "test coverage",
            "add tests",
            "fixture",
            "assertion",
            "flaky",
            "regression test",
        ],
    ),
    (
        TaskClass::Docs,
        &[
            "documentation",
            "readme",
            "changelog",
            "docstring",
            "guide",
            "tutorial",
            "docs for",
            "document the",
        ],
    ),
    (
        TaskClass::Ui,
        &[
            "ui",
            "ux",
            "css",
            "layout",
            "component",
            "screen",
            "accessibility",
            "a11y",
            "button",
            "dashboard page",
        ],
    ),
    (
        TaskClass::Research,
        &[
            "research",
            "investigate",
            "spike",
            "evaluate",
            "compare",
            "survey",
            "prototype",
            "feasibility",
        ],
    ),
    (
        TaskClass::Migration,
        &[
            "migration",
            "migrate",
            "upgrade",
            "schema change",
            "backfill",
            "version bump",
            "deprecate",
            "port to",
        ],
    ),
    (
        TaskClass::Ops,
        &[
            "deploy",
            "pipeline",
            "docker",
            "workflow",
            "release process",
            "infrastructure",
            "monitoring",
            "runner",
            "ci job",
        ],
    ),
    (
        TaskClass::Review,
        &[
            "review",
            "audit",
            "critique",
            "assess",
            "lgtm",
            "second opinion",
        ],
    ),
];

/// Verbs that mark work needing genuine design reasoning.
pub(super) const DEEP_VERBS: &[&str] = &[
    "design",
    "architect",
    "redesign",
    "reconcile",
    "decide",
    "trade-off",
    "tradeoff",
    "protocol",
    "consensus",
];

/// Verbs that mark mechanical work.
pub(super) const SHALLOW_VERBS: &[&str] = &[
    "typo", "copy", "transcribe", "reword", "bump", "mirror exactly", "one-line", "one line",
];

pub(super) const CRITICAL_PATH_MARKERS: &[&str] = &[
    "security",
    "auth",
    "credential",
    "secret",
    "safety",
    "install.sh",
    "bootstrap.sh",
    "uninstall.sh",
    "migrations/",
    "constitution",
];

pub(super) const HIGH_PATH_MARKERS: &[&str] = &[
    "/lib.rs",
    "cargo.toml",
    "cargo.lock",
    ".github/workflows",
    "autonomous",
    "scripts/autospec-",
    "schemas/",
    ".autospec/",
];

pub(super) const VISION_MARKERS: &[&str] = &[
    "screenshot",
    "mockup",
    "visual",
    "design comp",
    "pixel",
    "rendered page",
];

pub(super) const WEB_MARKERS: &[&str] = &[
    "web search",
    "upstream documentation",
    "upstream docs",
    "external api",
    "rfc ",
    "vendor documentation",
];

pub(super) const CROSS_CUTTING_MARKERS: &[&str] = &[
    "cross-skill",
    "cross skill",
    "multi-skill",
    "cross-repo",
    "cross repository",
    "shared scripts",
    "end-to-end",
];


pub(super) fn score_class(
    title: &str,
    body: &str,
    labels: &[String],
    evidence: &mut Vec<String>,
) -> (TaskClass, f64) {
    let mut scored: Vec<(TaskClass, f64, Vec<&str>)> = Vec::new();
    let mut label_class = None;

    for label in labels {
        if let Some(class) = TaskClass::parse(label) {
            label_class = Some(class);
            break;
        }
        if let Some(rest) = label.strip_prefix("type:") {
            if let Some(class) = TaskClass::parse(rest) {
                label_class = Some(class);
                break;
            }
        }
    }

    for (class, keywords) in CLASS_KEYWORDS {
        let mut score = 0.0;
        let mut matched = Vec::new();
        for keyword in *keywords {
            if title.contains(keyword) {
                score += 2.0;
                matched.push(*keyword);
            } else if body.contains(keyword) {
                score += 1.0;
                matched.push(*keyword);
            }
        }
        if label_class == Some(*class) {
            score += 4.0;
            matched.push("label");
        }
        scored.push((*class, score, matched));
    }

    scored.sort_by(|left, right| {
        right
            .1
            .partial_cmp(&left.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(left.0.cmp(&right.0))
    });

    let (top_class, top_score, top_matches) = scored[0].clone();
    let runner_up = scored.get(1).map(|entry| entry.1).unwrap_or(0.0);

    if top_score <= 0.0 {
        evidence.push("task_class=feature (no rubric keyword matched; default)".to_string());
        return (TaskClass::Feature, 0.2);
    }

    let margin_ratio = ((top_score - runner_up) / top_score).clamp(0.0, 1.0);
    let label_bonus = if label_class == Some(top_class) {
        0.15
    } else {
        0.0
    };
    let confidence = round2((0.30 + 0.50 * margin_ratio + label_bonus).clamp(0.05, 0.99));

    evidence.push(format!(
        "task_class={} (matched: {}; score={top_score}, runner_up={runner_up})",
        top_class.as_str(),
        top_matches.join(", ")
    ));
    (top_class, confidence)
}

pub(super) fn estimate_files(input: &ClassificationInput) -> usize {
    if input.estimated_files > 0 {
        return input.estimated_files;
    }
    input
        .referenced_paths
        .iter()
        .map(|path| path.trim())
        .filter(|path| !path.is_empty())
        .collect::<BTreeSet<_>>()
        .len()
}

pub(super) fn score_complexity(
    task_class: TaskClass,
    estimated_files: usize,
    title: &str,
    body: &str,
    labels: &[String],
    evidence: &mut Vec<String>,
) -> Complexity {
    if let Some(explicit) = labels
        .iter()
        .filter_map(|label| label.strip_prefix("complexity:"))
        .find_map(Complexity::parse)
    {
        evidence.push(format!("complexity={} (label)", explicit.as_str()));
        return explicit;
    }

    let deep = contains_any(title, DEEP_VERBS).or_else(|| contains_any(body, DEEP_VERBS));
    let shallow = contains_any(title, SHALLOW_VERBS).or_else(|| contains_any(body, SHALLOW_VERBS));
    let cross_cutting = contains_any(body, CROSS_CUTTING_MARKERS);

    let mut complexity = match estimated_files {
        0 | 1 => Complexity::Low,
        2..=3 => Complexity::Low,
        4..=8 => Complexity::Medium,
        9..=20 => Complexity::High,
        _ => Complexity::Exceptional,
    };

    if shallow.is_some() && deep.is_none() && estimated_files <= 1 {
        complexity = Complexity::Trivial;
    }
    if let Some(verb) = deep {
        complexity = complexity.max(Complexity::High);
        evidence.push(format!("complexity raised by deep verb '{verb}'"));
    }
    if let Some(marker) = cross_cutting {
        complexity = complexity.max(Complexity::High);
        evidence.push(format!("complexity raised by cross-cutting marker '{marker}'"));
    }
    if task_class == TaskClass::Research {
        complexity = complexity.max(Complexity::Medium);
    }
    if task_class == TaskClass::Docs && deep.is_none() && estimated_files <= 2 {
        complexity = complexity.min(Complexity::Low);
    }

    evidence.push(format!(
        "complexity={} (estimated_files={estimated_files})",
        complexity.as_str()
    ));
    complexity
}

pub(super) fn score_risk(
    task_class: TaskClass,
    complexity: Complexity,
    paths: &[String],
    labels: &[String],
    evidence: &mut Vec<String>,
) -> Risk {
    if labels.iter().any(|label| label == "priority:critical") {
        evidence.push("risk=critical (label priority:critical)".to_string());
        return Risk::Critical;
    }

    let normalized = paths
        .iter()
        .map(|path| path.replace('\\', "/").to_ascii_lowercase())
        .collect::<Vec<_>>();

    let mut risk = match task_class {
        TaskClass::Docs | TaskClass::Test | TaskClass::Research | TaskClass::Review => Risk::Low,
        TaskClass::Migration | TaskClass::Ops => Risk::Medium,
        _ => Risk::Low,
    };

    for path in &normalized {
        if let Some(marker) = contains_any(path, CRITICAL_PATH_MARKERS) {
            evidence.push(format!("risk=critical (path marker '{marker}' in {path})"));
            return Risk::Critical;
        }
        if let Some(marker) = contains_any(path, HIGH_PATH_MARKERS) {
            risk = risk.max(Risk::High);
            evidence.push(format!("risk raised by path marker '{marker}'"));
        }
    }

    if labels.iter().any(|label| label == "priority:high") {
        risk = risk.max(Risk::High);
        evidence.push("risk raised by label priority:high".to_string());
    }
    if labels.iter().any(|label| label == "regression") {
        risk = risk.max(Risk::High);
        evidence.push("risk raised by label regression".to_string());
    }
    if complexity >= Complexity::High {
        risk = risk.max(Risk::Medium);
    }
    if normalized.len() > 3 {
        risk = risk.max(Risk::Medium);
    }

    evidence.push(format!("risk={}", risk.as_str()));
    risk
}

pub(super) fn capabilities_for(
    task_class: TaskClass,
    requires_vision: bool,
    requires_long_context: bool,
) -> Vec<Capability> {
    let mut capabilities: BTreeSet<Capability> = match task_class {
        TaskClass::Bugfix => [
            Capability::Coding,
            Capability::Debugging,
            Capability::RepositoryReasoning,
            Capability::ToolUse,
        ]
        .into_iter()
        .collect(),
        TaskClass::Feature => [
            Capability::Coding,
            Capability::Planning,
            Capability::RepositoryReasoning,
            Capability::ToolUse,
        ]
        .into_iter()
        .collect(),
        TaskClass::Refactor => [Capability::Coding, Capability::RepositoryReasoning]
            .into_iter()
            .collect(),
        TaskClass::Test => [Capability::Coding, Capability::ToolUse].into_iter().collect(),
        TaskClass::Docs => [Capability::Documentation, Capability::TextualAnalysis]
            .into_iter()
            .collect(),
        TaskClass::Ui => [Capability::Coding, Capability::Vision].into_iter().collect(),
        TaskClass::Research => [
            Capability::Planning,
            Capability::TextualAnalysis,
            Capability::RepositoryReasoning,
        ]
        .into_iter()
        .collect(),
        TaskClass::Migration => [
            Capability::Coding,
            Capability::Planning,
            Capability::RepositoryReasoning,
        ]
        .into_iter()
        .collect(),
        TaskClass::Ops => [Capability::ToolUse, Capability::Coding].into_iter().collect(),
        TaskClass::Review => [
            Capability::Review,
            Capability::RepositoryReasoning,
            Capability::TextualAnalysis,
        ]
        .into_iter()
        .collect(),
    };

    if requires_vision {
        capabilities.insert(Capability::Vision);
    }
    if requires_long_context {
        capabilities.insert(Capability::ContextHandling);
    }
    capabilities.into_iter().collect()
}

pub(super) fn infer_language(paths: &[String]) -> String {
    for path in paths {
        let path = path.to_ascii_lowercase();
        let language = if path.ends_with(".rs") {
            "rust"
        } else if path.ends_with(".py") {
            "python"
        } else if path.ends_with(".go") {
            "go"
        } else if path.ends_with(".ts") || path.ends_with(".tsx") {
            "typescript"
        } else if path.ends_with(".sh") {
            "shell"
        } else if path.ends_with(".md") {
            "markdown"
        } else {
            continue;
        };
        return language.to_string();
    }
    "unknown".to_string()
}

pub(super) fn contains_any<'a>(haystack: &str, needles: &[&'a str]) -> Option<&'a str> {
    needles.iter().copied().find(|needle| haystack.contains(needle))
}

pub(super) fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}
