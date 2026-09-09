//! `autospec dispatch stage|freshness` — the staged-spec gate (#3864).
//!
//! The bug these tests pin is the one nobody sees until the work is wrong: a
//! spec staged on the merge host and dispatched on a cluster whose `gh` cannot
//! read the issue. The staged copy is then the only source of truth, so a
//! comment filed after staging is simply absent, and a dispatcher that cannot
//! tell treats yesterday's read as today's instructions.
//!
//! Every assertion is therefore on the exit code a wrapper branches on, on the
//! revision the staged file records, and on the refusal when freshness is
//! unknowable — the third arm is the one that must never wave work through.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[path = "support/temp_directory.rs"]
mod temp_directory;
use temp_directory::unique as temp_dir;

const SOURCE_UPDATED_AT: &str = "2026-09-08T07:03:00Z";
/// The same instant as epoch seconds; `--live-updated-at` accepts either form.
const SOURCE_EPOCH: &str = "1788850980";
const STAGED_AT: &str = "1788851000";

/// A temp `$HOME`, a temp bin dir for the fake `gh`, and pinned output paths,
/// so no test reads the operator's credentials or writes their real state.
struct Harness {
    temp: PathBuf,
}

impl Harness {
    fn new(tag: &str) -> Self {
        let temp = temp_dir(tag);
        std::fs::create_dir_all(temp.join("home")).expect("home");
        std::fs::create_dir_all(temp.join("bin")).expect("bin");
        Self { temp }
    }

    fn home(&self) -> PathBuf {
        self.temp.join("home")
    }

    /// Write a file under the temp root and return its path as a string.
    fn write(&self, name: &str, text: &str) -> String {
        let path = self.temp.join(name);
        std::fs::write(&path, text).expect("fixture written");
        path.display().to_string()
    }

    /// A `gh` that fails the way an unauthenticated one does on the cluster.
    fn gh_failing(&self) {
        self.gh("echo 'gh:gh: not authenticated' >&2", "exit 1");
    }

    /// A `gh` that answers with `updated_at` on stdout.
    fn gh_answering(&self, updated_at: &str) {
        self.gh(&format!("echo '{updated_at}'"), "exit 0");
    }

    fn gh(&self, body: &str, tail: &str) {
        let path = self.temp.join("bin").join("gh");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n{tail}\n")).expect("gh written");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("gh +x");
    }

    /// A `gh` that answers the issue endpoint with `issue_json` and the
    /// comments endpoint with `comments_json`, so the merge-host staging path
    /// is exercised without a network or a token.
    fn gh_endpoints(&self, issue_json: &str, comments_json: &str) {
        let body = format!(
            "case \"$*\" in\n*comments*) printf '%s\\n' '{comments_json}';;\n*) printf '%s\\n' '{issue_json}';;\nesac"
        );
        self.gh(&body, "exit 0");
    }

    fn stage(&self, args: &[&str]) -> Output {
        self.run("stage", args)
    }

    fn freshness(&self, args: &[&str]) -> Output {
        self.run("freshness", args)
    }

    fn run(&self, sub: &str, args: &[&str]) -> Output {
        self.run_with_env(sub, args, &[])
    }

    fn run_with_env(&self, sub: &str, args: &[&str], extra: &[(&str, &str)]) -> Output {
        let mut argv: Vec<String> = vec!["dispatch".to_string(), sub.to_string()];
        argv.extend(args.iter().map(|arg| arg.to_string()));
        let mut command = Command::new(env!("CARGO_BIN_EXE_autospec"));
        command
            .args(argv)
            // HOME pins the default staged-spec path; PATH selects the fake gh.
            // AUTOSPEC_REPO is cleared so a stray environment cannot supply a
            // repository the test never asked for.
            .env("HOME", self.home())
            .env("PATH", self.temp.join("bin"))
            .env_remove("AUTOSPEC_REPO");
        for (key, value) in extra {
            command.env(key, value);
        }
        command.output().expect("autospec runs")
    }
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

/// An issue payload shaped like `gh api repos/OWNER/REPO/issues/N`, with one
/// comment filed before the last body edit and one after it.
fn issue_payload() -> String {
    format!(
        r#"{{"number": 50, "title": "Stage comments too",
 "body": "Body paragraph one.\n\n- [ ] `autospec dispatch freshness --issue 50` exits 0",
 "updated_at": "{SOURCE_UPDATED_AT}", "body_updated_at": "2026-09-08T07:00:00Z",
 "user": {{"login": "maintainer"}}}}"#
    )
}

