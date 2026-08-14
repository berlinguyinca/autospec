use super::{
    heartbeat_root, heartbeat_session_key, parse_startup_heartbeat, ClaimMutationIdentity,
    StartupHeartbeatEvidence,
};
use crate::commands::autonomous::drain::repository_progress_key;
use crate::commands::CommandFailure;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TEMPORARY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(super) fn process_identity(pid: u32) -> Result<(String, String, String), CommandFailure> {
    let host = hostname()
        .map_err(|error| CommandFailure::diagnostic(format!("read heartbeat host: {error}")))?;
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

#[cfg(unix)]
pub(super) fn hostname() -> Result<String, String> {
    let mut buffer = [0_u8; 256];
    // SAFETY: buffer is writable for its full declared length.
    if unsafe { nix::libc::gethostname(buffer.as_mut_ptr().cast(), buffer.len()) } != 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let end = buffer
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(buffer.len());
    String::from_utf8(buffer[..end].to_vec()).map_err(|error| error.to_string())
}

#[cfg(windows)]
pub(super) fn hostname() -> Result<String, String> {
    #[link(name = "Kernel32")]
    unsafe extern "system" {
        fn GetComputerNameW(buffer: *mut u16, size: *mut u32) -> i32;
    }
    let mut buffer = [0_u16; 256];
    let mut length = buffer.len() as u32;
    // SAFETY: both pointers refer to writable values with the declared capacity.
    if unsafe { GetComputerNameW(buffer.as_mut_ptr(), &mut length) } == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    String::from_utf16(&buffer[..length as usize]).map_err(|error| error.to_string())
}

pub(super) fn publish(
    root: &Path,
    repo: &str,
    issue: u64,
    session_id: Option<&str>,
    document: &[u8],
) -> Result<(), CommandFailure> {
    let expected = parse_startup_heartbeat(document)
        .ok_or_else(|| CommandFailure::diagnostic("startup heartbeat document is malformed"))?;
    ensure_private_directory(root)?;
    let repo_path = open_or_create_private_directory(root, &repository_progress_key(repo))?;
    let repo_dir = open_existing_private_directory(&repo_path)?
        .ok_or_else(|| CommandFailure::diagnostic("heartbeat repository directory disappeared"))?;
    let _lock = RepositoryLock::acquire(&repo_dir)?;
    publish_exact(
        &repo_dir.path,
        &format!("{issue}.json"),
        &expected,
        document,
    )?;
    if let Some(session_id) = session_id {
        let sessions = open_or_create_private_directory(&repo_dir.path, "sessions")?;
        publish_exact(
            &sessions,
            &format!("{}.json", heartbeat_session_key(session_id)),
            &expected,
            document,
        )?;
    }
    Ok(())
}

fn open_or_create_private_directory(
    parent: &Path,
    name: &str,
) -> Result<std::path::PathBuf, CommandFailure> {
    if name.is_empty() || Path::new(name).is_absolute() || Path::new(name).components().count() != 1
    {
        return Err(CommandFailure::diagnostic(
            "heartbeat directory name must be one normal component",
        ));
    }
    ensure_private_directory(parent)?;
    let path = parent.join(name);
    ensure_private_directory(&path)?;
    Ok(path)
}

fn ensure_private_directory(path: &Path) -> Result<(), CommandFailure> {
    ensure_private_directory_with_hook(path, &mut |_| {})
}

fn ensure_private_directory_with_hook(
    path: &Path,
    after_create: &mut impl FnMut(&Path),
) -> Result<(), CommandFailure> {
    match create_private_directory(path) {
        Ok(()) => after_create(path),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => {
            return Err(CommandFailure::diagnostic(format!(
                "could not create heartbeat directory: {error}"
            )))
        }
    }
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        CommandFailure::diagnostic(format!("could not inspect heartbeat directory: {error}"))
    })?;
    #[cfg(windows)]
    validate_windows_path_components(path)?;
    if !metadata.file_type().is_dir() || !private_directory_metadata(&metadata) {
        return Err(CommandFailure::diagnostic(
            "heartbeat publication directory is not private",
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn create_private_directory(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    let mut builder = fs::DirBuilder::new();
    builder.mode(0o700);
    builder.create(path)
}

#[cfg(windows)]
fn create_private_directory(path: &Path) -> std::io::Result<()> {
    fs::create_dir(path)
}

#[cfg(unix)]
fn private_directory_metadata(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    metadata.uid() == unsafe { nix::libc::geteuid() }
        && metadata.permissions().mode() & 0o7777 == 0o700
}

#[cfg(windows)]
fn validate_windows_path_components(path: &Path) -> Result<(), CommandFailure> {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

    let mut current = std::path::PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        if matches!(
            component,
            std::path::Component::Prefix(_) | std::path::Component::RootDir
        ) {
            continue;
        }
        let metadata = fs::symlink_metadata(&current).map_err(|error| {
            CommandFailure::diagnostic(format!(
                "could not inspect heartbeat path component: {error}"
            ))
        })?;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(CommandFailure::diagnostic(
                "heartbeat path component is a reparse point",
            ));
        }
    }
    Ok(())
}

#[cfg(windows)]
fn private_directory_metadata(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0
}

struct PrivateDirectory {
    path: PathBuf,
    #[cfg(unix)]
    file: fs::File,
}

fn open_existing_private_directory(
    path: &Path,
) -> Result<Option<PrivateDirectory>, CommandFailure> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(CommandFailure::diagnostic(format!(
                "could not inspect heartbeat directory: {error}"
            )))
        }
        Ok(_) => {}
    }
    #[cfg(unix)]
    {
        use nix::fcntl::{open, OFlag};
        use nix::sys::stat::Mode;
        let file = fs::File::from(
            open(
                path,
                OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
                Mode::empty(),
            )
            .map_err(|error| {
                CommandFailure::diagnostic(format!("heartbeat directory secure open: {error}"))
            })?,
        );
        super::private_heartbeat_directory_identity(&file, "portable directory")?;
        Ok(Some(PrivateDirectory {
            path: path.to_path_buf(),
            file,
        }))
    }
    #[cfg(windows)]
    {
        validate_windows_path_components(path)?;
        let metadata = fs::symlink_metadata(path).map_err(|error| {
            CommandFailure::diagnostic(format!("could not inspect heartbeat directory: {error}"))
        })?;
        if !metadata.file_type().is_dir() || !private_directory_metadata(&metadata) {
            return Err(CommandFailure::diagnostic(
                "heartbeat publication directory is not private",
            ));
        }
        Ok(Some(PrivateDirectory {
            path: path.to_path_buf(),
        }))
    }
}

