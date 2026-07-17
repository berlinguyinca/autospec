use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{ComposePlan, ExportProtocol, IsolationDiagnostic, RuntimeEnvError};

pub struct ComposePolicy;

impl ComposePolicy {
    pub fn evaluate_json(
        source: &[u8],
        plan: &ComposePlan,
    ) -> Result<Vec<IsolationDiagnostic>, RuntimeEnvError> {
        let model = serde_json::from_slice(source).map_err(|error| {
            RuntimeEnvError::new(format!(
                "COMPOSE_CONFIG_INVALID_JSON: could not parse resolved Compose model: {error}"
            ))
        })?;
        Ok(Self::evaluate(&model, plan))
    }

    pub fn evaluate(model: &Value, plan: &ComposePlan) -> Vec<IsolationDiagnostic> {
        let context = PolicyContext::from_plan(plan);
        evaluate_with_context(model, plan, &context)
    }

    pub fn evaluate_in_context(
        model: &Value,
        plan: &ComposePlan,
        environment_id: &str,
        repo: &Path,
    ) -> Vec<IsolationDiagnostic> {
        let context = PolicyContext::new(environment_id, repo);
        evaluate_with_context(model, plan, &context)
    }

    pub fn evaluate_json_in_context(
        source: &[u8],
        plan: &ComposePlan,
        environment_id: &str,
        repo: &Path,
    ) -> Result<Vec<IsolationDiagnostic>, RuntimeEnvError> {
        let model = serde_json::from_slice(source).map_err(|error| {
            RuntimeEnvError::new(format!(
                "COMPOSE_CONFIG_INVALID_JSON: could not parse resolved Compose model: {error}"
            ))
        })?;
        Ok(Self::evaluate_in_context(
            &model,
            plan,
            environment_id,
            repo,
        ))
    }
}

fn evaluate_with_context(
    model: &Value,
    plan: &ComposePlan,
    context: &PolicyContext,
) -> Vec<IsolationDiagnostic> {
    let mut diagnostics = Vec::new();
    evaluate_services(model, plan, context, &mut diagnostics);
    evaluate_named_resources(model, plan, context, &mut diagnostics);
    diagnostics.sort_by(|left, right| {
        left.resource
            .cmp(&right.resource)
            .then_with(|| left.code.cmp(&right.code))
            .then_with(|| left.evidence.cmp(&right.evidence))
    });
    diagnostics
}

struct PolicyContext {
    environment_id: String,
    repo: PathBuf,
    canonical_repo: Option<PathBuf>,
}

impl PolicyContext {
    fn new(environment_id: &str, repo: &Path) -> Self {
        Self {
            environment_id: environment_id.to_string(),
            repo: repo.to_path_buf(),
            canonical_repo: std::fs::canonicalize(repo).ok(),
        }
    }

    fn from_plan(plan: &ComposePlan) -> Self {
        let repo = plan
            .files
            .first()
            .and_then(|file| file.parent())
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        Self::new(
            plan.project_name
                .strip_prefix("agent_")
                .unwrap_or(&plan.project_name),
            &repo,
        )
    }

    fn diagnostic(&self, code: &str, resource: String, evidence: String) -> IsolationDiagnostic {
        IsolationDiagnostic {
            schema_version: 1,
            code: code.to_string(),
            environment_id: self.environment_id.clone(),
            resource,
            evidence,
            recovery_command: format!(
                "autospec runtime env normalize-compose --repo {} --check",
                super::shell_quote(&self.repo.display().to_string())
            ),
        }
    }
}

fn evaluate_services(
    model: &Value,
    plan: &ComposePlan,
    context: &PolicyContext,
    diagnostics: &mut Vec<IsolationDiagnostic>,
) {
    let Some(services) = model.get("services").and_then(Value::as_object) else {
        return;
    };
    for (service_name, service) in services {
        let base = format!("services.{service_name}");
        reject_service_identity(service, &base, context, diagnostics);
        evaluate_ports(service_name, service, &base, plan, context, diagnostics);
        evaluate_fixed_addresses(service, &base, context, diagnostics);
        evaluate_binds(service, &base, context, diagnostics);
    }
}

fn reject_service_identity(
    service: &Value,
    base: &str,
    context: &PolicyContext,
    diagnostics: &mut Vec<IsolationDiagnostic>,
) {
    if let Some(value) = service.get("container_name") {
        diagnostics.push(context.diagnostic(
            "COMPOSE_CONTAINER_NAME",
            format!("{base}.container_name"),
            evidence(value),
        ));
    }
    if service.get("network_mode").and_then(Value::as_str) == Some("host") {
        diagnostics.push(context.diagnostic(
            "COMPOSE_HOST_NETWORK",
            format!("{base}.network_mode"),
            "host".to_string(),
        ));
    }
}

