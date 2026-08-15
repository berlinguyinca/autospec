#[cfg(windows)]
use super::*;

#[cfg(windows)]
pub(super) const WINDOWS_DELETE: u32 = 0x0001_0000;
#[cfg(windows)]
pub(super) const WINDOWS_FILE_LIST_DIRECTORY: u32 = 0x0000_0001;
#[cfg(windows)]
pub(super) const WINDOWS_FILE_TRAVERSE: u32 = 0x0000_0020;
#[cfg(windows)]
pub(super) const WINDOWS_FILE_READ_ATTRIBUTES: u32 = 0x0000_0080;
#[cfg(windows)]
pub(super) const WINDOWS_GENERIC_READ: u32 = 0x8000_0000;
#[cfg(windows)]
pub(super) const WINDOWS_GENERIC_WRITE: u32 = 0x4000_0000;
#[cfg(windows)]
pub(super) const WINDOWS_SYNCHRONIZE: u32 = 0x0010_0000;
#[cfg(windows)]
pub(super) const WINDOWS_FILE_OPEN: u32 = 0x0000_0001;
#[cfg(windows)]
pub(super) const WINDOWS_FILE_OPEN_IF: u32 = 0x0000_0003;
#[cfg(windows)]
pub(super) const WINDOWS_FILE_DIRECTORY_FILE: u32 = 0x0000_0001;
#[cfg(windows)]
pub(super) const WINDOWS_FILE_SYNCHRONOUS_IO_NONALERT: u32 = 0x0000_0020;
#[cfg(windows)]
pub(super) const WINDOWS_FILE_NON_DIRECTORY_FILE: u32 = 0x0000_0040;
#[cfg(windows)]
pub(super) const WINDOWS_FILE_OPEN_REPARSE_POINT: u32 = 0x0020_0000;

#[cfg(windows)]
pub(super) fn open_windows_directory(path: &Path) -> std::io::Result<fs::File> {
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
            WINDOWS_FILE_LIST_DIRECTORY
                | WINDOWS_FILE_TRAVERSE
                | WINDOWS_FILE_READ_ATTRIBUTES
                | WINDOWS_SYNCHRONIZE,
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
pub(super) fn open_windows_relative(
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
pub(super) fn windows_invalid_handle_value() -> std::os::windows::io::RawHandle {
    (-1_isize) as std::os::windows::io::RawHandle
}

#[cfg(windows)]
#[repr(C)]
pub(super) struct WindowsUnicodeString {
    length: u16,
    maximum_length: u16,
    buffer: *mut u16,
}

#[cfg(windows)]
#[repr(C)]
pub(super) struct WindowsObjectAttributes {
    length: u32,
    root_directory: std::os::windows::io::RawHandle,
    object_name: *mut WindowsUnicodeString,
    attributes: u32,
    security_descriptor: *mut std::ffi::c_void,
    security_quality_of_service: *mut std::ffi::c_void,
}

#[cfg(windows)]
#[repr(C)]
pub(super) struct WindowsIoStatusBlock {
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
    fn NtSetInformationFile(
        file_handle: std::os::windows::io::RawHandle,
        io_status_block: *mut WindowsIoStatusBlock,
        file_information: *mut std::ffi::c_void,
        length: u32,
        file_information_class: u32,
    ) -> i32;
    fn RtlNtStatusToDosError(status: i32) -> u32;
}

#[cfg(windows)]
pub(super) fn rename_windows_file_handle(
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
    let mut buffer =
        build_windows_file_rename_info_buffer(directory.as_raw_handle().cast(), &destination)?;
    let mut status_block = WindowsIoStatusBlock {
        status: 0,
        information: 0,
    };
    // SAFETY: file and directory are live handles and buffer contains
    // FILE_RENAME_INFORMATION with a NUL-terminated name backing its byte-counted payload.
    let status = unsafe {
        NtSetInformationFile(
            file.as_raw_handle(),
            &mut status_block,
            buffer.as_mut_ptr().cast(),
            buffer.len() as u32,
            10,
        )
    };
    if status < 0 {
        // SAFETY: conversion is a pure mapping from the returned NTSTATUS.
        let error = unsafe { RtlNtStatusToDosError(status) };
        Err(std::io::Error::from_raw_os_error(error as i32))
    } else {
        Ok(())
    }
}

#[cfg(windows)]
pub(super) fn delete_windows_file_handle(file: &fs::File) -> std::io::Result<()> {
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

#[repr(C)]
#[cfg(any(test, windows))]
pub(super) struct WindowsFileRenameInfo {
    pub(super) flags: u32,
    pub(super) root_directory: *mut std::ffi::c_void,
    pub(super) file_name_length: u32,
    pub(super) file_name: [u16; 1],
}

#[cfg(any(test, windows))]
pub(super) fn build_windows_file_rename_info_buffer(
    root_directory: *mut std::ffi::c_void,
    destination: &[u16],
) -> std::io::Result<Vec<u8>> {
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
    let header_size = std::mem::size_of::<WindowsFileRenameInfo>();
    let mut buffer = vec![0_u8; header_size + name_bytes as usize + std::mem::size_of::<u16>()];
    let info = buffer.as_mut_ptr().cast::<WindowsFileRenameInfo>();
    // SAFETY: buffer is sized for the fixed header plus the complete UTF-16 name.
    unsafe {
        (*info).flags = 0;
        (*info).root_directory = root_directory;
        (*info).file_name_length = name_bytes;
        std::ptr::copy_nonoverlapping(
            destination.as_ptr(),
            (*info).file_name.as_mut_ptr(),
            destination.len(),
        );
    }
    Ok(buffer)
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
