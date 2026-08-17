#![cfg(any(target_os = "linux", target_os = "macos"))]

use super::*;

#[test]
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn restart_dry_run_is_strictly_read_only() {
    let fixture = ForegroundFixture::new();
    git_fixture(&fixture.repo_dir, &["init", "-q"]);
    fs::write(&fixture.accountability, "CLOSED\n").expect("seed closed accountability epic");
    fs::create_dir_all(fixture.scoped_dir()).expect("create autonomous scope");
    fs::write(
        fixture.scoped_stop_sentinel(),
        "immediate\n2026-08-14T00:00:00Z test@localhost\n",
    )
    .expect("seed immediate-stop sentinel");
    let mut conductor_command = Command::new("sh");
    conductor_command
        .args(["-c", "while :; do sleep 1; done"])
        .process_group(0);
    let mut conductor = conductor_command.spawn().expect("spawn conductor fixture");
    let identity = native_process_identity(conductor.id()).expect("capture conductor identity");
    fs::write(
        fixture.scoped_dir().join("conductor.pid"),
        format!(
            "{{\"pid\":{},\"repo\":\"test/repo\",\"scope\":\"test_repo\",\"pgid\":{},\"start_time_ticks\":{}}}\n",
            conductor.id(),
            identity.pgid,
            identity.start_time_ticks,
        ),
    )
    .expect("record conductor metadata");
    assert_authoritative_conductor_metadata(
        &fixture.scoped_dir().join("conductor.pid"),
        conductor.id(),
    );
    let before = snapshot_tree(&fixture.root);

    let output = fixture
        .detached_command("restart")
        .args(["--dry-run", "--json"])
        .output()
        .expect("preview restart");

    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(conductor.try_wait().unwrap().is_none());
    assert_eq!(snapshot_tree(&fixture.root), before);
    assert!(!fixture.calls.exists());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["subcommand"], "restart");
    assert_eq!(json["status"], "dry-run");

    let restarted = fixture
        .detached_command("restart")
        .arg("--json")
        .output()
        .expect("restart after preview");
    let mut conductor_terminated = false;
    for _ in 0..100 {
        if conductor.try_wait().unwrap().is_some() {
            conductor_terminated = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(
        conductor_terminated,
        "non-preview restart must recognize and terminate the owned conductor group; stdout={} stderr={}",
        String::from_utf8_lossy(&restarted.stdout),
        String::from_utf8_lossy(&restarted.stderr)
    );

    terminate_process_group(conductor.id());
    let _ = conductor.wait();
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Debug, Eq, PartialEq)]
struct NativeProcessIdentity {
    pgid: u32,
    start_time_ticks: u64,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn assert_authoritative_conductor_metadata(path: &Path, pid: u32) {
    let metadata: serde_json::Value =
        serde_json::from_slice(&fs::read(path).expect("read conductor metadata"))
            .expect("parse conductor metadata");
    let observed = native_process_identity(pid).expect("capture live conductor identity");
    assert_eq!(metadata["pid"], pid);
    assert_eq!(metadata["pgid"], observed.pgid);
    assert_eq!(metadata["start_time_ticks"], observed.start_time_ticks);
}

#[cfg(target_os = "linux")]
fn native_process_identity(pid: u32) -> Option<NativeProcessIdentity> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let (_, fields) = stat.rsplit_once(") ")?;
    let fields = fields.split_whitespace().collect::<Vec<_>>();
    let pgid = u32::try_from(
        nix::unistd::getpgid(Some(nix::unistd::Pid::from_raw(i32::try_from(pid).ok()?)))
            .ok()?
            .as_raw(),
    )
    .ok()?;
    Some(NativeProcessIdentity {
        pgid,
        start_time_ticks: fields.get(19)?.parse().ok()?,
    })
}

#[cfg(target_os = "macos")]
fn native_process_identity(pid: u32) -> Option<NativeProcessIdentity> {
    let mut process = unsafe { std::mem::zeroed::<nix::libc::proc_bsdinfo>() };
    let process_size = std::mem::size_of::<nix::libc::proc_bsdinfo>();
    if unsafe {
        nix::libc::proc_pidinfo(
            i32::try_from(pid).ok()?,
            nix::libc::PROC_PIDTBSDINFO,
            0,
            &mut process as *mut _ as *mut _,
            i32::try_from(process_size).ok()?,
        )
    } != i32::try_from(process_size).ok()?
    {
        return None;
    }
    let start_time_ticks = process
        .pbi_start_tvsec
        .checked_mul(1_000_000)?
        .checked_add(process.pbi_start_tvusec)?;
    Some(NativeProcessIdentity {
        pgid: process.pbi_pgid,
        start_time_ticks,
    })
}
