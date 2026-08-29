pub(super) fn current_process_start() -> String {
    #[cfg(target_os = "linux")]
    {
        // Linux heartbeat ownership is keyed by PID plus /proc start time. Test
        // fixtures must use the same identity or they represent a dead process.
        let stat = std::fs::read_to_string("/proc/self/stat").expect("process identity");
        return stat
            .rsplit_once(") ")
            .expect("process stat fields")
            .1
            .split_whitespace()
            .nth(19)
            .expect("process start")
            .to_string();
    }

    #[cfg(target_os = "macos")]
    {
        let pid = i32::try_from(std::process::id()).expect("process PID fits i32");
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
        assert_eq!(
            observed as usize,
            std::mem::size_of::<nix::libc::proc_bsdinfo>(),
            "process identity"
        );
        // SAFETY: proc_pidinfo filled the complete structure above.
        let info = unsafe { info.assume_init() };
        return (info.pbi_start_tvsec * 1_000_000 + info.pbi_start_tvusec).to_string();
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    panic!("unsupported test host")
}

pub(super) fn current_host_identity() -> String {
    let output = std::process::Command::new("hostname")
        .output()
        .expect("read host identity");
    assert!(output.status.success(), "hostname command failed");
    String::from_utf8(output.stdout)
        .expect("host identity is UTF-8")
        .trim()
        .to_string()
}

pub(super) fn current_boot_identity() -> String {
    #[cfg(target_os = "linux")]
    {
        return std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
            .expect("boot identity")
            .trim()
            .to_string();
    }

    #[cfg(target_os = "macos")]
    {
        let name = b"kern.boottime\0";
        let mut boot = std::mem::MaybeUninit::<nix::libc::timeval>::zeroed();
        let mut length = std::mem::size_of::<nix::libc::timeval>();
        // SAFETY: sysctlbyname receives a static name and a correctly sized timeval buffer.
        let result = unsafe {
            nix::libc::sysctlbyname(
                name.as_ptr().cast(),
                boot.as_mut_ptr().cast(),
                &mut length,
                std::ptr::null_mut(),
                0,
            )
        };
        assert_eq!(result, 0, "boot identity");
        // SAFETY: sysctlbyname filled the complete structure above.
        let boot = unsafe { boot.assume_init() };
        return ((boot.tv_sec as u64) * 1_000_000 + boot.tv_usec as u64).to_string();
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    panic!("unsupported test host")
}

pub(super) fn bind_to_current_process(document: String) -> String {
    document
        .replace(
            "\"pid\":2147483647,",
            &format!("\"pid\":{},", std::process::id()),
        )
        .replace(
            "\"process_start\":\"1\"",
            &format!("\"process_start\":\"{}\"", current_process_start()),
        )
}