fn open_existing_private_child(
    parent: &PrivateDirectory,
    name: &str,
) -> Result<Option<PrivateDirectory>, CommandFailure> {
    if name.is_empty() || Path::new(name).components().count() != 1 {
        return Err(CommandFailure::diagnostic(
            "heartbeat directory name must be one normal component",
        ));
    }
    let path = parent.path.join(name);
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(CommandFailure::diagnostic(format!(
                "could not inspect heartbeat directory: {error}"
            )))
        }
        Ok(_) => {}
    }
    #[cfg(unix)]
    {
        let file = super::open_heartbeat_directory_beneath(&parent.file, Path::new(name))?;
        Ok(Some(PrivateDirectory { path, file }))
    }
    #[cfg(windows)]
    open_existing_private_directory(&path)
}

struct RepositoryLock {
    _file: fs::File,
    #[cfg(windows)]
    _overlapped: Box<WindowsOverlapped>,
}

impl RepositoryLock {
    fn acquire(directory: &PrivateDirectory) -> Result<Self, CommandFailure> {
        #[cfg(unix)]
        {
            use nix::fcntl::{openat, OFlag};
            use nix::sys::stat::Mode;
            use std::os::fd::AsRawFd;
            use std::os::unix::fs::PermissionsExt;
            let file = fs::File::from(
                openat(
                    &directory.file,
                    Path::new(".portable-heartbeat.lock"),
                    OFlag::O_CREAT | OFlag::O_RDWR | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
                    Mode::from_bits_truncate(0o600),
                )
                .map_err(|error| {
                    CommandFailure::diagnostic(format!("heartbeat repository lock open: {error}"))
                })?,
            );
            file.set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(|error| {
                    CommandFailure::diagnostic(format!("heartbeat repository lock chmod: {error}"))
                })?;
            let metadata = file.metadata().map_err(|error| {
                CommandFailure::diagnostic(format!("heartbeat repository lock inspect: {error}"))
            })?;
            if !metadata.file_type().is_file() || !private_file_metadata(&metadata) {
                return Err(CommandFailure::diagnostic(
                    "heartbeat repository lock is not private",
                ));
            }
            unsafe extern "C" {
                fn flock(fd: i32, operation: i32) -> i32;
            }
            const LOCK_EX: i32 = 2;
            // SAFETY: flock receives a live descriptor and a valid exclusive-lock operation.
            if unsafe { flock(file.as_raw_fd(), LOCK_EX) } != 0 {
                return Err(CommandFailure::diagnostic(format!(
                    "heartbeat repository lock: {}",
                    std::io::Error::last_os_error()
                )));
            }
            Ok(Self { _file: file })
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            use std::os::windows::io::AsRawHandle;
            const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
            let path = directory.path.join(".portable-heartbeat.lock");
            let file = OpenOptions::new()
                .create(true)
                .read(true)
                .write(true)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
                .open(&path)
                .map_err(|error| {
                    CommandFailure::diagnostic(format!("heartbeat repository lock open: {error}"))
                })?;
            let metadata = file.metadata().map_err(|error| {
                CommandFailure::diagnostic(format!("heartbeat repository lock inspect: {error}"))
            })?;
            if !metadata.file_type().is_file() || !private_file_metadata(&metadata) {
                return Err(CommandFailure::diagnostic(
                    "heartbeat repository lock is not private",
                ));
            }
            let mut overlapped = Box::new(WindowsOverlapped::zeroed());
            // SAFETY: the file handle is live and the OVERLAPPED storage remains owned by the lock.
            if unsafe {
                LockFileEx(
                    file.as_raw_handle(),
                    0x0000_0002,
                    0,
                    u32::MAX,
                    u32::MAX,
                    overlapped.as_mut(),
                )
            } == 0
            {
                return Err(CommandFailure::diagnostic(format!(
                    "heartbeat repository lock: {}",
                    std::io::Error::last_os_error()
                )));
            }
            Ok(Self {
                _file: file,
                _overlapped: overlapped,
            })
        }
    }
}

