use super::*;

pub(in crate::commands::claim) fn retire_released(
    identity: ClaimMutationIdentity<'_>,
) -> Result<(), CommandFailure> {
    let root = heartbeat_root()?;
    retire_released_at(&root, identity)
}

pub(super) fn retire_released_at(
    root: &Path,
    identity: ClaimMutationIdentity<'_>,
) -> Result<(), CommandFailure> {
    retire_released_at_with_hook(root, identity, &mut |_| Ok(()))
}

pub(super) fn retire_released_at_with_hook(
    root: &Path,
    identity: ClaimMutationIdentity<'_>,
    after_issue_detach: &mut impl FnMut(&Path) -> Result<(), CommandFailure>,
) -> Result<(), CommandFailure> {
    retire_released_at_with_boundary_hooks(
        root,
        identity,
        &mut |_| Ok(()),
        after_issue_detach,
        &mut |_| Ok(()),
    )
}

pub(super) fn retire_released_at_with_boundary_hooks(
    root: &Path,
    identity: ClaimMutationIdentity<'_>,
    after_repo_open: &mut impl FnMut(&Path) -> Result<(), CommandFailure>,
    after_issue_detach: &mut impl FnMut(&Path) -> Result<(), CommandFailure>,
    after_sessions_open: &mut impl FnMut(&Path) -> Result<(), CommandFailure>,
) -> Result<(), CommandFailure> {
    let Some(root) = open_existing_private_directory(root)? else {
        return Ok(());
    };
    let collision_safe = repository_progress_key(identity.repo);
    if let Some(repo) = open_existing_private_child(&root, &collision_safe)? {
        after_repo_open(&repo.path)?;
        let _lock = RepositoryLock::acquire(&repo)?;
        retire_session_candidates(&repo, identity, after_sessions_open)?;
        retire_exact_issue(&repo, identity, after_issue_detach)?;
    }
    let canonical = identity.repo.replace('/', "__");
    if canonical != collision_safe {
        if let Some(repo) = open_existing_private_child(&root, &canonical)? {
            after_repo_open(&repo.path)?;
            let _lock = RepositoryLock::acquire(&repo)?;
            retire_session_candidates(&repo, identity, after_sessions_open)?;
        }
    }
    Ok(())
}

pub(in crate::commands::claim) fn retire_session_bindings_at(
    root_path: &Path,
    identity: ClaimMutationIdentity<'_>,
) -> Result<(), CommandFailure> {
    let Some(root) = open_existing_private_directory(root_path)? else {
        return Ok(());
    };
    let collision_safe = repository_progress_key(identity.repo);
    let canonical = identity.repo.replace('/', "__");
    for repo_name in [collision_safe.as_str(), canonical.as_str()] {
        let Some(repo) = open_existing_private_child(&root, repo_name)? else {
            continue;
        };
        let _lock = RepositoryLock::acquire(&repo)?;
        retire_session_candidates(&repo, identity, &mut |_| Ok(()))?;
    }
    Ok(())
}

fn retire_exact_issue(
    repo: &PrivateDirectory,
    identity: ClaimMutationIdentity<'_>,
    after_issue_detach: &mut impl FnMut(&Path) -> Result<(), CommandFailure>,
) -> Result<(), CommandFailure> {
    let issue_name = format!("{}.json", identity.issue);
    let issue_stage_name = retirement_stage_name(&issue_name, identity);
    let issue_stage = match open_detached_heartbeat(&repo, &issue_stage_name)? {
        Some(stage) => stage,
        None => match detach_heartbeat(&repo, &issue_name, &issue_stage_name)? {
            Some(stage) => stage,
            None => return Ok(()),
        },
    };
    if let Err(error) = after_issue_detach(&repo.path.join(&issue_name)) {
        restore_detached_heartbeat(&repo, &issue_stage, &issue_name)?;
        return Err(error);
    }
    match detached_retirement_evidence(&repo, &issue_stage, identity) {
        Ok(Some(_)) => {}
        Ok(None) => {
            restore_detached_heartbeat(&repo, &issue_stage, &issue_name)?;
            return Ok(());
        }
        Err(error) => {
            restore_detached_heartbeat(&repo, &issue_stage, &issue_name)?;
            return Err(error);
        }
    }
    remove_detached_heartbeat(&repo, &issue_stage)?;
    sync_private_directory(&repo)
}

