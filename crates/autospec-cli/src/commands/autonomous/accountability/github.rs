use super::{
    AccountabilityError, AccountabilityStore, ProjectionDisposition, RecoveryManifest,
    RecoveryState, RenderedProjection, RepositoryIdentity,
};
use autospec_core::autonomous::waterfall::sha256_hex;
use serde_json::Value;
use std::collections::BTreeSet;
use std::fmt;
use std::io::Write;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

pub const ACCOUNTABILITY_LABEL: &str = "autospec:run-accountability";
pub const REQUIRED_LABELS: [&str; 4] = ["epic", "type:tracker", "no-auto", ACCOUNTABILITY_LABEL];
const MARKER_PREFIX: &str = "<!-- autospec:run-epic repo=";
const MANAGED_START: &str = "<!-- autospec:accountability:start -->";
const MANAGED_END: &str = "<!-- autospec:accountability:end -->";
const MANIFEST_START: &str = "<!-- autospec:recovery-manifest:start -->";
const MANIFEST_END: &str = "<!-- autospec:recovery-manifest:end -->";
const RECONCILE_ATTEMPTS: usize = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResumePolicy {
    ActiveOnly,
    ReopenClosed,
}

#[derive(Clone, Debug)]
pub struct EpicBindingRequest {
    pub repository: RepositoryIdentity,
    pub explicit_epic: Option<u64>,
    pub resume_policy: ResumePolicy,
    pub project_number: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EpicBinding {
    pub number: u64,
    pub url: String,
    pub run_id: String,
    pub project_warning: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GithubCommand {
    EnsureLabel {
        repository: String,
        name: String,
    },
    ListAccountabilityIssues {
        repository: String,
    },
    ViewIssue {
        repository: String,
        number: u64,
    },
    CreateIssue {
        repository: String,
        title: String,
        body: String,
        labels: Vec<String>,
    },
    EditIssue {
        repository: String,
        number: u64,
        body: String,
    },
    ReopenIssue {
        repository: String,
        number: u64,
    },
    CloseIssue {
        repository: String,
        number: u64,
    },
    AddToProject {
        repository: String,
        project_number: u64,
        issue_url: String,
    },
}

pub trait GithubTransport {
    fn execute(&mut self, command: GithubCommand) -> Result<String, GithubFailure>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GithubFailure {
    Retryable(String),
    RetryAfter { message: String, delay: Duration },
    Ambiguous(String),
    Definitive(String),
}

impl GithubFailure {
    fn retryable(&self) -> bool {
        matches!(
            self,
            Self::Retryable(_) | Self::RetryAfter { .. } | Self::Ambiguous(_)
        )
    }

    fn retry_after(&self) -> Option<Duration> {
        match self {
            Self::RetryAfter { delay, .. } => Some(*delay),
            _ => None,
        }
    }
}

impl fmt::Display for GithubFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Retryable(message) | Self::Ambiguous(message) | Self::Definitive(message) => {
                formatter.write_str(message)
            }
            Self::RetryAfter { message, .. } => formatter.write_str(message),
        }
    }
}

pub struct GhCli;

impl GithubTransport for GhCli {
    fn execute(&mut self, command: GithubCommand) -> Result<String, GithubFailure> {
        execute_gh(command)
    }
}

#[derive(Clone, Debug)]
struct RemoteIssue {
    number: u64,
    url: String,
    state: String,
    body: String,
    labels: BTreeSet<String>,
}

struct ReconcileError {
    error: AccountabilityError,
    retryable: bool,
    retry_after: Option<Duration>,
}

pub fn bind_epic<T, R>(
    store: &mut AccountabilityStore,
    github: &mut T,
    request: EpicBindingRequest,
    mut renew_lease: R,
) -> Result<EpicBinding, AccountabilityError>
where
    T: GithubTransport,
    R: FnMut() -> Result<(), String>,
{
    let result = if let Some(number) = request.explicit_epic {
        bind_explicit(store, github, request, number, &mut renew_lease)
    } else {
        bind_generated(store, github, request, &mut renew_lease)
    };
    result.map_err(|error| error.into_projection(ProjectionDisposition::IntegrityBlock))
}

