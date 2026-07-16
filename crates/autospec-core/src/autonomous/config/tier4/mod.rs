mod validation;

use std::collections::BTreeSet;

use validation::{
    error, parse_bounded_u32, parse_scalar, strip_comment, valid_host, valid_id, valid_path,
};

const MIN_SOURCES: usize = 1;
const MAX_SOURCES: usize = 4;
const MIN_MAX_BYTES: u32 = 1;
const MAX_MAX_BYTES: u32 = 1_048_576;
const MIN_DEADLINE_MILLIS: u32 = 100;
const MAX_DEADLINE_MILLIS: u32 = 30_000;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tier4Config {
    pub sources: Vec<Tier4SourceDescriptor>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tier4SourceDescriptor {
    pub id: String,
    pub host: String,
    pub path: String,
    pub max_bytes: u32,
    pub deadline_millis: u32,
}

pub(super) fn parse(source: &str) -> Result<Tier4Config, String> {
    let mut config = Tier4Config::default();
    let mut current = None;
    let mut ids = BTreeSet::new();
    let mut hosts = BTreeSet::new();
    let mut in_tier4 = false;
    let mut saw_tier4 = false;
    let mut saw_sources = false;
    let mut tier4_line = 0;
    let mut sources_line = 0;

    for (index, raw_line) in source.lines().enumerate() {
        let line_number = index + 1;
        let line = strip_comment(raw_line).trim_end();
        if line.trim().is_empty() {
            continue;
        }

        let indent = line.len() - line.trim_start().len();
        let trimmed = line.trim_start();
        if indent == 0 {
            if in_tier4 {
                finalize(&mut current, &mut config, &mut ids, &mut hosts)?;
                in_tier4 = false;
            }
            if trimmed == "tier4:" {
                if saw_tier4 {
                    return Err(error(line_number, "duplicate tier4 block"));
                }
                saw_tier4 = true;
                in_tier4 = true;
                tier4_line = line_number;
                continue;
            }
            if declares_tier4(trimmed) {
                return Err(error(line_number, "tier4 must be a mapping"));
            }
            continue;
        }

        if !in_tier4 {
            continue;
        }

        if raw_line
            .chars()
            .take_while(|character| character.is_whitespace())
            .any(|character| character == '\t')
        {
            return Err(error(
                line_number,
                "tabs are not valid indentation in tier4",
            ));
        }

        match indent {
            2 => {
                finalize(&mut current, &mut config, &mut ids, &mut hosts)?;
                let Some((key, value)) = trimmed.split_once(':') else {
                    return Err(error(line_number, "tier4 entry must use key: value syntax"));
                };
                if key != "sources" {
                    return Err(error(line_number, &format!("unknown tier4 field `{key}`")));
                }
                if saw_sources {
                    return Err(error(line_number, "duplicate tier4.sources"));
                }
                if !value.trim().is_empty() {
                    return Err(error(line_number, "tier4.sources must be a block list"));
                }
                saw_sources = true;
                sources_line = line_number;
            }
            4 => {
                if !saw_sources {
                    return Err(error(
                        line_number,
                        "tier4 source descriptors must be inside tier4.sources",
                    ));
                }
                let Some(value) = trimmed.strip_prefix("- id:") else {
                    return Err(error(
                        line_number,
                        "tier4.sources entries must start with - id: scalar",
                    ));
                };
                finalize(&mut current, &mut config, &mut ids, &mut hosts)?;
                current = Some(DescriptorBuilder::new(
                    parse_scalar(value, line_number, "id")?,
                    line_number,
                ));
            }
            6 => {
                let Some(builder) = current.as_mut() else {
                    return Err(error(
                        line_number,
                        "tier4 source fields require a preceding - id entry",
                    ));
                };
                let Some((key, value)) = trimmed.split_once(':') else {
                    return Err(error(
                        line_number,
                        "tier4 source field must use key: value syntax",
                    ));
                };
                builder.set(key, value, line_number)?;
            }
            _ => {
                return Err(error(
                    line_number,
                    "malformed indentation or nested value in tier4",
                ));
            }
        }
    }

    if in_tier4 {
        finalize(&mut current, &mut config, &mut ids, &mut hosts)?;
    }
    if !saw_tier4 {
        return Ok(config);
    }
    if !saw_sources {
        return Err(error(tier4_line, "tier4 requires tier4.sources"));
    }
    if !(MIN_SOURCES..=MAX_SOURCES).contains(&config.sources.len()) {
        return Err(error(
            sources_line,
            "tier4.sources must contain between 1 and 4 descriptors",
        ));
    }

    Ok(config)
}

fn declares_tier4(value: &str) -> bool {
    value
        .split_once(':')
        .is_some_and(|(key, _)| key.trim() == "tier4")
        || value == "tier4"
}

fn finalize(
    current: &mut Option<DescriptorBuilder>,
    config: &mut Tier4Config,
    ids: &mut BTreeSet<String>,
    hosts: &mut BTreeSet<String>,
) -> Result<(), String> {
    let Some(builder) = current.take() else {
        return Ok(());
    };
    let line = builder.line;
    let descriptor = builder.finish()?;
    if !ids.insert(descriptor.id.clone()) {
        return Err(error(
            line,
            &format!("duplicate tier4 source id `{}`", descriptor.id),
        ));
    }
    if !hosts.insert(descriptor.host.clone()) {
        return Err(error(
            line,
            &format!("duplicate tier4 source host `{}`", descriptor.host),
        ));
    }
    config.sources.push(descriptor);
    Ok(())
}

struct DescriptorBuilder {
    id: String,
    host: Option<String>,
    path: Option<String>,
    max_bytes: Option<u32>,
    deadline_millis: Option<u32>,
    line: usize,
}

impl DescriptorBuilder {
    fn new(id: String, line: usize) -> Self {
        Self {
            id,
            host: None,
            path: None,
            max_bytes: None,
            deadline_millis: None,
            line,
        }
    }

    fn set(&mut self, key: &str, value: &str, line: usize) -> Result<(), String> {
        match key {
            "id" => Err(error(line, "duplicate tier4 source field `id`")),
            "host" if self.host.is_some() => {
                Err(error(line, "duplicate tier4 source field `host`"))
            }
            "path" if self.path.is_some() => {
                Err(error(line, "duplicate tier4 source field `path`"))
            }
            "max_bytes" if self.max_bytes.is_some() => {
                Err(error(line, "duplicate tier4 source field `max_bytes`"))
            }
            "deadline_millis" if self.deadline_millis.is_some() => Err(error(
                line,
                "duplicate tier4 source field `deadline_millis`",
            )),
            "host" => {
                self.host = Some(parse_scalar(value, line, "host")?);
                Ok(())
            }
            "path" => {
                self.path = Some(parse_scalar(value, line, "path")?);
                Ok(())
            }
            "max_bytes" => {
                self.max_bytes = Some(parse_bounded_u32(
                    value,
                    MIN_MAX_BYTES,
                    MAX_MAX_BYTES,
                    line,
                    "max_bytes",
                )?);
                Ok(())
            }
            "deadline_millis" => {
                self.deadline_millis = Some(parse_bounded_u32(
                    value,
                    MIN_DEADLINE_MILLIS,
                    MAX_DEADLINE_MILLIS,
                    line,
                    "deadline_millis",
                )?);
                Ok(())
            }
            _ => Err(error(line, &format!("unknown tier4 source field `{key}`"))),
        }
    }

    fn finish(self) -> Result<Tier4SourceDescriptor, String> {
        if !valid_id(&self.id) {
            return Err(error(
                self.line,
                "tier4 source id must be lower-kebab ASCII",
            ));
        }
        let host = self
            .host
            .ok_or_else(|| error(self.line, "tier4 source is missing host"))?;
        if !valid_host(&host) {
            return Err(error(
                self.line,
                "tier4 source host must be a lowercase DNS name",
            ));
        }
        let path = self
            .path
            .ok_or_else(|| error(self.line, "tier4 source is missing path"))?;
        if !valid_path(&path) {
            return Err(error(
                self.line,
                "tier4 source path is not a safe absolute path",
            ));
        }

        Ok(Tier4SourceDescriptor {
            id: self.id,
            host,
            path,
            max_bytes: self
                .max_bytes
                .ok_or_else(|| error(self.line, "tier4 source is missing max_bytes"))?,
            deadline_millis: self
                .deadline_millis
                .ok_or_else(|| error(self.line, "tier4 source is missing deadline_millis"))?,
        })
    }
}