fn retire_session_candidates(
    repo: &PrivateDirectory,
    identity: ClaimMutationIdentity<'_>,
    after_sessions_open: &mut impl FnMut(&Path) -> Result<(), CommandFailure>,
) -> Result<(), CommandFailure> {
    let Some(sessions) = open_existing_private_child(repo, "sessions")? else {
        return Ok(());
    };
    after_sessions_open(&sessions.path)?;
    for name in private_directory_entry_names(&sessions)? {
        if Path::new(&name).extension() != Some("json".as_ref()) {
            continue;
        }
        let stage_name = retirement_stage_name(&name, identity);
        let detached = match open_detached_heartbeat(&sessions, &stage_name)? {
            Some(stage) => stage,
            None => match detach_heartbeat(&sessions, &name, &stage_name)? {
                Some(stage) => stage,
                None => continue,
            },
        };
        let document = match read_detached_private_file(&sessions, &detached) {
            Ok(document) => document,
            Err(error) => {
                restore_detached_heartbeat(&sessions, &detached, &name)?;
                return Err(CommandFailure::diagnostic(format!(
                    "read released session binding: {error}"
                )));
            }
        };
        let exact = parse_startup_heartbeat(&document)
            .as_ref()
            .is_some_and(|evidence| exact_retirement_identity(evidence, identity))
            || super::super::shell_session_binding_matches(&document, identity)?;
        if !exact {
            restore_detached_heartbeat(&sessions, &detached, &name)?;
            continue;
        }
        if let Err(error) = remove_detached_heartbeat(&sessions, &detached) {
            restore_detached_heartbeat(&sessions, &detached, &name)?;
            return Err(error);
        }
        sync_private_directory(&sessions)?;
    }
    Ok(())
}

pub(super) fn detached_retirement_evidence(
    directory: &PrivateDirectory,
    detached: &DetachedHeartbeat,
    identity: ClaimMutationIdentity<'_>,
) -> Result<Option<StartupHeartbeatEvidence>, CommandFailure> {
    let document = read_detached_private_file(directory, detached)
        .map_err(|error| CommandFailure::diagnostic(format!("read released heartbeat: {error}")))?;
    let Some(evidence) = parse_startup_heartbeat(&document) else {
        return Ok(None);
    };
    Ok(exact_retirement_identity(&evidence, identity).then_some(evidence))
}

pub(super) fn exact_retirement_identity(
    evidence: &StartupHeartbeatEvidence,
    identity: ClaimMutationIdentity<'_>,
) -> bool {
    super::super::exact_heartbeat_claim_identity(evidence, identity)
}

pub(super) struct DetachedHeartbeat {
    name: String,
    #[cfg(windows)]
    file: fs::File,
}

pub(super) fn retirement_stage_name(
    live_name: &str,
    identity: ClaimMutationIdentity<'_>,
) -> String {
    let digest = autospec_core::autonomous::waterfall::sha256_hex(
        format!(
            "{}\0{}\0{}\0{}\0{}\0{}",
            identity.repo,
            identity.issue,
            identity.worker_id,
            identity.branch,
            identity.claim_id,
            live_name
        )
        .as_bytes(),
    );
    format!(".autospec-retiring-{}", &digest[..24])
}

pub(super) fn detach_heartbeat(
    directory: &PrivateDirectory,
    live_name: &str,
    staged_name: &str,
) -> Result<Option<DetachedHeartbeat>, CommandFailure> {
    #[cfg(unix)]
    {
        use std::ffi::CString;
        use std::os::fd::AsRawFd;
        let live = CString::new(live_name)
            .map_err(|_| CommandFailure::diagnostic("heartbeat live name contains NUL"))?;
        let staged = CString::new(staged_name)
            .map_err(|_| CommandFailure::diagnostic("heartbeat staged name contains NUL"))?;
        // SAFETY: both names are NUL-terminated single components beneath the same live fd.
        if unsafe {
            nix::libc::renameat(
                directory.file.as_raw_fd(),
                live.as_ptr(),
                directory.file.as_raw_fd(),
                staged.as_ptr(),
            )
        } != 0
        {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::NotFound {
                return Ok(None);
            }
            return Err(CommandFailure::diagnostic(format!(
                "detach released heartbeat: {error}"
            )));
        }
    }
    #[cfg(windows)]
    {
        let file = match open_windows_relative(
            &directory.file,
            live_name,
            WINDOWS_GENERIC_READ | WINDOWS_DELETE | WINDOWS_SYNCHRONIZE,
            WINDOWS_FILE_OPEN,
            WINDOWS_FILE_NON_DIRECTORY_FILE
                | WINDOWS_FILE_OPEN_REPARSE_POINT
                | WINDOWS_FILE_SYNCHRONOUS_IO_NONALERT,
        ) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(CommandFailure::diagnostic(format!(
                    "open released heartbeat: {error}"
                )))
            }
        };
        let metadata = file.metadata().map_err(|error| {
            CommandFailure::diagnostic(format!("inspect released heartbeat: {error}"))
        })?;
        if !metadata.file_type().is_file() || !private_file_metadata(&metadata) {
            return Err(CommandFailure::diagnostic(
                "released heartbeat is not a private regular file",
            ));
        }
        match rename_windows_file_handle(&file, &directory.file, &staged_name) {
            Ok(()) => {}
            Err(error) => {
                return Err(CommandFailure::diagnostic(format!(
                    "detach released heartbeat: {error}"
                )))
            }
        }
        sync_private_directory(directory)?;
        return Ok(Some(DetachedHeartbeat {
            name: staged_name.to_string(),
            file,
        }));
    }
    #[cfg(unix)]
    {
        sync_private_directory(directory)?;
        Ok(Some(DetachedHeartbeat {
            name: staged_name.to_string(),
        }))
    }
}

