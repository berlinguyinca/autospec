const AUTOSPEC_RUN_SKILL_IDENTIFIER: &str = "$autospec-run";
const OMX_EXECUTOR_PROGRAM: &str = "omx";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrainExecutorInput {
    program: String,
    arguments: Vec<String>,
    skill_identifier: String,
}

impl DrainExecutorInput {
    pub fn omx_autospec_run(repo_dir: impl Into<String>) -> Result<Self, String> {
        let repo_dir = repo_dir.into();
        if repo_dir.trim().is_empty() {
            return Err("drain executor repo dir must not be empty".to_string());
        }
        let skill_identifier = AUTOSPEC_RUN_SKILL_IDENTIFIER.to_string();
        Ok(Self {
            program: OMX_EXECUTOR_PROGRAM.to_string(),
            arguments: vec![
                "exec".to_string(),
                "--cd".to_string(),
                repo_dir,
                "--dangerously-bypass-approvals-and-sandbox".to_string(),
                skill_identifier.clone(),
            ],
            skill_identifier,
        })
    }

    pub fn program(&self) -> &str {
        &self.program
    }

    pub fn arguments(&self) -> &[String] {
        &self.arguments
    }

    pub fn skill_identifier(&self) -> &str {
        &self.skill_identifier
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrainProgress {
    None,
    ChildOutput,
    Artifact,
    Heartbeat,
    Github,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrainObservation {
    pub child_exit_code: Option<i32>,
    pub elapsed_secs: u64,
    pub stall_secs: u64,
    pub progress: DrainProgress,
}

impl DrainObservation {
    pub fn live(elapsed_secs: u64, stall_secs: u64, progress: DrainProgress) -> Self {
        Self {
            child_exit_code: None,
            elapsed_secs,
            stall_secs,
            progress,
        }
    }

    pub fn completed(exit_code: i32, elapsed_secs: u64, stall_secs: u64) -> Self {
        Self {
            child_exit_code: Some(exit_code),
            elapsed_secs,
            stall_secs,
            progress: DrainProgress::None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrainDecision {
    Wait,
    WarnExternalProgress,
    Complete { exit_code: i32 },
    TerminateStalled,
}

pub fn decide(observation: &DrainObservation) -> DrainDecision {
    if let Some(exit_code) = observation.child_exit_code {
        return DrainDecision::Complete { exit_code };
    }
    if observation.elapsed_secs < observation.stall_secs {
        return DrainDecision::Wait;
    }
    match observation.progress {
        DrainProgress::None => DrainDecision::TerminateStalled,
        DrainProgress::Heartbeat | DrainProgress::Github => DrainDecision::WarnExternalProgress,
        DrainProgress::ChildOutput | DrainProgress::Artifact => DrainDecision::Wait,
    }
}
