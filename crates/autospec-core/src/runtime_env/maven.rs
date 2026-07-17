#![allow(clippy::result_large_err)] // Public plan contract returns the schema-stable diagnostic directly.

use std::ffi::{OsStr, OsString};
use std::iter::Peekable;
use std::path::{Component, Path, PathBuf};
use std::str::Chars;

use super::{IsolationDiagnostic, MavenPlan};

const MANAGED_PROPERTIES: [(&str, &str); 4] = [
    ("aether.lrm.enhanced.split", "true"),
    ("aether.lrm.enhanced.remotePrefix", "cached"),
    ("aether.lrm.enhanced.localPrefix", ""),
    ("aether.system.named.factory", "file-lock"),
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MavenArgs {
    tokens: Vec<OsString>,
}

impl MavenArgs {
    pub fn parse(source: &str) -> Result<Self, IsolationDiagnostic> {
        ArgumentParser::new(source)
            .parse()
            .map(|tokens| Self { tokens })
    }

    pub fn tokens(&self) -> &[OsString] {
        &self.tokens
    }

    pub fn append_property(&mut self, key: &str, value: &str) {
        self.tokens.push(OsString::from(format!("-D{key}={value}")));
    }

    pub fn render(&self) -> String {
        self.tokens
            .iter()
            .map(|token| quote_token(token.as_os_str()))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

struct ArgumentParser<'a> {
    characters: Peekable<Chars<'a>>,
    tokens: Vec<OsString>,
    token: String,
    quote: Option<char>,
    started: bool,
}

impl<'a> ArgumentParser<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            characters: source.chars().peekable(),
            tokens: Vec::new(),
            token: String::new(),
            quote: None,
            started: false,
        }
    }

    fn parse(mut self) -> Result<Vec<OsString>, IsolationDiagnostic> {
        while let Some(character) = self.characters.next() {
            self.consume(character)?;
        }
        self.finish()
    }

    fn consume(&mut self, character: char) -> Result<(), IsolationDiagnostic> {
        match self.quote {
            Some(marker) if character == marker => self.quote = None,
            Some('\'') => self.token.push(character),
            Some('"') if character == '\\' => self.consume_double_escape(),
            Some(_) => self.token.push(character),
            None if character == '\'' || character == '"' => self.open_quote(character),
            None if character.is_whitespace() => self.flush(),
            None if character == '\\' => self.consume_escape()?,
            None => self.push(character),
        }
        Ok(())
    }

    fn consume_double_escape(&mut self) {
        match self.characters.peek().copied() {
            Some(next) if next == '"' || next == '\\' => {
                self.characters.next();
                self.token.push(next);
            }
            _ => self.token.push('\\'),
        }
    }

    fn consume_escape(&mut self) -> Result<(), IsolationDiagnostic> {
        let next = self.characters.next().ok_or_else(|| {
            diagnostic(
                "MAVEN_ARGUMENT_PARSE",
                "MAVEN_ARGS",
                "trailing escape in MAVEN_ARGS",
                "",
            )
        })?;
        self.push(next);
        Ok(())
    }

    fn open_quote(&mut self, marker: char) {
        self.quote = Some(marker);
        self.started = true;
    }

    fn push(&mut self, character: char) {
        self.token.push(character);
        self.started = true;
    }

    fn flush(&mut self) {
        if self.started {
            self.tokens
                .push(OsString::from(std::mem::take(&mut self.token)));
            self.started = false;
        }
    }

    fn finish(mut self) -> Result<Vec<OsString>, IsolationDiagnostic> {
        if self.quote.is_some() {
            return Err(diagnostic(
                "MAVEN_ARGUMENT_PARSE",
                "MAVEN_ARGS",
                "unterminated quote in MAVEN_ARGS",
                "",
            ));
        }
        self.flush();
        Ok(self.tokens)
    }
}

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
        for token in parsed.tokens {
            if let Some(token) = caller_token(token, &expected, environment_id)? {
                arguments.tokens.push(token);
            }
        }
        for (key, value) in &expected {
            arguments.append_property(key, value);
        }
        Ok(arguments)
    }
}

fn caller_token(
    token: OsString,
    expected: &[(&str, String); 4],
    environment_id: &str,
) -> Result<Option<OsString>, IsolationDiagnostic> {
    let text = token.to_string_lossy();
    let Some((key, value)) = text
        .strip_prefix("-D")
        .and_then(|value| value.split_once('='))
    else {
        return Ok(Some(token));
    };
    let Some((_, managed_value)) = expected.iter().find(|(managed, _)| *managed == key) else {
        return Ok(Some(token));
    };
    if value != managed_value {
        return Err(diagnostic(
            "MAVEN_ARGUMENT_CONFLICT",
            key,
            &format!("managed Maven property {key} has conflicting value {value:?}"),
            environment_id,
        ));
    }
    Ok(None)
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

fn quote_token(token: &OsStr) -> String {
    let value = token.to_string_lossy();
    if !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-._/:=+".contains(character))
    {
        value.into_owned()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
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
