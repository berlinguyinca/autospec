use std::fs;
use std::process::{Command, Stdio};

pub(super) fn process_is_running(pid: u32) -> bool {
    Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .is_ok_and(|output| {
            output.status.success()
                && !String::from_utf8_lossy(&output.stdout)
                    .trim_start()
                    .starts_with('Z')
        })
}

pub(super) fn wait_for_process_exit(pid: u32) {
    for _ in 0..100 {
        if !process_is_running(pid) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

pub(super) fn process_identity(pid: u32) -> Option<(u32, u64)> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let (_, fields) = stat.rsplit_once(") ")?;
    let fields = fields.split_whitespace().collect::<Vec<_>>();
    Some((fields.get(2)?.parse().ok()?, fields.get(19)?.parse().ok()?))
}

pub(super) fn terminate_process_group(pid: u32) {
    let _ = Command::new("kill")
        .args(["-KILL", "--", &format!("-{pid}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

pub(super) fn terminate_process(pid: u32) {
    let _ = Command::new("kill")
        .args(["-KILL", "--", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}