fn comments_payload() -> String {
    "[{\"body\": \"Draft, predates the last edit.\", \"created_at\": \"2026-09-07T10:00:00Z\", \"user\": {\"login\": \"alice\"}},
     {\"body\": \"Clarification after the edit.\", \"created_at\": \"2026-09-08T08:15:00Z\", \"user\": {\"login\": \"bob\"}}]"
    .to_string()
}

/// Stage issue 50 into `<temp>/50.md` with the environment block fixed so the
/// rendered text is deterministic, and return that path as a string.
fn stage_issue(harness: &Harness) -> String {
    let issue = harness.write("issue.json", &issue_payload());
    let comments = harness.write("comments.json", &comments_payload());
    let out = harness.temp.join("50.md").display().to_string();
    let output = harness.stage(&[
        "--issue",
        "50",
        "--issue-json",
        &issue,
        "--comments-json",
        &comments,
        "--out",
        &out,
        "--staged-at",
        STAGED_AT,
        "--no-probe",
    ]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    out
}

fn read(path: &str) -> String {
    std::fs::read_to_string(path).expect("staged spec readable")
}

// ------------------------------------------------------- stage via --repo ----

#[test]
fn staging_from_the_repo_pulls_body_and_discussion_in_one_read() {
    // The merge-host path: the issue and its comments come from the same
    // `gh api` family, so a cluster that cannot read GitHub still gets the
    // conversation the decision depended on.
    let harness = Harness::new("stage-repo");
    harness.gh_endpoints(
        r#"{"number": 50, "title": "Cluster cannot read the plan", "body": "Body paragraph one.", "updated_at": "2026-09-08T07:03:00Z"}"#,
        r#"[{"user": {"login": "maintainer"}, "created_at": "2026-09-08T08:00:00Z", "body": "Clarification after the edit."}]"#,
    );
    let out = harness.temp.join("repo.md").display().to_string();
    let output = harness.stage(&[
        "--issue",
        "50",
        "--repo",
        "owner/repo",
        "--out",
        &out,
        "--staged-at",
        STAGED_AT,
        "--no-probe",
    ]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let text = read(&out);
    assert!(
        text.contains("Clarification after the edit."),
        "discussion must be staged from the live read: {text}"
    );
    assert!(text.contains("Body paragraph one."), "{text}");
    assert!(
        text.contains(&format!("# source-updated-at: {SOURCE_EPOCH}")),
        "{text}"
    );
    assert!(text.contains("# comments-included: 1"), "{text}");
}

#[test]
fn staging_from_the_repo_refuses_when_the_live_read_fails() {
    // Staging is the one side that CAN read GitHub. Failing there is a staging
    // problem (exit 2), never a half-staged spec that looks complete.
    let harness = Harness::new("stage-repo-gh-fails");
    harness.gh_failing();
    let out = harness.temp.join("repo.md").display().to_string();
    let output = harness.stage(&["--issue", "50", "--repo", "owner/repo", "--out", &out]);
    assert_eq!(output.status.code(), Some(2), "{}", stdout(&output));
    assert!(!harness.temp.join("repo.md").exists(), "nothing staged");
    let text = stderr(&output);
    assert!(text.contains("gh"), "{text}");
}

#[test]
fn staging_from_the_repo_skips_the_comment_fetch_when_a_file_supplies_them() {
    let harness = Harness::new("stage-repo-comments-file");
    // Anything the comments endpoint answers would duplicate the file.
    harness.gh_endpoints(
        r#"{"number": 50, "title": "T", "body": "B", "updated_at": "2026-09-08T07:03:00Z"}"#,
        r#"[{"user": "dup", "created_at": "2026-09-08T09:00:00Z", "body": "duplicate from the api"}]"#,
    );
    let comments = harness.write(
        "comments.json",
        r#"[{"user": "from-file", "created_at": "2026-09-08T08:00:00Z", "body": "only this one"}]"#,
    );
    let out = harness.temp.join("repo.md").display().to_string();
    let output = harness.stage(&[
        "--issue",
        "50",
        "--repo",
        "owner/repo",
        "--comments-json",
        &comments,
        "--out",
        &out,
        "--staged-at",
        STAGED_AT,
        "--no-probe",
    ]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let text = read(&out);
    assert!(text.contains("only this one"), "{text}");
    assert!(
        !text.contains("duplicate from the api"),
        "the explicit comments file wins: {text}"
    );
    assert!(text.contains("# comments-included: 1"), "{text}");
}

#[test]
fn staging_declared_inputs_never_touch_the_network_even_with_autospec_repo() {
    // A wrapper that hands over the body itself must not gain a network
    // dependency because $AUTOSPEC_REPO happens to be exported.
    let harness = Harness::new("stage-declared-offline");
    harness.gh_failing();
    let body = harness.write("body.md", "Body paragraph one.\n");
    let out = harness.temp.join("declared.md").display().to_string();
    let output = harness.run_with_env(
        "stage",
        &[
            "--issue",
            "50",
            "--body-file",
            &body,
            "--source-updated-at",
            SOURCE_UPDATED_AT,
            "--out",
            &out,
            "--staged-at",
            STAGED_AT,
            "--no-probe",
        ],
        &[("AUTOSPEC_REPO", "owner/repo")],
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(read(&out).contains("Body paragraph one."));
}

#[test]
fn staging_rejects_a_repository_that_is_not_owner_slash_name() {
    let harness = Harness::new("stage-repo-invalid");
    harness.gh_answering(SOURCE_UPDATED_AT);
    let out = harness.temp.join("repo.md").display().to_string();
    let output = harness.stage(&["--issue", "50", "--repo", "not-a-repo", "--out", &out]);
    assert_eq!(output.status.code(), Some(2), "{}", stdout(&output));
    assert!(
        stderr(&output).contains("owner/name"),
        "{}",
        stderr(&output)
    );
}

// ---------------------------------------------------------------- stage ----

#[test]
fn staging_pulls_the_comments_in_and_records_the_source_revision() {
    let harness = Harness::new("stage-comments");
    let out = stage_issue(&harness);
    let text = read(&out);

    assert!(text.contains("Clarification after the edit."), "{text}");
    assert!(
        text.contains(&format!("# source-updated-at: {SOURCE_EPOCH}")),
        "{text}"
    );
    assert!(
        text.contains(&format!("# staged-at: {STAGED_AT}")),
        "{text}"
    );
    assert!(text.contains("Body paragraph one."), "{text}");
    // The count says one of two, so the exclusion is visible rather than silent.
    assert!(text.contains("# comments-included: 1 of 2"), "{text}");
}

#[test]
fn staging_leaves_a_comment_that_predates_the_last_body_edit_out() {
    let harness = Harness::new("stage-excludes");
    let text = read(&stage_issue(&harness));

    assert!(
        !text.contains("Draft, predates the last edit."),
        "stale comment staged: {text}"
    );
}

#[test]
fn staging_declares_the_environment_it_probed_and_what_is_absent() {
    let harness = Harness::new("stage-env");
    let text = read(&stage_issue(&harness));

    assert!(text.contains("## Execution environment"), "{text}");
    assert!(text.contains("container-runtime: not probed"), "{text}");
    assert!(text.contains("registry: not probed"), "{text}");
}

#[test]
fn staging_records_an_explicit_environment_over_the_probe() {
    let harness = Harness::new("stage-env-explicit");
    let issue = harness.write("issue.json", &issue_payload());
    let out = harness.temp.join("explicit.md").display().to_string();
    let output = harness.stage(&[
        "--issue",
        "50",
        "--issue-json",
        &issue,
        "--out",
        &out,
        "--container-runtime",
        "/usr/bin/apptainer",
        "--database",
        "postgres:5432",
        "--registry",
        "absent",
    ]);

    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let text = read(&out);
    assert!(
        text.contains("container-runtime: /usr/bin/apptainer"),
        "{text}"
    );
    assert!(text.contains("database: postgres:5432"), "{text}");
    // A word, not a value, is how a probe reports a deliberate absence.
    assert!(text.contains("registry: absent"), "{text}");
}

#[test]
fn staging_writes_the_default_path_under_the_dispatch_home() {
    let harness = Harness::new("stage-default-path");
    let issue = harness.write("issue.json", &issue_payload());
    let output = harness.stage(&[
        "--issue",
        "50",
        "--issue-json",
        &issue,
        "--staged-at",
        STAGED_AT,
        "--no-probe",
    ]);

    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(
        harness
            .home()
            .join(".autospec/dispatch/specs/50.md")
            .is_file(),
        "default staged path not written"
    );
}

#[test]
fn staging_refuses_a_payload_that_carries_no_updated_at() {
    let harness = Harness::new("stage-no-revision");
    let issue = harness.write(
        "issue.json",
        r#"{"number": 50, "title": "No revision", "body": "text"}"#,
    );

    let output = harness.stage(&["--issue", "50", "--issue-json", &issue, "--no-probe"]);

    assert_eq!(output.status.code(), Some(2), "{}", stdout(&output));
    assert!(
        stderr(&output).contains("updated_at"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn staging_refuses_a_payload_that_belongs_to_another_issue() {
    let harness = Harness::new("stage-wrong-issue");
    let issue = harness.write(
        "issue.json",
        &format!(
            r#"{{"number": 51, "title": "Other", "body": "t", "updated_at": "{SOURCE_UPDATED_AT}"}}"#
        ),
    );

    let output = harness.stage(&["--issue", "50", "--issue-json", &issue, "--no-probe"]);

    assert_eq!(output.status.code(), Some(2), "{}", stdout(&output));
    assert!(
        stderr(&output).contains("wrong issue"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn staging_refuses_a_comment_that_cannot_be_dated() {
    let harness = Harness::new("stage-undated-comment");
    let issue = harness.write("issue.json", &issue_payload());
    let comments = harness.write(
        "comments.json",
        r#"[{"body": "no timestamp", "user": {"login": "a"}}]"#,
    );

    let output = harness.stage(&[
        "--issue",
        "50",
        "--issue-json",
        &issue,
        "--comments-json",
        &comments,
        "--no-probe",
    ]);

    assert_eq!(output.status.code(), Some(2), "{}", stdout(&output));
    assert!(
        stderr(&output).contains("created_at"),
        "{}",
        stderr(&output)
    );
}

// ------------------------------------------------------------ freshness ----

#[test]
fn freshness_dispatches_a_staged_spec_that_matches_the_live_issue() {
    let harness = Harness::new("fresh-current");
    let out = stage_issue(&harness);

    let output = harness.freshness(&[
        "--issue",
        "50",
        "--staged",
        &out,
        "--live-updated-at",
        SOURCE_UPDATED_AT,
    ]);

    assert_eq!(output.status.code(), Some(0), "{}", stdout(&output));
    assert!(stdout(&output).contains("current"), "{}", stdout(&output));
}

#[test]
fn freshness_accepts_the_live_revision_as_epoch_seconds() {
    let harness = Harness::new("fresh-epoch");
    let out = stage_issue(&harness);

    let output = harness.freshness(&[
        "--issue",
        "50",
        "--staged",
        &out,
        "--live-updated-at",
        SOURCE_EPOCH,
    ]);

    assert_eq!(output.status.code(), Some(0), "{}", stdout(&output));
}

#[test]
fn freshness_holds_a_stale_spec_and_names_the_re_stage_command() {
    let harness = Harness::new("fresh-stale");
    let out = stage_issue(&harness);

    let output = harness.freshness(&[
        "--issue",
        "50",
        "--staged",
        &out,
        "--live-updated-at",
        "2026-09-08T09:30:00Z",
    ]);

    assert_eq!(output.status.code(), Some(1), "{}", stdout(&output));
    let text = stdout(&output);
    assert!(text.contains("STALE"), "{text}");
    assert!(text.contains("re-stage"), "{text}");
    assert!(text.contains(&out), "{text}");
}

#[test]
fn freshness_refuses_when_the_live_issue_cannot_be_read() {
    let harness = Harness::new("fresh-unauthenticated");
    let out = stage_issue(&harness);
    harness.gh_failing();

    let output = harness.freshness(&["--issue", "50", "--staged", &out, "--repo", "acme/widgets"]);

    // The whole point: an unverifiable dispatch is refused, not waved through.
    assert_eq!(output.status.code(), Some(1), "{}", stdout(&output));
    let text = stdout(&output);
    assert!(text.contains("REFUSED"), "{text}");
    // A refusal names the issue and the last known staging time, because the
    // operator's next question is which copy is on disk.
    assert!(text.contains("issue 50"), "{text}");
    assert!(text.contains(STAGED_AT), "{text}");
    assert!(
        stderr(&output).contains("live revision unknown"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn freshness_refuses_when_there_is_no_way_to_reach_the_live_issue() {
    let harness = Harness::new("fresh-no-source");
    let out = stage_issue(&harness);

    // No --live-updated-at, no --live-json, no --repo, no AUTOSPEC_REPO, and no
    // gh on PATH: four ways to be unable to verify, one refusal.
    let output = harness.freshness(&["--issue", "50", "--staged", &out]);

    assert_eq!(output.status.code(), Some(1), "{}", stdout(&output));
    assert!(stdout(&output).contains("REFUSED"), "{}", stdout(&output));
}

#[test]
fn freshness_uses_the_live_revision_a_reachable_gh_reports() {
    let harness = Harness::new("fresh-gh-current");
    let out = stage_issue(&harness);
    harness.gh_answering(SOURCE_UPDATED_AT);

    let output = harness.freshness(&["--issue", "50", "--staged", &out, "--repo", "acme/widgets"]);

    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(stdout(&output).contains("current"), "{}", stdout(&output));
}

#[test]
fn freshness_holds_when_a_reachable_gh_reports_a_moved_issue() {
    let harness = Harness::new("fresh-gh-stale");
    let out = stage_issue(&harness);
    harness.gh_answering("2026-09-08T09:30:00Z");

    let output = harness.freshness(&["--issue", "50", "--staged", &out, "--repo", "acme/widgets"]);

    assert_eq!(output.status.code(), Some(1), "{}", stdout(&output));
    assert!(stdout(&output).contains("STALE"), "{}", stdout(&output));
}

#[test]
fn freshness_refuses_a_gh_that_answers_with_nothing_parseable() {
    let harness = Harness::new("fresh-gh-junk");
    let out = stage_issue(&harness);
    harness.gh_answering("");

    let output = harness.freshness(&["--issue", "50", "--staged", &out, "--repo", "acme/widgets"]);

    assert_eq!(output.status.code(), Some(1), "{}", stdout(&output));
    assert!(stdout(&output).contains("REFUSED"), "{}", stdout(&output));
}

#[test]
fn freshness_refuses_a_staged_spec_that_records_no_revision() {
    let harness = Harness::new("fresh-headerless");
    let out = harness.write("old.md", "# Issue #50: staged before headers existed\n");

    let output = harness.freshness(&[
        "--issue",
        "50",
        "--staged",
        &out,
        "--live-updated-at",
        SOURCE_UPDATED_AT,
    ]);

    assert_eq!(output.status.code(), Some(1), "{}", stdout(&output));
    assert!(stdout(&output).contains("REFUSED"), "{}", stdout(&output));
}

#[test]
fn freshness_refuses_a_staged_spec_that_does_not_exist() {
    let harness = Harness::new("fresh-missing");
    let missing = harness.temp.join("absent.md").display().to_string();

    let output = harness.freshness(&[
        "--issue",
        "50",
        "--staged",
        &missing,
        "--live-updated-at",
        SOURCE_UPDATED_AT,
    ]);

    assert_eq!(output.status.code(), Some(1), "{}", stdout(&output));
    let text = stdout(&output);
    assert!(text.contains("REFUSED"), "{text}");
    assert!(text.contains("no staged spec"), "{text}");
}

#[test]
fn freshness_reads_the_live_revision_from_a_recorded_payload() {
    let harness = Harness::new("fresh-live-json");
    let out = stage_issue(&harness);
    let live = harness.write(
        "live.json",
        &format!(r#"{{"number": 50, "updated_at": "2026-09-08T09:30:00Z"}}"#),
    );

    let output = harness.freshness(&["--issue", "50", "--staged", &out, "--live-json", &live]);

    assert_eq!(output.status.code(), Some(1), "{}", stdout(&output));
    assert!(stdout(&output).contains("STALE"), "{}", stdout(&output));
}

#[test]
fn freshness_reports_the_verdict_a_wrapper_branches_on_as_json() {
    let harness = Harness::new("fresh-json");
    let out = stage_issue(&harness);

    let output = harness.freshness(&[
        "--issue",
        "50",
        "--staged",
        &out,
        "--live-updated-at",
        "2026-09-08T09:30:00Z",
        "--json",
    ]);

    assert_eq!(output.status.code(), Some(1), "{}", stdout(&output));
    let text = stdout(&output);
    assert!(text.contains("\"held\": true"), "{text}");
    assert!(text.contains("\"needs_restage\": true"), "{text}");
    assert!(text.contains("\"restage\""), "{text}");
}

#[test]
fn a_malformed_live_revision_is_a_diagnostic_not_a_refusal() {
    let harness = Harness::new("fresh-bad-arg");
    let out = stage_issue(&harness);

    let output = harness.freshness(&[
        "--issue",
        "50",
        "--staged",
        &out,
        "--live-updated-at",
        "not-a-time",
    ]);

    // The caller supplied garbage: exit 2 says "fix your invocation", while exit
    // 1 would say "the dispatch is held" and send a wrapper off to re-stage.
    assert_eq!(output.status.code(), Some(2), "{}", stdout(&output));
    assert!(
        stderr(&output).contains("neither epoch seconds nor RFC 3339"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn freshness_requires_an_issue_number() {
    let harness = Harness::new("fresh-no-issue");
    let out = stage_issue(&harness);

    let output = harness.freshness(&["--staged", &out, "--live-updated-at", SOURCE_EPOCH]);

    assert_eq!(output.status.code(), Some(2), "{}", stdout(&output));
    assert!(stderr(&output).contains("--issue"), "{}", stderr(&output));
}

#[test]
fn a_re_staged_spec_is_current_again() {
    let harness = Harness::new("fresh-restage-loop");
    let out = stage_issue(&harness);
    let held = harness.freshness(&[
        "--issue",
        "50",
        "--staged",
        &out,
        "--live-updated-at",
        "2026-09-08T09:30:00Z",
    ]);
    assert_eq!(held.status.code(), Some(1), "{}", stdout(&held));

    // The recovery the message asks for, run for real: re-stage, re-check.
    let mut payload = issue_payload();
    payload = payload.replace(SOURCE_UPDATED_AT, "2026-09-08T09:30:00Z");
    let issue = harness.write("issue2.json", &payload);
    let comments = harness.write("comments.json", &comments_payload());
    let restaged = harness.stage(&[
        "--issue",
        "50",
        "--issue-json",
        &issue,
        "--comments-json",
        &comments,
        "--out",
        &out,
        "--staged-at",
        "1788859000",
        "--no-probe",
    ]);
    assert_eq!(restaged.status.code(), Some(0), "{}", stderr(&restaged));

    let current = harness.freshness(&[
        "--issue",
        "50",
        "--staged",
        &out,
        "--live-updated-at",
        "2026-09-08T09:30:00Z",
    ]);
    assert_eq!(current.status.code(), Some(0), "{}", stdout(&current));
    let text = read(&out);
    assert!(
        text.contains("Clarification after the edit."),
        "re-staging dropped the discussion: {text}"
    );
}

#[test]
fn dispatch_help_lists_the_staged_spec_subcommands() {
    let output = Command::new(env!("CARGO_BIN_EXE_autospec"))
        .args(["dispatch", "--help", "unused"])
        .output()
        .expect("autospec runs");

    assert_eq!(output.status.code(), Some(0));
    let text = stdout(&output);
    for token in ["stage", "freshness", "updatedAt"] {
        assert!(text.contains(token), "{text}");
    }
}

/// The header parser only reads the leading block, so a comment quoting the
/// header format cannot re-age a staged spec from inside its own body.
#[test]
fn a_revision_header_quoted_in_the_body_does_not_age_the_spec() {
    let harness = Harness::new("fresh-forged-header");
    let out = harness.write(
        "forged.md",
        &format!(
            "# Issue #50\n\nSome text quoting `{HEADER} 1799999999`.\n",
            HEADER = "# source-updated-at:"
        ),
    );

    let output = harness.freshness(&[
        "--issue",
        "50",
        "--staged",
        &out,
        "--live-updated-at",
        SOURCE_UPDATED_AT,
    ]);

    assert_eq!(output.status.code(), Some(1), "{}", stdout(&output));
    assert!(stdout(&output).contains("REFUSED"), "{}", stdout(&output));
}

/// `--out` pointing at an impossible path is the staging host's problem.
#[test]
fn staging_reports_an_unwritable_output_path() {
    let harness = Harness::new("stage-unwritable");
    let issue = harness.write("issue.json", &issue_payload());
    let out = Path::new("/proc/nonexistent-directory/50.md")
        .display()
        .to_string();

    let output = harness.stage(&[
        "--issue",
        "50",
        "--issue-json",
        &issue,
        "--out",
        &out,
        "--no-probe",
    ]);

    assert_eq!(output.status.code(), Some(2), "{}", stdout(&output));
    assert!(!stderr(&output).is_empty(), "no diagnostic emitted");
}