fn bind_generated<T, R>(
    store: &mut AccountabilityStore,
    github: &mut T,
    request: EpicBindingRequest,
    renew_lease: &mut R,
) -> Result<EpicBinding, AccountabilityError>
where
    T: GithubTransport,
    R: FnMut() -> Result<(), String>,
{
    let identity = store
        .identity()
        .cloned()
        .ok_or_else(|| AccountabilityError::new("launch intent is required before epic binding"))?;
    if identity.repository() != &request.repository {
        return Err(AccountabilityError::new(
            "launch intent repository does not match epic repository",
        ));
    }
    let marker = run_marker(&request.repository, identity.run_id());
    let bound = store.status().epic_number;
    let mut matches = reconcile(github, &request.repository, &marker, renew_lease)
        .map_err(|failure| failure.error)?;
    if matches.len() > 1 {
        return Err(AccountabilityError::new(
            "multiple accountability epics carry the exact run marker",
        ));
    }
    if let Some(bound_number) = bound {
        let issue = matches.pop().ok_or_else(|| {
            AccountabilityError::new("bound accountability epic is missing its run marker")
        })?;
        if issue.number != bound_number {
            return Err(AccountabilityError::new(
                "run marker resolved to a different epic than local binding",
            ));
        }
        validate_issue(&issue, &request.repository, Some(identity.run_id()))?;
        if store.status().pending_projection_count > 0 {
            project_and_ack(
                store,
                github,
                &request.repository,
                issue.clone(),
                &marker,
                renew_lease,
            )?;
        } else {
            let manifest =
                validate_remote_manifest(&issue, &request.repository, identity.run_id())?;
            let status = store.status();
            if manifest.projection_revision != status.projection_revision
                || Some(manifest.remote_digest.as_str()) != store.desired_projection_digest()
                || manifest.high_watermark != status.acknowledged_high_watermark
            {
                return Err(AccountabilityError::new(
                    "bound epic revision, digest, or high-watermark disagrees with local state",
                ));
            }
        }
        return finish_binding(store, github, request, issue);
    }

    let issue = if let Some(issue) = matches.pop() {
        issue
    } else {
        if !store.create_attempted() {
            for label in REQUIRED_LABELS {
                renew(renew_lease)?;
                github
                    .execute(GithubCommand::EnsureLabel {
                        repository: request.repository.as_str().to_owned(),
                        name: label.to_owned(),
                    })
                    .map_err(|error| {
                        github_projection_error(
                            format!("cannot ensure accountability label {label}"),
                            error,
                        )
                    })?;
            }
            store.mark_create_attempted()?;
            renew(renew_lease)?;
            let projection = store.render()?;
            let body = format!("{marker}\n\n{}", projection.markdown);
            let create_result = github.execute(GithubCommand::CreateIssue {
                repository: request.repository.as_str().to_owned(),
                title: format!("Autonomous run {}", &identity.run_id()[..12]),
                body,
                labels: REQUIRED_LABELS
                    .iter()
                    .map(|label| (*label).to_owned())
                    .collect(),
            });
            if let Err(error @ GithubFailure::Definitive(_)) = create_result {
                return Err(github_projection_error(
                    "cannot create accountability epic",
                    error,
                ));
            }
        }
        let mut found = None;
        let mut last_error = None;
        for attempt in 0..RECONCILE_ATTEMPTS {
            let mut reconciled = match reconcile(github, &request.repository, &marker, renew_lease)
            {
                Ok(reconciled) => reconciled,
                Err(failure) => {
                    last_error = Some(failure.error.to_string());
                    if failure.retryable && attempt + 1 < RECONCILE_ATTEMPTS {
                        thread::sleep(failure.retry_after.unwrap_or(Duration::from_millis(25)));
                        continue;
                    }
                    return Err(failure.error);
                }
            };
            if reconciled.len() > 1 {
                return Err(AccountabilityError::new(
                    "multiple accountability epics carry the exact run marker",
                ));
            }
            if let Some(issue) = reconciled.pop() {
                found = Some(issue);
                break;
            }
            if attempt + 1 < RECONCILE_ATTEMPTS {
                thread::sleep(Duration::from_millis(25));
            }
        }
        found.ok_or_else(|| {
            AccountabilityError::projection(
                format!(
                "accountability epic creation visibility is unresolved; refusing duplicate create{}",
                last_error.map_or_else(String::new, |error| format!(": {error}"))
                ),
                ProjectionDisposition::DegradableTransport,
            )
        })?
    };
    validate_issue(&issue, &request.repository, Some(identity.run_id()))?;
    store.bind_epic(issue.number, &issue.url)?;
    project_and_ack(
        store,
        github,
        &request.repository,
        issue.clone(),
        &marker,
        renew_lease,
    )?;
    finish_binding(store, github, request, issue)
}

