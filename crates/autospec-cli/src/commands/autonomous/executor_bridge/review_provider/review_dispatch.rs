use super::super::*;

pub(in crate::commands::autonomous::executor_bridge) fn bound_independent_reviewer_prompt(
    request: &ExecutorBridgeRequest,
    state: &PersistedInvocation,
    policy: &ResolvedReviewPolicy,
    evidence: &BoundReviewEvidence,
) -> Result<String, String> {
    let head = state
        .head_oid
        .as_deref()
        .ok_or_else(|| "executor independent review requires a stable head".to_string())?;
    if evidence.commit != head
        || evidence.requirements_digest
            != canonical_review_requirements_digest(&policy.requirements)
    {
        return Err("executor independent review context identity mismatch".to_string());
    }
    let context = serde_json::json!({
        "commit": evidence.commit,
        "policy_digest": canonical_review_policy_digest(policy),
        "requirements_digest": evidence.requirements_digest,
        "risk": format!("{:?}", policy.requirements.risk).to_ascii_lowercase(),
        "reviewer_reasoning": format!("{:?}", policy.requirements.reviewer_reasoning).to_ascii_lowercase(),
        "reviewer_harness": policy.reviewer_harness.as_str(),
        "provider_diversified": policy.provider_diversified,
        "selection_reason": policy.selection_reason,
        "review_reasons": policy.requirements.reasons,
        "changed_paths": evidence.inventory.changed_paths,
        "logical_components": evidence.inventory.logical_components,
        "producer_surfaces": evidence.inventory.producer_surfaces,
        "consumer_surfaces": evidence.inventory.consumer_surfaces,
        "integration_evidence_digest": evidence.integration_evidence_digest,
        "integration_command_records": evidence.integration_command_records,
        "required_integration_citations": evidence.integration_citations(),
    });
    Ok(format!(
        "Independently review commit {head} in the current worktree against GitHub issue #{}: {}.\n\
         Acceptance contract:\n{}\n\
         Commit-bound review context (inspect every cited immutable record before approval):\n{}\n\
         Inspect the base-to-HEAD diff, tests, security boundaries, and issue scope without \
         mutating local files, git state, GitHub state, or any external system. Return exactly one \
         JSON object with only these fields: schema (1), commit (exactly {head}), verdict, \
         surfaces_examined (nonempty string array), tests_examined (nonempty string array), \
         integration_paths_checked (string array), and blocking_findings. Use verdict lgtm with \
         an empty findings array only when no blocker remains; otherwise use verdict blocked with \
         concrete findings. integration_paths_checked must equal required_integration_citations \
         exactly, including order; do not invent or omit citations. Do not wrap the JSON in \
         Markdown or include any other text.",
        request.issue,
        request.issue_title,
        request.issue_body,
        serde_json::to_string_pretty(&context)
            .map_err(|error| format!("serialize independent review context: {error}"))?,
    ))
}