#[cfg(windows)]
#[repr(C)]
struct WindowsOverlapped {
    internal: usize,
    internal_high: usize,
    offset: u32,
    offset_high: u32,
    event: *mut std::ffi::c_void,
}

#[cfg(windows)]
impl WindowsOverlapped {
    fn zeroed() -> Self {
        Self {
            internal: 0,
            internal_high: 0,
            offset: 0,
            offset_high: 0,
            event: std::ptr::null_mut(),
        }
    }
}

#[cfg(windows)]
#[link(name = "Kernel32")]
unsafe extern "system" {
    fn LockFileEx(
        file: std::os::windows::io::RawHandle,
        flags: u32,
        reserved: u32,
        bytes_low: u32,
        bytes_high: u32,
        overlapped: *mut WindowsOverlapped,
    ) -> i32;
}

fn publish_exact(
    directory: &Path,
    name: &str,
    expected: &StartupHeartbeatEvidence,
    document: &[u8],
) -> Result<(), CommandFailure> {
    ensure_private_directory(directory)?;
    let destination = directory.join(name);
    if existing_generation(&destination, expected)? {
        return Ok(());
    }

    let temporary = directory.join(format!(
        ".autospec-heartbeat-{}-{}",
        std::process::id(),
        TEMPORARY_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut file = create_private_file(&temporary)?;
        file.write_all(document)
            .map_err(|error| CommandFailure::diagnostic(format!("heartbeat write: {error}")))?;
        file.sync_all()
            .map_err(|error| CommandFailure::diagnostic(format!("heartbeat fsync: {error}")))?;
        drop(file);
        atomic_rename_exclusive(&temporary, &destination).or_else(|error| {
            if existing_generation(&destination, expected)? {
                Ok(())
            } else {
                Err(error)
            }
        })?;
        sync_directory(directory)?;
        if !existing_generation(&destination, expected)? {
            return Err(CommandFailure::diagnostic(
                "heartbeat publication target conflicts",
            ));
        }
        Ok(())
    })();
    let _ = fs::remove_file(&temporary);
    result
}

fn existing_generation(
    path: &Path,
    expected: &StartupHeartbeatEvidence,
) -> Result<bool, CommandFailure> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(_) => {
            return Err(CommandFailure::diagnostic(
                "heartbeat publication target conflicts",
            ))
        }
    };
    if !metadata.file_type().is_file() || !private_file_metadata(&metadata) {
        return Err(CommandFailure::diagnostic(
            "heartbeat publication target conflicts",
        ));
    }
    let document = read_file_no_follow(path)
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

fn same_generation(left: &StartupHeartbeatEvidence, right: &StartupHeartbeatEvidence) -> bool {
    let mut left = left.clone();
    let mut right = right.clone();
    for evidence in [&mut left, &mut right] {
        evidence.ts = 0;
        evidence.pid = 0;
        evidence.process_start.clear();
    }
    left == right
}

#[cfg(unix)]
fn create_private_file(path: &Path) -> Result<fs::File, CommandFailure> {
    use std::os::unix::fs::OpenOptionsExt;
    use std::os::unix::fs::PermissionsExt;
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_CLOEXEC)
        .open(path)
        .map_err(|error| CommandFailure::diagnostic(format!("heartbeat stage create: {error}")))?;
    file.set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|error| CommandFailure::diagnostic(format!("heartbeat stage chmod: {error}")))?;
    Ok(file)
}

#[cfg(windows)]
fn create_private_file(path: &Path) -> Result<fs::File, CommandFailure> {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|error| CommandFailure::diagnostic(format!("heartbeat stage create: {error}")))
}

#[cfg(unix)]
fn read_file_no_follow(path: &Path) -> std::io::Result<Vec<u8>> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_CLOEXEC | nix::libc::O_NONBLOCK)
        .open(path)?;
    let mut document = Vec::new();
    file.read_to_end(&mut document)?;
    Ok(document)
}

#[cfg(windows)]
fn read_file_no_follow(path: &Path) -> std::io::Result<Vec<u8>> {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    let mut document = Vec::new();
    file.read_to_end(&mut document)?;
    Ok(document)
}

#[cfg(unix)]
fn private_file_metadata(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    metadata.uid() == unsafe { nix::libc::geteuid() }
        && metadata.permissions().mode() & 0o7777 == 0o600
        && metadata.nlink() == 1
}

#[cfg(windows)]
fn private_file_metadata(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0
}

