use std::path::PathBuf;

const DEFAULT_CHANGED_BASE: &str = "origin/main";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Jobs {
    Auto,
    Fixed(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationOptions {
    pub fast: bool,
    pub changed_base: Option<String>,
    pub since: Option<String>,
    pub jobs: Jobs,
    pub json: bool,
    pub paths: Vec<String>,
    pub shadow_results: Option<PathBuf>,
    execution_requested: bool,
}

impl Default for ValidationOptions {
    fn default() -> Self {
        Self {
            fast: false,
            changed_base: None,
            since: None,
            jobs: Jobs::Fixed(1),
            json: false,
            paths: Vec::new(),
            shadow_results: None,
            execution_requested: false,
        }
    }
}

impl ValidationOptions {
    pub fn parse(arguments: impl IntoIterator<Item = impl AsRef<str>>) -> Result<Self, String> {
        let arguments = arguments
            .into_iter()
            .map(|argument| argument.as_ref().to_string())
            .collect::<Vec<_>>();
        let mut options = Self::default();
        let mut index = 0;

        while index < arguments.len() {
            let argument = &arguments[index];
            match argument.as_str() {
                "--fast" | "--no-bats" => {
                    options.fast = true;
                    options.execution_requested = true;
                }
                "--changed" => {
                    if options.changed_base.is_none() {
                        options.changed_base = Some(DEFAULT_CHANGED_BASE.to_string());
                    }
                    options.execution_requested = true;
                }
                "--since" => {
                    let reference = required_value(&arguments, &mut index, "--since", "a ref")?;
                    options.changed_base = Some(reference.clone());
                    options.since = Some(reference);
                    options.execution_requested = true;
                }
                "--jobs" => {
                    let jobs = optional_jobs_value(&arguments, &mut index)?;
                    options.jobs = parse_jobs(jobs.as_deref())?;
                    options.execution_requested = true;
                }
                "--json" => options.json = true,
                "--path" => options
                    .paths
                    .push(required_value(&arguments, &mut index, "--path", "a path")?),
                "--shadow-results" => {
                    if !options.paths.is_empty() || options.shadow_results.is_some() {
                        return Err("autospec validate accepts only one mode".to_string());
                    }
                    options.shadow_results = Some(PathBuf::from(required_value(
                        &arguments,
                        &mut index,
                        "--shadow-results",
                        "a path",
                    )?));
                }
                _ if argument.starts_with("--changed=") => {
                    let base = argument
                        .strip_prefix("--changed=")
                        .expect("changed option prefix matched");
                    if base.is_empty() {
                        return Err("autospec validate --changed requires a base".to_string());
                    }
                    options.changed_base = Some(base.to_string());
                    options.execution_requested = true;
                }
                _ if argument.starts_with("--jobs=") => {
                    let jobs = argument
                        .strip_prefix("--jobs=")
                        .expect("jobs option prefix matched");
                    options.jobs = parse_jobs(Some(jobs))?;
                    options.execution_requested = true;
                }
                _ => return Err(format!("unknown autospec validate option: {argument}")),
            }
            index += 1;
        }

        if options.shadow_results.is_some()
            && (!options.paths.is_empty() || options.execution_requested)
        {
            return Err("autospec validate accepts only one mode".to_string());
        }

        Ok(options)
    }

    pub fn requests_execution(&self) -> bool {
        self.execution_requested
    }
}

fn required_value(
    arguments: &[String],
    index: &mut usize,
    option: &str,
    description: &str,
) -> Result<String, String> {
    *index += 1;
    arguments
        .get(*index)
        .filter(|value| !value.is_empty() && !value.starts_with("--"))
        .cloned()
        .ok_or_else(|| format!("autospec validate {option} requires {description}"))
}

fn optional_jobs_value(arguments: &[String], index: &mut usize) -> Result<Option<String>, String> {
    let Some(value) = arguments.get(*index + 1) else {
        return Ok(None);
    };
    if value.starts_with("--") {
        return Ok(None);
    }

    *index += 1;
    if value.is_empty() {
        return Err("autospec validate --jobs requires a positive integer or auto".to_string());
    }
    Ok(Some(value.clone()))
}

fn parse_jobs(value: Option<&str>) -> Result<Jobs, String> {
    match value {
        None | Some("auto") => Ok(Jobs::Auto),
        Some(value) => value
            .parse::<usize>()
            .ok()
            .filter(|jobs| *jobs > 0)
            .map(Jobs::Fixed)
            .ok_or_else(|| {
                "autospec validate --jobs requires a positive integer or auto".to_string()
            }),
    }
}
