use std::ffi::OsString;
use std::fs::{self, OpenOptions};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
#[cfg(unix)]
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::Duration;

use autospec_core::runtime_env::{
    ComposeNormalizer, EnvironmentLifecycle, ResourcePlan, RuntimeContext, RuntimeManifest,
    RuntimeState,
};

use crate::commands::CommandFailure;

mod compose;
mod gc;
mod isolation;
mod lifecycle;
mod maven;
mod options;
mod session;
mod state;
#[cfg(test)]
mod test_support;
mod worker;

#[cfg(test)]
use test_support::wait_for_cleanup_failure_test_hook;
#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use test_support::{
    install_cleanup_failure_test_hook, transition_environment_lifecycle_for_test,
    try_transition_environment_lifecycle_for_test,
};

use isolation::{
    bypass_without_planning, invocation_isolation, planning_identity, teardown_plan,
    whole_environment_disabled,
};
use lifecycle::{
    partial_state, provision_locked, teardown_locked, validate_authoritative,
    validate_cached_state, validate_teardown_lifecycle,
};
use maven::MavenAdapter;
use options::{parse_normalize_options, NormalizeMode, NormalizeOptions};
use session::{live_sessions, run_session_command, SessionLease};
use state::{
    claim_ports, layout_for_context, read_authoritative_state, read_runtime_state,
    validate_private_regular_file, write_runtime_state, EnvironmentLease, PortReservations,
    StateLayout,
};

#[cfg(unix)]
static RECEIVED_SIGNAL: AtomicI32 = AtomicI32::new(0);

#[cfg(unix)]
extern "C" fn record_signal(signal: i32) {
    RECEIVED_SIGNAL.store(signal, Ordering::Relaxed);
}

// SECURITY: fixed POSIX signal numbers and an atomic-only handler.
#[cfg(unix)]
extern "C" {
    fn signal(signal: i32, handler: extern "C" fn(i32)) -> usize;
}

#[derive(Debug, Clone)]
struct Options {
    repo: PathBuf,
    mode: String,
}

#[derive(Debug, Clone, Copy)]
enum ManifestKind {
    Agent,
    Autospec,
}

#[derive(Debug, Clone)]
struct InitOptions {
    repo: PathBuf,
    manifest: ManifestKind,
    force: bool,
}

#[derive(Debug, Clone)]
struct ExecOptions {
    options: Options,
    command: Vec<String>,
}

#[derive(Debug, Clone)]
struct SessionOptions {
    options: Options,
    command: Vec<String>,
    keep_alive: bool,
}

pub(crate) struct PreparedRuntimeSession {
    inner: Option<PreparedRuntimeSessionInner>,
}

struct PreparedRuntimeSessionInner {
    context: RuntimeContext,
    state: RuntimeState,
    plan: ResourcePlan,
    session_lease: SessionLease,
    bypassed: bool,
}

impl PreparedRuntimeSession {
    pub(crate) fn session_id(&self) -> &str {
        self.inner
            .as_ref()
            .expect("prepared runtime session is available before consumption")
            .session_lease
            .session_id()
    }

    pub(crate) fn environment_dir(&self) -> &Path {
        &self
            .inner
            .as_ref()
            .expect("prepared runtime session is available before consumption")
            .context
            .environment_dir
    }

    pub(crate) fn verify_active_session(
        &self,
        expected_session_id: &str,
    ) -> Result<(), CommandFailure> {
        self.inner
            .as_ref()
            .ok_or_else(|| {
                CommandFailure::diagnostic("prepared runtime session is already closed")
            })?
            .session_lease
            .verify_active(expected_session_id)
    }

    pub(crate) fn observed_environment(&self) -> Result<Vec<(OsString, OsString)>, CommandFailure> {
        let inner = self.inner.as_ref().ok_or_else(|| {
            CommandFailure::diagnostic("prepared runtime session is already closed")
        })?;
        inner
            .state
            .validate_child_environment(&inner.context)
            .map_err(|error| CommandFailure::diagnostic(error.to_string()))?;
        let mut values = inner
            .state
            .values()
            .map(|(key, value)| (OsString::from(key), OsString::from(value)))
            .collect::<Vec<_>>();
        if inner.bypassed {
            values.push((
                OsString::from("AUTOSPEC_ISOLATION_BYPASSED"),
                OsString::from("1"),
            ));
        }
        Ok(values)
    }

    pub(crate) fn run(mut self, command: &[String]) -> Result<(), CommandFailure> {
        let inner = self
            .inner
            .take()
            .expect("prepared runtime session can only run once");
        run_session_command(
            command,
            &inner.context,
            &inner.state,
            &inner.plan,
            inner.session_lease,
            false,
            inner.bypassed,
        )
    }

    pub(crate) fn abort(mut self) -> Result<(), CommandFailure> {
        let inner = self
            .inner
            .take()
            .expect("prepared runtime session can only be closed once");
        cleanup_prepared_session_verified(inner)
    }

    pub(crate) fn close_verified(self) -> Result<(), CommandFailure> {
        self.abort()
    }

    /// Release this process's lease without tearing down the durable
    /// environment so an exact persisted executor binding can reattach.
    pub(crate) fn relinquish(mut self) {
        let _ = self.inner.take();
    }
}

impl Drop for PreparedRuntimeSession {
    fn drop(&mut self) {
        let Some(inner) = self.inner.take() else {
            return;
        };
        let session_id = inner.session_lease.session_id().to_string();
        if let Err(error) = cleanup_prepared_session_verified(inner) {
            eprintln!(
                "RUNTIME_PREPARED_SESSION_CLEANUP_FAILED session_id={session_id} exit_code={} message={}",
                error.exit_code, error.message
            );
        }
    }
}

fn cleanup_prepared_session_verified(
    inner: PreparedRuntimeSessionInner,
) -> Result<(), CommandFailure> {
    let environment_dir = inner.context.environment_dir.clone();
    let session_id = inner.session_lease.session_id().to_string();
    let cleanup = cleanup_prepared_session(inner);
    let release = session::verify_session_released(&environment_dir, &session_id);
    let resources = session::verify_environment_released(&environment_dir);
    match (cleanup, release, resources) {
        (Ok(()), Ok(()), resources) => resources,
        (Err(primary), Ok(()), Ok(())) => Err(primary),
        (primary, release, resources) => {
            let mut error = primary.err();
            if let Err(release) = release {
                error = Some(session::add_secondary_failure(
                    error,
                    "runtime session release verification also failed",
                    release,
                ));
            }
            if let Err(resources) = resources {
                error = Some(session::add_secondary_failure(
                    error,
                    "runtime resource release verification also failed",
                    resources,
                ));
            }
            Err(error.expect("at least one runtime cleanup verification failed"))
        }
    }
}

fn cleanup_prepared_session(inner: PreparedRuntimeSessionInner) -> Result<(), CommandFailure> {
    session::cleanup_session(
        &inner.context,
        &inner.state,
        &inner.plan,
        inner.session_lease,
        true,
    )
}

