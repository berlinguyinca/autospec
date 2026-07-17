use serde_json::Value;

use super::super::{ComposePlan, ExportProtocol, RuntimeEnvError};

const ENVIRONMENT_LABEL: &str = "com.autospec.environment-id";
const OWNER_LABEL: &str = "com.autospec.owner-key";
const PLAN_LABEL: &str = "com.autospec.plan-digest";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComposeOwnership {
    pub environment_id: String,
    pub owner_key: String,
    pub plan_digest: String,
}

impl ComposeOwnership {
    pub fn label_filters(&self) -> [String; 3] {
        [
            format!("{ENVIRONMENT_LABEL}={}", self.environment_id),
            format!("{OWNER_LABEL}={}", self.owner_key),
            format!("{PLAN_LABEL}={}", self.plan_digest),
        ]
    }

    pub fn matches(&self, labels: &serde_json::Map<String, Value>) -> bool {
        labels.get(ENVIRONMENT_LABEL).and_then(Value::as_str) == Some(self.environment_id.as_str())
            && labels.get(OWNER_LABEL).and_then(Value::as_str) == Some(self.owner_key.as_str())
            && labels.get(PLAN_LABEL).and_then(Value::as_str) == Some(self.plan_digest.as_str())
    }

    pub fn matches_json(&self, source: &[u8]) -> Result<bool, RuntimeEnvError> {
        let labels = serde_json::from_slice::<serde_json::Map<String, Value>>(source)
            .map_err(|error| RuntimeEnvError::new(format!("invalid Docker label JSON: {error}")))?;
        Ok(self.matches(&labels))
    }
}

pub struct ComposeOverride;

impl ComposeOverride {
    pub fn render_json(
        plan: &ComposePlan,
        model: &[u8],
        ownership: &ComposeOwnership,
    ) -> Result<String, RuntimeEnvError> {
        let model = serde_json::from_slice(model).map_err(|error| {
            RuntimeEnvError::new(format!("could not parse resolved Compose model: {error}"))
        })?;
        Self::render(plan, &model, ownership)
    }

    pub fn render(
        plan: &ComposePlan,
        model: &Value,
        ownership: &ComposeOwnership,
    ) -> Result<String, RuntimeEnvError> {
        let root = model
            .as_object()
            .ok_or_else(|| RuntimeEnvError::new("resolved Compose model must be an object"))?;
        let mut output = String::new();
        render_services(&mut output, root.get("services"), plan, ownership)?;
        render_resources(&mut output, "networks", root.get("networks"), ownership)?;
        render_resources(&mut output, "volumes", root.get("volumes"), ownership)?;
        Ok(output)
    }
}

fn render_services(
    output: &mut String,
    value: Option<&Value>,
    plan: &ComposePlan,
    ownership: &ComposeOwnership,
) -> Result<(), RuntimeEnvError> {
    let services = object_or_empty(value, "services")?;
    output.push_str("services:\n");
    for service in services.keys() {
        output.push_str(&format!("  {}:\n", yaml_key(service)));
        render_labels(output, 4, ownership);
        let exports = plan
            .exports
            .iter()
            .filter(|export| export.service == *service)
            .collect::<Vec<_>>();
        if !exports.is_empty() {
            output.push_str("    ports:\n");
        }
        for export in exports {
            let protocol = match export.protocol {
                ExportProtocol::Udp => "udp",
                _ => "tcp",
            };
            output.push_str(&format!(
                "      - target: {}\n        published: '0'\n        host_ip: '127.0.0.1'\n        protocol: {protocol}\n",
                export.target
            ));
        }
    }
    Ok(())
}

fn render_resources(
    output: &mut String,
    name: &str,
    value: Option<&Value>,
    ownership: &ComposeOwnership,
) -> Result<(), RuntimeEnvError> {
    let resources = object_or_empty(value, name)?;
    if resources.is_empty() {
        return Ok(());
    }
    output.push_str(&format!("{name}:\n"));
    for key in resources.keys() {
        if resources
            .get(key)
            .and_then(Value::as_object)
            .and_then(|resource| resource.get("external"))
            .is_some_and(|external| external == true)
        {
            continue;
        }
        output.push_str(&format!("  {}:\n", yaml_key(key)));
        render_labels(output, 4, ownership);
    }
    Ok(())
}

fn object_or_empty<'a>(
    value: Option<&'a Value>,
    name: &str,
) -> Result<&'a serde_json::Map<String, Value>, RuntimeEnvError> {
    static EMPTY: std::sync::LazyLock<serde_json::Map<String, Value>> =
        std::sync::LazyLock::new(serde_json::Map::new);
    match value {
        None | Some(Value::Null) => Ok(&EMPTY),
        Some(Value::Object(object)) => Ok(object),
        Some(_) => Err(RuntimeEnvError::new(format!(
            "resolved Compose {name} must be an object"
        ))),
    }
}

fn render_labels(output: &mut String, spaces: usize, ownership: &ComposeOwnership) {
    let indent = " ".repeat(spaces);
    output.push_str(&format!("{indent}labels:\n"));
    for (key, value) in [
        (ENVIRONMENT_LABEL, ownership.environment_id.as_str()),
        (OWNER_LABEL, ownership.owner_key.as_str()),
        (PLAN_LABEL, ownership.plan_digest.as_str()),
    ] {
        output.push_str(&format!("{indent}  {key}: {}\n", yaml_string(value)));
    }
}

fn yaml_key(value: &str) -> String {
    if !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "_.-".contains(character))
    {
        value.to_string()
    } else {
        yaml_string(value)
    }
}

fn yaml_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}
