//! Machine-load observability for gate runs (#3963).
//!
//! An experiment claimed "serial on an otherwise idle machine" while a
//! 21-hour conversion loop ran `cargo test` in its own worktree for the
//! whole session. The "serial" arm had one competing test binary, the
//! "concurrent" arm had nine, and the difference was reported as if it were
//! the variable under test. The check that would have caught it was one
//! command — `pgrep -af cargo` — and it was never run: what *was* checked
//! was that the process the run itself had started had finished, and
//! idleness was inferred from that. "The thing I started is done" is not
//! "nothing is running".
//!
//! The policy is encoded here as pure, testable primitives; callers observe
//! the machine before and after the run and record what they saw:
//!
//! 1. **A run that claims an environmental precondition asserts it and
//!    records what it observed.** "No other test process running" is a
//!    check, not a description, and the observation belongs in the result
//!    alongside the numbers ([`parse_process_lines`], [`MachineLoad`]).
//! 2. **A gate result records the machine's concurrent load.** A verdict
//!    produced under contention is different evidence from one produced
//!    alone, and today the two render identically ([`GateRunRecord`]).
//! 3. **A run that claims exclusivity acquires it, or reports that it could
//!    not and labels the result accordingly** — the result is never a clean
//!    serial one when the lease was held by someone else ([`acquire_lease`],
//!    [`LeaseAcquisition`]).
//! 4. **Long-running background work registers itself where a later run
//!    will look**, and the later run reads the registry before it claims
//!    the machine was idle ([`register_job`], [`live_jobs`]).

use std::io::Write;
use std::path::{Path, PathBuf};

/// One observed process: its pid and its full command line, as `pgrep -af`
/// reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessObservation {
    pub pid: u32,
    pub command: String,
}

/// Parse `pgrep -af`-style output: one `<pid> <command...>` per nonblank
/// line.
///
/// Fails closed on a line whose first token is not a numeric pid: an
/// unreadable observation is not "no process", and a gate that parsed
/// half a listing and carried on would claim idleness it never measured.
pub fn parse_process_lines(text: &str) -> Result<Vec<ProcessObservation>, String> {
    let mut out = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((pid_text, command)) = line.split_once(' ') else {
            return Err(format!("process line {} has no command: {line}", index + 1));
        };
        let pid: u32 = pid_text.parse().map_err(|_| {
            format!(
                "process line {} has a non-numeric pid: {pid_text:?} (refusing to read an unreadable observation as \"no process\")",
                index + 1
            )
        })?;
        let command = command.trim();
        if command.is_empty() {
            return Err(format!("process line {} has an empty command", index + 1));
        }
        out.push(ProcessObservation {
            pid,
            command: command.to_string(),
        });
    }
    Ok(out)
}

/// Whether a command line is a competing *test* process: `test` appears as
/// a whole argv token (`cargo +1.91.0 test -p autospec-core --no-fail-fast`,
/// `timeout 2700 cargo test ...`).
///
/// A competing build is load but not a competing test binary, and
/// `test` inside another word (`latest`, `contest`) is not a test process:
/// the token check is on argv words, not substrings.
pub fn is_test_process(command: &str) -> bool {
    command
        .split_whitespace()
        .any(|token| token == "test" || token.ends_with("/test"))
}

/// What a gate run observed about the machine's competing test processes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MachineLoad {
    competing: Vec<ProcessObservation>,
}

impl MachineLoad {
    /// The observed load, given the run's own pid, its ancestor pids, and
    /// the full process listing.
    ///
    /// Self and ancestors never count against the run — the `cargo test`
    /// the gate itself started is not contention — and non-test processes
    /// are not competing test binaries. Everything else in the listing that
    /// is running a test is, by definition, contention.
    pub fn observe(self_pid: u32, ancestors: &[u32], processes: &[ProcessObservation]) -> Self {
        let competing = processes
            .iter()
            .filter(|p| p.pid != self_pid && !ancestors.contains(&p.pid))
            .filter(|p| is_test_process(&p.command))
            .cloned()
            .collect();
        Self { competing }
    }