#[cfg(not(test))]
fn wait_for_cleanup_failure_test_hook() {}

pub(crate) fn prepare_runtime_session(
    repo: &Path,
    mode: &str,
    harness: &str,
) -> Result<PreparedRuntimeSession, CommandFailure> {
    let repo = canonical_repo(repo)?;
    if selected_manifest(&repo).is_none() {
        return Err(CommandFailure::diagnostic(
            "runtime session preparation requires a runtime manifest",
        ));
    }
    if whole_environment_disabled()? {
        return Err(CommandFailure::diagnostic(
            "runtime session preparation refuses disabled isolation",
        ));
    }
    let invocation = invocation_isolation(&repo, mode)?;
    let plan = invocation
        .plan
        .expect("enabled runtime invocation has a plan");
    let context = context_from_plan(&repo, mode, &plan)?;
    let environment_lease = EnvironmentLease::acquire(&context.environment_dir)?;
    persist_runtime_manifest_snapshot(&context, invocation.bypassed, &plan.digest)?;
    let state = provision_locked(&context, &plan, invocation.bypassed)?;
    session::reconcile_for_exclusive_session(&context.environment_dir)?;
    reset_session_signal_handlers();
    let session_lease = SessionLease::register(&context.environment_dir, harness)?;
    drop(environment_lease);
    Ok(PreparedRuntimeSession {
        inner: Some(PreparedRuntimeSessionInner {
            context,
            state,
            plan,
            session_lease,
            bypassed: invocation.bypassed,
        }),
    })
}

pub(crate) fn reattach_runtime_session(
    repo: &Path,
    mode: &str,
    expected_environment_dir: &Path,
    session_id: &str,
    harness: &str,
) -> Result<PreparedRuntimeSession, CommandFailure> {
    let repo = canonical_repo(repo)?;
    let environment_id = expected_environment_dir
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| CommandFailure::diagnostic("RUNTIME_OWNER_MISMATCH: invalid environment"))?;
    let valid_environment_id = environment_id
        .rsplit_once('-')
        .is_some_and(|(name, digest)| {
            !name.is_empty()
                && digest.len() == 16
                && digest
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        });
    if !valid_environment_id {
        return Err(CommandFailure::diagnostic(
            "RUNTIME_OWNER_MISMATCH: invalid environment identity",
        ));
    }
    let root = expected_environment_dir
        .parent()
        .ok_or_else(|| CommandFailure::diagnostic("RUNTIME_OWNER_MISMATCH: invalid state root"))?;
    let layout = StateLayout::new(root, environment_id);
    if layout.environment_dir != expected_environment_dir {
        return Err(CommandFailure::diagnostic(
            "RUNTIME_OWNER_MISMATCH: reattached environment changed",
        ));
    }
    let environment_lease = EnvironmentLease::acquire(expected_environment_dir)?;
    let authoritative = read_authoritative_state(&layout)?.ok_or_else(|| {
        CommandFailure::diagnostic("RUNTIME_PARTIAL_STATE: reattached environment is absent")
    })?;
    if authoritative.plan.identity.canonical_repo != repo
        || (mode != "auto" && authoritative.plan.identity.mode != mode)
        || authoritative.plan.identity.environment_id != environment_id
    {
        return Err(CommandFailure::diagnostic(
            "RUNTIME_OWNER_MISMATCH: reattached runtime identity changed",
        ));
    }
    validate_authoritative(&authoritative, &authoritative.plan)?;
    let snapshot = read_runtime_manifest_snapshot(expected_environment_dir, &authoritative.plan)?;
    if snapshot.plan_digest != authoritative.plan.digest {
        return Err(CommandFailure::diagnostic(
            "RUNTIME_PLAN_MISMATCH: manifest snapshot does not match persisted plan",
        ));
    }
    let manifest = RuntimeManifest::parse_at(&snapshot.source, snapshot.path)
        .map_err(|error| CommandFailure::diagnostic(error.to_string()))?;
    let context = RuntimeContext::new_with_identity(
        manifest,
        &repo,
        mode,
        root,
        &authoritative.plan.identity,
    )
    .map_err(|error| CommandFailure::diagnostic(error.to_string()))?;
    if context.environment_dir != expected_environment_dir {
        return Err(CommandFailure::diagnostic(
            "RUNTIME_OWNER_MISMATCH: reattached environment changed",
        ));
    }
    if authoritative.owner.lifecycle != EnvironmentLifecycle::Active {
        return Err(CommandFailure::diagnostic(
            "RUNTIME_LIFECYCLE_MISMATCH: reattached environment is not active",
        ));
    }
    let cached = read_runtime_state(&context)?;
    let state = validate_cached_state(
        &context,
        &authoritative.plan,
        &authoritative.inventory,
        &cached,
    )?;
    reset_session_signal_handlers();
    let session_lease = SessionLease::reattach(expected_environment_dir, session_id, harness)?;
    drop(environment_lease);
    Ok(PreparedRuntimeSession {
        inner: Some(PreparedRuntimeSessionInner {
            context,
            state,
            plan: authoritative.plan,
            session_lease,
            bypassed: snapshot.bypassed,
        }),
    })
}

pub(crate) fn verify_runtime_session_released(
    environment_dir: &Path,
    session_id: &str,
) -> Result<(), CommandFailure> {
    session::verify_session_released(environment_dir, session_id)?;
    session::verify_environment_released(environment_dir)
}

pub(crate) fn retry_runtime_session_cleanup(
    repo: &Path,
    mode: &str,
    environment_dir: &Path,
    session_id: &str,
) -> Result<(), CommandFailure> {
    if verify_runtime_session_released(environment_dir, session_id).is_ok() {
        return Ok(());
    }
    let repo = canonical_repo(repo)?;
    let environment_id = environment_dir
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| CommandFailure::diagnostic("RUNTIME_OWNER_MISMATCH: invalid environment"))?;
    let root = environment_dir
        .parent()
        .ok_or_else(|| CommandFailure::diagnostic("RUNTIME_OWNER_MISMATCH: invalid state root"))?;
    let layout = StateLayout::new(root, environment_id);
    if layout.environment_dir != environment_dir {
        return Err(CommandFailure::diagnostic(
            "RUNTIME_OWNER_MISMATCH: cleanup environment path changed",
        ));
    }
    let environment_lease = EnvironmentLease::acquire(environment_dir)?;
    let authoritative = read_authoritative_state(&layout)?
        .ok_or_else(|| CommandFailure::diagnostic("RUNTIME_PARTIAL_STATE: cleanup state absent"))?;
    if authoritative.plan.identity.canonical_repo != repo
        || (mode != "auto" && authoritative.plan.identity.mode != mode)
        || authoritative.plan.identity.environment_id != environment_id
    {
        return Err(CommandFailure::diagnostic(
            "RUNTIME_OWNER_MISMATCH: cleanup identity changed",
        ));
    }
    validate_authoritative(&authoritative, &authoritative.plan)?;
    let snapshot = read_runtime_manifest_snapshot(environment_dir, &authoritative.plan)?;
    if snapshot.plan_digest != authoritative.plan.digest {
        return Err(CommandFailure::diagnostic(
            "RUNTIME_PLAN_MISMATCH: manifest snapshot does not match persisted plan",
        ));
    }
    let manifest = RuntimeManifest::parse_at(&snapshot.source, snapshot.path)
        .map_err(|error| CommandFailure::diagnostic(error.to_string()))?;
    let context = RuntimeContext::new_with_identity(
        manifest,
        &repo,
        mode,
        root,
        &authoritative.plan.identity,
    )
    .map_err(|error| CommandFailure::diagnostic(error.to_string()))?;
    if context.environment_dir != environment_dir {
        return Err(CommandFailure::diagnostic(
            "RUNTIME_OWNER_MISMATCH: cleanup recovery environment changed",
        ));
    }
    session::reconcile_for_exclusive_session(environment_dir)?;
    session::verify_session_released(environment_dir, session_id)?;
    validate_teardown_lifecycle(&authoritative.owner.lifecycle)?;
    let state = if context.env_file.is_file() {
        let cached = read_state(&context)?;
        Some(validate_cached_state(
            &context,
            &authoritative.plan,
            &authoritative.inventory,
            &cached,
        )?)
    } else {
        None
    };
    teardown_locked(&context, state.as_ref(), &authoritative.plan)?;
    drop(environment_lease);
    verify_runtime_session_released(environment_dir, session_id)
}