pub(super) fn open_detached_heartbeat(
    directory: &PrivateDirectory,
    staged_name: &str,
) -> Result<Option<DetachedHeartbeat>, CommandFailure> {
    #[cfg(unix)]
    {
        match open_private_file_relative(directory, staged_name) {
            Ok(_) => Ok(Some(DetachedHeartbeat {
                name: staged_name.to_string(),
            })),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(CommandFailure::diagnostic(format!(
                "open detached released heartbeat: {error}"
            ))),
        }
    }
    #[cfg(windows)]
    {
        let file = match open_windows_relative(
            &directory.file,
            staged_name,
            WINDOWS_GENERIC_READ | WINDOWS_DELETE | WINDOWS_SYNCHRONIZE,
            WINDOWS_FILE_OPEN,
            WINDOWS_FILE_NON_DIRECTORY_FILE
                | WINDOWS_FILE_OPEN_REPARSE_POINT
                | WINDOWS_FILE_SYNCHRONOUS_IO_NONALERT,
        ) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(CommandFailure::diagnostic(format!(
                    "open detached released heartbeat: {error}"
                )))
            }
        };
        Ok(Some(DetachedHeartbeat {
            name: staged_name.to_string(),
            file,
        }))
    }
}

pub(super) fn restore_detached_heartbeat(
    directory: &PrivateDirectory,
    detached: &DetachedHeartbeat,
    live_name: &str,
) -> Result<(), CommandFailure> {
    #[cfg(unix)]
    {
        use nix::fcntl::AtFlags;
        use nix::unistd::{linkat, unlinkat, UnlinkatFlags};
        match linkat(
            &directory.file,
            detached.name.as_str(),
            &directory.file,
            live_name,
            AtFlags::empty(),
        ) {
            Ok(()) => {
                unlinkat(
                    &directory.file,
                    detached.name.as_str(),
                    UnlinkatFlags::NoRemoveDir,
                )
                .map_err(|error| {
                    CommandFailure::diagnostic(format!(
                        "remove restored heartbeat staging: {error}"
                    ))
                })?;
            }
            Err(nix::errno::Errno::EEXIST) => return Ok(()),
            Err(error) => {
                return Err(CommandFailure::diagnostic(format!(
                    "restore detached heartbeat: {error}"
                )))
            }
        }
    }
    #[cfg(windows)]
    {
        match rename_windows_file_handle(&detached.file, &directory.file, live_name) {
            Ok(()) => {}
            Err(_)
                if open_windows_relative(
                    &directory.file,
                    live_name,
                    WINDOWS_FILE_READ_ATTRIBUTES | WINDOWS_SYNCHRONIZE,
                    WINDOWS_FILE_OPEN,
                    WINDOWS_FILE_NON_DIRECTORY_FILE
                        | WINDOWS_FILE_OPEN_REPARSE_POINT
                        | WINDOWS_FILE_SYNCHRONOUS_IO_NONALERT,
                )
                .is_ok() =>
            {
                return Ok(())
            }
            Err(error) => {
                return Err(CommandFailure::diagnostic(format!(
                    "restore detached heartbeat: {error}"
                )))
            }
        }
    }
    sync_private_directory(directory)
}

pub(super) fn remove_detached_heartbeat(
    directory: &PrivateDirectory,
    detached: &DetachedHeartbeat,
) -> Result<(), CommandFailure> {
    #[cfg(windows)]
    let _ = directory;
    #[cfg(unix)]
    nix::unistd::unlinkat(
        &directory.file,
        detached.name.as_str(),
        nix::unistd::UnlinkatFlags::NoRemoveDir,
    )
    .map_err(|error| CommandFailure::diagnostic(format!("remove detached heartbeat: {error}")))?;
    #[cfg(windows)]
    delete_windows_file_handle(&detached.file).map_err(|error| {
        CommandFailure::diagnostic(format!("remove detached heartbeat: {error}"))
    })?;
    Ok(())
}

#[cfg(unix)]
pub(super) fn read_detached_private_file(
    directory: &PrivateDirectory,
    detached: &DetachedHeartbeat,
) -> std::io::Result<Vec<u8>> {
    use nix::fcntl::{openat, OFlag};
    use nix::sys::stat::Mode;
    let mut file = fs::File::from(
        openat(
            &directory.file,
            Path::new(&detached.name),
            OFlag::O_RDONLY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC | OFlag::O_NONBLOCK,
            Mode::empty(),
        )
        .map_err(std::io::Error::from)?,
    );
    let metadata = file.metadata()?;
    if !metadata.file_type().is_file() || !private_file_metadata(&metadata) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "released heartbeat is not a private regular file",
        ));
    }
    let mut document = Vec::new();
    file.read_to_end(&mut document)?;
    Ok(document)
}

#[cfg(windows)]
pub(super) fn read_detached_private_file(
    _directory: &PrivateDirectory,
    detached: &DetachedHeartbeat,
) -> std::io::Result<Vec<u8>> {
    let metadata = detached.file.metadata()?;
    if !metadata.file_type().is_file() || !private_file_metadata(&metadata) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "released heartbeat is not a private regular file",
        ));
    }
    let mut file = &detached.file;
    let mut document = Vec::new();
    file.read_to_end(&mut document)?;
    Ok(document)
}
