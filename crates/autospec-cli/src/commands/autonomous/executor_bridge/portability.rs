use super::*;

#[cfg(any(target_os = "macos", target_os = "freebsd", windows))]
mod direct_attempt;
mod portable_runtime;

#[cfg(any(target_os = "macos", target_os = "freebsd", windows))]
pub(super) use direct_attempt::*;
pub(super) use portable_runtime::*;

#[cfg(test)]
static PORTABLE_LIFECYCLE_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod supported_host_tests;
#[cfg(all(test, windows))]
mod windows_tests;
