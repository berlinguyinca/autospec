use std::time::Duration;

use autospec_core::autonomous::tier2::{Tier2FailureCode, Tier2Stage};

use super::tier2_receipts_tests::collector;
use super::tier2_runner::{
    create_model_scratch, run_bounded_child, scan_collected_with, scan_native, HarnessInvocation,
    HarnessKind, ModelStage, Tier2Scan,
};

const GENERATED: &str = r#"{
  "proposals": [{
    "stable_key": "add-trading-smoke-test",
    "title": "test: add a trading configuration smoke test",
    "evidence": [{"file":"Cargo.toml","line":1,"match":"trading"}],
    "severity": "medium",
    "confidence_millis": 850,
    "complexity": "small",
    "named_consumer": "maintainer"
  }]
}"#;

const VERIFIED: &str = r#"{
  "verdicts": [{
    "stable_key": "add-trading-smoke-test",
    "verdict": "survived",
    "reason": "The cited configuration has no matching smoke test."
  }]
}"#;

#[test]
fn healthy_generator_and_verifier_produce_a_ranked_observation() {
    let scan = scan_collected_with(collector(), |stage, prompt| {
        assert!(prompt.contains("\"domains\""));
        Ok(match stage {
            ModelStage::Generator => GENERATED.to_string(),
            ModelStage::Verifier => VERIFIED.to_string(),
        })
    });

    let Tier2Scan::Complete(observation) = scan else {
        panic!("healthy native discovery must complete");
    };
    assert_eq!(observation.ranked().len(), 1);
    assert_eq!(observation.ranked()[0].stable_key, "add-trading-smoke-test");
}

#[test]
fn empty_generation_completes_only_after_an_empty_verifier_pass() {
    let mut calls = Vec::new();
    let scan = scan_collected_with(collector(), |stage, _| {
        calls.push(stage);
        Ok(match stage {
            ModelStage::Generator => r#"{"proposals":[]}"#.to_string(),
            ModelStage::Verifier => r#"{"verdicts":[]}"#.to_string(),
        })
    });

    let Tier2Scan::Complete(observation) = scan else {
        panic!("healthy empty generation is a completed scan");
    };
    assert!(observation.ranked().is_empty());
    assert_eq!(calls, vec![ModelStage::Generator, ModelStage::Verifier]);
}

#[test]
fn safe_harnesses_receive_evidence_only_prompts_as_one_argument() {
    for kind in [HarnessKind::Codex, HarnessKind::Claude] {
        let invocation = kind
            .invocation("/private/scratch".as_ref(), "{\"domains\":[]}")
            .expect("safe harness");
        assert_eq!(
            invocation.args.last().map(String::as_str),
            Some("{\"domains\":[]}")
        );
        assert_eq!(
            invocation.current_dir,
            std::path::Path::new("/private/scratch")
        );
        assert!(!invocation.args.iter().any(|arg| {
            matches!(
                arg.as_str(),
                "sh" | "bash"
                    | "sh -c"
                    | "omx"
                    | "gh"
                    | "--dangerously-bypass-approvals-and-sandbox"
                    | "--dangerously-skip-permissions"
            )
        }));
    }
    let codex = HarnessKind::Codex
        .invocation("/private/scratch".as_ref(), "{}")
        .expect("safe Codex invocation");
    assert!(codex
        .args
        .windows(2)
        .any(|pair| pair == ["--sandbox", "read-only"]));
    let claude = HarnessKind::Claude
        .invocation("/private/scratch".as_ref(), "{}")
        .expect("safe Claude invocation");
    assert!(claude
        .args
        .windows(2)
        .any(|pair| pair == ["--permission-mode", "plan"]));
}

#[test]
fn opencode_fails_closed_until_it_has_a_proven_no_tools_mode() {
    let error = HarnessKind::OpenCode
        .invocation("/private/scratch".as_ref(), "{}")
        .expect_err("OpenCode --pure still permits built-in mutation tools");

    assert!(error.contains("no-tools"), "unexpected error: {error}");
}

