//! T16 (D1/D2): the run-drain stale-handle recovery merges only through the review-quorum gate,
//! pinned to the head the gate returned, and logs a refusal instead of discarding it.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

fn write_exe(path: &Path, body: &str) {
    fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

struct Drain {
    home: PathBuf,
    bin: PathBuf,
}

impl Drain {
    fn new(name: &str, autospec: Option<&str>) -> Self {
        let dir = std::env::temp_dir().join(format!("{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let (home, bin) = (dir.join("home"), dir.join("bin"));
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&bin).unwrap();
        write_exe(
            &bin.join("codex"),
            "echo 'codex_core::tools::router: error=write_stdin failed: Unknown process id 83740' >&2\nexit 1",
        );
        let h = home.display();
        write_exe(
            &bin.join("gh"),
            &format!(
                r#"echo "gh $*" >> {h}/gh.log
case "$*" in
  *"repo view"*) echo berlinguyinca/autospec ;;
  "pr list --repo berlinguyinca/autospec --state open --json"*) echo '[{{"number":1578,"headRefName":"feat/issue-1545-x","isDraft":false,"statusCheckRollup":[{{"name":"pytest","status":"COMPLETED","conclusion":"SUCCESS"}}]}}]' ;;
  "issue view 1545 "*) echo '{{"state":"OPEN","labels":[{{"name":"in-progress-by-bot"}}]}}' ;;
  "pr merge "*) echo '{{"state":"MERGED"}}' > {h}/pr.json ;;
  "pr view 1578 "*) cat {h}/pr.json 2>/dev/null || echo '{{"state":"OPEN"}}' ;;
  *) exit 0 ;;
esac"#
            ),
        );
        if let Some(body) = autospec {
            write_exe(&bin.join("autospec"), body);
        }
        Self { home, bin }
    }

    fn run(&self) -> (String, String) {
        let out = Command::new("bash")
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../scripts/autospec-autonomous-run-drain.sh"
            ))
            .env("HOME", &self.home)
            .env("PATH", format!("{}:/usr/bin:/bin", self.bin.display()))
            .env("AUTOSPEC_CONFIG_FILE", self.home.join("missing.yml"))
            .env("AUTOSPEC_AUTONOMOUS_DRAIN_STALL_SECS", "10")
            .env("AUTOSPEC_AUTONOMOUS_DRAIN_POLL_SECS", "1")
            .output()
            .unwrap();
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    fn gh_log(&self) -> String {
        fs::read_to_string(self.home.join("gh.log")).unwrap_or_default()
    }
}

#[test]
fn d1_a_refused_quorum_merges_nothing_and_the_refusal_is_logged() {
    let d = Drain::new(
        "drain-refused",
        Some(&format!(
            "echo \"$*\" >> {}/autospec.args\necho '{{\"ok\":false,\"reasons\":[\"review quorum required\"]}}'\nexit 3",
            "$HOME"
        )),
    );
    let (out, err) = d.run();
    assert!(!d.gh_log().contains("pr merge"), "{}", d.gh_log());
    let args = fs::read_to_string(d.home.join("autospec.args")).unwrap_or_default();
    assert!(
        args.contains("review-quorum merge-gate")
            && args.contains("--pr 1578")
            && args.contains("--script-version 2"),
        "the gate must be asked, strictly, about this PR: {args}"
    );
    let all = format!("{out}{err}");
    assert!(all.contains("PR #1578 not merged, review quorum"), "{all}");
    assert!(
        all.contains("review quorum required"),
        "the reason is kept: {all}"
    );
}

#[test]
fn d1_without_an_autospec_binary_nothing_merges() {
    let d = Drain::new("drain-nobin", None);
    d.run();
    assert!(!d.gh_log().contains("pr merge"), "{}", d.gh_log());
}

#[test]
fn d2_the_merge_is_pinned_to_the_head_the_gate_returned() {
    let d = Drain::new(
        "drain-pin",
        Some("echo '{\"ok\":true,\"merge_head\":\"abc123\"}'"),
    );
    let (out, err) = d.run();
    let log = d.gh_log();
    assert!(
        log.contains("pr merge 1578 --repo berlinguyinca/autospec --admin --squash --delete-branch --match-head-commit abc123"),
        "{log}\n{out}{err}"
    );
}

#[test]
fn the_conductor_rollup_gate_logs_its_output_instead_of_discarding_it() {
    let text = fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../scripts/lib/autospec-loop.sh"
    ))
    .unwrap();
    for line in text
        .lines()
        .filter(|l| l.contains("review-quorum merge-gate"))
    {
        assert!(
            !line.contains("2>/dev/null"),
            "a gate refusal must reach the log: {line}"
        );
    }
}
