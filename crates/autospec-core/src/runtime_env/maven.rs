#![allow(clippy::result_large_err)] // Public plan contract returns the schema-stable diagnostic directly.

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

use super::{IsolationDiagnostic, MavenPlan};

mod arguments;

pub use arguments::{MavenArgPlatform, MavenArgs};

const MANAGED_PROPERTIES: [(&str, &str); 4] = [
    ("aether.lrm.enhanced.split", "true"),
    ("aether.lrm.enhanced.remotePrefix", "cached"),
    ("aether.lrm.enhanced.localPrefix", ""),
    ("aether.system.named.factory", "file-lock"),
];

impl MavenPlan {
    pub fn arguments(
        existing: &str,
        environment_id: &str,
    ) -> Result<MavenArgs, IsolationDiagnostic> {
        validate_environment_id(environment_id)?;
        let expected = MANAGED_PROPERTIES.map(|(key, value)| {
            if key == "aether.lrm.enhanced.localPrefix" {
                (key, format!("autospec/{environment_id}"))
            } else {
                (key, value.to_string())
            }
        });
        let parsed = MavenArgs::parse(existing)?;
        let mut arguments = MavenArgs { tokens: Vec::new() };
        let mut index = 0;
        while index < parsed.tokens.len() {
            let consumed = managed_property_at(&parsed.tokens, index, &expected, environment_id)?;
            if consumed == 0 {
                arguments.tokens.push(parsed.tokens[index].clone());
                index += 1;
            } else {
                index += consumed;
            }
        }
        for (key, value) in &expected {
            arguments.append_property(key, value);
        }
        Ok(arguments)
    }
}

fn managed_property_at(
    tokens: &[OsString],
    index: usize,
    expected: &[(&str, String); 4],
    environment_id: &str,
) -> Result<usize, IsolationDiagnostic> {
    let text = tokens[index].to_string_lossy();
    let compact = text.strip_prefix("-D").filter(|value| !value.is_empty());
    let separated = (text == "-D")
        .then(|| tokens.get(index + 1)?.to_str())
        .flatten();
    let Some((key, value)) = compact
        .or(separated)
        .and_then(|value| value.split_once('='))
    else {
        return Ok(0);
    };
    let Some((_, managed_value)) = expected.iter().find(|(managed, _)| *managed == key) else {
        return Ok(0);
    };
    if value != managed_value {
        return Err(diagnostic(
            "MAVEN_ARGUMENT_CONFLICT",
            key,
            &format!("managed Maven property {key} has conflicting value {value:?}"),
            environment_id,
        ));
    }
    Ok(if compact.is_some() { 1 } else { 2 })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MavenPurgeTarget {
    repository_root: PathBuf,
    target: PathBuf,
    environment_id: String,
}

impl MavenPurgeTarget {
    pub fn for_environment(
        repository_root: &Path,
        environment_id: &str,
    ) -> Result<Self, IsolationDiagnostic> {
        validate_environment_id(environment_id)?;
        let target = repository_root.join("autospec").join(environment_id);
        Self::new(repository_root, &target, environment_id)
    }

    pub fn new(
        repository_root: &Path,
        target: &Path,
        environment_id: &str,
    ) -> Result<Self, IsolationDiagnostic> {
        validate_environment_id(environment_id)?;
        if !repository_root.is_absolute()
            || repository_root
                .components()
                .any(|part| part == Component::ParentDir)
        {
            return Err(purge_diagnostic(
                "MAVEN_PURGE_OUTSIDE_REPOSITORY",
                target,
                environment_id,
            ));
        }
        let expected = repository_root.join("autospec").join(environment_id);
        if target != expected || target.components().any(|part| part == Component::ParentDir) {
            return Err(purge_diagnostic(
                "MAVEN_PURGE_OUTSIDE_REPOSITORY",
                target,
                environment_id,
            ));
        }
        Ok(Self {
            repository_root: repository_root.to_path_buf(),
            target: target.to_path_buf(),
            environment_id: environment_id.to_string(),
        })
    }

    pub fn repository_root(&self) -> &Path {
        &self.repository_root
    }

    pub fn target(&self) -> &Path {
        &self.target
    }

    pub fn environment_id(&self) -> &str {
        &self.environment_id
    }
}

fn validate_environment_id(environment_id: &str) -> Result<(), IsolationDiagnostic> {
    if environment_id.is_empty()
        || !environment_id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "_-".contains(character))
    {
        return Err(diagnostic(
            "MAVEN_PURGE_IDENTITY_MISMATCH",
            "maven.localPrefix",
            "environment identity is not a single safe path component",
            environment_id,
        ));
    }
    Ok(())
}

fn purge_diagnostic(code: &str, target: &Path, environment_id: &str) -> IsolationDiagnostic {
    diagnostic(
        code,
        "maven.localPrefix",
        &format!("refusing Maven purge target {}", target.display()),
        environment_id,
    )
}

fn diagnostic(
    code: &str,
    resource: &str,
    evidence: &str,
    environment_id: &str,
) -> IsolationDiagnostic {
    IsolationDiagnostic {
        schema_version: 1,
        code: code.to_string(),
        environment_id: environment_id.to_string(),
        resource: resource.to_string(),
        evidence: evidence.to_string(),
        recovery_command:
            "remove the conflicting Maven argument or repair authoritative runtime state"
                .to_string(),
    }
}
