//! #4572: the pass's progress lines are flushed as they are made.
//!
//! With stdout on a pipe or a file (the normal case for anything scheduled),
//! Rust block-buffers it, so an unflushed decision line makes a working pass
//! look dead. These tests run the real pass against a real fixture crate and
//! a real cargo gate, and assert that the decision lines arrive while the
//! process is still running — which only happens if they were flushed.

use std::fs;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

fn autospec_bin() -> &'static str {
    env!("CARGO_BIN_EXE_autospec")
}

fn run_git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .status()
        .expect("git runs");
    assert!(status.success(), "git {args:?} failed in {dir:?}");
}

fn run_git_output(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git runs");
    String::from_utf8_lossy(&output.stdout)
        .trim_end()
        .to_string()
}

/// A minimal, gate-green Rust crate. The agent's patch will add a module
/// whose test fails, so the gate runs every stage — fmt, build, clippy —
/// before it dies at test: a multi-second run with nothing to flush but our
/// progress lines.

/// Record the gate the pass will run: without a recorded gate the pass
/// refuses to judge at all (#4556), so a fixture that exercises the gate
/// records it the way a real checkout carries its registry file.
fn record_gate(repo: &Path) {
    let data = repo.join("data");
    fs::create_dir_all(&data).expect("data dir");
    fs::write(
        data.join("convert-gate-registry.json"),
        r#"{"schema":1,"repos":{"test/fake":{"base_ref":"main","stages":[["fmt","--check"],["build","@scope"],["clippy","--all-targets","@scope"],["test","--no-fail-fast","@scope"]]}}}"#,
    )
    .expect("registry");
}

fn init_fixture_repo(repo: &Path, origin: &Path) -> String {
    fs::create_dir_all(repo.join("src")).unwrap();
    fs::write(
        repo.join("Cargo.toml"),
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n",
    )
    .unwrap();
    fs::write(repo.join("src/lib.rs"), "pub mod a;\n").unwrap();
    fs::write(repo.join("src/a.rs"), "pub fn a() -> u32 {\n    1\n}\n").unwrap();
    run_git(repo, &["init", "-q", "-b", "main"]);
    let status = Command::new("git")
        .args(["init", "-q", "--bare", origin.to_str().unwrap()])
        .current_dir(repo)
        .status()
        .expect("git runs");
    assert!(status.success());
    run_git(repo, &["add", "-A"]);
    run_git(repo, &["commit", "-q", "-m", "base"]);
    run_git(repo, &["remote", "add", "origin", origin.to_str().unwrap()]);
    run_git(repo, &["push", "-q", "origin", "main"]);
    run_git_output(repo, &["rev-parse", "main:src/lib.rs"])
}

/// The agent's patch: a new module appended to the index (main never moved,
/// so it applies cleanly) and the module file, which carries a failing test.
fn agent_patch(lib_blob: &str) -> String {
    format!(
        "diff --git a/src/lib.rs b/src/lib.rs\nindex {lib_blob}..0000000\n\
         --- a/src/lib.rs\n+++ b/src/lib.rs\n\
         @@ -1 +1,2 @@\n pub mod a;\n+pub mod broken;\n\
         diff --git a/src/broken.rs b/src/broken.rs\nnew file mode 100644\n\
         --- /dev/null\n+++ b/src/broken.rs\n\
         @@ -0,0 +1,4 @@\n+\
         #[test]\n+fn deliberately_fails() {{\n+    assert!(false, \"the progress test needs a failing gate\");\n+}}\n"
    )
}

fn write_patch(llm_root: &Path, issue: u64, text: &str) {
    let issue_dir = llm_root
        .join("node-a")
        .join("out")
        .join(format!("issue-{issue}"));
    fs::create_dir_all(&issue_dir).unwrap();
    fs::write(issue_dir.join("changes.patch"), text).unwrap();
}

/// A stand-in for `gh` on PATH: it succeeds and records nothing, so the
/// pass's liveness check sees no live PR and the run reaches its gate.
fn install_fake_gh(work: &Path) -> PathBuf {
    let bin_dir = work.join("fakebin");
    fs::create_dir_all(&bin_dir).unwrap();
    let shim = bin_dir.join("gh");
    fs::write(&shim, "#!/bin/sh\nexit 0\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&shim, fs::Permissions::from_mode(0o755)).unwrap();
    }
    bin_dir
}

