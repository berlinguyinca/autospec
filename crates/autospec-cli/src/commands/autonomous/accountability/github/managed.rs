use super::*;
use serde_json::Value;

pub(super) fn validate_issue(
    issue: &RemoteIssue,
    repository: &RepositoryIdentity,
    expected_run_id: Option<&str>,
) -> Result<(), AccountabilityError> {
    let expected_url = format!("https://github.com/{}/issues/", repository.as_str());
    if !issue.url.starts_with(&expected_url) {
        return Err(AccountabilityError::new(
            "epic belongs to a different repository",
        ));
    }
    if !REQUIRED_LABELS
        .iter()
        .all(|required| issue.labels.contains(*required))
    {
        return Err(AccountabilityError::new(
            "accountability epic is missing mandatory labels",
        ));
    }
    let (marker_repo, run_id) = parse_single_marker(&issue.body)?;
    if marker_repo != repository.as_str() {
        return Err(AccountabilityError::new("epic marker repository mismatch"));
    }
    if expected_run_id.is_some_and(|expected| expected != run_id) {
        return Err(AccountabilityError::new(
            "epic marker run identity mismatch",
        ));
    }
    Ok(())
}

pub(super) fn validate_remote_manifest(
    issue: &RemoteIssue,
    repository: &RepositoryIdentity,
    run_id: &str,
) -> Result<RecoveryManifest, AccountabilityError> {
    let manifest = verified_manifest(&issue.body, repository)?;
    if manifest.identity.run_id() != run_id
        || manifest.epic_number != issue.number
        || manifest.epic_url != issue.url
    {
        return Err(AccountabilityError::new(
            "bound epic recovery manifest does not match its marker and identity",
        ));
    }
    validate_resume_policy(issue, &manifest, ResumePolicy::ActiveOnly, true, None)?;
    Ok(manifest)
}

pub(super) fn verified_manifest(
    body: &str,
    repository: &RepositoryIdentity,
) -> Result<RecoveryManifest, AccountabilityError> {
    let (projection, document) = extract_managed_projection(body)?;
    let manifest = RecoveryManifest::parse_for_repository(&document, repository)?;
    let digest = sha256_hex(format!("{}\n", projection.trim_end()).as_bytes());
    if manifest.remote_digest != digest {
        return Err(AccountabilityError::new(
            "managed accountability projection digest mismatch",
        ));
    }
    Ok(manifest)
}

pub(super) fn validate_pending_remote_body(
    issue: &RemoteIssue,
    repository: &RepositoryIdentity,
    run_id: &str,
    expected_digest: &str,
) -> Result<(), AccountabilityError> {
    let managed_marker_count = [MANAGED_START, MANAGED_END, MANIFEST_START, MANIFEST_END]
        .iter()
        .map(|marker| issue.body.matches(marker).count())
        .sum::<usize>();
    if managed_marker_count > 0 {
        let manifest = verified_manifest(&issue.body, repository)?;
        if manifest.identity.run_id() != run_id
            || manifest.epic_number != issue.number
            || manifest.epic_url != issue.url
        {
            return Err(AccountabilityError::new(
                "pending epic recovery manifest does not match its bound identity",
            ));
        }
        return Ok(());
    }
    let marker = run_marker(repository, run_id);
    let projection = issue
        .body
        .strip_prefix(&marker)
        .map(|body| body.trim_start_matches('\n'))
        .ok_or_else(|| AccountabilityError::new("initial epic body marker is not first"))?;
    if sha256_hex(projection.as_bytes()) != expected_digest {
        return Err(AccountabilityError::new(
            "initial post-create projection digest mismatch",
        ));
    }
    Ok(())
}

pub(super) fn validate_resume_policy(
    issue: &RemoteIssue,
    manifest: &RecoveryManifest,
    policy: ResumePolicy,
    has_local_identity: bool,
    adopted_lease_generation: Option<u64>,
) -> Result<(), AccountabilityError> {
    let open = issue.state.eq_ignore_ascii_case("open");
    let active_owner_proven = has_local_identity
        || adopted_lease_generation == Some(manifest.identity.lease_generation());
    let allowed = open && manifest.recovery_state == RecoveryState::Active && active_owner_proven
        || !open
            && matches!(
                manifest.recovery_state,
                RecoveryState::Parked | RecoveryState::Terminal
            )
            && policy == ResumePolicy::ReopenClosed;
    if allowed {
        Ok(())
    } else {
        Err(AccountabilityError::new(
            "accountability epic open/closed state and recovery ownership policy disagree",
        ))
    }
}

pub(super) fn github_projection_error(
    context: impl AsRef<str>,
    error: GithubFailure,
) -> AccountabilityError {
    let disposition = if error.retryable() {
        ProjectionDisposition::DegradableTransport
    } else {
        ProjectionDisposition::IntegrityBlock
    };
    let retry_after = error.retry_after().map(|delay| delay.as_secs());
    AccountabilityError::projection(format!("{}: {error}", context.as_ref()), disposition)
        .with_retry_after(retry_after)
}

