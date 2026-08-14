use super::{
    AccountabilityError, AccountabilityStore, ProjectionDisposition, RecoveryManifest,
    RecoveryState, RenderedProjection, RepositoryIdentity,
};
use autospec_core::autonomous::waterfall::sha256_hex;
use std::collections::BTreeSet;
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
    pub adopted_lease_generation: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EpicBinding {
    pub number: u64,
    pub url: String,
    pub run_id: String,
    pub project_warning: Option<String>,
}

#[path = "github/managed.rs"]
mod managed;
#[path = "github/transport.rs"]
mod transport;
pub use managed::compose_managed_body;
use managed::*;
use transport::json_error;
#[allow(unused_imports)]
pub use transport::{GhCli, GithubCommand, GithubFailure, GithubTransport};
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
    renew_lease: R,
) -> Result<EpicBinding, AccountabilityError>
where
    T: GithubTransport,
    R: FnMut() -> Result<(), String>,
{
    bind_epic_at(
        store,
        github,
        request,
        AccountabilityStore::projection_clock_now()?,
        renew_lease,
    )
}

pub fn bind_epic_at<T, R>(
    store: &mut AccountabilityStore,
    github: &mut T,
    request: EpicBindingRequest,
    now: u64,
    mut renew_lease: R,
) -> Result<EpicBinding, AccountabilityError>
where
    T: GithubTransport,
    R: FnMut() -> Result<(), String>,
{
    if store.status().pending_projection_count > 0 && !store.projection_retry_due_at(now) {
        return local_binding(store);
    }
    let result = if let Some(number) = request.explicit_epic {
        bind_explicit(store, github, request, number, &mut renew_lease)
    } else {
        bind_generated(store, github, request, &mut renew_lease)
    };
    result.map_err(|error| {
        let error = error.into_projection(ProjectionDisposition::IntegrityBlock);
        if error.projection_disposition() == Some(ProjectionDisposition::DegradableTransport) {
            if let Err(schedule_error) =
                store.schedule_projection_retry_at(error.retry_after_seconds(), now)
            {
                return schedule_error.into_projection(ProjectionDisposition::IntegrityBlock);
            }
        }
        error
    })
}

fn local_binding(store: &AccountabilityStore) -> Result<EpicBinding, AccountabilityError> {
    let status = store.status();
    Ok(EpicBinding {
        number: status
            .epic_number
            .ok_or_else(|| AccountabilityError::new("deferred projection has no bound epic"))?,
        url: status
            .epic_url
            .ok_or_else(|| AccountabilityError::new("deferred projection has no epic URL"))?,
        run_id: status
            .run_id
            .ok_or_else(|| AccountabilityError::new("deferred projection has no run identity"))?,
        project_warning: None,
    })
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
            validate_pending_remote_body(
                &issue,
                &request.repository,
                identity.run_id(),
                store.desired_projection_digest().ok_or_else(|| {
                    AccountabilityError::new("pending projection is missing its durable digest")
                })?,
            )?;
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
        let mut created_issue = None;
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
            match create_result {
                Ok(output) => {
                    created_issue = Some(parse_issue(
                        &serde_json::from_str(&output).map_err(json_error)?,
                    )?);
                }
                Err(error @ GithubFailure::Definitive(_)) => {
                    return Err(github_projection_error(
                        "cannot create accountability epic",
                        error,
                    ));
                }
                Err(_) => {}
            }
        }
        if let Some(issue) = created_issue {
            issue
        } else {
            let mut found = None;
            let mut last_error = None;
            for attempt in 0..RECONCILE_ATTEMPTS {
                let mut reconciled =
                    match reconcile(github, &request.repository, &marker, renew_lease) {
                        Ok(reconciled) => reconciled,
                        Err(failure) => {
                            last_error = Some(failure.error.to_string());
                            if failure.retryable && attempt + 1 < RECONCILE_ATTEMPTS {
                                thread::sleep(
                                    failure.retry_after.unwrap_or(Duration::from_millis(25)),
                                );
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
        }
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
        request.adopted_lease_generation,
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
                manifest.clone(),
                "Resume the managed autonomous run",
                "The operator explicitly selected its verified accountability epic",
            )?;
        }
        Some(identity) if identity.run_id() == marker_run_id => {
            store.bind_epic(issue.number, &issue.url)?;
            store.resume_bound_from_manifest(manifest.clone())?;
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
