#[cfg(target_os = "linux")]
use std::fs;

#[cfg(target_os = "linux")]
pub(crate) fn process_birth_identity(pid: u32) -> Result<Option<(String, String)>, String> {
    super::observe_process_birth(pid)
        .map(|birth| birth.map(|birth| (birth.boot_id, birth.start_identity)))
}

#[cfg(target_os = "linux")]
pub(crate) fn current_boot_identity() -> Result<String, String> {
    let boot_id = fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .map_err(|error| format!("read executor boot identity: {error}"))?
        .trim()
        .to_string();
    (!boot_id.is_empty())
        .then_some(boot_id)
        .ok_or_else(|| "executor boot identity is empty".to_string())
}

#[cfg(target_os = "linux")]
pub(crate) fn process_is_terminated(pid: u32) -> Result<bool, String> {
    let stat = match fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => stat,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(format!("read autonomous process stat: {error}")),
    };
    let close = stat
        .rfind(')')
        .ok_or_else(|| "autonomous process stat is malformed".to_string())?;
    Ok(stat[close + 1..]
        .split_whitespace()
        .next()
        .is_some_and(|state| matches!(state, "Z" | "X")))
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn process_is_terminated(_pid: u32) -> Result<bool, String> {
    Ok(false)
}

#[cfg(target_os = "macos")]
pub(crate) fn process_birth_identity(pid: u32) -> Result<Option<(String, String)>, String> {
    let pid = i32::try_from(pid).map_err(|_| "executor PID is out of range".to_string())?;
    let mut info = std::mem::MaybeUninit::<nix::libc::proc_bsdinfo>::zeroed();
    // SAFETY: proc_pidinfo receives a correctly sized writable proc_bsdinfo buffer.
    let observed = unsafe {
        nix::libc::proc_pidinfo(
            pid,
            nix::libc::PROC_PIDTBSDINFO,
            0,
            info.as_mut_ptr().cast(),
            std::mem::size_of::<nix::libc::proc_bsdinfo>() as i32,
        )
    };
    if observed == 0 {
        let error = std::io::Error::last_os_error();
        return if error
            .raw_os_error()
            .is_some_and(|code| code == nix::libc::ESRCH || code == nix::libc::ENOENT)
        {
            Ok(None)
        } else {
            Err(format!("observe executor process birth: {error}"))
        };
    }
    if observed as usize != std::mem::size_of::<nix::libc::proc_bsdinfo>() {
        return Err("observe executor process birth returned a partial record".to_string());
    }
    // SAFETY: proc_pidinfo filled the complete structure above.
    let info = unsafe { info.assume_init() };
    let start_identity = info
        .pbi_start_tvsec
        .checked_mul(1_000_000)
        .and_then(|seconds| seconds.checked_add(info.pbi_start_tvusec))
        .filter(|identity| *identity > 0)
        .ok_or_else(|| "executor process start identity is invalid".to_string())?;
    Ok(Some((current_boot_identity()?, start_identity.to_string())))
}

#[cfg(target_os = "freebsd")]
pub(crate) fn process_birth_identity(pid: u32) -> Result<Option<(String, String)>, String> {
    let pid = i32::try_from(pid).map_err(|_| "executor PID is out of range".to_string())?;
    let mut mib = [
        nix::libc::CTL_KERN,
        nix::libc::KERN_PROC,
        nix::libc::KERN_PROC_PID,
        pid,
    ];
    let mut info = std::mem::MaybeUninit::<nix::libc::kinfo_proc>::zeroed();
    let mut length = std::mem::size_of::<nix::libc::kinfo_proc>();
    // SAFETY: sysctl receives a valid MIB and correctly sized writable output buffer.
    let result = unsafe {
        nix::libc::sysctl(
            mib.as_mut_ptr(),
            mib.len() as u32,
            info.as_mut_ptr().cast(),
            &mut length,
            std::ptr::null_mut(),
            0,
        )
    };
    if result != 0 {
        let error = std::io::Error::last_os_error();
        return if error
            .raw_os_error()
            .is_some_and(|code| code == nix::libc::ESRCH || code == nix::libc::ENOENT)
        {
            Ok(None)
        } else {
            Err(format!("observe executor process birth: {error}"))
        };
    }
    if length == 0 {
        return Ok(None);
    }
    if length < std::mem::size_of::<nix::libc::kinfo_proc>() {
        return Err("observe executor process birth returned a partial record".to_string());
    }
    // SAFETY: sysctl filled the complete structure above.
    let info = unsafe { info.assume_init() };
    let start_identity = (info.ki_start.tv_sec as u64)
        .checked_mul(1_000_000)
        .and_then(|seconds| seconds.checked_add(info.ki_start.tv_usec as u64))
        .filter(|identity| *identity > 0)
        .ok_or_else(|| "executor process start identity is invalid".to_string())?;
    Ok(Some((current_boot_identity()?, start_identity.to_string())))
}

