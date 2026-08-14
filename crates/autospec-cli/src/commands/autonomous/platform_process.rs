#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProcessBirth {
    pub(crate) pid: u32,
    pub(crate) process_group: u32,
    pub(crate) boot_id: String,
    pub(crate) start_identity: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ProcessObservation {
    Exact(ProcessBirth),
    Dead,
    Mismatch,
    Unknown(String),
}

pub(crate) fn current_boot_identity() -> Result<String, String> {
    #[cfg(target_os = "linux")]
    {
        let boot_id = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
            .map_err(|error| format!("read executor boot identity: {error}"))?
            .trim()
            .to_string();
        return (!boot_id.is_empty())
            .then_some(boot_id)
            .ok_or_else(|| "executor boot identity is empty".to_string());
    }

    #[cfg(target_os = "macos")]
    {
        let mut boot_time = unsafe { std::mem::zeroed::<nix::libc::timeval>() };
        let expected_size = std::mem::size_of::<nix::libc::timeval>();
        let mut observed_size = expected_size;
        let mut mib = [nix::libc::CTL_KERN, nix::libc::KERN_BOOTTIME];
        nix::errno::Errno::clear();
        // SAFETY: `mib`, `boot_time`, and `observed_size` are valid writable buffers for the
        // documented KERN_BOOTTIME sysctl. No replacement value is supplied.
        let result = unsafe {
            nix::libc::sysctl(
                mib.as_mut_ptr(),
                mib.len() as u32,
                &mut boot_time as *mut _ as *mut nix::libc::c_void,
                &mut observed_size,
                std::ptr::null_mut(),
                0,
            )
        };
        if result != 0 {
            return Err(format!(
                "observe Darwin boot time: {}",
                nix::errno::Errno::last()
            ));
        }
        if observed_size != expected_size {
            return Err(format!(
                "observe Darwin boot time: kernel returned {observed_size} bytes, expected {expected_size}"
            ));
        }
        return canonical_time(
            boot_time.tv_sec,
            i32::try_from(boot_time.tv_usec)
                .map_err(|_| "boot time identity is out of range".to_string())?,
        );
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    Err("autonomous process identity requires Linux or macOS native process APIs".to_string())
}

pub(crate) fn observe_birth(pid: u32) -> Result<Option<ProcessBirth>, String> {
    #[cfg(target_os = "linux")]
    {
        let stat = match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            Ok(stat) => stat,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("read executor process stat: {error}")),
        };
        let close = stat
            .rfind(')')
            .ok_or_else(|| "executor process stat is malformed".to_string())?;
        let fields: Vec<_> = stat[close + 1..].split_whitespace().collect();
        let start_identity = fields
            .get(19)
            .ok_or_else(|| "executor process stat lacks start identity".to_string())?
            .to_string();
        let Some(process_group) = observe_process_group(pid)? else {
            return Ok(None);
        };
        let boot_id = current_boot_identity()?;
        return Ok(Some(ProcessBirth {
            pid,
            process_group,
            boot_id,
            start_identity,
        }));
    }

    #[cfg(target_os = "macos")]
    {
        let Some(before) = observe_darwin_process(pid)? else {
            return Ok(None);
        };
        let Some(process_group) = observe_process_group(pid)? else {
            return Ok(None);
        };
        let Some(after) = observe_darwin_process(pid)? else {
            return Ok(None);
        };
        if before.start != after.start {
            return Err("Darwin process start identity changed during observation".to_string());
        }
        if before.process_group != process_group || after.process_group != process_group {
            return Err("Darwin process group changed during observation".to_string());
        }
        return Ok(Some(ProcessBirth {
            pid,
            process_group,
            boot_id: current_boot_identity()?,
            start_identity: canonical_time(before.start.0, before.start.1)?,
        }));
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = pid;
        Err("autonomous process identity requires Linux or macOS native process APIs".to_string())
    }
}

pub(crate) fn observe_expected(
    pid: u32,
    expected_boot: &str,
    expected_start: &str,
) -> ProcessObservation {
    match current_boot_identity() {
        Ok(current_boot) if current_boot != expected_boot => return ProcessObservation::Mismatch,
        Ok(_) => {}
        Err(error) => return ProcessObservation::Unknown(error),
    }
    match observe_birth(pid) {
        Ok(None) => ProcessObservation::Dead,
        Ok(Some(birth))
            if birth.boot_id == expected_boot && birth.start_identity == expected_start =>
        {
            ProcessObservation::Exact(birth)
        }
        Ok(Some(_)) => ProcessObservation::Mismatch,
        Err(error) => ProcessObservation::Unknown(error),
    }
}