fn runtime_manifest_snapshot_path(environment_dir: &Path) -> PathBuf {
    environment_dir.join("manifest.snapshot.json")
}

fn persist_runtime_manifest_snapshot(
    context: &RuntimeContext,
    bypassed: bool,
    plan_digest: &str,
) -> Result<(), CommandFailure> {
    let path = runtime_manifest_snapshot_path(&context.environment_dir);
    let source = fs::read_to_string(context.manifest.path()).map_err(|error| {
        CommandFailure::diagnostic(format!("read runtime manifest snapshot source: {error}"))
    })?;
    let body = serde_json::json!({
        "schema": 2,
        "path": context.manifest.path(),
        "source": source,
        "plan_digest": plan_digest,
        "bypassed": bypassed,
    })
    .to_string();
    match fs::read_to_string(&path) {
        Ok(existing) if existing.trim() == body => return Ok(()),
        Ok(_) => {
            return Err(CommandFailure::diagnostic(
                "RUNTIME_OWNER_MISMATCH: persisted manifest snapshot changed",
            ))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(CommandFailure::diagnostic(format!(
                "read runtime manifest snapshot: {error}"
            )))
        }
    }
    if read_authoritative_state(&layout_for_context(context))?.is_some() {
        return Err(CommandFailure::diagnostic(
            "RUNTIME_PARTIAL_STATE: active environment has no original manifest snapshot",
        ));
    }
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(&path).map_err(|error| {
        CommandFailure::diagnostic(format!("create runtime manifest snapshot: {error}"))
    })?;
    use std::io::Write as _;
    file.write_all(format!("{body}\n").as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|error| {
            CommandFailure::diagnostic(format!("write runtime manifest snapshot: {error}"))
        })?;
    Ok(())
}

struct RuntimeManifestSnapshot {
    path: PathBuf,
    source: String,
    plan_digest: String,
    bypassed: bool,
}