pub(super) fn parse_single_marker(body: &str) -> Result<(&str, &str), AccountabilityError> {
    let markers = body
        .lines()
        .filter(|line| line.trim_start().starts_with(MARKER_PREFIX))
        .collect::<Vec<_>>();
    if markers.len() != 1 {
        return Err(AccountabilityError::new(
            "accountability epic must contain exactly one immutable run marker",
        ));
    }
    let marker = markers[0].trim();
    let content = marker
        .strip_prefix(MARKER_PREFIX)
        .and_then(|value| value.strip_suffix(" -->"))
        .ok_or_else(|| AccountabilityError::new("accountability epic marker is malformed"))?;
    let (repo, run_id) = content
        .split_once(" run_id=")
        .ok_or_else(|| AccountabilityError::new("accountability epic marker is malformed"))?;
    if run_id.len() != 64 || !run_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(AccountabilityError::new(
            "accountability epic run ID is malformed",
        ));
    }
    Ok((repo, run_id))
}

pub fn compose_managed_body(
    marker: &str,
    projection: &str,
    manifest: &RecoveryManifest,
    existing_body: &str,
) -> String {
    let human = strip_managed_content(existing_body);
    let managed = format!(
        "{marker}\n{MANAGED_START}\n{projection}\n\n{MANIFEST_START}\n{}\n{MANIFEST_END}\n{MANAGED_END}",
        manifest.to_json()
    );
    if human.is_empty() {
        format!("{managed}\n")
    } else {
        format!("{managed}\n\n{human}\n")
    }
}

pub(super) fn strip_managed_content(body: &str) -> String {
    let mut kept = Vec::new();
    let mut managed = false;
    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed == MANAGED_START {
            managed = true;
            continue;
        }
        if trimmed == MANAGED_END {
            managed = false;
            continue;
        }
        if managed || trimmed.starts_with(MARKER_PREFIX) {
            continue;
        }
        kept.push(line);
    }
    kept.join("\n").trim().to_owned()
}

pub(super) fn extract_managed_projection(
    body: &str,
) -> Result<(String, String), AccountabilityError> {
    if body.matches(MANAGED_START).count() != 1
        || body.matches(MANAGED_END).count() != 1
        || body.matches(MANIFEST_START).count() != 1
        || body.matches(MANIFEST_END).count() != 1
    {
        return Err(AccountabilityError::new(
            "accountability epic must contain exactly one managed block and recovery manifest",
        ));
    }
    let managed_start = body.find(MANAGED_START).unwrap() + MANAGED_START.len();
    let managed_end = body.find(MANAGED_END).unwrap();
    if managed_start >= managed_end {
        return Err(AccountabilityError::new(
            "managed accountability block is malformed",
        ));
    }
    let managed = &body[managed_start..managed_end];
    let start = body
        .find(MANIFEST_START)
        .ok_or_else(|| AccountabilityError::new("managed recovery manifest is missing"))?
        + MANIFEST_START.len();
    let suffix = &body[start..];
    let end = suffix
        .find(MANIFEST_END)
        .ok_or_else(|| AccountabilityError::new("managed recovery manifest is unterminated"))?;
    let manifest = suffix[..end].trim();
    if manifest.is_empty() || manifest.contains(MANIFEST_START) {
        return Err(AccountabilityError::new(
            "managed recovery manifest is ambiguous",
        ));
    }
    let projection_end = managed.find(MANIFEST_START).ok_or_else(|| {
        AccountabilityError::new("managed recovery manifest is outside its managed block")
    })?;
    let projection = managed[..projection_end].trim();
    if projection.is_empty() {
        return Err(AccountabilityError::new(
            "managed accountability projection is missing",
        ));
    }
    Ok((projection.to_owned(), manifest.to_owned()))
}

pub(super) fn run_marker(repository: &RepositoryIdentity, run_id: &str) -> String {
    format!(
        "<!-- autospec:run-epic repo={} run_id={} -->",
        repository.as_str(),
        run_id
    )
}

pub(super) fn parse_issue_pages(output: &str) -> Result<Vec<RemoteIssue>, AccountabilityError> {
    let value: Value = serde_json::from_str(output).map_err(json_error)?;
    let pages = value
        .as_array()
        .ok_or_else(|| AccountabilityError::new("GitHub issue pages must be an array"))?;
    let values: Vec<&Value> = if pages.iter().all(Value::is_array) {
        pages
            .iter()
            .flat_map(|page| page.as_array().expect("checked array"))
            .collect()
    } else {
        pages.iter().collect()
    };
    values.into_iter().map(parse_issue).collect()
}

pub(super) fn parse_issue(value: &Value) -> Result<RemoteIssue, AccountabilityError> {
    let object = value
        .as_object()
        .ok_or_else(|| AccountabilityError::new("GitHub issue must be an object"))?;
    let number = object
        .get("number")
        .and_then(Value::as_u64)
        .ok_or_else(|| AccountabilityError::new("GitHub issue number is missing"))?;
    let url = object
        .get("html_url")
        .or_else(|| object.get("url"))
        .and_then(Value::as_str)
        .ok_or_else(|| AccountabilityError::new("GitHub issue URL is missing"))?
        .to_owned();
    let state = object
        .get("state")
        .and_then(Value::as_str)
        .ok_or_else(|| AccountabilityError::new("GitHub issue state is missing"))?
        .to_owned();
    let body = object
        .get("body")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let labels = object
        .get("labels")
        .and_then(Value::as_array)
        .ok_or_else(|| AccountabilityError::new("GitHub issue labels are missing"))?
        .iter()
        .filter_map(|label| {
            label
                .as_str()
                .or_else(|| label.get("name").and_then(Value::as_str))
        })
        .map(str::to_owned)
        .collect();
    Ok(RemoteIssue {
        number,
        url,
        state,
        body,
        labels,
    })
}