#[cfg(target_os = "macos")]
fn atomic_rename_exclusive(source: &Path, destination: &Path) -> Result<(), CommandFailure> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    unsafe extern "C" {
        fn renamex_np(
            source: *const nix::libc::c_char,
            destination: *const nix::libc::c_char,
            flags: u32,
        ) -> i32;
    }
    const RENAME_EXCL: u32 = 0x0000_0004;
    let source = CString::new(source.as_os_str().as_bytes())
        .map_err(|_| CommandFailure::diagnostic("heartbeat stage path contains NUL"))?;
    let destination = CString::new(destination.as_os_str().as_bytes())
        .map_err(|_| CommandFailure::diagnostic("heartbeat destination path contains NUL"))?;
    // SAFETY: both strings are NUL-terminated and remain alive for the call.
    if unsafe { renamex_np(source.as_ptr(), destination.as_ptr(), RENAME_EXCL) } == 0 {
        Ok(())
    } else {
        Err(CommandFailure::diagnostic(format!(
            "heartbeat atomic rename: {}",
            std::io::Error::last_os_error()
        )))
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn atomic_rename_exclusive(source: &Path, destination: &Path) -> Result<(), CommandFailure> {
    fs::hard_link(source, destination)
        .and_then(|()| fs::remove_file(source))
        .map_err(|error| CommandFailure::diagnostic(format!("heartbeat atomic publish: {error}")))
}

#[cfg(windows)]
fn atomic_rename_exclusive(source: &Path, destination: &Path) -> Result<(), CommandFailure> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "Kernel32")]
    unsafe extern "system" {
        fn CreateHardLinkW(
            new_file_name: *const u16,
            existing_file_name: *const u16,
            security_attributes: *const std::ffi::c_void,
        ) -> i32;
        fn DeleteFileW(file_name: *const u16) -> i32;
    }
    let source = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    // SAFETY: both paths are NUL-terminated and the destination link is created only if absent.
    if unsafe { CreateHardLinkW(destination.as_ptr(), source.as_ptr(), std::ptr::null()) } == 0 {
        return Err(CommandFailure::diagnostic(format!(
            "heartbeat atomic publish: {}",
            std::io::Error::last_os_error()
        )));
    }
    // SAFETY: source is a NUL-terminated path owned by this publication attempt.
    if unsafe { DeleteFileW(source.as_ptr()) } == 0 {
        return Err(CommandFailure::diagnostic(format!(
            "heartbeat stage cleanup: {}",
            std::io::Error::last_os_error()
        )));
    }
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), CommandFailure> {
    use std::os::unix::fs::OpenOptionsExt;
    let directory = OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_DIRECTORY | nix::libc::O_NOFOLLOW | nix::libc::O_CLOEXEC)
        .open(path)
        .map_err(|error| {
            CommandFailure::diagnostic(format!("heartbeat directory open: {error}"))
        })?;
    directory
        .sync_all()
        .map_err(|error| CommandFailure::diagnostic(format!("heartbeat directory fsync: {error}")))
}

#[cfg(windows)]
fn sync_directory(path: &Path) -> Result<(), CommandFailure> {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    let directory = OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|error| {
            CommandFailure::diagnostic(format!("heartbeat directory open: {error}"))
        })?;
    match directory.sync_all() {
        Ok(()) => Ok(()),
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::InvalidInput
            ) =>
        {
            Ok(())
        }
        Err(error) => Err(CommandFailure::diagnostic(format!(
            "heartbeat directory flush: {error}"
        ))),
    }
}

pub(super) fn retire_released(identity: ClaimMutationIdentity<'_>) -> Result<(), CommandFailure> {
    let root = heartbeat_root()?;
    retire_released_at(&root, identity)
}

fn retire_released_at(
    root: &Path,
    identity: ClaimMutationIdentity<'_>,
) -> Result<(), CommandFailure> {
    retire_released_at_with_hook(root, identity, &mut |_| Ok(()))
}

fn retire_released_at_with_hook(
    root: &Path,
    identity: ClaimMutationIdentity<'_>,
    after_issue_detach: &mut impl FnMut(&Path) -> Result<(), CommandFailure>,
) -> Result<(), CommandFailure> {
    let Some(root) = open_existing_private_directory(root)? else {
        return Ok(());
    };
    let repo_name = repository_progress_key(identity.repo);
    let Some(repo) = open_existing_private_child(&root, &repo_name)? else {
        return Ok(());
    };
    let _lock = RepositoryLock::acquire(&repo)?;
    let issue_name = format!("{}.json", identity.issue);
    let Some(issue_stage) = detach_heartbeat(&repo, &issue_name)? else {
        return Ok(());
    };
    if let Err(error) = after_issue_detach(&repo.path.join(&issue_name)) {
        restore_detached_heartbeat(&repo, &issue_stage, &issue_name)?;
        return Err(error);
    }
    let evidence = match detached_retirement_evidence(&repo, &issue_stage, identity) {
        Ok(Some(evidence)) => evidence,
        Ok(None) => {
            restore_detached_heartbeat(&repo, &issue_stage, &issue_name)?;
            return Ok(());
        }
        Err(error) => {
            restore_detached_heartbeat(&repo, &issue_stage, &issue_name)?;
            return Err(error);
        }
    };
    if let Some(session_id) = evidence.session_id.as_deref() {
        match retire_matching_session(&repo, session_id, identity) {
            Ok(true) => {}
            Ok(false) => {
                restore_detached_heartbeat(&repo, &issue_stage, &issue_name)?;
                return Ok(());
            }
            Err(error) => {
                restore_detached_heartbeat(&repo, &issue_stage, &issue_name)?;
                return Err(error);
            }
        }
    }
    remove_detached_heartbeat(&repo, &issue_stage)?;
    sync_private_directory(&repo)
}