fn read_runtime_manifest_snapshot(
    environment_dir: &Path,
    authoritative_plan: &ResourcePlan,
) -> Result<RuntimeManifestSnapshot, CommandFailure> {
    let path = runtime_manifest_snapshot_path(environment_dir);
    validate_private_regular_file(&path)?;
    let value: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).map_err(|error| {
            CommandFailure::diagnostic(format!("read runtime manifest snapshot: {error}"))
        })?)
        .map_err(|error| {
            CommandFailure::diagnostic(format!("parse runtime manifest snapshot: {error}"))
        })?;
    let object = value.as_object().ok_or_else(|| {
        CommandFailure::diagnostic("RUNTIME_PARTIAL_STATE: manifest snapshot is not an object")
    })?;
    let schema = object
        .get("schema")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| {
            CommandFailure::diagnostic("RUNTIME_PARTIAL_STATE: manifest snapshot schema is invalid")
        })?;
    if !matches!((schema, object.len()), (1, 3) | (2, 5)) {
        return Err(CommandFailure::diagnostic(
            "RUNTIME_PARTIAL_STATE: manifest snapshot schema is invalid",
        ));
    }
    let manifest_path = object
        .get("path")
        .and_then(serde_json::Value::as_str)
        .map(PathBuf::from)
        .ok_or_else(|| {
            CommandFailure::diagnostic("RUNTIME_PARTIAL_STATE: manifest snapshot path is invalid")
        })?;
    let source = object
        .get("source")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            CommandFailure::diagnostic("RUNTIME_PARTIAL_STATE: manifest snapshot source is invalid")
        })?;
    let (plan_digest, bypassed) = if schema == 1 {
        (authoritative_plan.digest.clone(), true)
    } else {
        let plan_digest = object
            .get("plan_digest")
            .and_then(serde_json::Value::as_str)
            .filter(|digest| {
                digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
            .ok_or_else(|| {
                CommandFailure::diagnostic(
                    "RUNTIME_PARTIAL_STATE: manifest snapshot plan digest is invalid",
                )
            })?;
        let bypassed = object
            .get("bypassed")
            .and_then(serde_json::Value::as_bool)
            .ok_or_else(|| {
                CommandFailure::diagnostic(
                    "RUNTIME_PARTIAL_STATE: manifest snapshot bypass evidence is invalid",
                )
            })?;
        (plan_digest.to_string(), bypassed)
    };
    Ok(RuntimeManifestSnapshot {
        path: manifest_path,
        source: source.to_string(),
        plan_digest,
        bypassed,
    })
}

#[derive(Debug, Clone)]
struct DownOptions {
    options: Options,
    purge_maven: bool,
}

pub(super) fn run(args: &[String]) -> Result<(), CommandFailure> {
    let Some((operation, options)) = args.split_first() else {
        return Err(CommandFailure::diagnostic(
            "autospec runtime env requires a subcommand",
        ));
    };
    match operation.as_str() {
        "init" => init(parse_init_options(options)?),
        "up" => up(parse_options(options)?),
        "status" => status(parse_options(options)?),
        "down" => down(parse_down_options(options)?),
        "gc" => run_gc(parse_options(options)?),
        "exec" => exec(parse_exec_options(options)?),
        "session" => session(parse_session_options(options)?),
        "normalize-compose" => normalize_compose(parse_normalize_options(options)?),
        "lease-probe" => lease_probe(options),
        "-h" | "--help" | "help" => {
            print_help();
            Ok(())
        }
        _ => Err(CommandFailure::diagnostic(format!(
            "unknown autospec runtime env command: {operation}"
        ))),
    }
}

fn normalize_compose(options: NormalizeOptions) -> Result<(), CommandFailure> {
    let repo = canonical_repo(&options.repo)?;
    let plan = ComposeNormalizer::plan(&repo)
        .map_err(|error| CommandFailure::diagnostic(error.to_string()))?;
    match options.mode {
        NormalizeMode::Check => {}
        NormalizeMode::Apply => ComposeNormalizer::apply(
            &plan,
            options
                .fingerprint
                .as_deref()
                .expect("apply options require a fingerprint"),
        )
        .map_err(|error| CommandFailure::diagnostic(error.to_string()))?,
    }
    println!(
        "{}",
        plan.to_json()
            .map_err(|error| CommandFailure::diagnostic(error.to_string()))?
    );
    Ok(())
}

fn parse_options(args: &[String]) -> Result<Options, CommandFailure> {
    let mut repo = PathBuf::from(".");
    let mut mode = "auto".to_string();
    let mut index = 0;
    while let Some(argument) = args.get(index) {
        match argument.as_str() {
            "--repo" => {
                let value = args.get(index + 1).ok_or_else(|| {
                    CommandFailure::diagnostic("autospec runtime env --repo requires a path")
                })?;
                if value.is_empty() || value.starts_with("--") {
                    return Err(CommandFailure::diagnostic(
                        "autospec runtime env --repo requires a path",
                    ));
                }
                repo = PathBuf::from(value);
                index += 2;
            }
            "--mode" => {
                let value = args.get(index + 1).ok_or_else(|| {
                    CommandFailure::diagnostic("autospec runtime env --mode requires a value")
                })?;
                if value.is_empty() {
                    return Err(CommandFailure::diagnostic(
                        "autospec runtime env --mode requires a value",
                    ));
                }
                if value.starts_with("--") {
                    return Err(CommandFailure::diagnostic(
                        "autospec runtime env --mode requires a value",
                    ));
                }
                mode = value.clone();
                index += 2;
            }
            _ if argument.starts_with("--repo=") => {
                let value = argument.trim_start_matches("--repo=");
                if value.is_empty() {
                    return Err(CommandFailure::diagnostic(
                        "autospec runtime env --repo requires a path",
                    ));
                }
                repo = PathBuf::from(value);
                index += 1;
            }
            _ if argument.starts_with("--mode=") => {
                let value = argument.trim_start_matches("--mode=");
                if value.is_empty() {
                    return Err(CommandFailure::diagnostic(
                        "autospec runtime env --mode requires a value",
                    ));
                }
                mode = value.to_string();
                index += 1;
            }
            _ => {
                return Err(CommandFailure::diagnostic(format!(
                    "unknown autospec runtime env option: {argument}"
                )));
            }
        }
    }
    Ok(Options { repo, mode })
}

fn parse_down_options(args: &[String]) -> Result<DownOptions, CommandFailure> {
    let mut purge_maven = false;
    let options = args
        .iter()
        .filter(|argument| {
            if argument.as_str() == "--purge-maven" {
                purge_maven = true;
                false
            } else {
                true
            }
        })
        .cloned()
        .collect::<Vec<_>>();
    Ok(DownOptions {
        options: parse_options(&options)?,
        purge_maven,
    })
}

fn parse_init_options(args: &[String]) -> Result<InitOptions, CommandFailure> {
    let mut repo = PathBuf::from(".");
    let mut manifest = ManifestKind::Agent;
    let mut force = false;
    let mut index = 0;
    while let Some(argument) = args.get(index) {
        match argument.as_str() {
            "--repo" => {
                let value = option_value(args, index, "--repo", "path")?;
                repo = PathBuf::from(value);
                index += 2;
            }
            "--manifest" => {
                manifest = parse_manifest_kind(option_value(args, index, "--manifest", "value")?)?;
                index += 2;
            }
            "--force" => {
                force = true;
                index += 1;
            }
            _ if argument.starts_with("--repo=") => {
                repo = PathBuf::from(equals_option_value(argument, "--repo", "path")?);
                index += 1;
            }
            _ if argument.starts_with("--manifest=") => {
                manifest =
                    parse_manifest_kind(equals_option_value(argument, "--manifest", "value")?)?;
                index += 1;
            }
            _ => {
                return Err(CommandFailure::diagnostic(format!(
                    "unknown autospec runtime env init option: {argument}"
                )));
            }
        }
    }
    Ok(InitOptions {
        repo,
        manifest,
        force,
    })
}

fn parse_exec_options(args: &[String]) -> Result<ExecOptions, CommandFailure> {
    let mut options = Options {
        repo: PathBuf::from("."),
        mode: "auto".to_string(),
    };
    let mut index = 0;
    while let Some(argument) = args.get(index) {
        match argument.as_str() {
            "--" => {
                index += 1;
                break;
            }
            "--repo" => {
                options.repo = PathBuf::from(option_value(args, index, "--repo", "path")?);
                index += 2;
            }
            "--mode" => {
                options.mode = option_value(args, index, "--mode", "value")?.to_string();
                index += 2;
            }
            _ if argument.starts_with("--repo=") => {
                options.repo = PathBuf::from(equals_option_value(argument, "--repo", "path")?);
                index += 1;
            }
            _ if argument.starts_with("--mode=") => {
                options.mode = equals_option_value(argument, "--mode", "value")?.to_string();
                index += 1;
            }
            _ if argument.starts_with('-') => {
                return Err(CommandFailure::diagnostic(format!(
                    "unknown autospec runtime env exec option: {argument}"
                )));
            }
            _ => break,
        }
    }
    let command = args[index..].to_vec();
    if command.is_empty() {
        return Err(CommandFailure::diagnostic(
            "autospec runtime env exec requires a command after --",
        ));
    }
    Ok(ExecOptions { options, command })
}

fn parse_session_options(args: &[String]) -> Result<SessionOptions, CommandFailure> {
    let mut options = Options {
        repo: PathBuf::from("."),
        mode: "auto".to_string(),
    };
    let mut keep_alive = false;
    let mut index = 0;
    while let Some(argument) = args.get(index) {
        match argument.as_str() {
            "--" => {
                index += 1;
                break;
            }
            "--repo" => {
                options.repo = PathBuf::from(option_value(args, index, "--repo", "path")?);
                index += 2;
            }
            "--mode" => {
                options.mode = option_value(args, index, "--mode", "value")?.to_string();
                index += 2;
            }
            "--keep-alive" => {
                keep_alive = true;
                index += 1;
            }
            _ if argument.starts_with("--repo=") => {
                options.repo = PathBuf::from(equals_option_value(argument, "--repo", "path")?);
                index += 1;
            }
            _ if argument.starts_with("--mode=") => {
                options.mode = equals_option_value(argument, "--mode", "value")?.to_string();
                index += 1;
            }
            _ if argument.starts_with('-') => {
                return Err(CommandFailure::diagnostic(format!(
                    "unknown autospec runtime env session option: {argument}"
                )));
            }
            _ => break,
        }
    }
    let command = args[index..].to_vec();
    if command.is_empty() {
        return Err(CommandFailure::diagnostic(
            "autospec runtime env session requires a command after --",
        ));
    }
    Ok(SessionOptions {
        options,
        command,
        keep_alive,
    })
}

fn option_value<'a>(
    args: &'a [String],
    index: usize,
    option: &str,
    noun: &str,
) -> Result<&'a str, CommandFailure> {
    let value = args.get(index + 1).map(String::as_str).ok_or_else(|| {
        CommandFailure::diagnostic(format!("autospec runtime env {option} requires a {noun}"))
    })?;
    if value.is_empty() || value.starts_with("--") {
        return Err(CommandFailure::diagnostic(format!(
            "autospec runtime env {option} requires a {noun}"
        )));
    }
    Ok(value)
}