#[cfg(any(target_os = "macos", target_os = "freebsd"))]
pub(crate) fn current_boot_identity() -> Result<String, String> {
    use std::ffi::c_void;

    let name = b"kern.boottime\0";
    let mut boot = std::mem::MaybeUninit::<nix::libc::timeval>::zeroed();
    let mut length = std::mem::size_of::<nix::libc::timeval>();
    // SAFETY: sysctlbyname receives a static NUL-terminated name and writable timeval buffer.
    let result = unsafe {
        nix::libc::sysctlbyname(
            name.as_ptr().cast(),
            boot.as_mut_ptr().cast::<c_void>(),
            &mut length,
            std::ptr::null_mut(),
            0,
        )
    };
    if result != 0 || length < std::mem::size_of::<nix::libc::timeval>() {
        return Err(format!(
            "read executor boot identity: {}",
            std::io::Error::last_os_error()
        ));
    }
    // SAFETY: sysctlbyname filled the complete timeval above.
    let boot = unsafe { boot.assume_init() };
    let identity = (boot.tv_sec as u64)
        .checked_mul(1_000_000)
        .and_then(|seconds| seconds.checked_add(boot.tv_usec as u64))
        .filter(|identity| *identity > 0)
        .ok_or_else(|| "executor boot identity is empty".to_string())?;
    Ok(identity.to_string())
}

#[cfg(windows)]
pub(crate) fn process_birth_identity(pid: u32) -> Result<Option<(String, String)>, String> {
    type Handle = *mut std::ffi::c_void;
    #[repr(C)]
    struct FileTime {
        low: u32,
        high: u32,
    }
    // SAFETY: Kernel32 process APIs use the declared system ABI and Windows HANDLE/DWORD
    // layouts; every pointer-bearing call below documents its live buffer or handle lifetime.
    #[link(name = "Kernel32")]
    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
        fn GetProcessTimes(
            process: Handle,
            creation: *mut FileTime,
            exit: *mut FileTime,
            kernel: *mut FileTime,
            user: *mut FileTime,
        ) -> i32;
        fn CloseHandle(handle: Handle) -> i32;
    }
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    // SAFETY: OpenProcess has no borrowed pointer arguments.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        let error = std::io::Error::last_os_error();
        return if error
            .raw_os_error()
            .is_some_and(|code| code == 87 || code == 1168)
        {
            Ok(None)
        } else {
            Err(format!("observe executor process birth: {error}"))
        };
    }
    let mut creation = FileTime { low: 0, high: 0 };
    let mut exit = FileTime { low: 0, high: 0 };
    let mut kernel = FileTime { low: 0, high: 0 };
    let mut user = FileTime { low: 0, high: 0 };
    let creation_ptr = std::ptr::addr_of_mut!(creation);
    let exit_ptr = std::ptr::addr_of_mut!(exit);
    let kernel_ptr = std::ptr::addr_of_mut!(kernel);
    let user_ptr = std::ptr::addr_of_mut!(user);
    // SAFETY: process is an owned valid handle and all FILETIME pointers are writable for the
    // sole call; each local outlives the call and is not otherwise accessed until it returns.
    let result = unsafe { GetProcessTimes(process, creation_ptr, exit_ptr, kernel_ptr, user_ptr) };
    // SAFETY: process was returned by OpenProcess and is closed exactly once.
    unsafe { CloseHandle(process) };
    if result == 0 {
        return Err(format!(
            "observe executor process birth: {}",
            std::io::Error::last_os_error()
        ));
    }
    let creation = ((creation.high as u64) << 32) | creation.low as u64;
    if creation == 0 {
        return Err("executor process start identity is invalid".to_string());
    }
    Ok(Some((current_boot_identity()?, creation.to_string())))
}