fn retire_matching_session(
    repo: &PrivateDirectory,
    session_id: &str,
    identity: ClaimMutationIdentity<'_>,
) -> Result<bool, CommandFailure> {
    let Some(sessions) = open_existing_private_child(repo, "sessions")? else {
        return Ok(true);
    };
    let session_name = format!("{}.json", heartbeat_session_key(session_id));
    let Some(session_stage) = detach_heartbeat(&sessions, &session_name)? else {
        return Ok(true);
    };
    match detached_retirement_evidence(&sessions, &session_stage, identity) {
        Ok(Some(_)) => {}
        Ok(None) => {
            restore_detached_heartbeat(&sessions, &session_stage, &session_name)?;
            return Ok(false);
        }
        Err(error) => {
            restore_detached_heartbeat(&sessions, &session_stage, &session_name)?;
            return Err(error);
        }
    }
    if let Err(error) = remove_detached_heartbeat(&sessions, &session_stage) {
        restore_detached_heartbeat(&sessions, &session_stage, &session_name)?;
        return Err(error);
    }
    sync_private_directory(&sessions)?;
    Ok(true)
}

fn detached_retirement_evidence(
    directory: &PrivateDirectory,
    name: &str,
    identity: ClaimMutationIdentity<'_>,
) -> Result<Option<StartupHeartbeatEvidence>, CommandFailure> {
    let document = read_private_file_in(directory, name)
        .map_err(|error| CommandFailure::diagnostic(format!("read released heartbeat: {error}")))?;
    let Some(evidence) = parse_startup_heartbeat(&document) else {
        return Ok(None);
    };
    Ok(exact_retirement_identity(&evidence, identity).then_some(evidence))
}

fn exact_retirement_identity(
    evidence: &StartupHeartbeatEvidence,
    identity: ClaimMutationIdentity<'_>,
) -> bool {
    evidence.repo == identity.repo
        && evidence.issue == identity.issue.to_string()
        && evidence.worker_id == identity.worker_id
        && evidence.branch == identity.branch
        && evidence.claim_id == identity.claim_id
}

fn detach_heartbeat(
    directory: &PrivateDirectory,
    live_name: &str,
) -> Result<Option<String>, CommandFailure> {
    let staged_name = format!(
        ".autospec-retiring-{}-{}",
        std::process::id(),
        TEMPORARY_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    #[cfg(unix)]
    {
        use std::ffi::CString;
        use std::os::fd::AsRawFd;
        let live = CString::new(live_name)
            .map_err(|_| CommandFailure::diagnostic("heartbeat live name contains NUL"))?;
        let staged = CString::new(staged_name.as_str())
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
        let source = directory.path.join(live_name);
        let staged = directory.path.join(&staged_name);
        match move_file_exclusive(&source, &staged) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(CommandFailure::diagnostic(format!(
                    "detach released heartbeat: {error}"
                )))
            }
        }
    }
    sync_private_directory(directory)?;
    Ok(Some(staged_name))
}

