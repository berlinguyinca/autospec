use super::*;

#[cfg(unix)]
pub(super) fn open_private_heartbeat_directory(path: &Path) -> Result<fs::File, CommandFailure> {
    use nix::fcntl::{open, OFlag};
    use nix::sys::stat::Mode;

    prepare_heartbeat_root_parent_with_hook(&path.join(".publication-anchor"), |_| Ok(()))?;
    let directory = fs::File::from(
        open(
            path,
            OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
            Mode::empty(),
        )
        .map_err(|error| CommandFailure::diagnostic(format!("heartbeat dir open: {error}")))?,
    );
    private_heartbeat_directory_identity(&directory, "publication directory")?;
    Ok(directory)
}

#[cfg(unix)]
pub(super) fn inspect_heartbeat_target(
    directory: &impl std::os::fd::AsFd,
    name: &str,
    expected: &[u8],
) -> Result<Option<HeartbeatPublication>, CommandFailure> {
    use nix::fcntl::{openat, OFlag};
    use nix::sys::stat::{fstat, Mode, SFlag};

    let blocking = || CommandFailure::diagnostic("heartbeat publication target conflicts");
    let descriptor = match openat(
        directory,
        name,
        OFlag::O_RDWR | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC | OFlag::O_NONBLOCK,
        Mode::empty(),
    ) {
        Err(nix::errno::Errno::ENOENT) => return Ok(None),
        Ok(descriptor) => descriptor,
        Err(_) => return Err(blocking()),
    };
    let file = fs::File::from(descriptor);
    let stat = fstat(&file).map_err(|_| blocking())?;
    if SFlag::from_bits_truncate(stat.st_mode) & SFlag::S_IFMT != SFlag::S_IFREG
        || stat.st_uid != nix::unistd::geteuid().as_raw()
        || stat.st_nlink != 1
        || stat.st_mode & 0o7777 != 0o600
    {
        return Err(blocking());
    }
    let snapshot = file
        .try_clone()
        .and_then(read_regular_file)
        .map_err(|_| blocking())?;
    if !same_startup_heartbeat_generation(&snapshot.document, expected) {
        return Err(blocking());
    }
    let current = fstat(&file).map_err(|_| blocking())?;
    if current.st_mode & 0o7777 != 0o600 {
        return Err(blocking());
    }
    if heartbeat_final_binding(
        &file,
        directory,
        name,
        (stat.st_dev as u64, stat.st_ino as u64),
    )
    .ok()
        == Some((HeartbeatFinalBinding::Exact, 1))
    {
        Ok(Some(HeartbeatPublication {
            file,
            device: stat.st_dev as u64,
            inode: stat.st_ino as u64,
            durability: HeartbeatPublicationDurability::Unconfirmed,
        }))
    } else {
        Err(blocking())
    }
}

#[cfg(target_os = "linux")]
pub(super) fn heartbeat_publication_error(error: HeartbeatPublicationFailure) -> CommandFailure {
    match error {
        HeartbeatPublicationFailure::PreCommit(error)
        | HeartbeatPublicationFailure::PostCommit { error, .. } => error,
    }
}