fn evaluate_ports(
    service_name: &str,
    service: &Value,
    base: &str,
    plan: &ComposePlan,
    context: &PolicyContext,
    diagnostics: &mut Vec<IsolationDiagnostic>,
) {
    let Some(ports) = service.get("ports").and_then(Value::as_array) else {
        return;
    };
    for (index, port) in ports.iter().enumerate() {
        let path = format!("{base}.ports[{index}]");
        if let Some(published) = port.get("published").filter(|value| !value.is_null()) {
            diagnostics.push(context.diagnostic(
                "COMPOSE_FIXED_PORT",
                format!("{path}.published"),
                evidence(published),
            ));
        }
        let protocol = port
            .get("protocol")
            .and_then(Value::as_str)
            .unwrap_or("tcp");
        if !matches!(protocol, "tcp" | "udp") {
            diagnostics.push(context.diagnostic(
                "COMPOSE_UNDECLARED_PORT",
                format!("{path}.protocol"),
                protocol.to_string(),
            ));
            continue;
        }
        let target = port.get("target").and_then(port_number);
        if !target.is_some_and(|target| export_declared(plan, service_name, target, protocol)) {
            diagnostics.push(context.diagnostic(
                "COMPOSE_UNDECLARED_PORT",
                format!("{path}.target"),
                port.get("target").map(evidence).unwrap_or_default(),
            ));
        }
    }
}

fn export_declared(plan: &ComposePlan, service: &str, target: u16, protocol: &str) -> bool {
    plan.exports.iter().any(|export| {
        let declared_protocol = match export.protocol {
            ExportProtocol::Udp => "udp",
            ExportProtocol::Http | ExportProtocol::Https | ExportProtocol::Tcp => "tcp",
        };
        export.service == service && export.target == target && declared_protocol == protocol
    })
}

fn port_number(value: &Value) -> Option<u16> {
    value
        .as_u64()
        .and_then(|number| u16::try_from(number).ok())
        .or_else(|| value.as_str().and_then(|number| number.parse().ok()))
}

fn evaluate_fixed_addresses(
    service: &Value,
    base: &str,
    context: &PolicyContext,
    diagnostics: &mut Vec<IsolationDiagnostic>,
) {
    let Some(networks) = service.get("networks").and_then(Value::as_object) else {
        return;
    };
    for (network, settings) in networks {
        for field in ["ipv4_address", "ipv6_address"] {
            if let Some(value) = settings.get(field) {
                diagnostics.push(context.diagnostic(
                    "COMPOSE_FIXED_ADDRESS",
                    format!("{base}.networks.{network}.{field}"),
                    evidence(value),
                ));
            }
        }
    }
}

fn evaluate_binds(
    service: &Value,
    base: &str,
    context: &PolicyContext,
    diagnostics: &mut Vec<IsolationDiagnostic>,
) {
    let Some(volumes) = service.get("volumes").and_then(Value::as_array) else {
        return;
    };
    for (index, volume) in volumes.iter().enumerate() {
        if volume.get("type").and_then(Value::as_str) != Some("bind")
            || volume.get("read_only").and_then(Value::as_bool) == Some(true)
        {
            continue;
        }
        let Some(source) = volume.get("source").and_then(Value::as_str) else {
            continue;
        };
        let contained = context
            .canonical_repo
            .as_ref()
            .zip(std::fs::canonicalize(source).ok().as_ref())
            .is_some_and(|(repo, source)| source.starts_with(repo));
        if !Path::new(source).is_absolute() || !contained {
            diagnostics.push(context.diagnostic(
                "COMPOSE_WRITABLE_BIND_OUTSIDE_WORKTREE",
                format!("{base}.volumes[{index}].source"),
                source.to_string(),
            ));
        }
    }
}

fn evaluate_named_resources(
    model: &Value,
    plan: &ComposePlan,
    context: &PolicyContext,
    diagnostics: &mut Vec<IsolationDiagnostic>,
) {
    for (kind, shared) in [
        ("networks", plan.shared_networks.as_slice()),
        ("volumes", plan.shared_volumes.as_slice()),
    ] {
        let Some(resources) = model.get(kind).and_then(Value::as_object) else {
            continue;
        };
        for (logical_key, settings) in resources {
            let is_external = settings.get("external").is_some_and(external_enabled);
            if let Some(value) = settings.get("name").filter(|value| {
                value.as_str() != Some(&format!("{}_{logical_key}", plan.project_name))
                    && !is_external
            }) {
                diagnostics.push(context.diagnostic(
                    "COMPOSE_GLOBAL_NAME",
                    format!("{kind}.{logical_key}.name"),
                    evidence(value),
                ));
            }
            if is_external && !shared.iter().any(|key| key == logical_key) {
                diagnostics.push(context.diagnostic(
                    "COMPOSE_EXTERNAL_UNDECLARED",
                    format!("{kind}.{logical_key}.external"),
                    logical_key.clone(),
                ));
            }
        }
    }
}

fn external_enabled(value: &Value) -> bool {
    !matches!(value, Value::Null | Value::Bool(false))
}

fn evidence(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        other => other.to_string(),
    }
}