#[cfg(unix)]
#[test]
fn model_scratch_directory_is_private() {
    use std::os::unix::fs::PermissionsExt;

    let scratch = create_model_scratch().expect("private scratch");
    let mode = std::fs::metadata(&scratch)
        .expect("scratch metadata")
        .permissions()
        .mode()
        & 0o777;
    let _ = std::fs::remove_dir(scratch);

    assert_eq!(mode, 0o700);
}

#[test]
fn malformed_generator_output_is_a_sealed_generator_failure() {
    let scan = scan_collected_with(collector(), |_, _| Ok("not-json".to_string()));

    assert_failure(
        scan,
        Tier2Stage::Generator,
        Tier2FailureCode::InvalidProposal,
    );
}

#[test]
fn nonzero_generator_child_is_never_not_run_or_dry() {
    let scan = scan_collected_with(collector(), |_, _| {
        Err("generator child exited 17".to_string())
    });

    assert_failure(
        scan,
        Tier2Stage::Generator,
        Tier2FailureCode::InvalidProposal,
    );
}

#[test]
fn malformed_verifier_output_is_a_sealed_verifier_failure() {
    let scan = scan_collected_with(collector(), |stage, _| {
        Ok(match stage {
            ModelStage::Generator => GENERATED.to_string(),
            ModelStage::Verifier => r#"{"verdicts":"invalid"}"#.to_string(),
        })
    });

    assert_failure(
        scan,
        Tier2Stage::Verifier,
        Tier2FailureCode::InvalidVerdictCoverage,
    );
}

#[cfg(unix)]
#[test]
fn bounded_child_kills_a_timed_out_process_group() {
    let invocation = HarnessInvocation {
        program: "/bin/sleep".into(),
        args: vec!["10".to_string()],
        current_dir: "/tmp".into(),
    };

    let error = run_bounded_child(&invocation, Duration::from_millis(20))
        .expect_err("sleeping child must time out");

    assert!(
        error.contains("timeout"),
        "unexpected timeout error: {error}"
    );
}

#[cfg(target_os = "linux")]
fn process_state(pid: i32) -> Option<char> {
    let stat = match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => stat,
        Err(error)
            if error.kind() == std::io::ErrorKind::NotFound
                || error.raw_os_error() == Some(nix::libc::ESRCH) =>
        {
            return None
        }
        Err(error) => panic!("read process state for PID {pid}: {error}"),
    };
    let close = stat.rfind(')').expect("process stat command terminator");
    stat[close + 1..]
        .split_whitespace()
        .next()
        .expect("process stat state")
        .chars()
        .next()
}

#[cfg(target_os = "linux")]
fn process_is_running(pid: i32) -> bool {
    !matches!(process_state(pid), None | Some('Z' | 'X' | 'x'))
}

#[cfg(target_os = "linux")]
struct ChildFixture(Option<std::process::Child>);

#[cfg(target_os = "linux")]
impl ChildFixture {
    fn spawn(mut command: std::process::Command) -> Self {
        Self(Some(command.spawn().expect("spawn process fixture")))
    }

    fn pid(&self) -> i32 {
        self.0.as_ref().expect("live process fixture").id() as i32
    }

    fn reap(&mut self) {
        self.0
            .take()
            .expect("unreaped process fixture")
            .wait()
            .expect("reap process fixture");
    }
}

