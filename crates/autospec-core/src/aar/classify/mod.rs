//! Task classification (AAR spec section 3).
//!
//! Classification is deterministic-first, matching the repository rubric in
//! `scripts/classify-model-fit.sh`: a keyword/label rubric produces the class,
//! complexity, risk and capability set, and reports the evidence and a
//! confidence score so a caller can decide whether an LLM tie-breaker is worth
//! its cost. No I/O happens here.

use rubric::{
    capabilities_for, contains_any, estimate_files, infer_language, score_class, score_complexity,
    score_risk, CROSS_CUTTING_MARKERS, VISION_MARKERS, WEB_MARKERS,
};

/// Kind of work an executable unit represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TaskClass {
    Bugfix,
    Feature,
    Refactor,
    Test,
    Docs,
    Ui,
    Research,
    Migration,
    Ops,
    Review,
}

impl TaskClass {
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskClass::Bugfix => "bugfix",
            TaskClass::Feature => "feature",
            TaskClass::Refactor => "refactor",
            TaskClass::Test => "test",
            TaskClass::Docs => "docs",
            TaskClass::Ui => "ui",
            TaskClass::Research => "research",
            TaskClass::Migration => "migration",
            TaskClass::Ops => "ops",
            TaskClass::Review => "review",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value.trim().to_ascii_lowercase().as_str() {
            "bugfix" | "bug" | "fix" => TaskClass::Bugfix,
            "feature" | "feat" => TaskClass::Feature,
            "refactor" => TaskClass::Refactor,
            "test" | "tests" => TaskClass::Test,
            "docs" | "doc" | "documentation" => TaskClass::Docs,
            "ui" | "ux" => TaskClass::Ui,
            "research" | "spike" => TaskClass::Research,
            "migration" | "migrate" => TaskClass::Migration,
            "ops" | "infra" | "ci" => TaskClass::Ops,
            "review" => TaskClass::Review,
            _ => return None,
        })
    }

    pub fn all() -> [TaskClass; 10] {
        [
            TaskClass::Bugfix,
            TaskClass::Feature,
            TaskClass::Refactor,
            TaskClass::Test,
            TaskClass::Docs,
            TaskClass::Ui,
            TaskClass::Research,
            TaskClass::Migration,
            TaskClass::Ops,
            TaskClass::Review,
        ]
    }
}

/// How much work the unit is expected to take.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Complexity {
    Trivial,
    Low,
    Medium,
    High,
    Exceptional,
}

impl Complexity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Complexity::Trivial => "trivial",
            Complexity::Low => "low",
            Complexity::Medium => "medium",
            Complexity::High => "high",
            Complexity::Exceptional => "exceptional",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value.trim().to_ascii_lowercase().as_str() {
            "trivial" => Complexity::Trivial,
            "low" => Complexity::Low,
            "medium" => Complexity::Medium,
            "high" => Complexity::High,
            "exceptional" => Complexity::Exceptional,
            _ => return None,
        })
    }
}

/// Blast radius of getting the unit wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Risk {
    Low,
    Medium,
    High,
    Critical,
}

impl Risk {
    pub fn as_str(&self) -> &'static str {
        match self {
            Risk::Low => "low",
            Risk::Medium => "medium",
            Risk::High => "high",
            Risk::Critical => "critical",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value.trim().to_ascii_lowercase().as_str() {
            "low" => Risk::Low,
            "medium" => Risk::Medium,
            "high" => Risk::High,
            "critical" => Risk::Critical,
            _ => return None,
        })
    }
}

/// Model capability axes scored by profiles (AAR spec section 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Capability {
    Coding,
    Debugging,
    Planning,
    Review,
    RepositoryReasoning,
    ToolUse,
    TextualAnalysis,
    Documentation,
    Vision,
    ContextHandling,
    Concurrency,
}

impl Capability {
    pub fn as_str(&self) -> &'static str {
        match self {
            Capability::Coding => "coding",
            Capability::Debugging => "debugging",
            Capability::Planning => "planning",
            Capability::Review => "review",
            Capability::RepositoryReasoning => "repository_reasoning",
            Capability::ToolUse => "tool_use",
            Capability::TextualAnalysis => "textual_analysis",
            Capability::Documentation => "documentation",
            Capability::Vision => "vision",
            Capability::ContextHandling => "context_handling",
            Capability::Concurrency => "concurrency",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value.trim().to_ascii_lowercase().as_str() {
            "coding" => Capability::Coding,
            "debugging" => Capability::Debugging,
            "planning" => Capability::Planning,
            "review" => Capability::Review,
            "repository_reasoning" | "repository-reasoning" => Capability::RepositoryReasoning,
            "tool_use" | "tool-use" => Capability::ToolUse,
            "textual_analysis" | "textual-analysis" => Capability::TextualAnalysis,
            "documentation" => Capability::Documentation,
            "vision" => Capability::Vision,
            "context_handling" | "context-handling" => Capability::ContextHandling,
            "concurrency" => Capability::Concurrency,
            _ => return None,
        })
    }
}

