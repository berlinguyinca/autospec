use super::*;

pub(in crate::commands::claim) fn publish(
    root: &Path,
    repo: &str,
    issue: u64,
    session_id: Option<&str>,
    document: &[u8],
) -> Result<(), CommandFailure> {
    publish_with_hooks(
        root,
        repo,
        issue,
        session_id,
        document,
        &mut |_| {},
        &mut |_| {},
    )
}

pub(super) fn publish_with_hooks(
    root: &Path,
    repo: &str,
    issue: u64,
    session_id: Option<&str>,
    document: &[u8],
    after_repo_open: &mut impl FnMut(&Path),
    before_rename: &mut impl FnMut(&Path),
) -> Result<(), CommandFailure> {
    let expected = parse_startup_heartbeat(document)
        .ok_or_else(|| CommandFailure::diagnostic("startup heartbeat document is malformed"))?;
    ensure_private_directory(root)?;
    let root = open_existing_private_directory(root)?
        .ok_or_else(|| CommandFailure::diagnostic("heartbeat root directory disappeared"))?;
    let repo_dir = open_or_create_private_child(&root, &repository_progress_key(repo))?;
    let _lock = RepositoryLock::acquire(&repo_dir)?;
    after_repo_open(&repo_dir.path);
    publish_exact(
        &repo_dir,
        &format!("{issue}.json"),
        &expected,
        document,
        before_rename,
    )?;
    if let Some(session_id) = session_id {
        let sessions = open_or_create_private_child(&repo_dir, "sessions")?;
        publish_exact(
            &sessions,
            &format!("{}.json", heartbeat_session_key(session_id)),
            &expected,
            document,
            &mut |_| {},
        )?;
    }
    Ok(())
}

pub(super) fn publish_exact(
    directory: &PrivateDirectory,
    name: &str,
    expected: &StartupHeartbeatEvidence,
    document: &[u8],
    before_rename: &mut impl FnMut(&Path),
) -> Result<(), CommandFailure> {
    reconcile_staged_publications(directory, name, expected)?;
    if existing_generation(directory, name, expected)? {
        return Ok(());
    }
    let (temporary_name, mut file) = create_unique_stage(directory, name)?;
    let temporary_path = directory.path.join(&temporary_name);
    file.write_all(document)
        .map_err(|error| CommandFailure::diagnostic(format!("heartbeat write: {error}")))?;
    file.sync_all()
        .map_err(|error| CommandFailure::diagnostic(format!("heartbeat fsync: {error}")))?;
    before_rename(&temporary_path);
    atomic_rename_exclusive(directory, &temporary_name, name, &file).or_else(|error| {
        if existing_generation(directory, name, expected)? {
            Ok(())
        } else {
            Err(error)
        }
    })?;
    sync_private_directory(directory)?;
    if !existing_generation(directory, name, expected)? {
        return Err(CommandFailure::diagnostic(
            "heartbeat publication target conflicts",
        ));
    }
    let metadata = file.metadata().map_err(|error| {
        CommandFailure::diagnostic(format!("heartbeat published identity: {error}"))
    })?;
    let destination = open_private_file_relative(directory, name).map_err(|error| {
        CommandFailure::diagnostic(format!("heartbeat published target reopen: {error}"))
    })?;
    if !private_file_metadata(&metadata)
        || !private_file_handle(&file)?
        || !same_file_identity(&file, &destination)?
    {
        return Err(CommandFailure::diagnostic(
            "heartbeat publication target conflicts",
        ));
    }
    Ok(())
}

pub(super) fn existing_generation(
    directory: &PrivateDirectory,
    name: &str,
    expected: &StartupHeartbeatEvidence,
) -> Result<bool, CommandFailure> {
    let mut file = match open_private_file_relative(directory, name) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(_) => {
            return Err(CommandFailure::diagnostic(
                "heartbeat publication target conflicts",
            ))
        }
    };
    let metadata = file
        .metadata()
        .map_err(|_| CommandFailure::diagnostic("heartbeat publication target conflicts"))?;
    if !metadata.file_type().is_file()
        || !private_file_metadata(&metadata)
        || !private_file_handle(&file)?
    {
        return Err(CommandFailure::diagnostic(
            "heartbeat publication target conflicts",
        ));
    }
    let mut document = Vec::new();
    file.read_to_end(&mut document)
        .map_err(|_| CommandFailure::diagnostic("heartbeat publication target conflicts"))?;
    let Some(observed) = parse_startup_heartbeat(&document) else {
        return Err(CommandFailure::diagnostic(
            "heartbeat publication target conflicts",
        ));
    };
    if same_generation(&observed, expected) {
        Ok(true)
    } else {
        Err(CommandFailure::diagnostic(
            "heartbeat publication target conflicts",
        ))
    }
}