pub(crate) fn ensure_autonomous_runtime_supported() -> Result<(), String> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        Ok(())
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Err("autonomous runtime requires Linux or macOS native process identity support".into())
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) fn observe_process_group(pid: u32) -> Result<Option<u32>, String> {
    let pid = nix::unistd::Pid::from_raw(
        i32::try_from(pid).map_err(|_| "executor PID is out of range".to_string())?,
    );
    match nix::unistd::getpgid(Some(pid)) {
        Ok(group) => u32::try_from(group.as_raw())
            .map(Some)
            .map_err(|_| "executor process group is negative".to_string()),
        Err(nix::errno::Errno::ESRCH) => Ok(None),
        Err(error) => Err(format!("observe executor process group: {error}")),
    }
}

#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DarwinProcessMetadata {
    process_group: u32,
    start: (i64, i32),
}

#[cfg(target_os = "macos")]
fn observe_darwin_process(pid: u32) -> Result<Option<DarwinProcessMetadata>, String> {
    let pid = i32::try_from(pid).map_err(|_| "executor PID is out of range".to_string())?;
    let mut process = unsafe { std::mem::zeroed::<nix::libc::proc_bsdinfo>() };
    let expected_size = std::mem::size_of::<nix::libc::proc_bsdinfo>();
    let expected_size_i32 = i32::try_from(expected_size)
        .map_err(|_| "Darwin process metadata structure is too large".to_string())?;
    nix::errno::Errno::clear();
    // SAFETY: `process` is a correctly sized writable proc_bsdinfo buffer and `pid` fits the
    // proc_pidinfo ABI. The return size is checked exactly before any field is consumed.
    let observed_size = unsafe {
        nix::libc::proc_pidinfo(
            pid,
            nix::libc::PROC_PIDTBSDINFO,
            0,
            &mut process as *mut _ as *mut nix::libc::c_void,
            expected_size_i32,
        )
    };
    if observed_size == 0 {
        let error = nix::errno::Errno::last();
        return if error == nix::errno::Errno::ESRCH {
            Ok(None)
        } else {
            Err(format!("observe Darwin process metadata: {error}"))
        };
    }
    if observed_size != expected_size_i32 {
        return Err(format!(
            "observe Darwin process metadata: kernel returned {observed_size} bytes, expected {expected_size_i32}"
        ));
    }
    let seconds = i64::try_from(process.pbi_start_tvsec)
        .map_err(|_| "process time identity is out of range".to_string())?;
    let micros = i32::try_from(process.pbi_start_tvusec)
        .map_err(|_| "process time identity is out of range".to_string())?;
    canonical_time(seconds, micros)?;
    Ok(Some(DarwinProcessMetadata {
        process_group: process.pbi_pgid,
        start: (seconds, micros),
    }))
}

#[cfg(target_os = "macos")]
fn canonical_time(seconds: i64, micros: i32) -> Result<String, String> {
    (seconds >= 0 && (0..1_000_000).contains(&micros))
        .then(|| format!("{seconds}.{micros:06}"))
        .ok_or_else(|| "process time identity is out of range".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn repeated_self_observation_has_stable_process_birth_identity() {
        let birth = observe_birth(std::process::id())
            .expect("observe current process")
            .expect("current process is live");
        assert_eq!(
            observe_birth(std::process::id()).expect("repeat current process observation"),
            Some(birth)
        );
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn nonexistent_pid_is_observed_as_dead() {
        let boot = current_boot_identity().expect("observe current boot");
        assert!(matches!(
            observe_expected(i32::MAX as u32, &boot, "start"),
            ProcessObservation::Dead
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn darwin_birth_identity_is_canonical_and_mismatch_is_not_death() {
        let birth = observe_birth(std::process::id())
            .expect("observe current process")
            .expect("current process is live");
        assert!(canonical_decimal_time(&birth.boot_id));
        assert!(canonical_decimal_time(&birth.start_identity));
        assert!(matches!(
            observe_expected(birth.pid, &birth.boot_id, "different-start"),
            ProcessObservation::Mismatch
        ));
    }

    #[cfg(target_os = "macos")]
    fn canonical_decimal_time(identity: &str) -> bool {
        let Some((seconds, micros)) = identity.split_once('.') else {
            return false;
        };
        !seconds.is_empty()
            && seconds.bytes().all(|byte| byte.is_ascii_digit())
            && micros.len() == 6
            && micros.bytes().all(|byte| byte.is_ascii_digit())
    }
}