#[cfg(unix)]
pub(in crate::commands::autonomous::executor_bridge) fn prepare_bound_reviewer_normalizer(
    kind: HarnessKind,
    invocation: &ValidatedInvocation,
    artifact_root: &Path,
    expected_commit: &str,
    expected_integration_citations: &[String],
) -> Result<AutomaticReviewerArtifacts, String> {
    use std::os::unix::fs::PermissionsExt;

    let inner_stdout = artifact_root.join("harness.stdout");
    let inner_stderr = artifact_root.join("harness.stderr");
    prepare_private_reviewer_result(&inner_stdout)?;
    prepare_private_reviewer_result(&inner_stderr)?;
    let result = if kind == HarnessKind::Codex {
        let result = artifact_root.join("harness-result.txt");
        prepare_private_reviewer_result(&result)?;
        result
    } else {
        inner_stdout.clone()
    };
    let program = invocation
        .program
        .to_str()
        .ok_or_else(|| "automatic reviewer executable must be valid UTF-8".to_string())?;
    let env_utility = trusted_reviewer_utility("env")?;
    let wc_utility = trusted_reviewer_utility("wc")?;
    let truncate_utility = trusted_reviewer_utility("truncate")?;
    let python_utility = trusted_reviewer_utility("python3")?;
    let mut command = format!(
        "{} -i",
        posix_shell_quote(
            env_utility
                .to_str()
                .ok_or_else(|| "trusted env path must be valid UTF-8".to_string())?
        )
    );
    for (key, value) in &invocation.environment_overrides {
        let key = key
            .to_str()
            .ok_or_else(|| "automatic reviewer environment key must be UTF-8".to_string())?;
        let value = value
            .to_str()
            .ok_or_else(|| "automatic reviewer environment value must be UTF-8".to_string())?;
        command.push(' ');
        command.push_str(&posix_shell_quote(&format!("{key}={value}")));
    }
    command.push(' ');
    command.push_str(&posix_shell_quote(program));
    let result_argument = result
        .to_str()
        .ok_or_else(|| "reviewer result path must be valid UTF-8".to_string())?;
    let mut result_arguments = 0;
    for argument in &invocation.args {
        result_arguments += usize::from(kind == HarnessKind::Codex && argument == result_argument);
        command.push(' ');
        command.push_str(&posix_shell_quote(argument));
    }
    if kind == HarnessKind::Codex && result_arguments != 1 {
        return Err("automatic Codex reviewer result argument is missing or ambiguous".to_string());
    }
    let validator = r#"import json
import sys
class RejectDuplicates(dict):
    def __init__(self, pairs):
        if len(pairs) != len(dict(pairs)):
            raise ValueError("duplicate review verdict field")
        super().__init__(pairs)
with open(sys.argv[1], encoding="utf-8") as stream:
    data = json.load(stream, object_pairs_hook=RejectDuplicates)
allowed = ["schema", "commit", "verdict", "surfaces_examined", "tests_examined", "integration_paths_checked", "blocking_findings"]
if not isinstance(data, dict) or set(data) != set(allowed):
    raise ValueError("review verdict fields are invalid")
if type(data["schema"]) is not int or data["schema"] != 1:
    raise ValueError("review verdict schema is invalid")
if data["commit"] != sys.argv[2] or data["verdict"] != "lgtm":
    raise ValueError("review verdict identity is invalid")
def strings(name):
    value = data[name]
    if type(value) is not list or any(type(item) is not str or not item.strip() for item in value):
        raise ValueError(name + " is invalid")
    return value
surfaces = strings("surfaces_examined")
tests = strings("tests_examined")
integration = strings("integration_paths_checked")
findings = strings("blocking_findings")
expected_integration = json.loads(sys.argv[3])
if not surfaces or not tests or integration != expected_integration or findings:
    raise ValueError("review verdict evidence is insufficient")
print("LGTM")"#;
    let expected_integration_citations = serde_json::to_string(expected_integration_citations)
        .map_err(|error| format!("serialize expected integration citations: {error}"))?;
    let body = format!(
        "#!/bin/sh\n\
         set -u\n\
         umask 077\n\
         : > {result} || exit 65\n\
         if {command} >{stdout} 2>{stderr}; then\n\
         \tstatus=0\n\
         else\n\
         \tstatus=$?\n\
         fi\n\
         overflow=0\n\
         for artifact in {stdout} {stderr} {result}; do\n\
         \tsize=$({wc} -c < \"$artifact\") || exit 69\n\
         \tif [ \"$size\" -ge {output_bytes} ]; then\n\
         \t\toverflow=1\n\
         \t\t{truncate} -s {output_bytes} \"$artifact\" || exit 69\n\
         \tfi\n\
         done\n\
         [ \"$overflow\" -eq 0 ] || exit 70\n\
         [ \"$status\" -eq 0 ] || exit \"$status\"\n\
         {python} - {result} {expected_commit} {expected_integration_citations} <<'PY' || exit 67\n\
         {validator}\n\
         PY\n",
        output_bytes = MAX_DIRECT_OUTPUT_BYTES,
        wc = posix_shell_quote(
            wc_utility
                .to_str()
                .ok_or_else(|| "trusted wc path must be valid UTF-8".to_string())?
        ),
        python = posix_shell_quote(
            python_utility
                .to_str()
                .ok_or_else(|| "trusted python3 path must be valid UTF-8".to_string())?
        ),
        truncate = posix_shell_quote(
            truncate_utility
                .to_str()
                .ok_or_else(|| "trusted truncate path must be valid UTF-8".to_string())?
        ),
        stdout = posix_shell_quote(
            inner_stdout
                .to_str()
                .ok_or_else(|| "reviewer stdout path must be valid UTF-8".to_string())?
        ),
        stderr = posix_shell_quote(
            inner_stderr
                .to_str()
                .ok_or_else(|| "reviewer stderr path must be valid UTF-8".to_string())?
        ),
        result = posix_shell_quote(
            result
                .to_str()
                .ok_or_else(|| "reviewer result path must be valid UTF-8".to_string())?
        ),
        expected_commit = posix_shell_quote(expected_commit),
        expected_integration_citations = posix_shell_quote(&expected_integration_citations),
        validator = validator,
    );
    let legacy_normalizer = artifact_root.join("review-normalizer.sh");
    let normalizer = if legacy_normalizer.exists() {
        validate_private_state_file(&legacy_normalizer).map_err(|error| {
            format!("existing automatic reviewer normalizer is unsafe: {error}")
        })?;
        if fs::read(&legacy_normalizer)
            .map_err(|error| format!("read existing automatic reviewer normalizer: {error}"))?
            == body.as_bytes()
        {
            legacy_normalizer
        } else {
            artifact_root.join(format!(
                "review-normalizer-{}.sh",
                &sha256_hex(body.as_bytes())[..16]
            ))
        }
    } else {
        legacy_normalizer
    };
    write_private_create_once(
        &normalizer,
        body.as_bytes(),
        "automatic reviewer normalizer",
    )?;
    fs::set_permissions(&normalizer, fs::Permissions::from_mode(0o700))
        .map_err(|error| format!("make automatic reviewer normalizer executable: {error}"))?;
    validate_private_state_file(&normalizer)
        .map_err(|error| format!("automatic reviewer normalizer must be private: {error}"))?;
    Ok(AutomaticReviewerArtifacts {
        normalizer,
        inner_stdout,
        inner_stderr,
        result,
    })
}

#[cfg(not(unix))]
pub(in crate::commands::autonomous::executor_bridge) fn prepare_bound_reviewer_normalizer(
    _kind: HarnessKind,
    _invocation: &ValidatedInvocation,
    _artifact_root: &Path,
    _expected_commit: &str,
    _expected_integration_citations: &[String],
) -> Result<AutomaticReviewerArtifacts, String> {
    Err("automatic reviewer normalization requires a POSIX host".to_string())
}