/// The whole point of #4572: run the pass with stdout on a pipe and watch
/// the decision lines arrive while the gate is still running.
#[test]
fn decision_lines_arrive_while_the_pass_is_still_running() {
    let work = std::env::temp_dir().join(format!("autospec-conv-progress-{}", std::process::id()));
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).unwrap();
    let repo = work.join("repo");
    let origin = work.join("origin.git");
    let llm_root = work.join("llm");
    let stderr_path = work.join("stderr.log");
    let stderr_file = fs::File::create(&stderr_path).unwrap();
    let lib_blob = init_fixture_repo(&repo, &origin);
    record_gate(&repo);
    write_patch(&llm_root, 201, &agent_patch(&lib_blob));
    let bin_dir = install_fake_gh(&work);

    let mut path = bin_dir.to_string_lossy().into_owned();
    path.push(':');
    path.push_str(&std::env::var("PATH").unwrap_or_default());

    let held_path = llm_root.join("held.txt");
    let mut child = Command::new(autospec_bin())
        .args([
            "convert",
            "--apply",
            "--llm-root",
            llm_root.to_str().unwrap(),
            "--base",
            "main",
            "--repo",
            "test/fake",
        ])
        .current_dir(&repo)
        .env("LLM", "/nonexistent-llm-for-this-test")
        .env("PATH", path)
        // The pass commits the converted work; the identity must not depend
        // on the ambient git config (CI runners have none).
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .stdout(Stdio::piped())
        .stderr(Stdio::from(stderr_file))
        .spawn()
        .expect("autospec convert runs");

    // Read stdout line by line and hand each line to the main thread as it
    // arrives. A line that only arrives at process exit is an unflushed
    // buffer being released, and the test must be able to tell.
    let stdout = child.stdout.take().expect("stdout is piped");
    let (tx, rx) = mpsc::channel::<String>();
    let reader = thread::spawn(move || {
        let mut reader = std::io::BufReader::new(stdout);
        let mut line = String::new();
        loop {
            line.clear();
            let n = match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            };
            let _ = n;
            if tx.send(line.trim_end().to_string()).is_err() {
                break;
            }
        }
    });

    // Wait for a GATE line, then check the process's state at that moment.
    // Lines that arrive first (the startup banner) are kept, not dropped:
    // they are part of the output the assertions run against.
    let deadline = Instant::now() + Duration::from_secs(300);
    let mut lines: Vec<String> = Vec::new();
    let mut gate_line = String::new();
    while Instant::now() < deadline {
        let line = match rx.recv_timeout(Duration::from_secs(1)) {
            Ok(line) => line,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        lines.push(line.clone());
        if line.contains("GATE  #201") && gate_line.is_empty() {
            gate_line = line;
            break;
        }
    }
    assert!(
        !gate_line.is_empty(),
        "no GATE line arrived within the deadline: {lines:?}"
    );

    // The pass must still be running when its gate announces itself: the
    // gate takes multi seconds, and a line released at exit is not a line
    // made while working. This is the assertion a block-buffered stdout
    // fails.
    let still_running = child.try_wait().expect("try_wait").is_none();
    assert!(
        still_running,
        "the GATE line ({gate_line:?}) arrived after the process exited: \
         the decision was not flushed while the gate was running"
    );

    // Drain the rest: the remaining stage lines, the hold decision, the summary.
    for line in rx.iter() {
        lines.push(line);
    }
    reader.join().expect("the reader thread ends");
    let status = child.wait().expect("the pass exits");
    let output = lines.join("\n");

    let stderr = fs::read_to_string(&stderr_path).unwrap_or_default();
    assert!(
        status.success(),
        "the pass itself fails:\n{output}\nstderr:\n{stderr}"
    );
    // Acceptance 3: the startup banner names the held-ledger path.
    assert!(
        output
            .lines()
            .any(|l| l == format!("held ledger: {}", held_path.display())),
        "the startup banner must name the held ledger:\n{output}"
    );
    // Acceptance 2: the gate begins with a line naming the patch and the
    // derived scope, and the stages follow — test last, the longest one.
    assert!(
        output.contains("  GATE  #201: test"),
        "the test stage must announce itself before it runs:\n{output}\nstderr:\n{stderr}"
    );
    // Acceptance 1: the decision line arrives when the decision is made.
    let held_idx = output
        .lines()
        .position(|l| l.contains("  HELD  #201"))
        .unwrap_or_else(|| panic!("no HELD line for the failing patch:\n{output}"));
    let gate_idx = output
        .lines()
        .position(|l| l.contains("  GATE  #201"))
        .unwrap();
    assert!(
        gate_idx < held_idx,
        "the gate begins before it decides:\n{output}"
    );
    assert!(
        output.contains("convert: examined=1 converted=0 held=1 skipped=0"),
        "the summary must account for the hold:\n{output}"
    );
    // The durable channel: the ledger holds the same decision, and it exists
    // on disk whether or not anyone was reading the log.
    let ledger = fs::read_to_string(&held_path)
        .unwrap_or_else(|_| panic!("the held ledger was not written at {}", held_path.display()));
    assert!(
        ledger.lines().count() == 1 && ledger.contains("\"issue\":201"),
        "the ledger must carry the hold:\n{ledger}"
    );

    let _ = fs::remove_dir_all(&work);
}

/// The banner in plan mode too: the operator reads the ledger the pass will
/// write, not the buffer it is about to lie in.
#[test]
fn the_plan_banner_names_the_held_ledger() {
    let work = std::env::temp_dir().join(format!(
        "autospec-conv-progress-plan-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).unwrap();
    let repo = work.join("repo");
    let origin = work.join("origin.git");
    let llm_root = work.join("llm");
    let lib_blob = init_fixture_repo(&repo, &origin);
    record_gate(&repo);
    write_patch(&llm_root, 201, &agent_patch(&lib_blob));
    let bin_dir = install_fake_gh(&work);

    let mut path = bin_dir.to_string_lossy().into_owned();
    path.push(':');
    path.push_str(&std::env::var("PATH").unwrap_or_default());

    let output = Command::new(autospec_bin())
        .args([
            "convert",
            "--llm-root",
            llm_root.to_str().unwrap(),
            "--base",
            "main",
        ])
        .current_dir(&repo)
        .env("LLM", "/nonexistent-llm-for-this-test")
        .env("PATH", path)
        .output()
        .expect("autospec convert runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success(), "plan mode fails:\n{stdout}");
    let held_path = llm_root.join("held.txt");
    assert!(
        stdout
            .lines()
            .any(|l| l == format!("held ledger: {}", held_path.display())),
        "plan mode must name the held ledger at startup:\n{stdout}"
    );

    let _ = fs::remove_dir_all(&work);
}