#[cfg(target_os = "linux")]
pub(super) fn publish_startup_heartbeat_transaction_with_hook(
    root: &Path,
    repo: &str,
    issue: u64,
    session_id: Option<&str>,
    document: &[u8],
    boundary: &mut impl FnMut(&str, &str) -> Result<(), CommandFailure>,
) -> Result<(), CommandFailure> {
    let root_directory = open_private_heartbeat_directory(root)?;
    let repo_name = super::super::autonomous::drain::repository_progress_key(repo);
    let repo_path = root.join(&repo_name);
    drop(open_private_heartbeat_directory(&repo_path)?);
    let repo = open_heartbeat_directory_beneath(&root_directory, Path::new(&repo_name))?;
    let issue_name = format!("{issue}.json");
    let mut issue_guard = inspect_heartbeat_target(&repo, &issue_name, document)?;
    let sessions = session_id
        .map(|_| {
            drop(open_private_heartbeat_directory(
                &repo_path.join("sessions"),
            )?);
            open_heartbeat_directory_beneath(&repo, Path::new("sessions"))
        })
        .transpose()?;
    let session_name = session_id.map(|value| format!("{}.json", heartbeat_session_key(value)));
    let mut session_guard = match (&sessions, &session_name) {
        (Some(directory), Some(name)) => inspect_heartbeat_target(directory, name, document)?,
        _ => None,
    };
    let issue_file = issue_guard
        .is_none()
        .then(|| prepare_private_heartbeat_file(&repo, document, "issue", boundary))
        .transpose()
        .map_err(heartbeat_publication_error)?;
    let session_file = (session_guard.is_none() && sessions.is_some())
        .then(|| {
            prepare_private_heartbeat_file(
                sessions.as_ref().expect("session directory"),
                document,
                "session",
                boundary,
            )
        })
        .transpose()
        .map_err(heartbeat_publication_error)?;
    if let Some(prepared) = issue_file {
        issue_guard = Some(
            publish_prepared_heartbeat_file(&repo, &issue_name, prepared, "issue", boundary)
                .map_err(heartbeat_publication_error)?,
        );
    }
    if let (Some(directory), Some(name), Some(prepared)) =
        (sessions.as_ref(), session_name.as_ref(), session_file)
    {
        session_guard = Some(
            publish_prepared_heartbeat_file(directory, name, prepared, "session", boundary)
                .map_err(heartbeat_publication_error)?,
        );
    }
    boundary("issue", "transaction-fsync")?;
    nix::unistd::fsync(&repo)
        .map_err(|error| CommandFailure::diagnostic(format!("heartbeat repo fsync: {error}")))?;
    if let Some(directory) = sessions.as_ref() {
        boundary("session", "transaction-fsync")?;
        nix::unistd::fsync(directory)
            .map_err(|error| CommandFailure::diagnostic(format!("session fsync: {error}")))?;
    }
    let exact = |directory: &fs::File, name: &str, guard: &HeartbeatPublication| {
        let metadata = guard.file.metadata().ok();
        let content = guard.file.try_clone().and_then(read_regular_file).ok();
        metadata.is_some_and(|value| {
            value.is_file()
                && value.uid() == nix::unistd::geteuid().as_raw()
                && value.mode() & 0o7777 == 0o600
        }) && content
            .is_some_and(|value| same_startup_heartbeat_generation(&value.document, document))
            && heartbeat_final_binding(&guard.file, directory, name, (guard.device, guard.inode))
                .ok()
                == Some((HeartbeatFinalBinding::Exact, 1))
    };
    if !exact(
        &repo,
        &issue_name,
        issue_guard.as_ref().expect("issue guard"),
    ) || sessions
        .as_ref()
        .zip(session_name.as_deref())
        .zip(session_guard.as_ref())
        .is_some_and(|((directory, name), guard)| !exact(directory, name, guard))
    {
        return Err(CommandFailure::diagnostic("heartbeat binding changed"));
    }
    Ok(())
}

#[cfg(unix)]
pub(super) fn same_startup_heartbeat_generation(left: &[u8], right: &[u8]) -> bool {
    let normalize = |document: &[u8]| {
        parse_startup_heartbeat(document).map(|mut evidence| {
            evidence.ts = 0;
            evidence
        })
    };
    normalize(left) == normalize(right)
}

pub(super) fn valid_startup_process_identity(identity: &str) -> bool {
    if identity
        .parse::<u64>()
        .ok()
        .filter(|value| *value > 0)
        .is_some_and(|value| value.to_string() == identity)
    {
        return true;
    }
    identity.split_once('.').is_some_and(|(seconds, micros)| {
        !seconds.is_empty()
            && seconds.bytes().all(|byte| byte.is_ascii_digit())
            && (seconds == "0" || !seconds.starts_with('0'))
            && micros.len() == 6
            && micros.bytes().all(|byte| byte.is_ascii_digit())
    })
}

pub(super) fn startup_heartbeat_nonce(repo: &str, issue: u64, claim_id: &str) -> String {
    let mut identity = b"autospec-startup-heartbeat-nonce-v1".to_vec();
    for field in [repo, &issue.to_string(), claim_id] {
        identity.extend_from_slice(&(field.len() as u64).to_be_bytes());
        identity.extend_from_slice(field.as_bytes());
    }
    autospec_core::autonomous::waterfall::sha256_hex(&identity)
}

#[cfg(target_os = "linux")]
pub(super) fn startup_process_identity(
    pid: u32,
) -> Result<(String, String, String), CommandFailure> {
    let host = fs::read_to_string("/proc/sys/kernel/hostname")
        .map_err(|error| CommandFailure::diagnostic(format!("read heartbeat host: {error}")))?
        .trim()
        .to_string();
    let (boot_id, process_start) = super::super::autonomous::process_birth_identity(pid)
        .map_err(|error| CommandFailure::diagnostic(format!("read heartbeat process: {error}")))?
        .ok_or_else(|| CommandFailure::diagnostic("heartbeat process identity disappeared"))?;
    if host.is_empty() || boot_id.is_empty() || process_start.is_empty() {
        return Err(CommandFailure::diagnostic(
            "heartbeat process identity is incomplete",
        ));
    }
    Ok((host, boot_id, process_start))
}

#[cfg(not(target_os = "linux"))]
pub(super) fn startup_process_identity(
    pid: u32,
) -> Result<(String, String, String), CommandFailure> {
    heartbeat_portable::process_identity(pid)
}