pub(super) fn same_generation(
    left: &StartupHeartbeatEvidence,
    right: &StartupHeartbeatEvidence,
) -> bool {
    let mut left = left.clone();
    let mut right = right.clone();
    for evidence in [&mut left, &mut right] {
        evidence.ts = 0;
    }
    left == right
}

pub(super) static NEXT_STAGE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub(super) fn create_unique_stage(
    directory: &PrivateDirectory,
    destination: &str,
) -> Result<(String, fs::File), CommandFailure> {
    use std::sync::atomic::Ordering;
    use std::time::{SystemTime, UNIX_EPOCH};

    for _ in 0..32 {
        let epoch_nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
        let sequence = NEXT_STAGE.fetch_add(1, Ordering::Relaxed);
        let token = format!(
            "{epoch_nanos:016x}{:08x}{:08x}",
            std::process::id(),
            sequence as u32
        );
        let name = format!(".autospec-heartbeat-{destination}.{token}.stage");
        match create_private_file_relative(directory, &name) {
            Ok(file) => return Ok((name, file)),
            Err(error) => match open_private_file_relative(directory, &name) {
                Ok(_) => continue,
                Err(open_error) if open_error.kind() == std::io::ErrorKind::NotFound => {
                    return Err(error)
                }
                Err(_) => continue,
            },
        }
    }
    Err(CommandFailure::diagnostic(
        "heartbeat stage create exhausted unique names",
    ))
}

pub(super) fn reconcile_staged_publications(
    directory: &PrivateDirectory,
    destination: &str,
    expected: &StartupHeartbeatEvidence,
) -> Result<(), CommandFailure> {
    for source in staged_publication_names(directory, destination)? {
        let mut source_file = match open_staged_file_relative(directory, &source) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => continue,
        };
        let metadata = match source_file.metadata() {
            Ok(metadata) if metadata.file_type().is_file() => metadata,
            _ => continue,
        };
        let links = file_link_count(&source_file)?;
        if !private_stage_metadata(&metadata, links) || !matches!(links, 1 | 2) {
            continue;
        }
        let mut document = Vec::new();
        if source_file.read_to_end(&mut document).is_err() {
            continue;
        }
        let Some(observed) = parse_startup_heartbeat(&document) else {
            continue;
        };
        if observed.repo != expected.repo
            || observed.issue != expected.issue
            || !staged_document_targets(&observed, destination)
        {
            continue;
        }
        if links == 2 {
            let destination_file = match open_private_file_relative(directory, destination) {
                Ok(file) => file,
                Err(_) => continue,
            };
            let destination_metadata = destination_file.metadata().map_err(|_| {
                CommandFailure::diagnostic("heartbeat publication target conflicts")
            })?;
            if !private_stage_metadata(&destination_metadata, 2)
                || !same_file_identity(&source_file, &destination_file)?
                || !same_generation(&observed, expected)
            {
                continue;
            }
        }
        unlink_staged_file(directory, &source, &source_file)?;
        sync_private_directory(directory)?;
    }
    Ok(())
}

pub(super) fn staged_document_targets(
    observed: &StartupHeartbeatEvidence,
    destination: &str,
) -> bool {
    if destination == format!("{}.json", observed.issue) {
        return true;
    }
    observed.session_id.as_deref().is_some_and(|session_id| {
        destination == format!("{}.json", heartbeat_session_key(session_id))
    })
}