fn bind_explicit<T, R>(
    store: &mut AccountabilityStore,
    github: &mut T,
    request: EpicBindingRequest,
    number: u64,
    renew_lease: &mut R,
) -> Result<EpicBinding, AccountabilityError>
where
    T: GithubTransport,
    R: FnMut() -> Result<(), String>,
{
    if number == 0 {
        return Err(AccountabilityError::new("epic number must be positive"));
    }
    let mut issue = view_issue(github, &request.repository, number, renew_lease)?;
    validate_issue(&issue, &request.repository, None)?;
    let (marker_repo, marker_run_id) = parse_single_marker(&issue.body)?;
    let marker_repo = marker_repo.to_owned();
    let marker_run_id = marker_run_id.to_owned();
    if marker_repo != request.repository.as_str() {
        return Err(AccountabilityError::new("epic marker repository mismatch"));
    }
    let manifest = verified_manifest(&issue.body, &request.repository)?;
    if marker_run_id != manifest.identity.run_id()
        || manifest.epic_number != issue.number
        || manifest.epic_url != issue.url
    {
        return Err(AccountabilityError::new(
            "epic marker and managed recovery manifest do not agree",
        ));
    }
    validate_resume_policy(
        &issue,
        &manifest,
        request.resume_policy,
        store.identity().is_some(),
    )?;
    if issue.state.eq_ignore_ascii_case("closed") {
        if request.resume_policy != ResumePolicy::ReopenClosed {
            return Err(AccountabilityError::new(
                "closed accountability epic requires resume --epic",
            ));
        }
        renew(renew_lease)?;
        github
            .execute(GithubCommand::ReopenIssue {
                repository: request.repository.as_str().to_owned(),
                number,
            })
            .map_err(|error| github_projection_error("cannot reopen epic", error))?;
        issue = view_issue(github, &request.repository, number, renew_lease)?;
        validate_issue(&issue, &request.repository, Some(&marker_run_id))?;
        if issue.state.eq_ignore_ascii_case("closed") {
            return Err(AccountabilityError::new("resumed epic remained closed"));
        }
    }

    match store.identity() {
        None => {
            store.resume_from_manifest(
                manifest,
                "Resume the managed autonomous run",
                "The operator explicitly selected its verified accountability epic",
            )?;
        }
        Some(identity) if identity.run_id() == marker_run_id => {
            store.bind_epic(issue.number, &issue.url)?;
            store.ensure_resume_event()?;
        }
        Some(_) => {
            return Err(AccountabilityError::new(
                "local accountability state belongs to a different run",
            ))
        }
    }
    let marker = run_marker(&request.repository, &marker_run_id);
    project_and_ack(
        store,
        github,
        &request.repository,
        issue.clone(),
        &marker,
        renew_lease,
    )?;
    finish_binding(store, github, request, issue)
}

