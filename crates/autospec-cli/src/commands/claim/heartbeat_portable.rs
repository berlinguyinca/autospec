use super::{
    heartbeat_root, heartbeat_session_key, parse_startup_heartbeat, ClaimMutationIdentity,
    StartupHeartbeatEvidence,
};
use crate::commands::autonomous::drain::repository_progress_key;
use crate::commands::CommandFailure;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

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

fn publish_with_hooks(
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

fn open_or_create_private_child(
    parent: &PrivateDirectory,
    name: &str,
) -> Result<PrivateDirectory, CommandFailure> {
    if name.is_empty() || Path::new(name).is_absolute() || Path::new(name).components().count() != 1
    {
        return Err(CommandFailure::diagnostic(
            "heartbeat directory name must be one normal component",
        ));
    }
    #[cfg(unix)]
    match nix::sys::stat::mkdirat(
        &parent.file,
        name,
        nix::sys::stat::Mode::from_bits_truncate(0o700),
    ) {
        Ok(()) | Err(nix::errno::Errno::EEXIST) => {}
        Err(error) => {
            return Err(CommandFailure::diagnostic(format!(
                "could not create heartbeat directory: {error}"
            )))
        }
    }
    #[cfg(windows)]
    {
        let _ = open_windows_relative(
            &parent.file,
            name,
            WINDOWS_FILE_LIST_DIRECTORY | WINDOWS_FILE_READ_ATTRIBUTES | WINDOWS_SYNCHRONIZE,
            WINDOWS_FILE_OPEN_IF,
            WINDOWS_FILE_DIRECTORY_FILE
                | WINDOWS_FILE_OPEN_REPARSE_POINT
                | WINDOWS_FILE_SYNCHRONOUS_IO_NONALERT,
        )
        .map_err(|error| {
            CommandFailure::diagnostic(format!("could not create heartbeat directory: {error}"))
        })?;
    }
    open_existing_private_child(parent, name)?.ok_or_else(|| {
        CommandFailure::diagnostic("heartbeat child directory disappeared after creation")
    })
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
        let file = open_windows_directory(path).map_err(|error| {
            CommandFailure::diagnostic(format!("heartbeat directory secure open: {error}"))
        })?;
        let metadata = file.metadata().map_err(|error| {
            CommandFailure::diagnostic(format!("could not inspect heartbeat directory: {error}"))
        })?;
        if !metadata.file_type().is_dir() || !private_directory_metadata(&metadata) {
            return Err(CommandFailure::diagnostic(
                "heartbeat publication directory is not private",
            ));
        }
        Ok(Some(PrivateDirectory {
            path: path.to_path_buf(),
            file,
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
    #[cfg(unix)]
    {
        use nix::fcntl::{openat, OFlag};
        use nix::sys::stat::Mode;
        let file = match openat(
            &parent.file,
            Path::new(name),
            OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
            Mode::empty(),
        ) {
            Ok(file) => fs::File::from(file),
            Err(nix::errno::Errno::ENOENT) => return Ok(None),
            Err(error) => {
                return Err(CommandFailure::diagnostic(format!(
                    "heartbeat directory secure open: {error}"
                )))
            }
        };
        super::private_heartbeat_directory_identity(&file, "portable child directory")?;
        Ok(Some(PrivateDirectory { path, file }))
    }
    #[cfg(windows)]
    {
        let file = match open_windows_relative(
            &parent.file,
            name,
            WINDOWS_FILE_LIST_DIRECTORY | WINDOWS_FILE_READ_ATTRIBUTES | WINDOWS_SYNCHRONIZE,
            WINDOWS_FILE_OPEN,
            WINDOWS_FILE_DIRECTORY_FILE
                | WINDOWS_FILE_OPEN_REPARSE_POINT
                | WINDOWS_FILE_SYNCHRONOUS_IO_NONALERT,
        ) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(CommandFailure::diagnostic(format!(
                    "heartbeat directory secure open: {error}"
                )))
            }
        };
        let metadata = file.metadata().map_err(|error| {
            CommandFailure::diagnostic(format!("could not inspect heartbeat directory: {error}"))
        })?;
        if !metadata.file_type().is_dir() || !private_directory_metadata(&metadata) {
            return Err(CommandFailure::diagnostic(
                "heartbeat publication directory is not private",
            ));
        }
        Ok(Some(PrivateDirectory { path, file }))
    }
}

#[cfg(windows)]
const WINDOWS_DELETE: u32 = 0x0001_0000;
#[cfg(windows)]
const WINDOWS_FILE_LIST_DIRECTORY: u32 = 0x0000_0001;
#[cfg(windows)]
const WINDOWS_FILE_READ_ATTRIBUTES: u32 = 0x0000_0080;
#[cfg(windows)]
const WINDOWS_GENERIC_READ: u32 = 0x8000_0000;
#[cfg(windows)]
const WINDOWS_GENERIC_WRITE: u32 = 0x4000_0000;
#[cfg(windows)]
const WINDOWS_SYNCHRONIZE: u32 = 0x0010_0000;
#[cfg(windows)]
const WINDOWS_FILE_OPEN: u32 = 0x0000_0001;
#[cfg(windows)]
const WINDOWS_FILE_OPEN_IF: u32 = 0x0000_0003;
#[cfg(windows)]
const WINDOWS_FILE_DIRECTORY_FILE: u32 = 0x0000_0001;
#[cfg(windows)]
const WINDOWS_FILE_SYNCHRONOUS_IO_NONALERT: u32 = 0x0000_0020;
#[cfg(windows)]
const WINDOWS_FILE_NON_DIRECTORY_FILE: u32 = 0x0000_0040;
#[cfg(windows)]
const WINDOWS_FILE_OPEN_REPARSE_POINT: u32 = 0x0020_0000;

#[cfg(windows)]
fn open_windows_directory(path: &Path) -> std::io::Result<fs::File> {
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::FromRawHandle;
    const FILE_SHARE_ALL: u32 = 0x0000_0007;
    const OPEN_EXISTING: u32 = 3;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    let path = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    // SAFETY: path is NUL-terminated and all optional pointers are null.
    let handle = unsafe {
        CreateFileW(
            path.as_ptr(),
            WINDOWS_FILE_LIST_DIRECTORY | WINDOWS_FILE_READ_ATTRIBUTES | WINDOWS_SYNCHRONIZE,
            FILE_SHARE_ALL,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            std::ptr::null_mut(),
        )
    };
    if handle == windows_invalid_handle_value() {
        Err(std::io::Error::last_os_error())
    } else {
        // SAFETY: CreateFileW returned a new owned handle.
        Ok(unsafe { fs::File::from_raw_handle(handle) })
    }
}

#[cfg(windows)]
fn open_windows_relative(
    directory: &fs::File,
    name: &str,
    desired_access: u32,
    disposition: u32,
    options: u32,
) -> std::io::Result<fs::File> {
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle};
    if name.is_empty() || Path::new(name).components().count() != 1 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "heartbeat name must be one normal component",
        ));
    }
    let mut name = Path::new(name)
        .as_os_str()
        .encode_wide()
        .collect::<Vec<_>>();
    let byte_length = name
        .len()
        .checked_mul(std::mem::size_of::<u16>())
        .and_then(|length| u16::try_from(length).ok())
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "heartbeat name is too long",
            )
        })?;
    let mut unicode = WindowsUnicodeString {
        length: byte_length,
        maximum_length: byte_length,
        buffer: name.as_mut_ptr(),
    };
    let mut attributes = WindowsObjectAttributes {
        length: std::mem::size_of::<WindowsObjectAttributes>() as u32,
        root_directory: directory.as_raw_handle(),
        object_name: &mut unicode,
        attributes: 0x0000_0040,
        security_descriptor: std::ptr::null_mut(),
        security_quality_of_service: std::ptr::null_mut(),
    };
    let mut status_block = WindowsIoStatusBlock {
        status: 0,
        information: 0,
    };
    let mut handle = windows_invalid_handle_value();
    // SAFETY: every pointer refers to initialized storage for the duration of the call;
    // the object name is relative to the live directory handle.
    let status = unsafe {
        NtCreateFile(
            &mut handle,
            desired_access,
            &mut attributes,
            &mut status_block,
            std::ptr::null_mut(),
            0x0000_0080,
            0x0000_0007,
            disposition,
            options,
            std::ptr::null_mut(),
            0,
        )
    };
    if status < 0 {
        // SAFETY: conversion is a pure mapping from the returned NTSTATUS.
        let error = unsafe { RtlNtStatusToDosError(status) };
        Err(std::io::Error::from_raw_os_error(error as i32))
    } else {
        // SAFETY: NtCreateFile returned a new owned handle on success.
        Ok(unsafe { fs::File::from_raw_handle(handle) })
    }
}

