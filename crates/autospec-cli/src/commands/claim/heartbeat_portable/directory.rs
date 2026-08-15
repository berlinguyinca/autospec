use super::*;

pub(super) fn open_or_create_private_child(
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

pub(super) fn ensure_private_directory(path: &Path) -> Result<(), CommandFailure> {
    ensure_private_directory_with_hook(path, &mut |_| {})
}

pub(super) fn ensure_private_directory_with_hook(
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
pub(super) fn create_private_directory(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    let mut builder = fs::DirBuilder::new();
    builder.mode(0o700);
    builder.create(path)
}

#[cfg(windows)]
pub(super) fn create_private_directory(path: &Path) -> std::io::Result<()> {
    fs::create_dir(path)
}

#[cfg(unix)]
pub(super) fn private_directory_metadata(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    metadata.uid() == unsafe { nix::libc::geteuid() }
        && metadata.permissions().mode() & 0o7777 == 0o700
}

#[cfg(windows)]
pub(super) fn validate_windows_path_components(path: &Path) -> Result<(), CommandFailure> {
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
pub(super) fn private_directory_metadata(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0
}

pub(super) struct PrivateDirectory {
    pub(super) path: PathBuf,
    pub(super) file: fs::File,
}

pub(super) fn open_existing_private_directory(
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
        super::super::private_heartbeat_directory_identity(&file, "portable directory")?;
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

pub(super) fn open_existing_private_child(
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
        super::super::private_heartbeat_directory_identity(&file, "portable child directory")?;
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

pub(super) struct RepositoryLock {
    _file: fs::File,
    #[cfg(windows)]
    _overlapped: Box<WindowsOverlapped>,
}

impl RepositoryLock {
    pub(super) fn acquire(directory: &PrivateDirectory) -> Result<Self, CommandFailure> {
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
pub(super) struct WindowsOverlapped {
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