fn project_and_ack<T, R>(
    store: &mut AccountabilityStore,
    github: &mut T,
    repository: &RepositoryIdentity,
    issue: RemoteIssue,
    marker: &str,
    renew_lease: &mut R,
) -> Result<(), AccountabilityError>
where
    T: GithubTransport,
    R: FnMut() -> Result<(), String>,
{
    let projection = store.projection_for_delivery()?;
    let identity = store
        .identity()
        .cloned()
        .ok_or_else(|| AccountabilityError::new("launch intent disappeared before projection"))?;
    let status = store.status();
    let manifest = RecoveryManifest::new(
        identity,
        issue.number,
        &issue.url,
        projection.revision,
        &projection.digest,
        projection.desired_high_watermark,
        status.journal_segment.max(1),
    )?;
    let (recovery_state, linked_issues, linked_pull_requests) = store.recovery_projection();
    let manifest =
        manifest.with_recovery_state(recovery_state, linked_issues, linked_pull_requests)?;
    let body = compose_managed_body(marker, &projection.markdown, &manifest, &issue.body);
    renew(renew_lease)?;
    github
        .execute(GithubCommand::EditIssue {
            repository: repository.as_str().to_owned(),
            number: issue.number,
            body,
        })
        .map_err(|error| github_projection_error("cannot project epic", error))?;
    let verified = view_issue(github, repository, issue.number, renew_lease)?;
    validate_issue(
        &verified,
        repository,
        Some(store.identity().unwrap().run_id()),
    )?;
    let (returned_projection, returned_manifest) = extract_managed_projection(&verified.body)?;
    let returned_manifest = RecoveryManifest::parse_for_repository(&returned_manifest, repository)?;
    if returned_manifest.projection_revision != projection.revision
        || returned_manifest.remote_digest != projection.digest
        || returned_manifest.high_watermark != projection.desired_high_watermark
        || sha256_hex(format!("{}\n", returned_projection.trim_end()).as_bytes())
            != projection.digest
    {
        return Err(AccountabilityError::new(
            "GitHub returned a stale accountability projection",
        ));
    }
    if recovery_state != RecoveryState::Active {
        renew(renew_lease)?;
        github
            .execute(GithubCommand::CloseIssue {
                repository: repository.as_str().to_owned(),
                number: issue.number,
            })
            .map_err(|error| github_projection_error("cannot close projected epic", error))?;
    }
    store.ack_projection(
        projection.revision,
        &projection.digest,
        projection.desired_high_watermark,
    )
}

fn finish_binding<T: GithubTransport>(
    store: &AccountabilityStore,
    github: &mut T,
    request: EpicBindingRequest,
    issue: RemoteIssue,
) -> Result<EpicBinding, AccountabilityError> {
    let project_warning = request.project_number.and_then(|project_number| {
        github
            .execute(GithubCommand::AddToProject {
                repository: request.repository.as_str().to_owned(),
                project_number,
                issue_url: issue.url.clone(),
            })
            .err()
            .map(|error| error.to_string())
    });
    Ok(EpicBinding {
        number: issue.number,
        url: issue.url,
        run_id: store
            .identity()
            .expect("binding has launch identity")
            .run_id()
            .to_owned(),
        project_warning,
    })
}

