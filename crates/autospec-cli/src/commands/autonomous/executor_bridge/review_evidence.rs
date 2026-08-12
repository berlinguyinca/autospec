use super::*;
use autospec_core::autonomous::blast_radius::{classify_paths, default_legacy_registry};
use autospec_core::autonomous::review_policy::{
    classify_review_requirements, ReviewPolicyInput, ReviewReasoning, ReviewRisk,
};

mod integration;
pub(super) use integration::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ExecutorReviewInventory {
    pub(super) changed_paths: Vec<String>,
    pub(super) logical_components: Vec<String>,
    pub(super) producer_surfaces: Vec<String>,
    pub(super) consumer_surfaces: Vec<String>,
}

pub(super) fn classify_executor_review_requirements(
    request: &ExecutorBridgeRequest,
    state: &PersistedInvocation,
) -> Result<ReviewRequirements, String> {
    let inventory = executor_review_inventory(state)?;
    Ok(review_requirements_for_inventory(request, &inventory))
}

pub(super) fn review_requirements_for_inventory(
    request: &ExecutorBridgeRequest,
    inventory: &ExecutorReviewInventory,
) -> ReviewRequirements {
    let blast = classify_paths(&inventory.changed_paths, &default_legacy_registry());
    classify_review_requirements(&ReviewPolicyInput {
        changed_paths: inventory.changed_paths.clone(),
        serialization_reasons: request.serialization_reasons.clone(),
        logical_component_count: inventory.logical_components.len(),
        has_producer_surface: !inventory.producer_surfaces.is_empty(),
        has_consumer_surface: !inventory.consumer_surfaces.is_empty(),
        critical_boundary: blast.label == "blast:fenced",
    })
}

pub(super) fn executor_review_inventory(
    state: &PersistedInvocation,
) -> Result<ExecutorReviewInventory, String> {
    let changed = changed_paths_since_base(&state.identity.worktree, &state.identity.base_oid)?;
    let changed_paths = changed.all.into_iter().collect::<Vec<_>>();
    let logical_components = changed_paths
        .iter()
        .filter_map(|path| logical_review_component(path))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let producer_surfaces = changed_paths
        .iter()
        .filter(|path| review_surface_matches(path, PRODUCER_SURFACE_TOKENS))
        .cloned()
        .collect();
    let consumer_surfaces = changed_paths
        .iter()
        .filter(|path| review_surface_matches(path, CONSUMER_SURFACE_TOKENS))
        .cloned()
        .collect();
    Ok(ExecutorReviewInventory {
        changed_paths,
        logical_components,
        producer_surfaces,
        consumer_surfaces,
    })
}

const PRODUCER_SURFACE_TOKENS: &[&str] = &[
    "producer",
    "publisher",
    "publish",
    "emitter",
    "emit",
    "writer",
    "encoder",
    "serialize",
    "serializer",
];
const CONSUMER_SURFACE_TOKENS: &[&str] = &[
    "consumer",
    "consume",
    "subscriber",
    "reader",
    "parser",
    "parse",
    "loader",
    "decoder",
    "deserialize",
    "deserializer",
];

fn review_surface_matches(path: &str, role_tokens: &[&str]) -> bool {
    path.split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .any(|token| {
            role_tokens
                .iter()
                .any(|role| token.eq_ignore_ascii_case(role))
        })
}

fn logical_review_component(path: &str) -> Option<String> {
    let mut components = path.split('/').filter(|component| !component.is_empty());
    let first = components.next()?;
    let second = components.next();
    Some(match (first, second) {
        ("crates" | "skills" | "tests", Some(second)) => format!("{first}/{second}"),
        _ => first.to_string(),
    })
}

pub(super) fn canonical_review_requirements_digest(requirements: &ReviewRequirements) -> String {
    let risk = match requirements.risk {
        ReviewRisk::Normal => "normal",
        ReviewRisk::High => "high",
        ReviewRisk::Integration => "integration",
        ReviewRisk::Critical => "critical",
    };
    let reasoning = match requirements.reviewer_reasoning {
        ReviewReasoning::Standard => "standard",
        ReviewReasoning::High => "high",
    };
    sha256_hex(
        format!(
            "{risk}\0{reasoning}\0{}\0{}\0{}\0{}\0{}",
            requirements.integration_shaped,
            requirements.require_integration_smoke,
            requirements.prefer_provider_diversity,
            requirements.require_provider_diversity,
            requirements.reasons.join("\0"),
        )
        .as_bytes(),
    )
}

pub(super) fn evidence_input_digests(
    lane: &PremergeLaneIdentity,
    request: &DeterministicEvidenceRequest<'_>,
) -> Result<(String, String), String> {
    let scanner_policy_digest = gitleaks_policy_digest(&request.state.identity.worktree)?;
    let review_requirements_digest =
        canonical_review_requirements_digest(&request.review_requirements);
    let semantic_input_digest = sha256_hex(
        format!(
            "{}\0{}\0{}\0{}\0{}\0{}\0{}\0{}",
            lane.lane_digest(),
            request.state.identity.base_ref,
            request.state.identity.base_oid,
            request.issue_body,
            request.spec_documents.join("\0"),
            scanner_policy_digest,
            REQUIRED_SCANNER_POLICY_SCHEMA,
            review_requirements_digest,
        )
        .as_bytes(),
    );
    let input_digest = sha256_hex(
        format!(
            "{}\0{}",
            semantic_input_digest,
            request
                .runtime
                .map(DirectRuntimeAdapter::session_id)
                .unwrap_or("")
        )
        .as_bytes(),
    );
    Ok((semantic_input_digest, input_digest))
}

