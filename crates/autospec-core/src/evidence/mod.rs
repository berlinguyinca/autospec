use crate::state::{SpecLifecycle, SpecRunState};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceCommand {
    pub command: String,
    pub exit_code: i32,
    pub stdout_path: String,
    pub stderr_path: String,
}

impl EvidenceCommand {
    pub fn new(
        command: impl Into<String>,
        exit_code: i32,
        stdout_path: impl Into<String>,
        stderr_path: impl Into<String>,
    ) -> Self {
        Self {
            command: command.into(),
            exit_code,
            stdout_path: stdout_path.into(),
            stderr_path: stderr_path.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceBundle {
    pub run_id: String,
    pub commands: Vec<EvidenceCommand>,
    pub artifacts: Vec<String>,
}

impl EvidenceBundle {
    pub fn new(
        run_id: impl Into<String>,
        commands: Vec<EvidenceCommand>,
        artifacts: Vec<String>,
    ) -> Self {
        Self {
            run_id: run_id.into(),
            commands,
            artifacts,
        }
    }

    pub fn to_json(&self) -> String {
        let commands = self
            .commands
            .iter()
            .map(|command| {
                format!(
                    "{{\"command\":\"{}\",\"exit_code\":{},\"stdout_path\":\"{}\",\"stderr_path\":\"{}\"}}",
                    escape_json(&command.command),
                    command.exit_code,
                    escape_json(&command.stdout_path),
                    escape_json(&command.stderr_path)
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "{{\"run_id\":\"{}\",\"commands\":[{}],\"artifacts\":{}}}",
            escape_json(&self.run_id),
            commands,
            json_array(&self.artifacts)
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseReport {
    pub version: String,
    pub passed: usize,
    pub failed: usize,
    pub blocked: usize,
    pub deferred: usize,
    pub superseded: usize,
}

impl ReleaseReport {
    pub fn from_states(
        version: impl Into<String>,
        states: &[SpecLifecycle],
    ) -> Result<Self, String> {
        let mut report = Self {
            version: version.into(),
            passed: 0,
            failed: 0,
            blocked: 0,
            deferred: 0,
            superseded: 0,
        };

        for state in states {
            match state.state {
                SpecRunState::Passed => report.passed += 1,
                SpecRunState::Failed => report.failed += 1,
                SpecRunState::Blocked => report.blocked += 1,
                SpecRunState::Deferred => report.deferred += 1,
                SpecRunState::Superseded => report.superseded += 1,
                SpecRunState::Planned | SpecRunState::Ready | SpecRunState::Running => {
                    return Err(format!(
                        "{} has unknown or unfinished state {}",
                        state.spec_id,
                        state.state.as_str()
                    ));
                }
            }
        }

        Ok(report)
    }

    pub fn to_markdown(&self) -> String {
        format!(
            "# AutoSpec Release Report {}\n\npassed: {}\nfailed: {}\nblocked: {}\ndeferred: {}\nsuperseded: {}\n",
            self.version, self.passed, self.failed, self.blocked, self.deferred, self.superseded
        )
    }

    pub fn to_json(&self) -> String {
        format!(
            "{{\"version\":\"{}\",\"passed\":{},\"failed\":{},\"blocked\":{},\"deferred\":{},\"superseded\":{}}}",
            escape_json(&self.version),
            self.passed,
            self.failed,
            self.blocked,
            self.deferred,
            self.superseded
        )
    }
}

fn json_array(values: &[String]) -> String {
    let values = values
        .iter()
        .map(|value| format!("\"{}\"", escape_json(value)))
        .collect::<Vec<_>>()
        .join(",");
    format!("[{values}]")
}

fn escape_json(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}