    /// No competing test process observed.
    pub fn is_idle(&self) -> bool {
        self.competing.is_empty()
    }

    /// The competing test processes, observation order.
    pub fn competing(&self) -> &[ProcessObservation] {
        &self.competing
    }

    /// The observation line, recorded in the result alongside the numbers.
    /// Idleness is reported as *observed*, never assumed.
    pub fn line(&self) -> String {
        if self.competing.is_empty() {
            "machine idle: no competing test process observed".to_string()
        } else {
            let list = self
                .competing
                .iter()
                .map(|p| format!("{} {}", p.pid, p.command))
                .collect::<Vec<_>>()
                .join("; ");
            format!(
                "machine contended: {} competing test process(es): {list}",
                self.competing.len()
            )
        }
    }
}

/// The holder of a named gate lease, as recorded in the lease's owner file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaseHolder {
    pub name: String,
    pub pid: u32,
    pub command: String,
}

/// The result of asking the machine for exclusivity.
#[derive(Debug)]
pub enum LeaseAcquisition {
    /// The lease is ours for as long as the handle lives; dropping it
    /// releases the lease.
    Exclusive(LeaseHandle),
    /// A live process holds the lease; the run must label its result as
    /// non-exclusive rather than claim a clean serial condition.
    Contended(LeaseHolder),
}

impl LeaseAcquisition {
    /// Whether exclusivity was acquired.
    pub fn is_exclusive(&self) -> bool {
        matches!(self, Self::Exclusive { .. })
    }

    /// The line recorded in the result, for either outcome: the failure to
    /// acquire is reported, with the holder named.
    pub fn line(&self) -> String {
        match self {
            Self::Exclusive(handle) => format!("exclusivity: held lease {}", handle.name()),
            Self::Contended(holder) => format!(
                "exclusivity: NOT held — lease {} held by {} {}; this result ran under contention",
                holder.name, holder.pid, holder.command
            ),
        }
    }
}

/// A held gate lease. Releases itself when dropped.
#[derive(Debug)]
pub struct LeaseHandle {
    dir: PathBuf,
    name: String,
}

impl LeaseHandle {
    /// The lease name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The lease directory, for a caller that needs to point at it.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Release the lease now; dropping releases it as well.
    pub fn release(&mut self) {
        remove_best_effort(&self.dir.join("owner"));
        remove_best_effort(&self.dir);
    }
}

impl Drop for LeaseHandle {
    fn drop(&mut self) {
        self.release();
    }
}

fn remove_best_effort(path: &Path) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_dir(path);
}

/// Acquire the named gate lease under `base_dir`, or report the live holder.
///
/// The lease is a directory with an `owner` file naming the holder's pid;
/// the file is created with `O_EXCL`, so two runs cannot both take it. A
/// holder whose process is dead left a stale lease: it is reclaimed once.
/// A lease that still cannot be taken is reported as contended — never
/// stolen, never read as "nobody is home".
pub fn acquire_lease(
    base_dir: &Path,
    name: &str,
    pid: u32,
    command: &str,
) -> Result<LeaseAcquisition, String> {
    if name.is_empty() || name.contains('/') || name == "." || name == ".." {
        return Err(format!("unsafe lease name: {name:?}"));
    }
    let dir = base_dir.join(format!("{name}.lease"));
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("create lease dir {}: {e}", dir.display()))?;
    let owner = dir.join("owner");
    for _ in 0..2 {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&owner)
        {
            Ok(mut file) => {
                let _ = writeln!(file, "name={name}");
                let _ = writeln!(file, "pid={pid}");
                let _ = writeln!(file, "command={command}");
                return Ok(LeaseAcquisition::Exclusive(LeaseHandle {
                    dir: dir.clone(),
                    name: name.to_string(),
                }));
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(format!("open lease {}: {e}", owner.display())),
        }
        // Someone holds it. Read who, and whether they are still there.
        let holder = read_owner(&owner)?;
        if process_alive(holder.pid) {
            return Ok(LeaseAcquisition::Contended(holder));
        }
        // Stale: the holder is gone. Reclaim and try once more; if a live
        // run took it in the meantime, the second round reports that run.
        remove_best_effort(&owner);
    }
    Err(format!(
        "lease {} could not be taken and its holder could not be resolved; fail closed",
        dir.display()
    ))
}