fn equals_option_value<'a>(
    argument: &'a str,
    option: &str,
    noun: &str,
) -> Result<&'a str, CommandFailure> {
    let value = argument
        .strip_prefix(&format!("{option}="))
        .unwrap_or_default();
    if value.is_empty() {
        return Err(CommandFailure::diagnostic(format!(
            "autospec runtime env {option} requires a {noun}"
        )));
    }
    Ok(value)
}

fn parse_manifest_kind(value: &str) -> Result<ManifestKind, CommandFailure> {
    match value {
        "agent" => Ok(ManifestKind::Agent),
        "autospec" => Ok(ManifestKind::Autospec),
        _ => Err(CommandFailure::diagnostic(format!(
            "agent-env: unknown manifest kind: {value}"
        ))),
    }
}

fn init(options: InitOptions) -> Result<(), CommandFailure> {
    let repo = canonical_repo(&options.repo)?;
    let target = init_manifest(&repo, options.manifest, options.force)?;
    println!("agent-env: created {}", target.display());
    Ok(())
}

fn up(options: Options) -> Result<(), CommandFailure> {
    let repo = canonical_repo(&options.repo)?;
    let invocation = invocation_isolation(&repo, &options.mode)?;
    compose::ComposeAdapter::reject_caller_project_name(
        invocation
            .plan
            .as_ref()
            .and_then(|plan| plan.compose.as_ref()),
    )?;
    gc::collect(&state_root()?, None)?;
    if invocation.whole_environment_disabled {
        println!("AUTOSPEC_ISOLATION_BYPASSED=1");
        return Ok(());
    }
    let plan = invocation
        .plan
        .expect("enabled runtime invocation has a plan");
    let context = context_from_plan(&repo, &options.mode, &plan)?;
    let state = provision(&context, &plan, invocation.bypassed)?;
    print_protocol(&context, &state, invocation.bypassed);
    Ok(())
}

fn status(options: Options) -> Result<(), CommandFailure> {
    let repo = canonical_repo(&options.repo)?;
    gc::collect(&state_root()?, None)?;
    let identity = planning_identity(&repo, &options.mode)?;
    let context = context_from_identity(&repo, &options.mode, &identity)?;
    if !context.environment_dir.exists() {
        return inactive(&context);
    }
    let _lease = EnvironmentLease::acquire(&context.environment_dir)?;
    let layout = layout_for_context(&context);
    let authoritative = match read_authoritative_state(&layout)? {
        Some(state) => state,
        None if !context.env_file.is_file() => return inactive(&context),
        None => return Err(partial_state(&layout)),
    };
    let invocation = invocation_isolation(&repo, &options.mode)?;
    let plan = required_plan(invocation.plan)?;
    validate_authoritative(&authoritative, &plan)?;
    if authoritative.owner.lifecycle != EnvironmentLifecycle::Active {
        return Err(CommandFailure::diagnostic(format!(
            "RUNTIME_LIFECYCLE_MISMATCH: environment is {:?}",
            authoritative.owner.lifecycle
        )));
    }
    if !context.env_file.is_file() {
        return Err(partial_state(&layout));
    }
    let cached = read_state(&context)?;
    let state = validate_cached_state(&context, &plan, &authoritative.inventory, &cached)?;
    print_protocol(
        &context,
        &state,
        bypass_without_planning(whole_environment_disabled()?)?,
    );
    Ok(())
}

fn run_gc(options: Options) -> Result<(), CommandFailure> {
    let repo = absolute_repo_candidate(&options.repo)?;
    let removed = gc::collect(&state_root()?, Some(&repo))?;
    println!("AUTOSPEC_RUNTIME_GC_REMOVED={removed}");
    Ok(())
}

fn absolute_repo_candidate(repo: &Path) -> Result<PathBuf, CommandFailure> {
    if repo.is_absolute() {
        return Ok(repo.to_path_buf());
    }
    std::env::current_dir()
        .map(|cwd| cwd.join(repo))
        .map_err(|error| CommandFailure::diagnostic(error.to_string()))
}

fn down(options: DownOptions) -> Result<(), CommandFailure> {
    let repo = canonical_repo(&options.options.repo)?;
    let identity = planning_identity(&repo, &options.options.mode)?;
    let context = context_from_identity(&repo, &options.options.mode, &identity)?;
    if !context.environment_dir.exists() {
        return Ok(());
    }
    let _lease = EnvironmentLease::acquire(&context.environment_dir)?;
    let layout = layout_for_context(&context);
    let authoritative = match read_authoritative_state(&layout)? {
        Some(state) => state,
        None if !context.env_file.is_file() => return Ok(()),
        None => return Err(partial_state(&layout)),
    };
    let plan = teardown_plan(&repo, &options.options.mode)?;
    validate_authoritative(&authoritative, &plan)?;
    validate_teardown_lifecycle(&authoritative.owner.lifecycle)?;
    let live = live_sessions(&context.environment_dir)?;
    if !live.is_empty() {
        return Err(CommandFailure::diagnostic(format!(
            "RUNTIME_LIVE_SESSIONS: {} live runtime session(s) prevent teardown",
            live.len()
        )));
    }
    let state = if context.env_file.is_file() {
        let cached = read_state(&context)?;
        Some(validate_cached_state(
            &context,
            &plan,
            &authoritative.inventory,
            &cached,
        )?)
    } else {
        None
    };
    if options.purge_maven {
        MavenAdapter::purge_owned_prefix(
            plan.maven.as_ref(),
            &context,
            state.as_ref().ok_or_else(|| partial_state(&layout))?,
            &layout,
            &authoritative.inventory,
        )?;
    }
    teardown_locked(&context, state.as_ref(), &plan)
}

fn exec(options: ExecOptions) -> Result<(), CommandFailure> {
    let repo = canonical_repo(&options.options.repo)?;
    let invocation = invocation_isolation(&repo, &options.options.mode)?;
    if invocation.whole_environment_disabled {
        return run_direct_command(&options.command, &repo, None, true);
    }
    let plan = invocation
        .plan
        .expect("enabled runtime invocation has a plan");
    let context = context_from_plan(&repo, &options.options.mode, &plan)?;
    let state = provision(&context, &plan, invocation.bypassed)?;
    run_direct_command(
        &options.command,
        &context.repo,
        Some((&context, &state)),
        invocation.bypassed,
    )
}

