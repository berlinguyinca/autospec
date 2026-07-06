#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentTask {
    pub spec_id: String,
    pub instructions: String,
    pub validation_command: String,
}

impl AgentTask {
    pub fn new(
        spec_id: impl Into<String>,
        instructions: impl Into<String>,
        validation_command: impl Into<String>,
    ) -> Self {
        Self {
            spec_id: spec_id.into(),
            instructions: instructions.into(),
            validation_command: validation_command.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentResult {
    pub result: String,
    pub files_changed: Vec<String>,
    pub validation: String,
    pub blockers: Vec<String>,
    pub handoff: String,
}

impl AgentResult {
    pub fn new(
        result: impl Into<String>,
        files_changed: Vec<String>,
        validation: impl Into<String>,
        blockers: Vec<String>,
        handoff: impl Into<String>,
    ) -> Self {
        Self {
            result: result.into(),
            files_changed,
            validation: validation.into(),
            blockers,
            handoff: handoff.into(),
        }
    }

    pub fn to_json(&self) -> String {
        format!(
            "{{\"result\":\"{}\",\"files_changed\":{},\"validation\":\"{}\",\"blockers\":{},\"handoff\":\"{}\"}}",
            escape_json(&self.result),
            json_array(&self.files_changed),
            escape_json(&self.validation),
            json_array(&self.blockers),
            escape_json(&self.handoff)
        )
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SafeModePolicy {
    pub allow_destructive: bool,
}

impl SafeModePolicy {
    pub fn check(&self, task: &AgentTask) -> Result<(), String> {
        if self.allow_destructive {
            return Ok(());
        }
        let text = task.instructions.to_ascii_lowercase();
        let checks = [
            (
                "destructive git",
                ["git reset --hard", "git push --force"].as_slice(),
            ),
            ("filesystem deletion", ["rm -rf", "unlink "].as_slice()),
            (
                "credential access",
                ["aws_secret", "github_token", "private key"].as_slice(),
            ),
            (
                "network publication",
                ["gh pr merge", "gh release upload"].as_slice(),
            ),
            (
                "production mutation",
                ["production", "prod database"].as_slice(),
            ),
        ];
        for (category, patterns) in checks {
            if patterns.iter().any(|pattern| text.contains(pattern)) {
                return Err(format!("safe mode blocked {category}"));
            }
        }
        Ok(())
    }
}

pub fn render_handoff_prompt(agent: &str, task: &AgentTask) -> String {
    let agent_name = match agent {
        "codex" => "Codex",
        "claude" => "Claude",
        "fable" => "Fable",
        _ => "Generic",
    };
    format!(
        "# {agent_name} Agent Handoff\n\nSpec: {}\n\nInstructions:\n{}\n\nValidation:\n{}\n",
        task.spec_id, task.instructions, task.validation_command
    )
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