fn reconcile<T, R>(
    github: &mut T,
    repository: &RepositoryIdentity,
    marker: &str,
    renew_lease: &mut R,
) -> Result<Vec<RemoteIssue>, ReconcileError>
where
    T: GithubTransport,
    R: FnMut() -> Result<(), String>,
{
    renew(renew_lease).map_err(|error| ReconcileError {
        error,
        retryable: false,
        retry_after: None,
    })?;
    let output = github
        .execute(GithubCommand::ListAccountabilityIssues {
            repository: repository.as_str().to_owned(),
        })
        .map_err(|error| {
            let retryable = error.retryable();
            let retry_after = error.retry_after();
            ReconcileError {
                retryable,
                retry_after,
                error: github_projection_error("cannot reconcile epics", error),
            }
        })?;
    Ok(parse_issue_pages(&output)
        .map_err(|error| ReconcileError {
            error,
            retryable: false,
            retry_after: None,
        })?
        .into_iter()
        .filter(|issue| issue.body.matches(marker).count() == 1)
        .collect())
}

fn view_issue<T, R>(
    github: &mut T,
    repository: &RepositoryIdentity,
    number: u64,
    renew_lease: &mut R,
) -> Result<RemoteIssue, AccountabilityError>
where
    T: GithubTransport,
    R: FnMut() -> Result<(), String>,
{
    renew(renew_lease)?;
    let output = github
        .execute(GithubCommand::ViewIssue {
            repository: repository.as_str().to_owned(),
            number,
        })
        .map_err(|error| github_projection_error("cannot view epic", error))?;
    parse_issue(&serde_json::from_str(&output).map_err(json_error)?)
}

fn renew<R: FnMut() -> Result<(), String>>(renew_lease: &mut R) -> Result<(), AccountabilityError> {
    renew_lease().map_err(|error| {
        AccountabilityError::new(format!("lifecycle lease lost during epic binding: {error}"))
    })
}

