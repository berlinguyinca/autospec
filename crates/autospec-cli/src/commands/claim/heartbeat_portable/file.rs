use super::*;

#[cfg(unix)]
pub(super) fn file_link_count(file: &fs::File) -> Result<u64, CommandFailure> {
    use std::os::unix::fs::MetadataExt;
    file.metadata()
        .map(|metadata| metadata.nlink())
        .map_err(|error| CommandFailure::diagnostic(format!("heartbeat stage inspect: {error}")))
}

#[cfg(windows)]
pub(super) fn file_link_count(file: &fs::File) -> Result<u64, CommandFailure> {
    windows_file_identity(file).map(|identity| u64::from(identity.2))
}

#[cfg(unix)]
pub(super) fn private_stage_metadata(metadata: &fs::Metadata, links: u64) -> bool {
    private_file_metadata_with_links(metadata, links)
}

#[cfg(windows)]
pub(super) fn private_stage_metadata(metadata: &fs::Metadata, _links: u64) -> bool {
    private_file_metadata(metadata)
}

#[cfg(unix)]
pub(super) fn unlink_staged_file(
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
pub(super) fn unlink_staged_file(
    _directory: &PrivateDirectory,
    _name: &str,
    file: &fs::File,
) -> Result<(), CommandFailure> {
    delete_windows_file_handle(file)
        .map_err(|error| CommandFailure::diagnostic(format!("heartbeat stage cleanup: {error}")))
}

#[cfg(unix)]
pub(super) fn create_private_file_relative(
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
pub(super) fn create_private_file_relative(
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
pub(super) fn open_private_file_relative(
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
pub(super) fn open_staged_file_relative(
    directory: &PrivateDirectory,
    name: &str,
) -> std::io::Result<fs::File> {
    open_private_file_relative(directory, name)
}

#[cfg(windows)]
pub(super) fn open_private_file_relative(
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
pub(super) fn open_staged_file_relative(
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
pub(super) fn private_file_metadata(metadata: &fs::Metadata) -> bool {
    private_file_metadata_with_links(metadata, 1)
}

#[cfg(unix)]
pub(super) fn private_file_metadata_with_links(metadata: &fs::Metadata, links: u64) -> bool {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    metadata.uid() == unsafe { nix::libc::geteuid() }
        && metadata.permissions().mode() & 0o7777 == 0o600
        && metadata.nlink() == links
}

#[cfg(unix)]
pub(super) fn private_file_handle(_file: &fs::File) -> Result<bool, CommandFailure> {
    Ok(true)
}

#[cfg(unix)]
pub(super) fn same_file_identity(
    left: &fs::File,
    right: &fs::File,
) -> Result<bool, CommandFailure> {
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
pub(super) fn private_file_metadata(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0
}

#[cfg(windows)]
pub(super) fn private_file_handle(file: &fs::File) -> Result<bool, CommandFailure> {
    Ok(windows_file_identity(file)?.2 == 1)
}

#[cfg(windows)]
pub(super) fn same_file_identity(
    left: &fs::File,
    right: &fs::File,
) -> Result<bool, CommandFailure> {
    let left = windows_file_identity(left)?;
    let right = windows_file_identity(right)?;
    Ok(left.0 == right.0 && left.1 == right.1)
}

#[cfg(windows)]
pub(super) fn windows_file_identity(file: &fs::File) -> Result<(u32, u64, u32), CommandFailure> {
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
pub(super) struct WindowsByHandleFileInformation {
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

pub(super) fn sync_private_directory(directory: &PrivateDirectory) -> Result<(), CommandFailure> {
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