fn restore_detached_heartbeat(
    directory: &PrivateDirectory,
    staged_name: &str,
    live_name: &str,
) -> Result<(), CommandFailure> {
    #[cfg(unix)]
    {
        use nix::fcntl::AtFlags;
        use nix::unistd::{linkat, unlinkat, UnlinkatFlags};
        match linkat(
            &directory.file,
            staged_name,
            &directory.file,
            live_name,
            AtFlags::empty(),
        ) {
            Ok(()) => {
                unlinkat(&directory.file, staged_name, UnlinkatFlags::NoRemoveDir).map_err(
                    |error| {
                        CommandFailure::diagnostic(format!(
                            "remove restored heartbeat staging: {error}"
                        ))
                    },
                )?;
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
        let staged = directory.path.join(staged_name);
        let live = directory.path.join(live_name);
        match atomic_rename_exclusive(&staged, &live) {
            Ok(()) => {}
            Err(_) if fs::symlink_metadata(&live).is_ok() => return Ok(()),
            Err(error) => return Err(error),
        }
    }
    sync_private_directory(directory)
}

fn remove_detached_heartbeat(
    directory: &PrivateDirectory,
    staged_name: &str,
) -> Result<(), CommandFailure> {
    #[cfg(unix)]
    nix::unistd::unlinkat(
        &directory.file,
        staged_name,
        nix::unistd::UnlinkatFlags::NoRemoveDir,
    )
    .map_err(|error| CommandFailure::diagnostic(format!("remove detached heartbeat: {error}")))?;
    #[cfg(windows)]
    fs::remove_file(directory.path.join(staged_name)).map_err(|error| {
        CommandFailure::diagnostic(format!("remove detached heartbeat: {error}"))
    })?;
    Ok(())
}

#[cfg(unix)]
fn read_private_file_in(directory: &PrivateDirectory, name: &str) -> std::io::Result<Vec<u8>> {
    use nix::fcntl::{openat, OFlag};
    use nix::sys::stat::Mode;
    let mut file = fs::File::from(
        openat(
            &directory.file,
            Path::new(name),
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
fn read_private_file_in(directory: &PrivateDirectory, name: &str) -> std::io::Result<Vec<u8>> {
    let path = directory.path.join(name);
    let metadata = fs::symlink_metadata(&path)?;
    if !metadata.file_type().is_file() || !private_file_metadata(&metadata) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "released heartbeat is not a private regular file",
        ));
    }
    read_file_no_follow(&path)
}

fn sync_private_directory(directory: &PrivateDirectory) -> Result<(), CommandFailure> {
    #[cfg(unix)]
    return directory.file.sync_all().map_err(|error| {
        CommandFailure::diagnostic(format!("heartbeat directory fsync: {error}"))
    });
    #[cfg(windows)]
    sync_directory(&directory.path)
}

#[cfg(windows)]
fn move_file_exclusive(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "Kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(source: *const u16, destination: *const u16, flags: u32) -> i32;
    }
    const MOVEFILE_WRITE_THROUGH: u32 = 0x0000_0008;
    let source = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    // SAFETY: both paths are NUL-terminated and REPLACE_EXISTING is intentionally absent.
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::claim::ClaimMutationIdentity;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

    struct Fixture {
        root: std::path::PathBuf,
    }

    impl Fixture {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "autospec-heartbeat-portable-{name}-{}-{}",
                std::process::id(),
                NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&root).expect("private heartbeat root");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
                    .expect("private root permissions");
            }
            Self { root }
        }

        fn document(&self, claim_id: &str, session_id: Option<&str>) -> Vec<u8> {
            let session =
                session_id.map_or_else(String::new, |value| format!(r#","session_id":"{value}""#));
            format!(
                r#"{{"repo":"owner/repo","issue":"42","worker_id":"worker-a","branch":"feat/worker","pr":"","claim_id":"{claim_id}","step":"claimed","ts":1,"ttl_seconds":10,"pid":7,"nonce":"nonce-{claim_id}","host":"host-a","boot_id":"boot-a","process_start":"9"{session}}}"#
            )
            .into_bytes()
        }

        fn issue_path(&self) -> std::path::PathBuf {
            self.root
                .join(crate::commands::autonomous::drain::repository_progress_key(
                    "owner/repo",
                ))
                .join("42.json")
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn publication_is_idempotent_but_rejects_another_generation() {
        let fixture = Fixture::new("generation");
        let first = fixture.document("claim-a", Some("session-a"));
        publish(&fixture.root, "owner/repo", 42, Some("session-a"), &first)
            .expect("initial publication");
        publish(&fixture.root, "owner/repo", 42, Some("session-a"), &first)
            .expect("idempotent replay");

        let conflict = fixture.document("claim-b", Some("session-b"));
        let error = publish(
            &fixture.root,
            "owner/repo",
            42,
            Some("session-b"),
            &conflict,
        )
        .expect_err("generation conflict");

        assert_eq!(error.message, "heartbeat publication target conflicts");
    }

    #[cfg(unix)]
    #[test]
    fn publication_rejects_a_final_symlink() {
        use std::os::unix::fs::{symlink, PermissionsExt};

        let fixture = Fixture::new("symlink");
        let repo = fixture.issue_path().parent().unwrap().to_path_buf();
        std::fs::create_dir(&repo).expect("repo directory");
        std::fs::set_permissions(&repo, std::fs::Permissions::from_mode(0o700))
            .expect("repo permissions");
        let outside = fixture.root.join("outside");
        std::fs::write(&outside, b"caller-owned").expect("outside file");
        symlink(&outside, fixture.issue_path()).expect("final symlink");

        let error = publish(
            &fixture.root,
            "owner/repo",
            42,
            None,
            &fixture.document("claim-a", None),
        )
        .expect_err("final symlink conflict");

        assert_eq!(error.message, "heartbeat publication target conflicts");
        assert_eq!(std::fs::read(outside).unwrap(), b"caller-owned");
    }

    #[test]
    fn retirement_removes_only_the_exact_generation() {
        let fixture = Fixture::new("retirement");
        let document = fixture.document("claim-a", Some("session-a"));
        publish(
            &fixture.root,
            "owner/repo",
            42,
            Some("session-a"),
            &document,
        )
        .expect("heartbeat");

        retire_released_at(
            &fixture.root,
            ClaimMutationIdentity {
                repo: "owner/repo",
                issue: 42,
                worker_id: "worker-a",
                branch: "feat/worker",
                claim_id: "claim-b",
            },
        )
        .expect("mismatch is not retired");
        assert!(fixture.issue_path().exists());

        retire_released_at(
            &fixture.root,
            ClaimMutationIdentity {
                repo: "owner/repo",
                issue: 42,
                worker_id: "worker-a",
                branch: "feat/worker",
                claim_id: "claim-a",
            },
        )
        .expect("exact retirement");
        assert!(!fixture.issue_path().exists());
    }

    #[test]
    fn retirement_of_an_exact_issue_tolerates_a_missing_session_copy() {
        let fixture = Fixture::new("retirement-missing-session");
        let document = fixture.document("claim-a", Some("session-a"));
        publish(
            &fixture.root,
            "owner/repo",
            42,
            Some("session-a"),
            &document,
        )
        .expect("heartbeat");
        std::fs::remove_dir_all(
            fixture
                .issue_path()
                .parent()
                .expect("repo")
                .join("sessions"),
        )
        .expect("remove session copy");

        retire_released_at(
            &fixture.root,
            ClaimMutationIdentity {
                repo: "owner/repo",
                issue: 42,
                worker_id: "worker-a",
                branch: "feat/worker",
                claim_id: "claim-a",
            },
        )
        .expect("exact issue retirement");

        assert!(!fixture.issue_path().exists());
    }

    #[cfg(unix)]
    #[test]
    fn retirement_rejects_an_intermediate_repository_symlink() {
        use std::os::unix::fs::{symlink, PermissionsExt};

        let fixture = Fixture::new("retirement-repo-symlink");
        let repo_name = crate::commands::autonomous::drain::repository_progress_key("owner/repo");
        let outside = fixture.root.join("outside-repo");
        std::fs::create_dir(&outside).expect("outside repo");
        std::fs::set_permissions(&outside, std::fs::Permissions::from_mode(0o700))
            .expect("outside repo permissions");
        let document = fixture.document("claim-a", None);
        std::fs::write(outside.join("42.json"), &document).expect("outside heartbeat");
        std::fs::set_permissions(
            outside.join("42.json"),
            std::fs::Permissions::from_mode(0o600),
        )
        .expect("outside heartbeat permissions");
        symlink(&outside, fixture.root.join(repo_name)).expect("repository symlink");

        let result = retire_released_at(
            &fixture.root,
            ClaimMutationIdentity {
                repo: "owner/repo",
                issue: 42,
                worker_id: "worker-a",
                branch: "feat/worker",
                claim_id: "claim-a",
            },
        );

        assert!(result.is_err(), "intermediate symlink was accepted");
        assert_eq!(
            std::fs::read(outside.join("42.json")).expect("outside heartbeat retained"),
            document
        );
    }

    #[cfg(unix)]
    #[test]
    fn retirement_deletes_the_detached_generation_not_its_replacement() {
        use std::os::unix::fs::PermissionsExt;

        let fixture = Fixture::new("retirement-replacement-race");
        let original = fixture.document("claim-a", None);
        publish(&fixture.root, "owner/repo", 42, None, &original).expect("original heartbeat");
        let replacement = fixture.document("claim-b", None);
        let replacement_path = fixture.issue_path().with_extension("replacement");
        std::fs::write(&replacement_path, &replacement).expect("replacement heartbeat");
        std::fs::set_permissions(&replacement_path, std::fs::Permissions::from_mode(0o600))
            .expect("replacement heartbeat permissions");

        retire_released_at_with_hook(
            &fixture.root,
            ClaimMutationIdentity {
                repo: "owner/repo",
                issue: 42,
                worker_id: "worker-a",
                branch: "feat/worker",
                claim_id: "claim-a",
            },
            &mut |vacated_issue| {
                std::fs::rename(&replacement_path, vacated_issue)
                    .map_err(|error| CommandFailure::diagnostic(error.to_string()))
            },
        )
        .expect("exact detached retirement");

        assert_eq!(
            std::fs::read(fixture.issue_path()).expect("replacement retained"),
            replacement
        );
    }

    #[cfg(unix)]
    #[test]
    fn retirement_rejects_an_intermediate_session_symlink_without_losing_the_issue() {
        use std::os::unix::fs::{symlink, PermissionsExt};

        let fixture = Fixture::new("retirement-session-symlink");
        let document = fixture.document("claim-a", Some("session-a"));
        publish(
            &fixture.root,
            "owner/repo",
            42,
            Some("session-a"),
            &document,
        )
        .expect("heartbeat");
        let repo = fixture.issue_path().parent().expect("repo").to_path_buf();
        std::fs::remove_dir_all(repo.join("sessions")).expect("remove real sessions");
        let outside = fixture.root.join("outside-sessions");
        std::fs::create_dir(&outside).expect("outside sessions");
        std::fs::set_permissions(&outside, std::fs::Permissions::from_mode(0o700))
            .expect("outside sessions permissions");
        let session_name = format!("{}.json", heartbeat_session_key("session-a"));
        std::fs::write(outside.join(&session_name), &document).expect("outside session heartbeat");
        std::fs::set_permissions(
            outside.join(&session_name),
            std::fs::Permissions::from_mode(0o600),
        )
        .expect("outside session permissions");
        symlink(&outside, repo.join("sessions")).expect("sessions symlink");

        let result = retire_released_at(
            &fixture.root,
            ClaimMutationIdentity {
                repo: "owner/repo",
                issue: 42,
                worker_id: "worker-a",
                branch: "feat/worker",
                claim_id: "claim-a",
            },
        );

        assert!(result.is_err(), "intermediate session symlink was accepted");
        assert_eq!(
            std::fs::read(fixture.issue_path()).expect("issue heartbeat restored"),
            document
        );
        assert!(outside.join(session_name).exists());
    }

    #[cfg(unix)]
    #[test]
    fn repository_lock_serializes_retirement_and_publication() {
        let fixture = Fixture::new("retirement-publication-lock");
        let original = fixture.document("claim-a", None);
        publish(&fixture.root, "owner/repo", 42, None, &original).expect("original heartbeat");
        let replacement = fixture.document("claim-b", None);
        let root = fixture.root.clone();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (completed_tx, completed_rx) = std::sync::mpsc::channel();
        let mut publisher = None;

        retire_released_at_with_hook(
            &fixture.root,
            ClaimMutationIdentity {
                repo: "owner/repo",
                issue: 42,
                worker_id: "worker-a",
                branch: "feat/worker",
                claim_id: "claim-a",
            },
            &mut |_| {
                let root = root.clone();
                let replacement = replacement.clone();
                let started_tx = started_tx.clone();
                let completed_tx = completed_tx.clone();
                publisher = Some(std::thread::spawn(move || {
                    started_tx.send(()).expect("publisher started");
                    let result = publish(&root, "owner/repo", 42, None, &replacement);
                    completed_tx.send(result).expect("publisher completed");
                }));
                started_rx
                    .recv_timeout(std::time::Duration::from_secs(1))
                    .expect("publisher reached publication");
                assert!(
                    completed_rx
                        .recv_timeout(std::time::Duration::from_millis(100))
                        .is_err(),
                    "publisher crossed retirement's repository lock"
                );
                Ok(())
            },
        )
        .expect("serialized retirement");

        completed_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("publisher completed after retirement")
            .expect("replacement publication");
        publisher
            .take()
            .expect("publisher handle")
            .join()
            .expect("publisher thread");
        assert_eq!(
            std::fs::read(fixture.issue_path()).expect("replacement heartbeat"),
            replacement
        );
    }

    #[cfg(unix)]
    #[test]
    fn unix_directory_is_private_at_its_creation_boundary() {
        use nix::sys::stat::{umask, Mode};
        use std::os::unix::fs::PermissionsExt;

        const CHILD: &str = "AUTOSPEC_TEST_PORTABLE_PRIVATE_DIRECTORY_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let output = std::process::Command::new(std::env::current_exe().expect("test binary"))
                .args([
                    "--exact",
                    "commands::claim::heartbeat_portable::tests::unix_directory_is_private_at_its_creation_boundary",
                    "--nocapture",
                ])
                .env(CHILD, "1")
                .output()
                .expect("isolated umask test");
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }

        let fixture = Fixture::new("atomic-private-directory");
        let directory = fixture.root.join("created-private");
        let previous = umask(Mode::empty());
        let mut observed_mode = None;
        let result = ensure_private_directory_with_hook(&directory, &mut |created| {
            observed_mode = Some(
                std::fs::symlink_metadata(created)
                    .expect("created directory metadata")
                    .permissions()
                    .mode()
                    & 0o7777,
            );
        });
        umask(previous);
        result.expect("private directory creation");

        assert_eq!(observed_mode, Some(0o700));
    }

    #[cfg(windows)]
    #[test]
    fn concurrent_windows_publication_has_exactly_one_winner() {
        let fixture = std::sync::Arc::new(Fixture::new("windows-exclusive-publication"));
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        let mut publishers = Vec::new();
        for claim_id in ["claim-a", "claim-b"] {
            let fixture = std::sync::Arc::clone(&fixture);
            let barrier = std::sync::Arc::clone(&barrier);
            publishers.push(std::thread::spawn(move || {
                let source = fixture.root.join(format!("{claim_id}.stage"));
                std::fs::write(&source, claim_id).expect("stage heartbeat");
                let destination = fixture.root.join("42.json");
                barrier.wait();
                atomic_rename_exclusive(&source, &destination)
            }));
        }
        barrier.wait();
        let results = publishers
            .into_iter()
            .map(|publisher| publisher.join().expect("publisher thread"))
            .collect::<Vec<_>>();

        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        let winner =
            std::fs::read_to_string(fixture.root.join("42.json")).expect("winning heartbeat");
        assert!(winner == "claim-a" || winner == "claim-b");
    }
}