fn read_owner(owner: &Path) -> Result<LeaseHolder, String> {
    let text = std::fs::read_to_string(owner)
        .map_err(|e| format!("read lease owner {}: {e}", owner.display()))?;
    let mut holder = LeaseHolder {
        name: String::new(),
        pid: 0,
        command: String::new(),
    };
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("name=") {
            holder.name = value.to_string();
        } else if let Some(value) = line.strip_prefix("pid=") {
            holder.pid = value
                .parse()
                .map_err(|_| format!("lease owner {} has a non-numeric pid", owner.display()))?;
        } else if let Some(value) = line.strip_prefix("command=") {
            holder.command = value.to_string();
        }
    }
    if holder.pid == 0 {
        return Err(format!(
            "lease owner {} names no pid; a lease nobody can be checked against is contended, not idle",
            owner.display()
        ));
    }
    Ok(holder)
}

/// Whether a process is alive. Fail closed: on platforms where liveness
/// cannot be checked, assume the holder is still there.
#[cfg(unix)]
fn process_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }
    let alive = unsafe { kill(pid as i32, 0) == 0 };
    if alive {
        return true;
    }
    // EPERM: the process exists but belongs to someone else.
    std::io::Error::last_os_error().raw_os_error() == Some(1)
}

#[cfg(not(unix))]
fn process_alive(_pid: u32) -> bool {
    true
}

/// One registered long-running job — the record a later run will look for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobRecord {
    pub name: String,
    pub pid: u32,
    pub command: String,
    /// When the job registered, RFC 3339 or the registrant's own clock.
    pub started: String,
}

fn safe_job_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.contains('/') || name == "." || name == ".." {
        return Err(format!("unsafe job name: {name:?}"));
    }
    Ok(())
}

/// Register a long-running job under `dir` so that a later run looking
/// there finds it. Re-registering under the same name refreshes the record.
pub fn register_job(dir: &Path, record: &JobRecord) -> Result<(), String> {
    safe_job_name(&record.name)?;
    std::fs::create_dir_all(dir).map_err(|e| format!("create job registry dir {dir:?}: {e}"))?;
    let path = dir.join(&record.name);
    let text = format!(
        "name={}\npid={}\ncommand={}\nstarted={}\n",
        record.name, record.pid, record.command, record.started
    );
    std::fs::write(&path, text).map_err(|e| format!("write job record {path:?}: {e}"))
}

/// Remove a job registration. A missing record is not an error: the job
/// already left.
pub fn unregister_job(dir: &Path, name: &str) -> Result<(), String> {
    safe_job_name(name)?;
    remove_best_effort(&dir.join(name));
    Ok(())
}

fn read_job(path: &Path) -> Result<Option<JobRecord>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("read job record {}: {e}", path.display())),
    };
    let mut record = JobRecord {
        name: String::new(),
        pid: 0,
        command: String::new(),
        started: String::new(),
    };
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("name=") {
            record.name = value.to_string();
        } else if let Some(value) = line.strip_prefix("pid=") {
            record.pid = value
                .parse()
                .map_err(|_| format!("job record {} has a non-numeric pid", path.display()))?;
        } else if let Some(value) = line.strip_prefix("command=") {
            record.command = value.to_string();
        } else if let Some(value) = line.strip_prefix("started=") {
            record.started = value.to_string();
        }
    }
    if record.pid == 0 || record.name.is_empty() {
        return Err(format!(
            "job record {} names no pid or no name; an unreadable registration is a finding, not absence",
            path.display()
        ));
    }
    Ok(Some(record))
}