fn first_fenced_command_under<'a>(body: &'a str, heading: &str) -> Result<&'a str, String> {
    let lines = body.lines().collect::<Vec<_>>();
    let heading_index = lines
        .iter()
        .position(|line| normalized_level_three_heading(line).as_deref() == Some(heading))
        .ok_or_else(|| format!("executor issue is missing the {heading} heading"))?;
    let section_end = lines[heading_index + 1..]
        .iter()
        .position(|line| line.trim_start().starts_with("###"))
        .map_or(lines.len(), |offset| heading_index + 1 + offset);
    let section = &lines[heading_index + 1..section_end];
    let fence = section
        .iter()
        .position(|line| line.trim_start().starts_with("```"))
        .ok_or_else(|| format!("executor {heading} requires a fenced command"))?;
    let fence_end = section[fence + 1..]
        .iter()
        .position(|line| line.trim() == "```")
        .map(|offset| fence + 1 + offset)
        .ok_or_else(|| format!("executor {heading} fence is unterminated"))?;
    let commands = section[fence + 1..fence_end]
        .iter()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect::<Vec<_>>();
    if commands.len() != 1 {
        return Err(format!(
            "executor {heading} requires exactly one non-comment command line"
        ));
    }
    Ok(commands[0])
}

pub(super) fn normalized_level_three_heading(line: &str) -> Option<String> {
    let trimmed = line.trim();
    let hash_count = trimmed
        .chars()
        .take_while(|character| *character == '#')
        .count();
    if hash_count != 3 {
        return None;
    }
    let content = trimmed.get(hash_count..)?;
    if !content.starts_with(char::is_whitespace) {
        return None;
    }
    Some(
        content
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_ascii_lowercase(),
    )
}

pub(super) fn parse_direct_command_plan(line: &str) -> Result<DirectCommandPlan, String> {
    if line.is_empty()
        || line.len() > MAX_DIRECT_COMMAND_LINE
        || line.contains('\n')
        || line.contains('\r')
        || line.contains('\0')
    {
        return Err("executor direct command line is empty, multiline, or oversized".to_string());
    }
    if line.contains("$(") || line.contains('`') {
        return Err("executor direct command rejects command substitution".to_string());
    }

    let mut segments = Vec::<Vec<String>>::new();
    let mut argv = Vec::<String>::new();
    let mut token = String::new();
    let mut token_started = false;
    let mut quote = None;
    let mut escaped = false;
    let characters = line.chars().collect::<Vec<_>>();
    let mut index = 0;
    while index < characters.len() {
        let character = characters[index];
        if escaped {
            token.push(character);
            token_started = true;
            escaped = false;
            index += 1;
            continue;
        }
        if character == '\\' && quote != Some('\'') {
            token_started = true;
            escaped = true;
            index += 1;
            continue;
        }
        if let Some(delimiter) = quote {
            if character == delimiter {
                quote = None;
            } else {
                token.push(character);
            }
            index += 1;
            continue;
        }
        if matches!(character, '\'' | '"') {
            token_started = true;
            quote = Some(character);
            index += 1;
            continue;
        }
        if character == '&' && characters.get(index + 1) == Some(&'&') {
            finish_direct_token(&mut argv, &mut token, &mut token_started)?;
            if argv.is_empty() {
                return Err("executor direct command contains an empty segment".to_string());
            }
            segments.push(std::mem::take(&mut argv));
            index += 2;
            continue;
        }
        if matches!(character, '|' | '<' | '>' | ';' | '&') {
            return Err(format!(
                "executor direct command rejects shell operator {character}"
            ));
        }
        if character.is_whitespace() {
            finish_direct_token(&mut argv, &mut token, &mut token_started)?;
        } else {
            token.push(character);
            token_started = true;
        }
        index += 1;
    }
    if escaped || quote.is_some() {
        return Err("executor direct command has unterminated quoting or escaping".to_string());
    }
    finish_direct_token(&mut argv, &mut token, &mut token_started)?;
    if argv.is_empty() {
        return Err("executor direct command contains an empty segment".to_string());
    }
    segments.push(argv);
    if segments.len() > MAX_DIRECT_COMMAND_SEGMENTS {
        return Err("executor direct command has too many sequential segments".to_string());
    }
    const CONTROL_WORDS: [&str; 42] = [
        ".", "[", "[[", "]", "]]", "{", "}", "break", "case", "cd", "command", "continue",
        "coproc", "do", "done", "elif", "else", "esac", "eval", "exec", "exit", "export", "fi",
        "for", "function", "if", "readonly", "return", "select", "set", "shift", "source", "then",
        "time", "times", "trap", "umask", "unset", "until", "wait", "while", "in",
    ];
    let commands = segments
        .into_iter()
        .map(|argv| validate_direct_argv(argv, &CONTROL_WORDS))
        .collect::<Result<Vec<_>, String>>()?;
    Ok(DirectCommandPlan { commands })
}

fn validate_direct_argv(
    argv: Vec<String>,
    control_words: &[&str],
) -> Result<DirectCommand, String> {
    if argv.len() > MAX_DIRECT_COMMAND_ARGS {
        return Err("executor direct command has too many arguments".to_string());
    }
    if control_words.contains(&argv[0].as_str()) {
        return Err(format!(
            "executor direct command rejects shell control builtin {}",
            argv[0]
        ));
    }
    Ok(DirectCommand::success(argv))
}

fn finish_direct_token(
    argv: &mut Vec<String>,
    token: &mut String,
    token_started: &mut bool,
) -> Result<(), String> {
    if !*token_started {
        return Ok(());
    }
    if token.len() > MAX_DIRECT_ARGUMENT_LENGTH {
        return Err("executor direct command argument is oversized".to_string());
    }
    argv.push(std::mem::take(token));
    *token_started = false;
    Ok(())
}