#[cfg(windows)]
fn windows_invalid_handle_value() -> std::os::windows::io::RawHandle {
    (-1_isize) as std::os::windows::io::RawHandle
}

#[cfg(windows)]
#[repr(C)]
struct WindowsUnicodeString {
    length: u16,
    maximum_length: u16,
    buffer: *mut u16,
}

#[cfg(windows)]
#[repr(C)]
struct WindowsObjectAttributes {
    length: u32,
    root_directory: std::os::windows::io::RawHandle,
    object_name: *mut WindowsUnicodeString,
    attributes: u32,
    security_descriptor: *mut std::ffi::c_void,
    security_quality_of_service: *mut std::ffi::c_void,
}

#[cfg(windows)]
#[repr(C)]
struct WindowsIoStatusBlock {
    status: usize,
    information: usize,
}

#[cfg(all(windows, target_pointer_width = "64"))]
const _: () = {
    assert!(std::mem::size_of::<WindowsUnicodeString>() == 16);
    assert!(std::mem::size_of::<WindowsObjectAttributes>() == 48);
    assert!(std::mem::size_of::<WindowsIoStatusBlock>() == 16);
};

#[cfg(windows)]
#[link(name = "Kernel32")]
unsafe extern "system" {
    fn CreateFileW(
        file_name: *const u16,
        desired_access: u32,
        share_mode: u32,
        security_attributes: *const std::ffi::c_void,
        creation_disposition: u32,
        flags_and_attributes: u32,
        template_file: std::os::windows::io::RawHandle,
    ) -> std::os::windows::io::RawHandle;
}