/// Every live registered job under `dir`, sorted by name.
///
/// A record whose process is dead is pruned as it is found: the registry
/// reports who is running, and a dead pid is not running. A record that
/// cannot be read is an error — fail closed, like the lease.
pub fn live_jobs(dir: &Path) -> Result<Vec<JobRecord>, String> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("read job registry dir {dir:?}: {e}")),
    };
    let mut jobs = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("read job registry entry: {e}"))?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(record) = read_job(&path)? else {
            continue;
        };
        if process_alive(record.pid) {
            jobs.push(record);
        } else {
            // The job ended without unregistering; its record would lie.
            remove_best_effort(&path);
        }
    }
    jobs.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(jobs)
}

/// The exclusivity a run held (or failed to hold) for its gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Exclusivity {
    /// The run made no exclusivity claim; nothing is asserted.
    NotClaimed,
    /// The run held the named lease for the whole run.
    Held { name: String },
    /// The run could not take the lease; the result is not a clean serial
    /// condition, and the record says so.
    Contended { holder: LeaseHolder },
}

impl Exclusivity {
    /// The recorded line for the claim.
    pub fn line(&self) -> String {
        match self {
            Self::NotClaimed => "exclusivity: not claimed".to_string(),
            Self::Held { name } => format!("exclusivity: held lease {name}"),
            Self::Contended { holder } => format!(
                "exclusivity: NOT held — lease {} held by {} {}; this result ran under contention",
                holder.name, holder.pid, holder.command
            ),
        }
    }
}

/// The recorded result of one gate run: the result itself, plus what the
/// run observed about the machine, plus whether the run held the
/// exclusivity it claimed.
///
/// The three are recorded together because a verdict produced under
/// contention is different evidence from one produced alone, and a
/// precondition that was asserted rather than measured is not a control.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateRunRecord {
    /// The gate's own result line, e.g. "gate clean: every evaluated check found nothing".
    pub result: String,
    /// What the run observed on the machine.
    pub load: MachineLoad,
    /// Whether the run held the exclusivity it claimed.
    pub exclusivity: Exclusivity,
}

impl GateRunRecord {
    pub fn new(result: impl Into<String>, load: MachineLoad, exclusivity: Exclusivity) -> Self {
        Self {
            result: result.into(),
            load,
            exclusivity,
        }
    }

    /// The recorded line: result, observation, exclusivity. A record whose
    /// result says "clean" while its observation says "contended" reads as
    /// a contradiction on one line — that is the point.
    pub fn line(&self) -> String {
        format!(
            "{} | {} | {}",
            self.result,
            self.load.line(),
            self.exclusivity.line()
        )
    }

