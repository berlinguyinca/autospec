use std::process::Command;

use autospec_core::runtime_env::{
    ComposeIsolation, ComposePlan, ComposePolicy, IsolationDiagnostic, RuntimeContext,
};

use crate::commands::CommandFailure;

pub(super) struct ComposeAdapter;

impl ComposeAdapter {
    pub(super) fn validate_resolved_model(
        plan: Option<&ComposePlan>,
        context: &RuntimeContext,
    ) -> Result<(), CommandFailure> {
        let Some(plan) = plan.filter(|plan| plan.isolation == ComposeIsolation::Managed) else {
            return Ok(());
        };
        let resolved = Self::resolved_model(plan, context)?;
        let diagnostics = ComposePolicy::evaluate_json_in_context(
            &resolved,
            plan,
            &context.environment_id,
            &context.repo,
        )
        .map_err(|error| CommandFailure::diagnostic(error.to_string()))?;
        if diagnostics.is_empty() {
            Ok(())
        } else {
            Err(CommandFailure::diagnostic(render_diagnostics(&diagnostics)))
        }
    }

    fn resolved_model(
        plan: &ComposePlan,
        context: &RuntimeContext,
    ) -> Result<Vec<u8>, CommandFailure> {
        let mut command = Command::new("docker");
        command.arg("compose");
        for file in &plan.files {
            command.arg("-f").arg(file);
        }
        let output = command
            .args([
                "--project-name",
                &plan.project_name,
                "config",
                "--format",
                "json",
            ])
            .current_dir(&context.repo)
            .output()
            .map_err(|error| {
                CommandFailure::diagnostic(format!(
                    "could not execute docker compose config: {error}"
                ))
            })?;
        if output.status.success() {
            Ok(output.stdout)
        } else {
            Err(CommandFailure::status(
                String::from_utf8_lossy(&output.stderr)
                    .trim_end()
                    .to_string(),
                output.status.code().unwrap_or(2),
            ))
        }
    }
}

fn render_diagnostics(diagnostics: &[IsolationDiagnostic]) -> String {
    diagnostics
        .iter()
        .map(|diagnostic| {
            format!(
                "{}: {}={} (environment {}; recovery: {})",
                diagnostic.code,
                diagnostic.resource,
                diagnostic.evidence,
                diagnostic.environment_id,
                diagnostic.recovery_command
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}