fn session(options: SessionOptions) -> Result<(), CommandFailure> {
    let repo = canonical_repo(&options.options.repo)?;
    let whole_environment_disabled = whole_environment_disabled()?;
    if whole_environment_disabled {
        bypass_without_planning(true)?;
        return run_direct_command(&options.command, &repo, None, true);
    }
    if selected_manifest(&repo).is_none() {
        if environment_flag("AUTOSPEC_ENV_AUTO_INIT") {
            init_manifest(&repo, ManifestKind::Agent, false)?;
        } else {
            let bypassed = bypass_without_planning(false)?;
            return run_direct_command(&options.command, &repo, None, bypassed);
        }
    }
    if !worker::consume_session_worker_handoff() {
        return worker::supervise_session_worker(
            &options.command,
            &repo,
            &options.options.mode,
            options.keep_alive,
        );
    }
    let invocation = invocation_isolation(&repo, &options.options.mode)?;
    let plan = invocation
        .plan
        .expect("enabled runtime invocation has a plan");
    let context = context_from_plan(&repo, &options.options.mode, &plan)?;
    let lease = EnvironmentLease::acquire(&context.environment_dir)?;
    let state = provision_locked(&context, &plan, invocation.bypassed)?;
    let harness = options
        .command
        .first()
        .expect("session command is not empty");
    reset_session_signal_handlers();
    let session_lease = SessionLease::register(&context.environment_dir, harness)?;
    drop(lease);
    let keep_alive = options.keep_alive || environment_flag("AUTOSPEC_ENV_KEEP_ALIVE");
    run_session_command(
        &options.command,
        &context,
        &state,
        &plan,
        session_lease,
        keep_alive,
        invocation.bypassed,
    )
}

fn context_from_plan(
    repo: &Path,
    mode: &str,
    plan: &ResourcePlan,
) -> Result<RuntimeContext, CommandFailure> {
    context_from_identity(repo, mode, &plan.identity)
}

fn context_from_identity(
    repo: &Path,
    mode: &str,
    identity: &autospec_core::runtime_env::EnvironmentIdentity,
) -> Result<RuntimeContext, CommandFailure> {
    let manifest = RuntimeManifest::read_from_repo(repo)
        .map_err(|error| CommandFailure::diagnostic(error.to_string()))?;
    let root = state_root()?;
    let layout = StateLayout::new(&root, &identity.environment_id);
    let context = RuntimeContext::new_with_identity(manifest, repo, mode, &root, identity)
        .map_err(|error| CommandFailure::diagnostic(error.to_string()))?;
    debug_assert_eq!(context.environment_dir, layout.environment_dir);
    Ok(context)
}

fn canonical_repo(repo: &Path) -> Result<PathBuf, CommandFailure> {
    fs::canonicalize(repo).map_err(|error| {
        CommandFailure::diagnostic(format!("repo does not exist: {} ({error})", repo.display()))
    })
}

fn init_manifest(repo: &Path, kind: ManifestKind, force: bool) -> Result<PathBuf, CommandFailure> {
    if let Some(existing) = selected_manifest(repo) {
        if !force {
            return Err(CommandFailure::status(
                format!(
                    "agent-env: runtime manifest already exists: {}",
                    existing.display()
                ),
                4,
            ));
        }
    }
    let target = match kind {
        ManifestKind::Agent => repo.join(".agent-runtime.yml"),
        ManifestKind::Autospec => repo.join(".autospec/runtime.yml"),
    };
    let name = repo
        .file_name()
        .and_then(|name| name.to_str())
        .map(slugify)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "agent_env".to_string());
    let parent = target.parent().expect("runtime manifest target has parent");
    fs::create_dir_all(parent).map_err(|error| {
        CommandFailure::diagnostic(format!(
            "could not create runtime manifest directory {}: {error}",
            parent.display()
        ))
    })?;
    fs::write(
        &target,
        format!(
            "version: 1\nname: {name}\ndefault_mode: local\nmodes:\n  local:\n    command: sh -c 'true'\n    down: sh -c 'true'\nports:\n  frontend:\n    env: AGENT_FRONTEND_PORT\n    default: dynamic\npublic_url_env:\n  - AUTOSPEC_PUBLIC_URL\n  - AGENT_PUBLIC_URL\n"
        ),
    )
    .map_err(|error| {
        CommandFailure::diagnostic(format!(
            "could not write runtime manifest {}: {error}",
            target.display()
        ))
    })?;
    Ok(target)
}

fn selected_manifest(repo: &Path) -> Option<PathBuf> {
    let autospec = repo.join(".autospec/runtime.yml");
    if autospec.is_file() {
        return Some(autospec);
    }
    let agent = repo.join(".agent-runtime.yml");
    agent.is_file().then_some(agent)
}

fn slugify(value: &str) -> String {
    let mut slug = String::new();
    let mut previous_was_underscore = false;
    for character in value.chars() {
        let next = if character.is_ascii_uppercase() {
            character.to_ascii_lowercase()
        } else if character.is_ascii_lowercase()
            || character.is_ascii_digit()
            || character == '-'
            || character == '_'
        {
            character
        } else {
            '_'
        };
        if next == '_' && previous_was_underscore {
            continue;
        }
        slug.push(next);
        previous_was_underscore = next == '_';
    }
    slug.trim_matches('_').to_string()
}

fn state_root() -> Result<PathBuf, CommandFailure> {
    #[cfg(test)]
    if let Some(root) = TEST_STATE_ROOT.with(|root| root.borrow().clone()) {
        return Ok(root);
    }
    if let Some(root) = std::env::var_os("AGENT_ENV_STATE_ROOT").filter(|root| !root.is_empty()) {
        return Ok(PathBuf::from(root));
    }
    let home = std::env::var_os("HOME").ok_or_else(|| {
        CommandFailure::diagnostic("HOME is required for runtime environment state")
    })?;
    Ok(PathBuf::from(home).join(".autospec/envs"))
}