#[cfg(target_os = "linux")]
impl Drop for ChildFixture {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[cfg(target_os = "linux")]
#[test]
fn process_running_probe_rejects_zombies_without_hiding_live_children() {
    let mut exited = std::process::Command::new("/bin/sh");
    exited.args(["-c", "exit 0"]);
    let mut zombie = ChildFixture::spawn(exited);
    let zombie_pid = zombie.pid();
    for _ in 0..100 {
        if process_state(zombie_pid) == Some('Z') {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        process_state(zombie_pid),
        Some('Z'),
        "fixture child never reached an unreaped zombie state"
    );
    assert!(
        !process_is_running(zombie_pid),
        "unreaped zombie was classified as running"
    );

    let mut sleeping = std::process::Command::new("/bin/sleep");
    sleeping.arg("30");
    let live = ChildFixture::spawn(sleeping);
    assert!(
        process_is_running(live.pid()),
        "live sleep was classified as terminated"
    );

    zombie.reap();
    assert!(
        !process_is_running(zombie_pid),
        "reaped zombie was classified as running"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn bounded_child_kills_descendants_in_its_process_group() {
    let child_pid_path = std::env::temp_dir().join(format!(
        "autospec-tier2-descendant-{}.pid",
        std::process::id()
    ));
    let script = format!(
        "sleep 30 & printf '%s' \"$!\" > {}; wait",
        child_pid_path.display()
    );
    let invocation = HarnessInvocation {
        program: "/bin/sh".into(),
        args: vec!["-c".to_string(), script],
        current_dir: "/tmp".into(),
    };

    run_bounded_child(&invocation, Duration::from_millis(100))
        .expect_err("parent and descendant must time out");
    let descendant = std::fs::read_to_string(&child_pid_path)
        .expect("child records descendant pid")
        .parse::<i32>()
        .expect("descendant pid is numeric");
    let mut running = process_is_running(descendant);
    for _ in 0..100 {
        if !running {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
        running = process_is_running(descendant);
    }
    let _ = std::fs::remove_file(child_pid_path);

    assert!(
        !running,
        "descendant remained in a running process state after process-group cleanup"
    );
}

#[cfg(unix)]
#[test]
fn bounded_child_stops_oversized_output_while_the_child_is_live() {
    let invocation = HarnessInvocation {
        program: "/usr/bin/yes".into(),
        args: Vec::new(),
        current_dir: "/tmp".into(),
    };
    let started = std::time::Instant::now();

    let error = run_bounded_child(&invocation, Duration::from_secs(5))
        .expect_err("unbounded output must trip the live cap");

    assert!(error.contains("256 KiB"), "unexpected cap error: {error}");
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "output cap must stop the child before its timeout"
    );
}

#[cfg(unix)]
#[test]
fn bounded_child_output_is_private() {
    let invocation = HarnessInvocation {
        program: "/usr/bin/python3".into(),
        args: vec![
            "-c".to_string(),
            "import os; print(oct(os.fstat(1).st_mode & 0o777)[2:])".to_string(),
        ],
        current_dir: "/tmp".into(),
    };

    let mode = run_bounded_child(&invocation, Duration::from_secs(1))
        .expect("child can inspect its output descriptor");

    assert_eq!(mode.trim(), "600");
}

#[cfg(unix)]
#[test]
fn bounded_child_preserves_a_nonzero_status() {
    let invocation = HarnessInvocation {
        program: "/usr/bin/false".into(),
        args: Vec::new(),
        current_dir: "/tmp".into(),
    };

    let error = run_bounded_child(&invocation, Duration::from_secs(1))
        .expect_err("failing child must stay failed");

    assert!(error.contains("exited"), "unexpected child error: {error}");
}

#[test]
fn native_scan_uses_the_supplied_repository_directory_for_collection() {
    let missing = std::env::temp_dir().join(format!(
        "autospec-tier2-missing-root-{}",
        std::process::id()
    ));

    assert_failure(
        scan_native(&missing),
        Tier2Stage::Collector,
        Tier2FailureCode::InvalidRoot,
    );
}

fn assert_failure(scan: Tier2Scan, stage: Tier2Stage, code: Tier2FailureCode) {
    let Tier2Scan::Failed(failure) = scan else {
        panic!("incomplete native discovery must be a failure");
    };
    assert_eq!(failure.stage(), stage);
    assert_eq!(failure.code(), code);
    assert!(failure.documents().is_some(), "failure must be sealed");
}
