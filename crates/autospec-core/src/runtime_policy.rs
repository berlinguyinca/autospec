#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Runtime {
    Rust,
    Shell,
    Python,
    Node,
    Go,
    Unknown,
}

impl Runtime {
    pub fn as_str(&self) -> &'static str {
        match self {
            Runtime::Rust => "rust",
            Runtime::Shell => "shell",
            Runtime::Python => "python",
            Runtime::Node => "node",
            Runtime::Go => "go",
            Runtime::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeClass {
    R0,
    R1,
    R2,
    R3,
    R4,
}

impl RuntimeClass {
    pub fn as_str(&self) -> &'static str {
        match self {
            RuntimeClass::R0 => "R0",
            RuntimeClass::R1 => "R1",
            RuntimeClass::R2 => "R2",
            RuntimeClass::R3 => "R3",
            RuntimeClass::R4 => "R4",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimePolicyVerdict {
    pub path: String,
    pub runtime: Runtime,
    pub class: RuntimeClass,
    pub reasons: Vec<String>,
}

impl RuntimePolicyVerdict {
    pub fn new(
        path: impl Into<String>,
        runtime: Runtime,
        class: RuntimeClass,
        reasons: Vec<String>,
    ) -> Self {
        Self {
            path: path.into(),
            runtime,
            class,
            reasons,
        }
    }
}

pub fn classify_path(path: &str) -> RuntimePolicyVerdict {
    let normalized = path.replace('\\', "/");
    let runtime = runtime_for_path(&normalized);

    if is_shell_wrapper(&normalized) {
        return RuntimePolicyVerdict::new(
            normalized,
            runtime,
            RuntimeClass::R0,
            vec!["shell wrapper or install entrypoint".to_string()],
        );
    }

    if is_exception_path(&normalized) {
        return RuntimePolicyVerdict::new(
            normalized,
            runtime,
            RuntimeClass::R4,
            vec!["domain-specific non-platform runtime exception".to_string()],
        );
    }

    if is_delete_or_merge_candidate(&normalized) {
        return RuntimePolicyVerdict::new(
            normalized,
            runtime,
            RuntimeClass::R3,
            vec!["generated or fixture path should be deleted or merged when touched".to_string()],
        );
    }

    if is_stateful_platform_path(&normalized) {
        return RuntimePolicyVerdict::new(
            normalized,
            runtime,
            RuntimeClass::R1,
            vec!["stateful platform behavior belongs in Rust core".to_string()],
        );
    }

    if matches!(runtime, Runtime::Rust) {
        return RuntimePolicyVerdict::new(
            normalized,
            runtime,
            RuntimeClass::R0,
            vec!["Rust core ownership".to_string()],
        );
    }

    RuntimePolicyVerdict::new(
        normalized,
        runtime,
        RuntimeClass::R2,
        vec!["stable helper; add parity fixture before porting".to_string()],
    )
}

fn runtime_for_path(path: &str) -> Runtime {
    if path.ends_with(".rs") || path.contains("/crates/") || path.starts_with("crates/") {
        Runtime::Rust
    } else if path.ends_with(".sh") || path.ends_with(".bats") {
        Runtime::Shell
    } else if path.ends_with(".py") {
        Runtime::Python
    } else if path.ends_with(".mjs")
        || path.ends_with(".js")
        || path.ends_with(".ts")
        || path.ends_with(".tsx")
        || path.ends_with("package.json")
    {
        Runtime::Node
    } else if path.ends_with(".go") || path.ends_with("go.mod") {
        Runtime::Go
    } else {
        Runtime::Unknown
    }
}

fn is_shell_wrapper(path: &str) -> bool {
    path.ends_with("/install.sh")
        || path.ends_with("/uninstall.sh")
        || path == "install.sh"
        || path == "uninstall.sh"
        || path.contains("/codex/")
        || path.contains("/opencode/")
}

fn is_exception_path(path: &str) -> bool {
    path.starts_with("skills/autospec-fab/scripts/")
        || path.starts_with("skills/autospec-fab/docker/wrappers/")
        || path.starts_with("skills/autospec-fab/tests/")
        || path.contains("/test-targets/")
        || path.contains("/tests/fixtures/")
}

fn is_delete_or_merge_candidate(path: &str) -> bool {
    path.contains("/fixtures/generated/")
        || path.contains("/__generated__/")
        || path.contains("/generated/")
}

fn is_stateful_platform_path(path: &str) -> bool {
    const STATEFUL_HINTS: &[&str] = &[
        "validate",
        "lint-issue",
        "lint-implementation",
        "claim",
        "lease",
        "run-state",
        "watchdog",
        "autonomous",
        "closed-issue-audit",
        "context_monitor",
        "autospec_context_monitor",
        "queue",
        "ledger",
        "supervisor",
    ];

    if path.starts_with("scripts/") || path.starts_with("skills/") || path.starts_with("packages/")
    {
        STATEFUL_HINTS.iter().any(|hint| path.contains(hint))
    } else {
        false
    }
}