fn validate_issue(
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

fn validate_remote_manifest(
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
    validate_resume_policy(issue, &manifest, ResumePolicy::ActiveOnly, true)?;
    Ok(manifest)
}

fn verified_manifest(
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

fn validate_resume_policy(
    issue: &RemoteIssue,
    manifest: &RecoveryManifest,
    policy: ResumePolicy,
    has_local_identity: bool,
) -> Result<(), AccountabilityError> {
    let open = issue.state.eq_ignore_ascii_case("open");
    let allowed = open && manifest.recovery_state == RecoveryState::Active && has_local_identity
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

fn github_projection_error(context: impl AsRef<str>, error: GithubFailure) -> AccountabilityError {
    let disposition = if error.retryable() {
        ProjectionDisposition::DegradableTransport
    } else {
        ProjectionDisposition::IntegrityBlock
    };
    AccountabilityError::projection(format!("{}: {error}", context.as_ref()), disposition)
}

fn parse_single_marker(body: &str) -> Result<(&str, &str), AccountabilityError> {
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

fn strip_managed_content(body: &str) -> String {
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

fn extract_managed_projection(body: &str) -> Result<(String, String), AccountabilityError> {
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

fn run_marker(repository: &RepositoryIdentity, run_id: &str) -> String {
    format!(
        "<!-- autospec:run-epic repo={} run_id={} -->",
        repository.as_str(),
        run_id
    )
}

fn parse_issue_pages(output: &str) -> Result<Vec<RemoteIssue>, AccountabilityError> {
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

fn parse_issue(value: &Value) -> Result<RemoteIssue, AccountabilityError> {
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

fn execute_gh(command: GithubCommand) -> Result<String, GithubFailure> {
    let create_issue = matches!(&command, GithubCommand::CreateIssue { .. });
    let mutating = !matches!(
        command,
        GithubCommand::ListAccountabilityIssues { .. } | GithubCommand::ViewIssue { .. }
    );
    let (args, stdin) = match command {
        GithubCommand::EnsureLabel { repository, name } => (
            vec![
                "label".to_string(),
                "create".to_string(),
                name,
                "--repo".to_string(),
                repository,
                "--color".to_string(),
                "5319e7".to_string(),
                "--force".to_string(),
            ],
            None,
        ),
        GithubCommand::ListAccountabilityIssues { repository } => (
            vec![
                "api".to_string(),
                "--paginate".to_string(),
                "--slurp".to_string(),
                format!(
                    "repos/{repository}/issues?state=all&labels=autospec%3Arun-accountability&per_page=100"
                ),
            ],
            None,
        ),
        GithubCommand::ViewIssue { repository, number } => (
            vec![
                "issue".to_string(),
                "view".to_string(),
                number.to_string(),
                "--repo".to_string(),
                repository,
                "--json".to_string(),
                "number,url,state,body,labels".to_string(),
            ],
            None,
        ),
        GithubCommand::CreateIssue {
            repository,
            title,
            body,
            labels,
        } => {
            let mut args = vec![
                "issue".to_string(),
                "create".to_string(),
                "--repo".to_string(),
                repository,
                "--title".to_string(),
                title,
                "--body-file".to_string(),
                "-".to_string(),
            ];
            for label in labels {
                args.push("--label".to_string());
                args.push(label);
            }
            (args, Some(body))
        }
        GithubCommand::EditIssue {
            repository,
            number,
            body,
        } => (
            vec![
                "issue".to_string(),
                "edit".to_string(),
                number.to_string(),
                "--repo".to_string(),
                repository,
                "--body-file".to_string(),
                "-".to_string(),
            ],
            Some(body),
        ),
        GithubCommand::ReopenIssue { repository, number } => (
            vec![
                "issue".to_string(),
                "reopen".to_string(),
                number.to_string(),
                "--repo".to_string(),
                repository,
            ],
            None,
        ),
        GithubCommand::CloseIssue { repository, number } => (
            vec![
                "issue".to_string(),
                "close".to_string(),
                number.to_string(),
                "--repo".to_string(),
                repository,
            ],
            None,
        ),
        GithubCommand::AddToProject {
            repository,
            project_number,
            issue_url,
        } => {
            let owner = repository.split('/').next().unwrap_or_default().to_owned();
            (
                vec![
                    "project".to_string(),
                    "item-add".to_string(),
                    project_number.to_string(),
                    "--owner".to_string(),
                    owner,
                    "--url".to_string(),
                    issue_url,
                ],
                None,
            )
        }
    };
    let mut process = Command::new("gh");
    process
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if stdin.is_some() {
        process.stdin(Stdio::piped());
    }
    let mut child = process
        .spawn()
        .map_err(|error| GithubFailure::Retryable(format!("cannot execute gh: {error}")))?;
    if let Some(input) = stdin {
        child
            .stdin
            .take()
            .ok_or_else(|| GithubFailure::Ambiguous("cannot open gh stdin".to_string()))?
            .write_all(input.as_bytes())
            .map_err(|error| GithubFailure::Ambiguous(format!("cannot write gh stdin: {error}")))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|error| GithubFailure::Ambiguous(format!("cannot wait for gh: {error}")))?;
    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        if let Some(delay) = parse_retry_after(&message) {
            return Err(GithubFailure::RetryAfter { message, delay });
        }
        return Err(if mutating {
            if create_issue && !definitive_gh_failure(&message) {
                GithubFailure::Ambiguous(message)
            } else {
                GithubFailure::Definitive(message)
            }
        } else {
            GithubFailure::Retryable(message)
        });
    }
    String::from_utf8(output.stdout)
        .map_err(|error| GithubFailure::Definitive(format!("gh returned invalid UTF-8: {error}")))
}

fn parse_retry_after(message: &str) -> Option<Duration> {
    message.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case("retry-after")
            .then(|| value.trim().parse::<u64>().ok().map(Duration::from_secs))?
    })
}

fn definitive_gh_failure(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    [
        "validation failed",
        "authentication",
        "forbidden",
        "http 400",
        "http 401",
        "http 403",
        "http 404",
        "unprocessable entity",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn json_error(error: serde_json::Error) -> AccountabilityError {
    AccountabilityError::new(format!("invalid GitHub response: {error}"))
}

#[allow(dead_code)]
fn _projection_type_anchor(_: &RenderedProjection) {}
