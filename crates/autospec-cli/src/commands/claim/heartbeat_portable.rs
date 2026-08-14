use super::{
    heartbeat_root, heartbeat_session_key, parse_startup_heartbeat, ClaimMutationIdentity,
    StartupHeartbeatEvidence,
};
use crate::commands::autonomous::drain::repository_progress_key;
use crate::commands::CommandFailure;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

mod directory;
mod file;
mod publication;
mod retirement;
#[cfg(any(test, windows))]
mod windows;

use directory::*;
use file::*;
pub(super) use publication::publish;
pub(super) use retirement::retire_released;
#[cfg(any(test, windows))]
use windows::*;

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

#[cfg(test)]
mod tests;