#[cfg(windows)]
#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtCreateFile(
        file_handle: *mut std::os::windows::io::RawHandle,
        desired_access: u32,
        object_attributes: *mut WindowsObjectAttributes,
        io_status_block: *mut WindowsIoStatusBlock,
        allocation_size: *mut i64,
        file_attributes: u32,
        share_access: u32,
        create_disposition: u32,
        create_options: u32,
        ea_buffer: *mut std::ffi::c_void,
        ea_length: u32,
    ) -> i32;
    fn RtlNtStatusToDosError(status: i32) -> u32;
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
            use std::os::windows::io::AsRawHandle;
            let file = open_windows_relative(
                &directory.file,
                ".portable-heartbeat.lock",
                WINDOWS_GENERIC_READ | WINDOWS_GENERIC_WRITE | WINDOWS_SYNCHRONIZE,
                WINDOWS_FILE_OPEN_IF,
                WINDOWS_FILE_NON_DIRECTORY_FILE
                    | WINDOWS_FILE_OPEN_REPARSE_POINT
                    | WINDOWS_FILE_SYNCHRONOUS_IO_NONALERT,
            )
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

fn existing_generation(
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

fn same_generation(left: &StartupHeartbeatEvidence, right: &StartupHeartbeatEvidence) -> bool {
    let mut left = left.clone();
    let mut right = right.clone();
    for evidence in [&mut left, &mut right] {
        evidence.ts = 0;
    }
    left == right
}

static NEXT_STAGE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn create_unique_stage(
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

fn reconcile_staged_publications(
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

fn staged_document_targets(observed: &StartupHeartbeatEvidence, destination: &str) -> bool {
    if destination == format!("{}.json", observed.issue) {
        return true;
    }
    observed.session_id.as_deref().is_some_and(|session_id| {
        destination == format!("{}.json", heartbeat_session_key(session_id))
    })
}

fn staged_publication_names(
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

#[cfg(unix)]
fn file_link_count(file: &fs::File) -> Result<u64, CommandFailure> {
    use std::os::unix::fs::MetadataExt;
    file.metadata()
        .map(|metadata| metadata.nlink())
        .map_err(|error| CommandFailure::diagnostic(format!("heartbeat stage inspect: {error}")))
}

#[cfg(windows)]
fn file_link_count(file: &fs::File) -> Result<u64, CommandFailure> {
    windows_file_identity(file).map(|identity| u64::from(identity.2))
}

#[cfg(unix)]
fn private_stage_metadata(metadata: &fs::Metadata, links: u64) -> bool {
    private_file_metadata_with_links(metadata, links)
}

#[cfg(windows)]
fn private_stage_metadata(metadata: &fs::Metadata, _links: u64) -> bool {
    private_file_metadata(metadata)
}

#[cfg(unix)]
fn unlink_staged_file(
    directory: &PrivateDirectory,
    name: &str,
    _file: &fs::File,
) -> Result<(), CommandFailure> {
    nix::unistd::unlinkat(
        &directory.file,
        name,
        nix::unistd::UnlinkatFlags::NoRemoveDir,
    )
    .map_err(|error| CommandFailure::diagnostic(format!("heartbeat stage cleanup: {error}")))
}

#[cfg(windows)]
fn unlink_staged_file(
    _directory: &PrivateDirectory,
    _name: &str,
    file: &fs::File,
) -> Result<(), CommandFailure> {
    delete_windows_file_handle(file)
        .map_err(|error| CommandFailure::diagnostic(format!("heartbeat stage cleanup: {error}")))
}

#[cfg(unix)]
fn create_private_file_relative(
    directory: &PrivateDirectory,
    name: &str,
) -> Result<fs::File, CommandFailure> {
    use nix::fcntl::{openat, OFlag};
    use nix::sys::stat::Mode;
    let descriptor = openat(
        &directory.file,
        Path::new(name),
        OFlag::O_CREAT | OFlag::O_EXCL | OFlag::O_RDWR | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
        Mode::from_bits_truncate(0o600),
    )
    .map_err(std::io::Error::from)
    .map_err(|error| CommandFailure::diagnostic(format!("heartbeat stage create: {error}")))?;
    Ok(fs::File::from(descriptor))
}

#[cfg(windows)]
fn create_private_file_relative(
    directory: &PrivateDirectory,
    name: &str,
) -> Result<fs::File, CommandFailure> {
    const WINDOWS_FILE_CREATE: u32 = 0x0000_0002;
    open_windows_relative(
        &directory.file,
        name,
        WINDOWS_GENERIC_READ | WINDOWS_GENERIC_WRITE | WINDOWS_DELETE | WINDOWS_SYNCHRONIZE,
        WINDOWS_FILE_CREATE,
        WINDOWS_FILE_NON_DIRECTORY_FILE
            | WINDOWS_FILE_OPEN_REPARSE_POINT
            | WINDOWS_FILE_SYNCHRONOUS_IO_NONALERT,
    )
    .map_err(|error| CommandFailure::diagnostic(format!("heartbeat stage create: {error}")))
}

#[cfg(unix)]
fn open_private_file_relative(
    directory: &PrivateDirectory,
    name: &str,
) -> std::io::Result<fs::File> {
    use nix::fcntl::{openat, OFlag};
    use nix::sys::stat::Mode;
    Ok(fs::File::from(
        openat(
            &directory.file,
            Path::new(name),
            OFlag::O_RDONLY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC | OFlag::O_NONBLOCK,
            Mode::empty(),
        )
        .map_err(std::io::Error::from)?,
    ))
}

#[cfg(unix)]
fn open_staged_file_relative(
    directory: &PrivateDirectory,
    name: &str,
) -> std::io::Result<fs::File> {
    open_private_file_relative(directory, name)
}

#[cfg(windows)]
fn open_private_file_relative(
    directory: &PrivateDirectory,
    name: &str,
) -> std::io::Result<fs::File> {
    open_windows_relative(
        &directory.file,
        name,
        WINDOWS_GENERIC_READ | WINDOWS_SYNCHRONIZE,
        WINDOWS_FILE_OPEN,
        WINDOWS_FILE_NON_DIRECTORY_FILE
            | WINDOWS_FILE_OPEN_REPARSE_POINT
            | WINDOWS_FILE_SYNCHRONOUS_IO_NONALERT,
    )
}

#[cfg(windows)]
fn open_staged_file_relative(
    directory: &PrivateDirectory,
    name: &str,
) -> std::io::Result<fs::File> {
    open_windows_relative(
        &directory.file,
        name,
        WINDOWS_GENERIC_READ | WINDOWS_GENERIC_WRITE | WINDOWS_DELETE | WINDOWS_SYNCHRONIZE,
        WINDOWS_FILE_OPEN,
        WINDOWS_FILE_NON_DIRECTORY_FILE
            | WINDOWS_FILE_OPEN_REPARSE_POINT
            | WINDOWS_FILE_SYNCHRONOUS_IO_NONALERT,
    )
}

#[cfg(unix)]
fn private_file_metadata(metadata: &fs::Metadata) -> bool {
    private_file_metadata_with_links(metadata, 1)
}

#[cfg(unix)]
fn private_file_metadata_with_links(metadata: &fs::Metadata, links: u64) -> bool {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    metadata.uid() == unsafe { nix::libc::geteuid() }
        && metadata.permissions().mode() & 0o7777 == 0o600
        && metadata.nlink() == links
}

#[cfg(unix)]
fn private_file_handle(_file: &fs::File) -> Result<bool, CommandFailure> {
    Ok(true)
}

#[cfg(unix)]
fn same_file_identity(left: &fs::File, right: &fs::File) -> Result<bool, CommandFailure> {
    use std::os::unix::fs::MetadataExt;
    let left = left.metadata().map_err(|error| {
        CommandFailure::diagnostic(format!("heartbeat source identity: {error}"))
    })?;
    let right = right.metadata().map_err(|error| {
        CommandFailure::diagnostic(format!("heartbeat target identity: {error}"))
    })?;
    Ok(left.dev() == right.dev() && left.ino() == right.ino())
}

#[cfg(windows)]
fn private_file_metadata(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0
}

#[cfg(windows)]
fn private_file_handle(file: &fs::File) -> Result<bool, CommandFailure> {
    Ok(windows_file_identity(file)?.2 == 1)
}

#[cfg(windows)]
fn same_file_identity(left: &fs::File, right: &fs::File) -> Result<bool, CommandFailure> {
    let left = windows_file_identity(left)?;
    let right = windows_file_identity(right)?;
    Ok(left.0 == right.0 && left.1 == right.1)
}

#[cfg(windows)]
fn windows_file_identity(file: &fs::File) -> Result<(u32, u64, u32), CommandFailure> {
    use std::os::windows::io::AsRawHandle;
    let mut information = WindowsByHandleFileInformation::zeroed();
    // SAFETY: file is a live handle and information is writable for the declared structure.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut information) } == 0 {
        return Err(CommandFailure::diagnostic(format!(
            "heartbeat file identity: {}",
            std::io::Error::last_os_error()
        )));
    }
    Ok((
        information.volume_serial_number,
        ((information.file_index_high as u64) << 32) | information.file_index_low as u64,
        information.number_of_links,
    ))
}

#[cfg(windows)]
#[repr(C)]
struct WindowsByHandleFileInformation {
    file_attributes: u32,
    creation_time_low: u32,
    creation_time_high: u32,
    access_time_low: u32,
    access_time_high: u32,
    write_time_low: u32,
    write_time_high: u32,
    volume_serial_number: u32,
    file_size_high: u32,
    file_size_low: u32,
    number_of_links: u32,
    file_index_high: u32,
    file_index_low: u32,
}

#[cfg(windows)]
impl WindowsByHandleFileInformation {
    fn zeroed() -> Self {
        // SAFETY: this C information structure is valid when initialized to all zeroes.
        unsafe { std::mem::zeroed() }
    }
}

#[cfg(windows)]
#[link(name = "Kernel32")]
unsafe extern "system" {
    fn GetFileInformationByHandle(
        file: std::os::windows::io::RawHandle,
        information: *mut WindowsByHandleFileInformation,
    ) -> i32;
}

#[cfg(target_os = "macos")]
fn atomic_rename_exclusive(
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
static FREEBSD_CRASH_AFTER_LINK: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

#[cfg(target_os = "freebsd")]
fn atomic_rename_exclusive(
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
fn atomic_rename_exclusive(
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
fn atomic_rename_exclusive(
    directory: &PrivateDirectory,
    _source: &str,
    destination: &str,
    file: &fs::File,
) -> Result<(), CommandFailure> {
    rename_windows_file_handle(file, &directory.file, destination)
        .map_err(|error| CommandFailure::diagnostic(format!("heartbeat atomic publish: {error}")))
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
    retire_released_at_with_boundary_hooks(
        root,
        identity,
        &mut |_| Ok(()),
        after_issue_detach,
        &mut |_| Ok(()),
    )
}

fn retire_released_at_with_boundary_hooks(
    root: &Path,
    identity: ClaimMutationIdentity<'_>,
    after_repo_open: &mut impl FnMut(&Path) -> Result<(), CommandFailure>,
    after_issue_detach: &mut impl FnMut(&Path) -> Result<(), CommandFailure>,
    after_sessions_open: &mut impl FnMut(&Path) -> Result<(), CommandFailure>,
) -> Result<(), CommandFailure> {
    let Some(root) = open_existing_private_directory(root)? else {
        return Ok(());
    };
    let repo_name = repository_progress_key(identity.repo);
    let Some(repo) = open_existing_private_child(&root, &repo_name)? else {
        return Ok(());
    };
    after_repo_open(&repo.path)?;
    let _lock = RepositoryLock::acquire(&repo)?;
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
        match retire_matching_session(&repo, session_id, identity, after_sessions_open) {
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
    after_sessions_open: &mut impl FnMut(&Path) -> Result<(), CommandFailure>,
) -> Result<bool, CommandFailure> {
    let Some(sessions) = open_existing_private_child(repo, "sessions")? else {
        return Ok(true);
    };
    after_sessions_open(&sessions.path)?;
    let session_name = format!("{}.json", heartbeat_session_key(session_id));
    let session_stage_name = retirement_stage_name(&session_name, identity);
    let session_stage = match open_detached_heartbeat(&sessions, &session_stage_name)? {
        Some(stage) => stage,
        None => match detach_heartbeat(&sessions, &session_name, &session_stage_name)? {
            Some(stage) => stage,
            None => return Ok(true),
        },
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

struct DetachedHeartbeat {
    name: String,
    #[cfg(windows)]
    file: fs::File,
}

fn retirement_stage_name(live_name: &str, identity: ClaimMutationIdentity<'_>) -> String {
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

fn detach_heartbeat(
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

fn open_detached_heartbeat(
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

fn restore_detached_heartbeat(
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

fn remove_detached_heartbeat(
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
fn read_detached_private_file(
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
fn read_detached_private_file(
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

fn sync_private_directory(directory: &PrivateDirectory) -> Result<(), CommandFailure> {
    #[cfg(unix)]
    return directory.file.sync_all().map_err(|error| {
        CommandFailure::diagnostic(format!("heartbeat directory fsync: {error}"))
    });
    #[cfg(windows)]
    match directory.file.sync_all() {
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

#[cfg(windows)]
fn rename_windows_file_handle(
    file: &fs::File,
    directory: &fs::File,
    destination_name: &str,
) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::AsRawHandle;
    if destination_name.is_empty() || Path::new(destination_name).components().count() != 1 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "heartbeat destination must be one normal component",
        ));
    }
    let destination = Path::new(destination_name)
        .as_os_str()
        .encode_wide()
        .collect::<Vec<_>>();
    let name_bytes = destination
        .len()
        .checked_mul(std::mem::size_of::<u16>())
        .and_then(|length| u32::try_from(length).ok())
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "heartbeat destination is too long",
            )
        })?;
    let header_size = std::mem::offset_of!(WindowsFileRenameInfo, file_name);
    let mut buffer = vec![0_u8; header_size + name_bytes as usize];
    let info = buffer.as_mut_ptr().cast::<WindowsFileRenameInfo>();
    // SAFETY: buffer is sized for the fixed header plus the complete UTF-16 name.
    unsafe {
        (*info).flags = 0;
        (*info).root_directory = directory.as_raw_handle();
        (*info).file_name_length = name_bytes;
        std::ptr::copy_nonoverlapping(
            destination.as_ptr(),
            (*info).file_name.as_mut_ptr(),
            destination.len(),
        );
    }
    // SAFETY: file and directory are live handles and buffer contains FILE_RENAME_INFO.
    if unsafe {
        SetFileInformationByHandle(
            file.as_raw_handle(),
            3,
            buffer.as_mut_ptr().cast(),
            buffer.len() as u32,
        )
    } == 0
    {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(windows)]
fn delete_windows_file_handle(file: &fs::File) -> std::io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    let mut delete_file: u8 = 1;
    // SAFETY: file is a live DELETE-capable handle and delete_file is FILE_DISPOSITION_INFO.
    if unsafe {
        SetFileInformationByHandle(
            file.as_raw_handle(),
            4,
            (&mut delete_file as *mut u8).cast(),
            std::mem::size_of_val(&delete_file) as u32,
        )
    } == 0
    {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(windows)]
#[repr(C)]
struct WindowsFileRenameInfo {
    flags: u32,
    root_directory: std::os::windows::io::RawHandle,
    file_name_length: u32,
    file_name: [u16; 1],
}

#[cfg(all(windows, target_pointer_width = "64"))]
const _: () = {
    assert!(std::mem::size_of::<WindowsFileRenameInfo>() == 24);
    assert!(std::mem::offset_of!(WindowsFileRenameInfo, file_name) == 20);
};

#[cfg(windows)]
#[link(name = "Kernel32")]
unsafe extern "system" {
    fn SetFileInformationByHandle(
        file: std::os::windows::io::RawHandle,
        information_class: i32,
        information: *mut std::ffi::c_void,
        buffer_size: u32,
    ) -> i32;
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

        fn repo_path(&self) -> std::path::PathBuf {
            self.root.join(repository_progress_key("owner/repo"))
        }

        fn staging_paths(&self) -> Vec<std::path::PathBuf> {
            std::fs::read_dir(self.repo_path())
                .expect("repository entries")
                .filter_map(Result::ok)
                .filter(|entry| {
                    let name = entry.file_name();
                    let name = name.to_string_lossy();
                    name.starts_with(".autospec-heartbeat-") && name.ends_with(".stage")
                })
                .map(|entry| entry.path())
                .collect()
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

    #[test]
    fn publication_rejects_process_identity_changes_but_allows_timestamp_refreshes() {
        let fixture = Fixture::new("immutable-process-identity");
        let original = fixture.document("claim-a", None);
        publish(&fixture.root, "owner/repo", 42, None, &original).expect("initial publication");

        let timestamp_refresh = String::from_utf8(original.clone())
            .expect("heartbeat is UTF-8")
            .replace(r#""ts":1"#, r#""ts":2"#)
            .into_bytes();
        publish(&fixture.root, "owner/repo", 42, None, &timestamp_refresh)
            .expect("timestamp-only replay");

        for changed in [
            String::from_utf8(original.clone())
                .expect("heartbeat is UTF-8")
                .replace(r#""pid":7"#, r#""pid":8"#)
                .into_bytes(),
            String::from_utf8(original.clone())
                .expect("heartbeat is UTF-8")
                .replace(r#""process_start":"9""#, r#""process_start":"10""#)
                .into_bytes(),
        ] {
            let error = publish(&fixture.root, "owner/repo", 42, None, &changed)
                .expect_err("process identity change must conflict");
            assert_eq!(error.message, "heartbeat publication target conflicts");
            assert_eq!(
                std::fs::read(fixture.issue_path()).expect("original heartbeat retained"),
                original
            );
        }
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

    #[test]
    fn retirement_resumes_after_crash_between_issue_and_session_detachment() {
        let fixture = Fixture::new("retirement-resume-after-issue-detach");
        let document = fixture.document("claim-a", Some("session-a"));
        publish(
            &fixture.root,
            "owner/repo",
            42,
            Some("session-a"),
            &document,
        )
        .expect("heartbeat");
        let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = retire_released_at_with_hook(
                &fixture.root,
                ClaimMutationIdentity {
                    repo: "owner/repo",
                    issue: 42,
                    worker_id: "worker-a",
                    branch: "feat/worker",
                    claim_id: "claim-a",
                },
                &mut |_| panic!("simulated retirement crash after issue detachment"),
            );
        }));
        assert!(interrupted.is_err());
        assert!(!fixture.issue_path().exists(), "issue was not detached");

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
        .expect("resume exact retirement");

        let session = fixture
            .repo_path()
            .join("sessions")
            .join(format!("{}.json", heartbeat_session_key("session-a")));
        assert!(!session.exists(), "resumed retirement left stale session");
        let successor = fixture.document("claim-b", Some("session-b"));
        publish(
            &fixture.root,
            "owner/repo",
            42,
            Some("session-b"),
            &successor,
        )
        .expect("publish successor after resumed retirement");
        assert!(fixture.issue_path().exists());
    }

    #[cfg(unix)]
    #[test]
    fn publication_remains_bound_to_open_repository_after_parent_swap() {
        use std::os::unix::fs::PermissionsExt;

        let fixture = Fixture::new("publication-parent-swap");
        let repo_name = repository_progress_key("owner/repo");
        let repo = fixture.root.join(&repo_name);
        std::fs::create_dir(&repo).expect("repository directory");
        std::fs::set_permissions(&repo, std::fs::Permissions::from_mode(0o700))
            .expect("repository permissions");
        let retained = fixture.root.join("retained-repository");
        let replacement = fixture.root.join("replacement-repository");
        std::fs::create_dir(&replacement).expect("replacement repository");
        std::fs::set_permissions(&replacement, std::fs::Permissions::from_mode(0o700))
            .expect("replacement permissions");
        let document = fixture.document("claim-a", Some("session-a"));

        publish_with_hooks(
            &fixture.root,
            "owner/repo",
            42,
            Some("session-a"),
            &document,
            &mut |_| {
                std::fs::rename(&repo, &retained).expect("retain opened repository");
                std::fs::rename(&replacement, &repo).expect("swap repository path");
            },
            &mut |_| {},
        )
        .expect("handle-bound publication");

        assert!(retained.join("42.json").is_file());
        assert!(retained
            .join("sessions")
            .join(format!("{}.json", heartbeat_session_key("session-a")))
            .is_file());
        assert!(!repo.join("42.json").exists());
        assert!(!repo.join("sessions").exists());
    }

    #[cfg(target_os = "freebsd")]
    #[test]
    fn freebsd_atomic_publication_rejects_destination_collision() {
        use std::os::unix::fs::PermissionsExt;

        let fixture = Fixture::new("freebsd-publication-collision");
        let repo = fixture.repo_path();
        std::fs::create_dir(&repo).expect("repository directory");
        std::fs::set_permissions(&repo, std::fs::Permissions::from_mode(0o700))
            .expect("repository permissions");
        let directory = open_existing_private_directory(&repo)
            .expect("open repository")
            .expect("repository exists");
        let source =
            create_private_file_relative(&directory, ".source.stage").expect("create source stage");
        std::fs::write(repo.join("42.json"), b"destination").expect("destination");
        std::fs::set_permissions(repo.join("42.json"), std::fs::Permissions::from_mode(0o600))
            .expect("destination permissions");

        let error = atomic_rename_exclusive(&directory, ".source.stage", "42.json", &source)
            .expect_err("destination collision");

        assert!(error.message.contains("heartbeat atomic publish"));
        assert_eq!(std::fs::read(repo.join("42.json")).unwrap(), b"destination");
        assert!(repo.join(".source.stage").is_file());
    }

    #[cfg(target_os = "freebsd")]
    #[test]
    fn freebsd_publication_resumes_after_crash_between_link_and_stage_cleanup() {
        let fixture = Fixture::new("freebsd-publication-crash-after-link");
        let document = fixture.document("claim-a", None);
        *FREEBSD_CRASH_AFTER_LINK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(fixture.issue_path());

        let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = publish(&fixture.root, "owner/repo", 42, None, &document);
        }));
        assert!(interrupted.is_err(), "publication did not stop after link");
        assert!(fixture.issue_path().is_file(), "destination was not linked");
        let stages = fixture.staging_paths();
        assert_eq!(stages.len(), 1, "linked stage alias was not retained");
        use std::os::unix::fs::MetadataExt;
        let linked_identity = std::fs::metadata(&stages[0]).expect("linked stage metadata");
        let destination_identity =
            std::fs::metadata(fixture.issue_path()).expect("linked destination metadata");
        assert_eq!(linked_identity.dev(), destination_identity.dev());
        assert_eq!(linked_identity.ino(), destination_identity.ino());
        assert_eq!(linked_identity.nlink(), 2);

        publish(&fixture.root, "owner/repo", 42, None, &document)
            .expect("resume exact publication");

        assert_eq!(std::fs::read(&fixture.issue_path()).unwrap(), document);
        assert!(fixture.staging_paths().is_empty());
        let recovered_identity = std::fs::metadata(fixture.issue_path()).unwrap();
        assert_eq!(recovered_identity.dev(), destination_identity.dev());
        assert_eq!(recovered_identity.ino(), destination_identity.ino());
        assert_eq!(recovered_identity.nlink(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn publication_retry_cleans_crash_staging_aliases() {
        let fixture = Fixture::new("publication-crash-staging");
        let document = fixture.document("claim-a", None);
        let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = publish_with_hooks(
                &fixture.root,
                "owner/repo",
                42,
                None,
                &document,
                &mut |_| {},
                &mut |_| panic!("simulated publication crash before rename"),
            );
        }));
        assert!(interrupted.is_err());

        publish(&fixture.root, "owner/repo", 42, None, &document).expect("restart publication");
        assert!(
            fixture.staging_paths().is_empty(),
            "publication left staging aliases"
        );
    }

    #[test]
    fn successor_publication_reconciles_an_abandoned_pre_rename_stage() {
        let fixture = Fixture::new("publication-successor-after-abandoned-stage");
        let abandoned = fixture.document("claim-a", None);
        let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = publish_with_hooks(
                &fixture.root,
                "owner/repo",
                42,
                None,
                &abandoned,
                &mut |_| {},
                &mut |_| panic!("simulated publication crash before rename"),
            );
        }));
        assert!(interrupted.is_err());

        let successor = fixture.document("claim-b", None);
        publish(&fixture.root, "owner/repo", 42, None, &successor)
            .expect("publish successor generation");

        assert_eq!(
            std::fs::read(fixture.issue_path()).expect("successor heartbeat"),
            successor
        );
        assert!(
            fixture.staging_paths().is_empty(),
            "successor left an abandoned staging file"
        );
    }

    #[cfg(unix)]
    #[test]
    fn stage_reconciliation_ignores_noncanonical_entries_and_symlinks() {
        use std::os::unix::fs::symlink;

        let fixture = Fixture::new("publication-safe-stage-reconciliation");
        let abandoned = fixture.document("claim-a", None);
        let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = publish_with_hooks(
                &fixture.root,
                "owner/repo",
                42,
                None,
                &abandoned,
                &mut |_| {},
                &mut |_| panic!("simulated publication crash before rename"),
            );
        }));
        assert!(interrupted.is_err());

        let caller_owned = fixture.repo_path().join("caller-owned");
        std::fs::write(&caller_owned, b"retain me").expect("caller-owned file");
        let stage_symlink = fixture
            .repo_path()
            .join(".autospec-heartbeat-42.json.00000000000000000000000000000000.stage");
        symlink(&caller_owned, &stage_symlink).expect("canonical-looking stage symlink");

        publish(
            &fixture.root,
            "owner/repo",
            42,
            None,
            &fixture.document("claim-b", None),
        )
        .expect("successor publication");

        assert_eq!(std::fs::read(caller_owned).unwrap(), b"retain me");
        assert!(
            stage_symlink.is_symlink(),
            "stage symlink was followed or removed"
        );
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
                let document = fixture.document(claim_id, None);
                barrier.wait();
                publish(&fixture.root, "owner/repo", 42, None, &document)
            }));
        }
        barrier.wait();
        let results = publishers
            .into_iter()
            .map(|publisher| publisher.join().expect("publisher thread"))
            .collect::<Vec<_>>();

        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        let winner = std::fs::read_to_string(fixture.issue_path()).expect("winning heartbeat");
        assert!(winner.contains("claim-a") || winner.contains("claim-b"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_publication_rejects_multi_link_target() {
        let fixture = Fixture::new("windows-multi-link-target");
        let document = fixture.document("claim-a", None);
        publish(&fixture.root, "owner/repo", 42, None, &document).expect("initial heartbeat");
        std::fs::hard_link(fixture.issue_path(), fixture.repo_path().join("alias.json"))
            .expect("create second link");

        let error = publish(&fixture.root, "owner/repo", 42, None, &document)
            .expect_err("multi-link target must be rejected");

        assert_eq!(error.message, "heartbeat publication target conflicts");
    }

    #[cfg(windows)]
    fn replace_directory_with_junction(path: &Path, replacement: &Path, backup: &Path) {
        std::fs::rename(path, backup).expect("move validated directory aside");
        let output = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(path)
            .arg(replacement)
            .output()
            .expect("create replacement junction");
        assert!(
            output.status.success(),
            "junction creation failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_retirement_stays_bound_when_repository_component_is_replaced() {
        let fixture = Fixture::new("windows-retirement-repo-reparse-race");
        let document = fixture.document("claim-a", None);
        publish(&fixture.root, "owner/repo", 42, None, &document).expect("heartbeat");
        let repo = fixture.repo_path();
        let original_repo = fixture.root.join("original-repo");
        let outside = fixture.root.join("outside-repo");
        std::fs::create_dir(&outside).expect("outside repo");
        std::fs::write(outside.join("42.json"), &document).expect("outside heartbeat");

        retire_released_at_with_boundary_hooks(
            &fixture.root,
            ClaimMutationIdentity {
                repo: "owner/repo",
                issue: 42,
                worker_id: "worker-a",
                branch: "feat/worker",
                claim_id: "claim-a",
            },
            &mut |_| {
                replace_directory_with_junction(&repo, &outside, &original_repo);
                Ok(())
            },
            &mut |_| Ok(()),
            &mut |_| Ok(()),
        )
        .expect("handle-bound repository retirement");

        assert!(
            outside.join("42.json").exists(),
            "replacement target heartbeat was deleted"
        );
        assert!(
            !original_repo.join("42.json").exists(),
            "validated repository heartbeat was not retired"
        );
        std::fs::remove_dir(&repo).expect("remove repository junction");
    }

    #[cfg(windows)]
    #[test]
    fn windows_retirement_stays_bound_when_sessions_component_is_replaced() {
        let fixture = Fixture::new("windows-retirement-session-reparse-race");
        let document = fixture.document("claim-a", Some("session-a"));
        publish(
            &fixture.root,
            "owner/repo",
            42,
            Some("session-a"),
            &document,
        )
        .expect("heartbeat");
        let sessions = fixture.repo_path().join("sessions");
        let original_sessions = fixture.repo_path().join("original-sessions");
        let outside = fixture.root.join("outside-sessions");
        std::fs::create_dir(&outside).expect("outside sessions");
        let session_name = format!("{}.json", heartbeat_session_key("session-a"));
        std::fs::write(outside.join(&session_name), &document).expect("outside heartbeat");

        retire_released_at_with_boundary_hooks(
            &fixture.root,
            ClaimMutationIdentity {
                repo: "owner/repo",
                issue: 42,
                worker_id: "worker-a",
                branch: "feat/worker",
                claim_id: "claim-a",
            },
            &mut |_| Ok(()),
            &mut |_| Ok(()),
            &mut |_| {
                replace_directory_with_junction(&sessions, &outside, &original_sessions);
                Ok(())
            },
        )
        .expect("handle-bound sessions retirement");

        assert!(
            outside.join(&session_name).exists(),
            "replacement target session heartbeat was deleted"
        );
        assert!(
            !original_sessions.join(&session_name).exists(),
            "validated session heartbeat was not retired"
        );
        assert!(!fixture.issue_path().exists(), "issue heartbeat remained");
        std::fs::remove_dir(&sessions).expect("remove sessions junction");
    }
}