    /// Whether this record may be reported as having run serially on an
    /// idle machine: only when the observation says idle and the run was
    /// not contended for a lease it claimed.
    pub fn claims_idle_machine(&self) -> bool {
        self.load.is_idle() && !matches!(self.exclusivity, Exclusivity::Contended { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn temp_dir() -> PathBuf {
        let n = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("autospec-gate-load-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The process listing from the incident: a 21-hour conversion loop
    /// running `cargo test` in its own worktree, alongside the gate's own
    /// process tree.
    const INCIDENT_LISTING: &str = "\
104486 /bin/bash
110502 timeout 2700 cargo +1.91.0 test -p autospec-cli -p autospec-core --no-fail-fast
110503 /bin/sh -c autospec-core
110504 autospec-core-9f21d4a8d3e25e6d";

    #[test]
    fn parses_a_pgrep_listing() {
        let processes = parse_process_lines(INCIDENT_LISTING).unwrap();
        assert_eq!(processes.len(), 4);
        assert_eq!(processes[0].pid, 104486);
        assert_eq!(processes[0].command, "/bin/bash");
        assert_eq!(
            processes[1].command,
            "timeout 2700 cargo +1.91.0 test -p autospec-cli -p autospec-core --no-fail-fast"
        );
    }

    #[test]
    fn an_unreadable_listing_fails_closed() {
        assert!(parse_process_lines("not-a-pid cargo test").is_err());
        assert!(parse_process_lines("12345").is_err());
        assert_eq!(
            parse_process_lines("").unwrap(),
            Vec::<ProcessObservation>::new()
        );
    }

    #[test]
    fn a_test_process_is_one_running_test_as_an_argv_token() {
        assert!(is_test_process(
            "timeout 2700 cargo +1.91.0 test -p autospec-core --no-fail-fast"
        ));
        assert!(is_test_process("cargo test"));
        assert!(!is_test_process("cargo build --workspace"));
        assert!(!is_test_process("/bin/bash"));
        // Substring matches are not test processes.
        assert!(!is_test_process("cargo update latest"));
    }

    #[test]
    fn self_and_ancestors_do_not_count_as_contention() {
        let processes = parse_process_lines(INCIDENT_LISTING).unwrap();
        // The gate is pid 110504 (its own test binary), under 110503 under
        // 110502 — the loop's own tree. None of it is contention against
        // itself.
        let load = MachineLoad::observe(110504, &[110503, 110502, 104486], &processes);
        assert!(load.is_idle());
        assert_eq!(
            load.line(),
            "machine idle: no competing test process observed"
        );
    }

    #[test]
    fn a_gate_started_while_another_test_binary_runs_reports_contention() {
        // Populated case: the gate (pid 200000, ancestors [199999]) starts
        // while the incident loop's test binary is running.
        let processes = parse_process_lines(INCIDENT_LISTING).unwrap();
        let load = MachineLoad::observe(200000, &[199999], &processes);
        assert!(!load.is_idle());
        assert_eq!(load.competing().len(), 1);
        assert_eq!(load.competing()[0].pid, 110502);
        assert!(load
            .line()
            .contains("machine contended: 1 competing test process(es)"));
        assert!(load.line().contains("110502"));

        // And the recorded run does not claim a clean serial condition.
        let record = GateRunRecord::new(
            "gate clean: every evaluated check found nothing",
            load,
            Exclusivity::NotClaimed,
        );
        assert!(!record.claims_idle_machine());
        assert!(record.line().contains("machine contended"));
    }

    /// The pid of an already-exited child: dead for the rest of the test.
    fn dead_pid() -> u32 {
        let mut child = std::process::Command::new("true")
            .stdin(std::process::Stdio::null())
            .spawn()
            .expect("spawn `true`");
        let pid = child.id();
        child.wait().expect("reap child");
        pid
    }

    #[test]
    fn a_run_claims_exclusivity_and_holds_the_lease() {
        let dir = temp_dir();
        let me = std::process::id();
        let acq = acquire_lease(&dir, "cargo-test", me, "cargo test --workspace").unwrap();
        assert!(acq.is_exclusive());
        assert!(acq.line().contains("held lease cargo-test"));
        let dir2 = dir.clone();
        drop(acq);
        // Released: the lease is takeable again.
        let acq2 = acquire_lease(&dir2, "cargo-test", me, "cargo test").unwrap();
        assert!(acq2.is_exclusive());
        drop(acq2);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_contended_lease_is_reported_with_the_holder_named() {
        let dir = temp_dir();
        let me = std::process::id();
        let first = acquire_lease(&dir, "cargo-test", me, "cargo test --workspace").unwrap();
        assert!(first.is_exclusive());
        // A second run asks for the same machine and finds it taken by a
        // live process: it does not steal the lease, it reports the holder.
        let second =
            acquire_lease(&dir, "cargo-test", me + 1, "cargo test -p autospec-core").unwrap();
        match &second {
            LeaseAcquisition::Contended(holder) => {
                assert_eq!(holder.name, "cargo-test");
                assert_eq!(holder.pid, me);
                assert!(holder.command.contains("cargo test"));
            }
            other => panic!("expected contended, got {other:?}"),
        }
        assert!(!second.is_exclusive());
        assert!(second.line().contains("NOT held"));
        assert!(second.line().contains(&me.to_string()));

        // The contended run's record cannot claim a clean serial condition.
        let record = GateRunRecord::new(
            "gate clean: every evaluated check found nothing",
            MachineLoad::default(),
            Exclusivity::Contended {
                holder: LeaseHolder {
                    name: "cargo-test".into(),
                    pid: me,
                    command: "cargo test --workspace".into(),
                },
            },
        );
        assert!(!record.claims_idle_machine());
        drop(first);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_dead_holders_lease_is_reclaimed() {
        let dir = temp_dir();
        let dead = dead_pid();
        // Seed a stale lease under the name.
        let lease_dir = dir.join("cargo-test.lease");
        std::fs::create_dir_all(&lease_dir).unwrap();
        std::fs::write(
            lease_dir.join("owner"),
            format!("name=cargo-test\npid={dead}\ncommand=cargo test --workspace\n"),
        )
        .unwrap();
        let me = std::process::id();
        let acq = acquire_lease(&dir, "cargo-test", me, "cargo test").unwrap();
        assert!(
            acq.is_exclusive(),
            "a stale lease from a dead pid must be reclaimed, not reported as contention"
        );
        drop(acq);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_unsafe_lease_name_is_refused() {
        let dir = temp_dir();
        assert!(acquire_lease(&dir, "../escape", 1, "x").is_err());
        assert!(acquire_lease(&dir, "", 1, "x").is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn registered_jobs_are_visible_to_a_later_run_and_dead_ones_pruned() {
        let dir = temp_dir();
        let me = std::process::id();
        let dead = dead_pid();
        register_job(
            &dir,
            &JobRecord {
                name: "conversion-loop".into(),
                pid: me,
                command: "autospec-run monitor".into(),
                started: "2026-09-21T02:47:25Z".into(),
            },
        )
        .unwrap();
        register_job(
            &dir,
            &JobRecord {
                name: "old-gate".into(),
                pid: dead,
                command: "cargo test --workspace".into(),
                started: "2026-09-20T00:00:00Z".into(),
            },
        )
        .unwrap();

        // A later run looks where the work registered itself.
        let jobs = live_jobs(&dir).unwrap();
        assert_eq!(
            jobs.len(),
            1,
            "the dead job must be pruned, the live one listed"
        );
        assert_eq!(jobs[0].name, "conversion-loop");
        assert_eq!(jobs[0].pid, me);
        // The dead record was removed, not just filtered: a second look
        // agrees.
        assert!(live_jobs(&dir)
            .unwrap()
            .iter()
            .all(|j| j.name != "old-gate"));

        unregister_job(&dir, "conversion-loop").unwrap();
        assert!(live_jobs(&dir).unwrap().is_empty());
        // Unregistering twice is not an error: the job already left.
        unregister_job(&dir, "conversion-loop").unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_unreadable_job_record_fails_closed() {
        let dir = temp_dir();
        std::fs::write(dir.join("broken"), "nope").unwrap();
        assert!(live_jobs(&dir).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_record_reports_idle_only_when_observed_idle_and_uncontended() {
        let record = GateRunRecord::new(
            "gate clean: every evaluated check found nothing",
            MachineLoad::default(),
            Exclusivity::Held {
                name: "cargo-test".into(),
            },
        );
        assert!(record.claims_idle_machine());
        assert!(record
            .line()
            .contains("machine idle: no competing test process observed"));
        assert!(record.line().contains("exclusivity: held lease cargo-test"));
    }
}