#[cfg(test)]
thread_local! {
    static TEST_STATE_ROOT: std::cell::RefCell<Option<PathBuf>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn set_test_state_root(root: Option<PathBuf>) {
    TEST_STATE_ROOT.with(|current| *current.borrow_mut() = root);
}

fn state_from_context(
    context: &RuntimeContext,
) -> Result<(RuntimeState, PortReservations), CommandFailure> {
    let frontend_override = caller_override("AGENT_FRONTEND_PORT");
    let backend_override = caller_override("AGENT_BACKEND_PORT");
    let reservations = claim_ports(
        context,
        frontend_override.as_deref(),
        backend_override.as_deref(),
    )?;
    let (frontend, backend) = reservations.ports();
    let mut state = RuntimeState::from_context(context, frontend, backend);

    let public_url = caller_override("AGENT_PUBLIC_URL").unwrap_or_else(|| {
        format!(
            "http://127.0.0.1:{}",
            state
                .value("AGENT_FRONTEND_PORT")
                .expect("new runtime state includes the frontend port")
        )
    });
    replace_state_value(&mut state, "AGENT_PUBLIC_URL", public_url.clone())?;
    replace_state_value(
        &mut state,
        "AUTOSPEC_PUBLIC_URL",
        caller_override("AUTOSPEC_PUBLIC_URL").unwrap_or(public_url),
    )?;
    if let Some(value) = caller_override("COMPOSE_PROJECT_NAME") {
        replace_state_value(&mut state, "COMPOSE_PROJECT_NAME", value)?;
    }
    Ok((state, reservations))
}

fn caller_override(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|value| !value.is_empty())
}

fn replace_state_value(
    state: &mut RuntimeState,
    key: &str,
    value: String,
) -> Result<(), CommandFailure> {
    state
        .replace_existing_value(key, value)
        .map_err(|error| CommandFailure::diagnostic(error.to_string()))
}

fn provision(
    context: &RuntimeContext,
    plan: &ResourcePlan,
    bypassed: bool,
) -> Result<RuntimeState, CommandFailure> {
    let _lease = EnvironmentLease::acquire(&context.environment_dir)?;
    provision_locked(context, plan, bypassed)
}

fn inactive(context: &RuntimeContext) -> Result<(), CommandFailure> {
    Err(CommandFailure::status(
        format!(
            "agent-env: no active environment for {} mode {}",
            context.repo.display(),
            context.mode.name()
        ),
        3,
    ))
}

fn required_plan(plan: Option<ResourcePlan>) -> Result<ResourcePlan, CommandFailure> {
    plan.ok_or_else(|| {
        CommandFailure::diagnostic(
            "RUNTIME_PLAN_MISMATCH: disabled runtime cannot authenticate persisted state",
        )
    })
}

pub(super) fn missing_mode_command(context: &RuntimeContext) -> CommandFailure {
    CommandFailure::status(
        format!(
            "agent-env: mode '{}' has no command in {}",
            context.mode.name(),
            context.manifest.path().display()
        ),
        1,
    )
}

pub(super) fn write_state(
    context: &RuntimeContext,
    state: &RuntimeState,
) -> Result<(), CommandFailure> {
    write_runtime_state(context, state)
}

pub(super) fn read_state(context: &RuntimeContext) -> Result<RuntimeState, CommandFailure> {
    read_runtime_state(context)
}

