use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FencedSurface {
    pub id: String,
    pub severity: String,
    pub reason: String,
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FencedMatch {
    pub path: String,
    pub surface: String,
    pub severity: String,
    pub reason: String,
    pub pattern: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BlastRadiusClassification {
    pub decision: String,
    pub reason: Option<String>,
    pub label: String,
    pub fenced: bool,
    pub reversibility: String,
    pub paths: Vec<String>,
    pub fenced_matches: Vec<FencedMatch>,
    pub registry: String,
}

impl BlastRadiusClassification {
    pub fn to_json_string(&self) -> Result<String, String> {
        serde_json::to_string(self).map_err(|error| error.to_string())
    }
}

pub fn default_legacy_registry() -> Vec<FencedSurface> {
    vec![
        surface(
            "autonomous-control-plane",
            "fenced",
            "autonomous conductor or guardrail control plane",
            &[
                "scripts/autospec-autonomous.sh",
                "scripts/autonomous-*.sh",
                "scripts/autospec-autonomous-run-drain.sh",
                "scripts/worktree-guard.sh",
                "scripts/claim-guard.sh",
                "scripts/autospec-autonomy-gate.sh",
            ],
        ),
        surface(
            "autospec-policy-config",
            "fenced",
            "autospec policy config controls autonomous safety policy",
            &[".autospec/**"],
        ),
        surface(
            "skill-contracts",
            "high",
            "autospec skill public contracts",
            &[
                "skills/autospec*/SKILL.md",
                "skills/autospec*/codex/prompt.md",
                "skills/autospec*/opencode/agent.md",
            ],
        ),
        surface(
            "release-and-ci",
            "high",
            "release, install, or CI surface",
            &[
                ".github/workflows/*",
                "install.sh",
                "bootstrap.sh",
                "uninstall.sh",
            ],
        ),
        surface(
            "schema-package-core",
            "high",
            "schema/package/crate core surface",
            &[
                "schemas/*",
                "packages/*",
                "crates/*",
                "Cargo.toml",
                "Cargo.lock",
            ],
        ),
        surface(
            "trading-money-risk",
            "fenced",
            "trading system money/risk/execution paths",
            &[
                "trading-system/money/**",
                "trading-system/risk/**",
                "trading-system/execution/**",
            ],
        ),
        surface(
            "sensitive-keywords",
            "high",
            "migration/auth/secret/token path keyword",
            &["*migration*", "*secret*", "*auth*", "*token*"],
        ),
    ]
}

pub fn parse_fenced_surfaces(source: &str) -> Result<Vec<FencedSurface>, String> {
    let mut rows = Vec::new();
    let mut active = false;
    let mut base_indent = 0usize;
    let mut current: Option<FencedSurface> = None;
    let mut in_paths = false;

    for (index, raw_line) in source.lines().enumerate() {
        let line_number = index + 1;
        let trimmed_without_comment = raw_line.split('#').next().unwrap_or_default();
        if trimmed_without_comment.trim().is_empty() {
            continue;
        }
        let indent = trimmed_without_comment
            .chars()
            .take_while(|character| *character == ' ')
            .count();
        let stripped = trimmed_without_comment.trim();

        if !active {
            if stripped == "fenced_surfaces:" {
                active = true;
                base_indent = indent;
            }
            continue;
        }
        if indent <= base_indent && stripped != "fenced_surfaces:" {
            break;
        }
        if let Some(value) = stripped.strip_prefix("- id:") {
            push_current_surface(&mut rows, &mut current);
            current = Some(FencedSurface {
                id: scalar(value),
                severity: "high".to_string(),
                reason: "configured fenced surface".to_string(),
                paths: Vec::new(),
            });
            in_paths = false;
            continue;
        }

        let surface = match current.as_mut() {
            Some(surface) => surface,
            None => continue,
        };
        if stripped == "paths:" {
            in_paths = true;
            continue;
        }
        if in_paths {
            if let Some(path) = stripped.strip_prefix("- ") {
                surface.paths.push(scalar(path));
                continue;
            }
        }
        if let Some((key, value)) = stripped.split_once(':') {
            in_paths = apply_surface_field(surface, key.trim(), value);
        } else if in_paths {
            return Err(format!(
                "line {line_number}: fenced_surfaces paths entries must be list items"
            ));
        }
    }

    push_current_surface(&mut rows, &mut current);

    Ok(rows
        .into_iter()
        .filter(|surface| !surface.id.is_empty() && !surface.paths.is_empty())
        .collect())
}

pub fn classify_paths<I, P>(paths: I, registry: &[FencedSurface]) -> BlastRadiusClassification
where
    I: IntoIterator<Item = P>,
    P: AsRef<str>,
{
    let paths = paths
        .into_iter()
        .map(|path| normalize_path(path.as_ref()))
        .filter(|path| !path.is_empty())
        .collect::<Vec<_>>();
    let fenced_matches = match_registry(&paths, registry);
    let fenced = !fenced_matches.is_empty();
    let has_fenced_severity = fenced_matches
        .iter()
        .any(|matched| matched.severity == "fenced");
    let label = if has_fenced_severity {
        "blast:fenced"
    } else if fenced {
        "blast:high"
    } else if is_medium_radius(&paths) {
        "blast:medium"
    } else {
        "blast:low"
    }
    .to_string();

    BlastRadiusClassification {
        decision: if fenced { "quarantine" } else { "allow" }.to_string(),
        reason: fenced.then(|| "fenced_surface".to_string()),
        label,
        fenced,
        reversibility: if paths.iter().any(|path| {
            let lower = path.to_ascii_lowercase();
            ["migration", "schema", "auth", "secret", "token"]
                .iter()
                .any(|keyword| lower.contains(keyword))
        }) {
            "requires-review"
        } else {
            "reversible"
        }
        .to_string(),
        paths,
        fenced_matches,
        registry: "configured".to_string(),
    }
}

pub fn classify_paths_with_registry_name<I, P>(
    paths: I,
    registry: &[FencedSurface],
    registry_name: impl Into<String>,
) -> BlastRadiusClassification
where
    I: IntoIterator<Item = P>,
    P: AsRef<str>,
{
    let mut classification = classify_paths(paths, registry);
    classification.registry = registry_name.into();
    classification
}

fn match_registry(paths: &[String], registry: &[FencedSurface]) -> Vec<FencedMatch> {
    paths
        .iter()
        .flat_map(|changed| {
            registry
                .iter()
                .filter_map(move |surface| match_surface(changed, surface))
        })
        .collect()
}

fn match_surface(changed: &str, surface: &FencedSurface) -> Option<FencedMatch> {
    let pattern = surface
        .paths
        .iter()
        .map(|pattern| normalize_path(pattern))
        .find(|pattern| matches_pattern(pattern, changed))?;

    Some(FencedMatch {
        path: changed.to_string(),
        surface: surface.id.clone(),
        severity: surface.severity.clone(),
        reason: surface.reason.clone(),
        pattern,
    })
}

fn matches_pattern(pattern: &str, path: &str) -> bool {
    if let Some(prefix) = pattern.strip_suffix("/**") {
        return path == prefix || path.starts_with(&format!("{prefix}/"));
    }
    glob_match(pattern.as_bytes(), path.as_bytes())
}

fn glob_match(pattern: &[u8], text: &[u8]) -> bool {
    match (pattern.split_first(), text.split_first()) {
        (None, None) => true,
        (None, Some(_)) => false,
        (Some((&b'*', pattern_rest)), _) => {
            glob_match(pattern_rest, text)
                || text
                    .split_first()
                    .is_some_and(|(_, text_rest)| glob_match(pattern, text_rest))
        }
        (Some((&expected, pattern_rest)), Some((&actual, text_rest))) if expected == actual => {
            glob_match(pattern_rest, text_rest)
        }
        _ => false,
    }
}

fn is_medium_radius(paths: &[String]) -> bool {
    let mut top_level = Vec::<&str>::new();
    for path in paths {
        let segment = path.split('/').next().unwrap_or_default();
        if !top_level.contains(&segment) {
            top_level.push(segment);
        }
    }
    top_level.len() > 3 || paths.len() > 10
}

fn scalar(value: &str) -> String {
    let value = value.trim();
    if value.len() >= 2
        && ((value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\'')))
    {
        value[1..value.len() - 1].to_string()
    } else {
        value.to_string()
    }
}

fn push_current_surface(rows: &mut Vec<FencedSurface>, current: &mut Option<FencedSurface>) {
    if let Some(surface) = current.take() {
        rows.push(surface);
    }
}

fn apply_surface_field(surface: &mut FencedSurface, key: &str, value: &str) -> bool {
    match key {
        "id" => surface.id = scalar(value),
        "severity" => surface.severity = scalar(value),
        "reason" => surface.reason = scalar(value),
        "paths" => return true,
        _ => {}
    }
    false
}

fn normalize_path(path: &str) -> String {
    path.trim()
        .strip_prefix("./")
        .unwrap_or(path.trim())
        .to_string()
}

fn surface(id: &str, severity: &str, reason: &str, paths: &[&str]) -> FencedSurface {
    FencedSurface {
        id: id.to_string(),
        severity: severity.to_string(),
        reason: reason.to_string(),
        paths: paths.iter().map(|path| path.to_string()).collect(),
    }
}