pub(super) fn staged_publication_names(
    directory: &PrivateDirectory,
    destination: &str,
) -> Result<Vec<String>, CommandFailure> {
    let Some(path_directory) = open_existing_private_directory(&directory.path)? else {
        return Ok(Vec::new());
    };
    if !same_file_identity(&directory.file, &path_directory.file)? {
        return Ok(Vec::new());
    }
    let prefix = format!(".autospec-heartbeat-{destination}.");
    let mut names = Vec::new();
    for entry in fs::read_dir(&directory.path)
        .map_err(|error| CommandFailure::diagnostic(format!("heartbeat stage scan: {error}")))?
    {
        let Ok(entry) = entry else { continue };
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Some(token) = name
            .strip_prefix(&prefix)
            .and_then(|value| value.strip_suffix(".stage"))
        else {
            continue;
        };
        if token.len() == 32
            && token
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            names.push(name);
        }
    }
    Ok(names)
}

#[cfg(target_os = "macos")]
pub(super) fn atomic_rename_exclusive(
    directory: &PrivateDirectory,
    source: &str,
    destination: &str,
    _file: &fs::File,
) -> Result<(), CommandFailure> {
    use std::ffi::CString;
    use std::os::fd::AsRawFd;
    unsafe extern "C" {
        fn renameatx_np(
            source_fd: nix::libc::c_int,
            source: *const nix::libc::c_char,
            destination_fd: nix::libc::c_int,
            destination: *const nix::libc::c_char,
            flags: u32,
        ) -> i32;
    }
    const RENAME_EXCL: u32 = 0x0000_0004;
    let source = CString::new(source)
        .map_err(|_| CommandFailure::diagnostic("heartbeat stage path contains NUL"))?;
    let destination = CString::new(destination)
        .map_err(|_| CommandFailure::diagnostic("heartbeat destination path contains NUL"))?;
    // SAFETY: both names are single components beneath the retained directory descriptor.
    if unsafe {
        renameatx_np(
            directory.file.as_raw_fd(),
            source.as_ptr(),
            directory.file.as_raw_fd(),
            destination.as_ptr(),
            RENAME_EXCL,
        )
    } == 0
    {
        Ok(())
    } else {
        Err(CommandFailure::diagnostic(format!(
            "heartbeat atomic rename: {}",
            std::io::Error::last_os_error()
        )))
    }
}

#[cfg(all(test, target_os = "freebsd"))]
pub(super) static FREEBSD_CRASH_AFTER_LINK: std::sync::Mutex<Option<PathBuf>> =
    std::sync::Mutex::new(None);

#[cfg(target_os = "freebsd")]
pub(super) fn atomic_rename_exclusive(
    directory: &PrivateDirectory,
    source: &str,
    destination: &str,
    _file: &fs::File,
) -> Result<(), CommandFailure> {
    use nix::fcntl::AtFlags;
    use nix::unistd::{linkat, unlinkat, UnlinkatFlags};

    linkat(
        &directory.file,
        source,
        &directory.file,
        destination,
        AtFlags::empty(),
    )
    .map_err(|error| CommandFailure::diagnostic(format!("heartbeat atomic publish: {error}")))?;
    sync_private_directory(directory)?;
    #[cfg(test)]
    let crash_after_link = {
        let mut target = FREEBSD_CRASH_AFTER_LINK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if target.as_ref() == Some(&directory.path.join(destination)) {
            target.take();
            true
        } else {
            false
        }
    };
    #[cfg(test)]
    if crash_after_link {
        panic!("simulated publication crash after link");
    }
    unlinkat(&directory.file, source, UnlinkatFlags::NoRemoveDir)
        .map_err(|error| CommandFailure::diagnostic(format!("heartbeat stage cleanup: {error}")))?;
    sync_private_directory(directory)
}

#[cfg(target_os = "linux")]
pub(super) fn atomic_rename_exclusive(
    directory: &PrivateDirectory,
    source: &str,
    destination: &str,
    _file: &fs::File,
) -> Result<(), CommandFailure> {
    nix::fcntl::renameat2(
        &directory.file,
        source,
        &directory.file,
        destination,
        nix::fcntl::RenameFlags::RENAME_NOREPLACE,
    )
    .map_err(|error| CommandFailure::diagnostic(format!("heartbeat atomic publish: {error}")))
}

#[cfg(windows)]
pub(super) fn atomic_rename_exclusive(
    directory: &PrivateDirectory,
    _source: &str,
    destination: &str,
    file: &fs::File,
) -> Result<(), CommandFailure> {
    rename_windows_file_handle(file, &directory.file, destination)
        .map_err(|error| CommandFailure::diagnostic(format!("heartbeat atomic publish: {error}")))
}