#[cfg(windows)]
pub(crate) fn current_boot_identity() -> Result<String, String> {
    // SAFETY: Kernel32's ProcessIdToSessionId uses the declared system ABI and DWORD/u32
    // types; the sole pointer-bearing call below documents the exact local lifetime.
    #[link(name = "Kernel32")]
    unsafe extern "system" {
        fn ProcessIdToSessionId(pid: u32, session_id: *mut u32) -> i32;
    }
    let mut session_id = 0_u32;
    // SAFETY: Kernel32's ProcessIdToSessionId uses the declared system ABI and DWORD/u32
    // types. This sole call passes a pointer to the live, writable session_id local, whose
    // lifetime covers the call and which is not aliased until the function returns.
    if unsafe { ProcessIdToSessionId(std::process::id(), &mut session_id) } == 0 {
        return Err(format!(
            "read executor session identity: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(format!("windows-session-{session_id}"))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ProcessObservation {
    Exact,
    Dead,
    Mismatch,
    Unknown(String),
}

pub(crate) fn observe_expected_process(
    pid: u32,
    expected_boot: &str,
    expected_start: &str,
) -> ProcessObservation {
    match current_boot_identity() {
        Ok(current_boot) if current_boot != expected_boot => return ProcessObservation::Mismatch,
        Ok(_) => {}
        Err(error) => return ProcessObservation::Unknown(error),
    }
    match process_birth_identity(pid) {
        Ok(None) => ProcessObservation::Dead,
        Ok(Some((boot_id, process_start)))
            if boot_id == expected_boot && process_start == expected_start =>
        {
            ProcessObservation::Exact
        }
        Ok(Some(_)) => ProcessObservation::Mismatch,
        Err(error) => ProcessObservation::Unknown(error),
    }
}

pub(crate) fn observe_runtime_process_identity(
    pid: u32,
) -> Result<Option<(u32, String, String)>, String> {
    let Some((boot_id, process_start)) = process_birth_identity(pid)? else {
        return Ok(None);
    };
    #[cfg(unix)]
    let container_id = {
        let pid = nix::unistd::Pid::from_raw(
            i32::try_from(pid).map_err(|_| "autonomous process PID is out of range".to_string())?,
        );
        match nix::unistd::getpgid(Some(pid)) {
            Ok(group) => u32::try_from(group.as_raw())
                .map_err(|_| "autonomous process group is negative".to_string())?,
            Err(nix::errno::Errno::ESRCH) => return Ok(None),
            Err(error) => return Err(format!("observe autonomous process group: {error}")),
        }
    };
    #[cfg(windows)]
    let container_id = pid;
    Ok(Some((container_id, boot_id, process_start)))
}

pub(crate) fn ensure_autonomous_runtime_supported() -> Result<(), String> {
    #[cfg(any(
        target_os = "linux",
        target_os = "macos",
        target_os = "freebsd",
        windows
    ))]
    {
        Ok(())
    }
    #[cfg(not(any(
        target_os = "linux",
        target_os = "macos",
        target_os = "freebsd",
        windows
    )))]
    {
        Err("autonomous runtime is unsupported on this host".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_host_is_admitted_by_shared_platform_identity() {
        ensure_autonomous_runtime_supported().expect("current CI host must be supported");
    }

    #[test]
    fn process_observation_only_reports_dead_for_an_absent_pid() {
        let pid = std::process::id();
        let (boot_id, process_start) = process_birth_identity(pid)
            .expect("observe current process")
            .expect("current process is present");
        assert_eq!(
            observe_expected_process(pid, &boot_id, &process_start),
            ProcessObservation::Exact
        );
        assert_eq!(
            observe_expected_process(pid, &boot_id, "different-start"),
            ProcessObservation::Mismatch
        );
        assert_eq!(
            observe_expected_process(i32::MAX as u32, &boot_id, "missing"),
            ProcessObservation::Dead
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn process_disappearance_accepts_procfs_not_found_and_esrch() {
        assert!(super::super::process_disappeared_error(
            &std::io::Error::from(std::io::ErrorKind::NotFound)
        ));
        assert!(super::super::process_disappeared_error(
            &std::io::Error::from_raw_os_error(nix::libc::ESRCH)
        ));
        assert!(!super::super::process_disappeared_error(
            &std::io::Error::from(std::io::ErrorKind::PermissionDenied)
        ));
    }
}
