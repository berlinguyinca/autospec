use autospec_core::autonomous::config::AutonomousConfig;
use autospec_core::autonomous::tier4::{Tier4SourcePolicy, TIER4_SCHEMA};
use autospec_core::autonomous::waterfall::sha256_hex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WaterfallPolicy {
    tier4_source: Option<Tier4SourcePolicy>,
}

impl WaterfallPolicy {
    pub(super) fn from_config(config: &AutonomousConfig) -> Result<Self, String> {
        if config.tier4.sources.is_empty() {
            return Ok(Self { tier4_source: None });
        }

        let descriptors = config.tier4.sources.clone();
        let mut identity_input = format!("schema:{TIER4_SCHEMA}\n");
        for descriptor in &descriptors {
            append_field(&mut identity_input, "id", &descriptor.id);
            append_field(&mut identity_input, "host", &descriptor.host);
            append_field(&mut identity_input, "path", &descriptor.path);
            identity_input.push_str(&format!(
                "max_bytes:{}\ndeadline_millis:{}\n",
                descriptor.max_bytes, descriptor.deadline_millis
            ));
        }
        let policy_identity = format!(
            "autospec-tier4-policy-v1:{}",
            sha256_hex(identity_input.as_bytes())
        );
        Ok(Self {
            tier4_source: Some(Tier4SourcePolicy {
                schema_version: TIER4_SCHEMA,
                policy_identity,
                descriptors,
            }),
        })
    }

    pub(super) fn tier4_source(&self) -> Option<&Tier4SourcePolicy> {
        self.tier4_source.as_ref()
    }

    #[cfg(test)]
    pub(super) fn from_tier4_source_for_test(tier4_source: Tier4SourcePolicy) -> Self {
        Self {
            tier4_source: Some(tier4_source),
        }
    }
}

fn append_field(output: &mut String, name: &str, value: &str) {
    output.push_str(&format!("{name}:{}:{value}\n", value.len()));
}