/// Everything the deterministic rubric is allowed to look at.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClassificationInput {
    pub title: String,
    pub body: String,
    pub labels: Vec<String>,
    /// Files the issue named under `## Files to read first`, or the diff paths.
    pub referenced_paths: Vec<String>,
    /// Caller override; when zero the rubric derives it from `referenced_paths`.
    pub estimated_files: usize,
    pub language: String,
}

impl ClassificationInput {
    pub fn new(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            ..Self::default()
        }
    }

    pub fn with_labels(mut self, labels: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.labels = labels.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_paths(mut self, paths: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.referenced_paths = paths.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_language(mut self, language: impl Into<String>) -> Self {
        self.language = language.into();
        self
    }

    pub fn with_estimated_files(mut self, files: usize) -> Self {
        self.estimated_files = files;
        self
    }
}

/// Structured classification with the evidence that produced it.
#[derive(Debug, Clone, PartialEq)]
pub struct TaskClassification {
    pub task_class: TaskClass,
    pub complexity: Complexity,
    pub risk: Risk,
    pub language: String,
    pub estimated_files: usize,
    pub capabilities: Vec<Capability>,
    pub requires_vision: bool,
    pub requires_web: bool,
    pub requires_long_context: bool,
    pub confidence: f64,
    pub evidence: Vec<String>,
}

impl TaskClassification {
    /// True when the rubric is too unsure to act on without a tie-breaker.
    ///
    /// The caller decides what to do with that: the repository convention is a
    /// cheap-tier LLM call, never a top-tier one.
    pub fn needs_tie_breaker(&self, threshold: f64) -> bool {
        self.confidence < threshold
    }
}

/// Default confidence below which the rubric asks for a tie-breaker.
///
/// Mirrors `LLM_ESCALATION_THRESHOLD` in `scripts/classify-model-fit.sh`.
pub const DEFAULT_TIE_BREAKER_THRESHOLD: f64 = 0.3;

mod rubric;

/// Classify one executable unit with the deterministic rubric.
pub fn classify(input: &ClassificationInput) -> TaskClassification {
    let title = input.title.to_ascii_lowercase();
    let body = input.body.to_ascii_lowercase();
    let labels = input
        .labels
        .iter()
        .map(|label| label.trim().to_ascii_lowercase())
        .collect::<Vec<_>>();
    let mut evidence = Vec::new();

    let (task_class, confidence) = score_class(&title, &body, &labels, &mut evidence);
    let estimated_files = estimate_files(input);
    let complexity = score_complexity(
        task_class,
        estimated_files,
        &title,
        &body,
        &labels,
        &mut evidence,
    );
    let risk = score_risk(
        task_class,
        complexity,
        &input.referenced_paths,
        &labels,
        &mut evidence,
    );

    let requires_vision = task_class == TaskClass::Ui
        || contains_any(&title, VISION_MARKERS).is_some()
        || contains_any(&body, VISION_MARKERS).is_some();
    let requires_web = contains_any(&body, WEB_MARKERS).is_some()
        || contains_any(&title, WEB_MARKERS).is_some();
    let requires_long_context = estimated_files > 7
        || complexity >= Complexity::High
        || contains_any(&body, CROSS_CUTTING_MARKERS).is_some();

    if requires_vision {
        evidence.push("requires_vision=true".to_string());
    }
    if requires_web {
        evidence.push("requires_web=true".to_string());
    }
    if requires_long_context {
        evidence.push(format!(
            "requires_long_context=true (estimated_files={estimated_files})"
        ));
    }

    let capabilities = capabilities_for(task_class, requires_vision, requires_long_context);
    let language = if input.language.trim().is_empty() {
        infer_language(&input.referenced_paths)
    } else {
        input.language.trim().to_ascii_lowercase()
    };

    TaskClassification {
        task_class,
        complexity,
        risk,
        language,
        estimated_files,
        capabilities,
        requires_vision,
        requires_web,
        requires_long_context,
        confidence,
        evidence,
    }
}