fn lease_probe(args: &[String]) -> Result<(), CommandFailure> {
    let [environment_dir, ready_path, release_path] = args else {
        return Err(CommandFailure::diagnostic(
            "autospec runtime env lease-probe requires ENVIRONMENT READY RELEASE",
        ));
    };
    let _lease = EnvironmentLease::acquire(Path::new(environment_dir))?;
    fs::write(ready_path, "ready\n").map_err(|error| {
        CommandFailure::diagnostic(format!("could not mark runtime lease probe ready: {error}"))
    })?;
    while !Path::new(release_path).exists() {
        std::thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}

fn run_direct_command(
    command: &[String],
    repo: &Path,
    runtime: Option<(&RuntimeContext, &RuntimeState)>,
    bypassed: bool,
) -> Result<(), CommandFailure> {
    let mut child = worker::spawn_direct_command(command, repo, runtime, bypassed, false)?;
    let status = child.wait().map_err(|error| {
        CommandFailure::diagnostic(format!("could not wait for runtime child command: {error}"))
    })?;
    child_status(status)
}

fn environment_flag(key: &str) -> bool {
    matches!(std::env::var(key).as_deref(), Ok("1"))
}

pub(super) fn run_mode_command(
    command: Option<&str>,
    context: &RuntimeContext,
    state: Option<&RuntimeState>,
    bypassed: bool,
) -> Result<(), CommandFailure> {
    let Some(command) = command else {
        return Ok(());
    };
    let mut process = Command::new("sh");
    process.args(["-c", command]).current_dir(&context.repo);
    worker::scrub_external_environment(&mut process);
    if let Some(state) = state {
        worker::configure_runtime_environment(&mut process, context, state, bypassed)?;
    }
    let status = process.status().map_err(|error| {
        CommandFailure::diagnostic(format!("could not run runtime manifest command: {error}"))
    })?;
    child_status(status)
}

fn child_status(status: std::process::ExitStatus) -> Result<(), CommandFailure> {
    if status.success() {
        Ok(())
    } else {
        #[cfg(unix)]
        let code = {
            use std::os::unix::process::ExitStatusExt;
            status
                .code()
                .or_else(|| status.signal().map(|signal| 128 + signal))
                .unwrap_or(1)
        };
        #[cfg(not(unix))]
        let code = status.code().unwrap_or(1);
        Err(CommandFailure::status(String::new(), code))
    }
}

pub(super) fn reset_session_signal_handlers() {
    #[cfg(unix)]
    install_signal_handlers();
}

pub(super) fn received_session_signal() -> i32 {
    #[cfg(unix)]
    return RECEIVED_SIGNAL.load(Ordering::Relaxed);
    #[cfg(not(unix))]
    0
}

#[cfg(unix)]
fn install_signal_handlers() {
    RECEIVED_SIGNAL.store(0, Ordering::Relaxed);
    // SAFETY: SIGINT/SIGTERM are fixed and the handler only writes an atomic.
    unsafe {
        signal(2, record_signal);
        signal(15, record_signal);
    }
}

fn print_protocol(context: &RuntimeContext, state: &RuntimeState, bypassed: bool) {
    print_state_value(state, "AGENT_ENV_ID");
    print_state_value(state, "AGENT_ENV_MODE");
    print_state_value(state, "AGENT_ENV_REPO");
    println!("AGENT_ENV_FILE={}", context.env_file.display());
    print_state_value(state, "AGENT_FRONTEND_PORT");
    print_state_value(state, "AGENT_BACKEND_PORT");
    print_state_value(state, "AGENT_PUBLIC_URL");
    print_state_value(state, "AUTOSPEC_PUBLIC_URL");
    print_state_value(state, "COMPOSE_PROJECT_NAME");
    for (key, _) in context.mode.env() {
        print_state_value(state, key);
    }
    if bypassed {
        println!("AUTOSPEC_ISOLATION_BYPASSED=1");
    }
}

fn print_state_value(state: &RuntimeState, key: &str) {
    if let Some(value) = state.value(key) {
        println!("{key}={value}");
    }
}

fn print_help() {
    println!(
        "autospec runtime env\n\nUSAGE:\n    autospec runtime env init [--repo PATH] [--manifest agent|autospec] [--force]\n    autospec runtime env up [--repo PATH] [--mode MODE]\n    autospec runtime env status [--repo PATH] [--mode MODE]\n    autospec runtime env down [--repo PATH] [--mode MODE] [--purge-maven]\n    autospec runtime env gc [--repo PATH]\n    autospec runtime env exec [--repo PATH] [--mode MODE] -- COMMAND [ARGS...]\n    autospec runtime env session [--repo PATH] [--mode MODE] [--keep-alive] -- COMMAND [ARGS...]\n    autospec runtime env normalize-compose --repo PATH --check|--apply --fingerprint SHA256"
    );
}

#[cfg(test)]
mod runtime_session_tests {
    use super::{
        prepare_runtime_session, reattach_runtime_session, runtime_manifest_snapshot_path,
    };
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    static ENVIRONMENT: Mutex<()> = Mutex::new(());

    struct RuntimeFixture {
        root: PathBuf,
        repo: PathBuf,
        state_root: PathBuf,
    }

    impl RuntimeFixture {
        fn new(name: &str, failing_down: bool) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock")
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "autospec-observed-runtime-{name}-{}-{nanos}",
                std::process::id()
            ));
            let repo = root.join("repo");
            let state_root = root.join("state");
            fs::create_dir_all(repo.join(".autospec")).expect("runtime fixture directory");
            git(&repo, &["init", "-b", "main"]);
            git(&repo, &["config", "user.name", "Autospec Test"]);
            git(&repo, &["config", "user.email", "autospec@example.invalid"]);
            fs::write(
                repo.join("runtime-up.py"),
                "import http.server, os\npid=os.fork()\nif pid == 0:\n http.server.HTTPServer(('127.0.0.1', int(os.environ['AGENT_FRONTEND_PORT'])), http.server.SimpleHTTPRequestHandler).serve_forever()\nelse:\n open('runtime.pid','w').write(str(pid))\n",
            )
            .expect("runtime up fixture");
            let down = if failing_down {
                "import os, signal, sys\npid=int(open('runtime.pid').read())\nos.kill(pid, signal.SIGTERM)\nos.remove('runtime.pid')\nsys.exit (42)\n"
            } else {
                "import os, signal\npid=int(open('runtime.pid').read())\nos.kill(pid, signal.SIGTERM)\nos.remove('runtime.pid')\n"
            };
            fs::write(repo.join("runtime-down.py"), down).expect("runtime down fixture");
            fs::write(
                repo.join(".autospec/runtime.yml"),
                "version: 1\ndefault_mode: local\nmodes:\n  local:\n    command: python3 runtime-up.py\n    down: python3 runtime-down.py\n",
            )
            .expect("runtime manifest");
            fs::write(repo.join("tracked.txt"), "fixture\n").expect("tracked fixture");
            git(&repo, &["add", "."]);
            git(&repo, &["commit", "-m", "runtime fixture"]);
            super::set_test_state_root(Some(state_root.clone()));
            Self {
                root,
                repo,
                state_root,
            }
        }

        fn session_record_exists(&self, session_id: &str) -> bool {
            contains_named_file(&self.state_root, &format!("{session_id}.json"))
        }
    }

    impl Drop for RuntimeFixture {
        fn drop(&mut self) {
            super::set_test_state_root(None);
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn git(repo: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .output()
            .expect("git starts");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn contains_named_file(root: &Path, name: &str) -> bool {
        let Ok(entries) = fs::read_dir(root) else {
            return false;
        };
        entries.filter_map(Result::ok).any(|entry| {
            let path = entry.path();
            path.file_name().and_then(|value| value.to_str()) == Some(name)
                || (path.is_dir() && contains_named_file(&path, name))
        })
    }

    #[test]
    fn prepared_runtime_cleanup_failure_is_reported_after_session_release() {
        let _environment = ENVIRONMENT.lock().expect("runtime test environment");
        let fixture = RuntimeFixture::new("cleanup-failure", true);
        let session =
            prepare_runtime_session(&fixture.repo, "auto", "observed-test").expect("session");
        let session_id = session.session_id().to_string();

        let error = session
            .close_verified()
            .expect_err("failing broker down must fail close");
        assert_eq!(error.exit_code, 42);
        assert!(!fixture.session_record_exists(&session_id));
    }

    #[test]
    fn active_runtime_reattach_uses_original_snapshot_after_manifest_changes_or_disappears() {
        let _environment = ENVIRONMENT.lock().expect("runtime test environment");
        for mutation in ["changed", "removed"] {
            let fixture = RuntimeFixture::new(&format!("reattach-{mutation}"), false);
            let session =
                prepare_runtime_session(&fixture.repo, "auto", "observed-test").expect("session");
            let session_id = session.session_id().to_string();
            let environment_dir = session.environment_dir().to_path_buf();
            session.relinquish();
            let manifest = fixture.repo.join(".autospec/runtime.yml");
            if mutation == "changed" {
                fs::write(
                    &manifest,
                    "version: 1\ndefault_mode: local\nmodes:\n  local:\n    command: /usr/bin/false\n    down: /usr/bin/false\n",
                )
                .expect("replace live manifest");
            } else {
                fs::remove_file(&manifest).expect("remove live manifest");
            }

            let reattached = reattach_runtime_session(
                &fixture.repo,
                "auto",
                &environment_dir,
                &session_id,
                "observed-test",
            )
            .unwrap_or_else(|error| panic!("{mutation}: {}", error.message));
            reattached
                .verify_active_session(&session_id)
                .expect("exact reattached session remains active");
            reattached.abort().expect("close reattached runtime");
        }
    }

    #[test]
    fn active_runtime_reattach_migrates_private_schema_one_snapshot_conservatively() {
        let _environment = ENVIRONMENT.lock().expect("runtime test environment");
        let fixture = RuntimeFixture::new("reattach-schema-one", false);
        let session =
            prepare_runtime_session(&fixture.repo, "auto", "observed-test").expect("session");
        let session_id = session.session_id().to_string();
        let environment_dir = session.environment_dir().to_path_buf();
        session.relinquish();
        let snapshot_path = runtime_manifest_snapshot_path(&environment_dir);
        let current: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(&snapshot_path).expect("read schema-two snapshot"),
        )
        .expect("parse schema-two snapshot");
        let legacy = serde_json::json!({
            "schema": 1,
            "path": current["path"],
            "source": current["source"],
        });
        fs::write(&snapshot_path, format!("{legacy}\n")).expect("seed private schema-one snapshot");
        fs::remove_file(fixture.repo.join(".autospec/runtime.yml"))
            .expect("remove current manifest after legacy snapshot");

        let reattached = reattach_runtime_session(
            &fixture.repo,
            "auto",
            &environment_dir,
            &session_id,
            "observed-test",
        )
        .unwrap_or_else(|error| panic!("schema-one upgrade: {}", error.message));
        assert!(
            reattached
                .inner
                .as_ref()
                .expect("reattached runtime")
                .bypassed,
            "legacy snapshots must conservatively retain bypass evidence"
        );
        reattached
            .verify_active_session(&session_id)
            .expect("schema-one reattached session remains active");
        reattached.abort().expect("close reattached runtime");
    }
}
