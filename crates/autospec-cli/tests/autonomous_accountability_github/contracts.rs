use super::*;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::collections::BTreeMap;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::os::unix::process::CommandExt;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn github_project_transport_commands_have_explicit_owner_contracts() {
    let cases = [
        (
            GithubCommand::ListProjects {
                owner: "berlinguyinca".into(),
            },
            vec![
                "api",
                "graphql",
                "--paginate",
                "--slurp",
                "-f",
                "query=query($owner:String!,$endCursor:String){repositoryOwner(login:$owner){... on Organization{projectsV2(first:100,after:$endCursor){nodes{number title}pageInfo{hasNextPage endCursor}}}... on User{projectsV2(first:100,after:$endCursor){nodes{number title}pageInfo{hasNextPage endCursor}}}}}",
                "-F",
                "owner=berlinguyinca",
            ],
            None,
        ),
        (
            GithubCommand::ListOwnerRepositories {
                owner: "berlinguyinca".into(),
                limit: 25,
            },
            vec![
                "repo",
                "list",
                "berlinguyinca",
                "--limit",
                "25",
                "--json",
                "nameWithOwner",
            ],
            None,
        ),
        (
            GithubCommand::CreateProject {
                owner: "berlinguyinca".into(),
                title: "Autospec".into(),
            },
            vec![
                "project",
                "create",
                "--owner",
                "berlinguyinca",
                "--title",
                "Autospec",
                "--format",
                "json",
            ],
            None,
        ),
        (
            GithubCommand::ListProjectItems {
                owner: "berlinguyinca".into(),
                number: 7,
            },
            vec![
                "project",
                "item-list",
                "7",
                "--owner",
                "berlinguyinca",
                "--format",
                "json",
                "--limit",
                "500",
            ],
            None,
        ),
        (
            GithubCommand::EditProjectMarker {
                owner: "berlinguyinca".into(),
                number: 7,
                readme: "managed marker".into(),
            },
            vec![
                "project",
                "edit",
                "7",
                "--owner",
                "berlinguyinca",
                "--readme",
                "managed marker",
            ],
            None,
        ),
        (
            GithubCommand::AddToProject {
                owner: "berlinguyinca".into(),
                project_number: 7,
                issue_url: "https://github.com/berlinguyinca/autospec/issues/42".into(),
            },
            vec![
                "project",
                "item-add",
                "7",
                "--owner",
                "berlinguyinca",
                "--url",
                "https://github.com/berlinguyinca/autospec/issues/42",
            ],
            None,
        ),
    ];

    for (command, expected_args, expected_stdin) in cases {
        let (args, stdin) = command.into_parts();
        assert_eq!(args, expected_args);
        assert_eq!(stdin.as_deref(), expected_stdin);
    }
}

#[test]
fn recovery_events_replay_into_the_active_existing_epic_projection() {
    let fixture = Fixture::new("recovery-projection");
    let mut store = store(&fixture);
    store
        .bind_epic(97, "https://github.com/acme/widgets/issues/97")
        .unwrap();
    store.mark_spawned().unwrap();

    for (kind, what, why, evidence) in [
        (
            EventKind::HeartbeatPublicationDeferred {
                issue: 42,
                claim_id: "claim-generation-1".to_owned(),
            },
            "Heartbeat publication deferred for issue 42",
            "The authoritative claim remains pending until startup ownership can be proven",
            "claim claim-generation-1 remains pending",
        ),
        (
            EventKind::StartupClaimRecovered {
                issue: 42,
                previous_claim_id: "claim-generation-1".to_owned(),
                next_claim_id: "claim-generation-2".to_owned(),
            },
            "Startup claim recovered for issue 42",
            "The authoritative recovery CAS replaced the stale generation before reacquisition",
            "claim-generation-1 advanced to claim-generation-2",
        ),
        (
            EventKind::HeartbeatPublicationDeferred {
                issue: 42,
                claim_id: "claim-generation-2".to_owned(),
            },
            "Heartbeat publication deferred again for issue 42",
            "The successor claim also awaits authoritative recovery evidence",
            "claim claim-generation-2 remains pending",
        ),
        (
            EventKind::StartupClaimRecovered {
                issue: 42,
                previous_claim_id: "claim-generation-2".to_owned(),
                next_claim_id: "claim-generation-3".to_owned(),
            },
            "Second startup claim recovered for issue 42",
            "The second authoritative recovery CAS advanced its exact generation",
            "claim-generation-2 advanced to claim-generation-3",
        ),
        (
            EventKind::HeartbeatPublicationDeferred {
                issue: 42,
                claim_id: "claim-generation-unmatched".to_owned(),
            },
            "Later heartbeat publication deferred for issue 42",
            "No authoritative recovery has matched this later generation",
            "claim claim-generation-unmatched remains pending",
        ),
    ] {
        store
            .append_event(
                AccountabilityEvent::new(kind, what, why, vec![Evidence::outcome(evidence)])
                    .unwrap(),
            )
            .unwrap();
    }

    let projection = store.render().unwrap();
    assert!(projection
        .markdown
        .contains("Heartbeat publication deferred"));
    assert!(projection.markdown.contains("Startup claim recovered"));
    assert!(projection.markdown.contains("**What:**"));
    assert!(projection.markdown.contains("**Why:**"));
    assert!(projection.markdown.contains("**Evidence:**"));
    assert!(projection
        .markdown
        .contains("deferred_42_1 --> recovered_42_2"));
    assert!(projection
        .markdown
        .contains("deferred_42_3 --> recovered_42_4"));
    assert!(!projection.markdown.contains("deferred_42_5 -->"));
    assert!(!projection
        .markdown
        .contains("recovered_42_2 --> deferred_42_1"));
    assert_eq!(
        store.recovery_projection().0,
        accountability::RecoveryState::Active
    );

    drop(store);
    let reopened = AccountabilityStore::open(fixture.path()).unwrap();
    assert_eq!(reopened.status().epic_number, Some(97));
    assert_eq!(reopened.status().event_count, 5);
    assert_eq!(
        reopened.recovery_projection().0,
        accountability::RecoveryState::Active
    );
}

#[test]
fn autonomous_cli_exposes_explicit_epic_start_and_resume_contract() {
    let help = Command::new(env!("CARGO_BIN_EXE_autospec"))
        .args(["autonomous", "--help"])
        .output()
        .unwrap();
    assert!(help.status.success());
    let help = String::from_utf8_lossy(&help.stdout);
    assert!(help.contains("resume"));
    assert!(help.contains("--epic N"));

    let missing = Command::new(env!("CARGO_BIN_EXE_autospec"))
        .args(["autonomous", "resume", "--repo", "acme/widgets"])
        .output()
        .unwrap();
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("resume requires --epic N"));
}

#[test]
fn resume_rejects_force_before_touching_a_stopped_run() {
    let fixture = CliResumeFixture::new("force-rejected");
    fixture.record_immediate_stop();
    let stop_before = fs::read_to_string(fixture.stop_flag()).expect("read stop flag");

    let output = fixture
        .command("resume")
        .args(["--epic", "12", "--force"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--force is not valid with resume"));
    assert_eq!(
        fs::read_to_string(fixture.stop_flag()).unwrap(),
        stop_before
    );
}

#[test]
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn autonomous_resume_dry_run_is_strictly_read_only() {
    let fixture = CliResumeFixture::new("resume-dry-run");
    fixture.record_immediate_stop();
    fixture.install_closed_epic();
    let mut conductor = fixture.install_running_conductor();
    assert_authoritative_conductor_metadata(&fixture.scope().join("conductor.pid"), conductor.id());
    let stop_before = fs::read_to_string(fixture.stop_flag()).expect("read stop flag");
    let files_before = snapshot_tree(&fixture.root);

    let output = fixture
        .command("resume")
        .args(["--epic", "12", "--dry-run", "--json"])
        .output()
        .unwrap();
    let conductor_survived = conductor.try_wait().unwrap().is_none();
    let files_after = snapshot_tree(&fixture.root);
    let stop_after = fs::read_to_string(fixture.stop_flag()).ok();
    let issue_after = fs::read_to_string(fixture.issue_state()).unwrap();
    let gh_was_called = fixture.gh_calls().exists();
    let launch_was_written = fixture.scope().join("launch.json").exists();

    let resumed = fixture
        .command("resume")
        .args(["--epic", "12", "--json"])
        .output()
        .expect("resume after preview");
    let mut conductor_terminated = false;
    for _ in 0..100 {
        if conductor.try_wait().unwrap().is_some() {
            conductor_terminated = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    fixture.stop_spawned_run();
    let _ = conductor.kill();
    let _ = conductor.wait();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "stdout={stdout} stderr={stderr}");
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["subcommand"], "resume");
    assert_eq!(result["status"], "dry-run");
    assert!(
        conductor_survived,
        "dry-run must not stop the active conductor"
    );
    assert!(!gh_was_called, "dry-run must not invoke gh");
    assert_eq!(issue_after, "CLOSED\n", "dry-run must not reopen the epic");
    assert_eq!(stop_after.as_deref(), Some(stop_before.as_str()));
    assert!(!launch_was_written, "dry-run must not write launch.json");
    assert_eq!(
        files_after, files_before,
        "dry-run must not write run state"
    );
    assert!(
        resumed.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&resumed.stdout),
        String::from_utf8_lossy(&resumed.stderr)
    );
    assert!(
        conductor_terminated,
        "non-preview resume must recognize and terminate the owned conductor group"
    );
}

#[test]
#[cfg(unix)]
fn autonomous_resume_reopens_the_same_epic_and_clears_an_immediate_stop() {
    let fixture = CliResumeFixture::new("stopped-resume");
    fixture.record_immediate_stop();
    fixture.install_closed_epic();

    let output = fixture
        .command("resume")
        .args(["--epic", "12", "--json"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(output.status.success(), "stdout={stdout} stderr={stderr}");
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["subcommand"], "resume");
    assert_eq!(result["epic_number"], 12);
    assert_eq!(fs::read_to_string(fixture.issue_state()).unwrap(), "OPEN\n");
    assert!(
        !fixture.stop_flag().exists(),
        "resume must clear the stop sentinel"
    );
    assert!(
        fixture.scope().join("launch.json").exists(),
        "resume must reach spawn"
    );
    let calls = fs::read_to_string(fixture.gh_calls()).unwrap();
    assert!(calls.contains("issue reopen 12"), "calls={calls}");
    fixture.stop_spawned_run();
}

struct CliResumeFixture {
    root: std::path::PathBuf,
}

impl CliResumeFixture {
    fn new(name: &str) -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "autospec-cli-resume-{name}-{}-{stamp}",
            std::process::id()
        ));
        fs::create_dir_all(root.join("repo")).unwrap();
        assert!(Command::new("git")
            .args(["init", "-q"])
            .current_dir(root.join("repo"))
            .status()
            .unwrap()
            .success());
        Self { root }
    }

    fn command(&self, subcommand: &str) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_autospec"));
        command
            .args(["autonomous", subcommand])
            .args(["--repo", "acme/widgets", "--repo-dir"])
            .arg(self.root.join("repo"))
            .args(["--max-cycles", "1", "--poll-interval-sec", "1"])
            .env(
                "AUTOSPEC_AUTONOMOUS_OPERATOR_DIR",
                self.root.join("operator"),
            )
            .env("AUTOSPEC_STATE_DIR", self.root.join("state"))
            .env("AUTOSPEC_AUTONOMOUS_SPEND_DIR", self.root.join("spend"))
            .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", self.root.join("logs"))
            .env("AUTOSPEC_AUTONOMOUS_COMPANIONS", "0")
            .env("AUTOSPEC_TEST_ISSUE_BODY", self.root.join("issue-body"))
            .env("AUTOSPEC_TEST_ISSUE_STATE", self.issue_state())
            .env("AUTOSPEC_TEST_GH_CALLS", self.gh_calls());
        let bin = self.root.join("bin");
        if bin.exists() {
            command.env(
                "PATH",
                format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
            );
        }
        command
    }

    fn record_immediate_stop(&self) {
        let output = self.command("stop").arg("--immediate").output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(self.stop_flag().exists());
    }

    #[cfg(unix)]
    fn install_closed_epic(&self) {
        let projection = "Existing accountable autonomous run";
        let manifest = accountability::RecoveryManifest::new(
            run(),
            12,
            "https://github.com/acme/widgets/issues/12",
            4,
            autospec_core::autonomous::waterfall::sha256_hex(format!("{projection}\n").as_bytes()),
            12,
            2,
        )
        .unwrap()
        .with_recovery_state(accountability::RecoveryState::Terminal, vec![], vec![])
        .unwrap();
        let marker = format!(
            "<!-- autospec:run-epic repo=acme/widgets run_id={} -->",
            run().run_id()
        );
        fs::write(
            self.root.join("issue-body"),
            accountability::github::compose_managed_body(
                &marker,
                projection,
                &manifest,
                "human notes",
            ),
        )
        .unwrap();
        fs::write(self.issue_state(), "CLOSED\n").unwrap();
        let bin = self.root.join("bin");
        fs::create_dir_all(&bin).unwrap();
        let gh = bin.join("gh");
        fs::write(&gh, r#"#!/bin/sh
set -eu
printf '%s\n' "$*" >> "$AUTOSPEC_TEST_GH_CALLS"
case "$1 $2 $3" in
  "issue view 12")
    body=$(awk 'BEGIN{ORS=""; printf "\""} {gsub(/\\/,"\\\\"); gsub(/\"/,"\\\""); if (NR>1) printf "\\n"; printf "%s",$0} END{printf "\""}' "$AUTOSPEC_TEST_ISSUE_BODY")
    printf '{"number":12,"url":"https://github.com/acme/widgets/issues/12","state":"%s","body":%s,"labels":[{"name":"epic"},{"name":"type:tracker"},{"name":"no-auto"},{"name":"autospec:run-accountability"}]}\n' "$(tr -d '\n' < "$AUTOSPEC_TEST_ISSUE_STATE")" "$body"
    ;;
  "issue reopen 12") printf 'OPEN\n' > "$AUTOSPEC_TEST_ISSUE_STATE" ;;
  "issue edit 12") cat > "$AUTOSPEC_TEST_ISSUE_BODY" ;;
  *) printf 'unexpected gh invocation: %s\n' "$*" >&2; exit 1 ;;
esac
"#).unwrap();
        fs::set_permissions(gh, fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn install_running_conductor(&self) -> std::process::Child {
        let bin = self.root.join("bin");
        fs::create_dir_all(&bin).unwrap();
        let autospec = bin.join("autospec");
        fs::write(&autospec, "#!/bin/sh\nexec sleep 300\n").unwrap();
        fs::set_permissions(&autospec, fs::Permissions::from_mode(0o755)).unwrap();
        let conductor = Command::new(&autospec)
            .arg("run-foreground")
            .process_group(0)
            .spawn()
            .unwrap();
        let identity = native_process_identity(conductor.id()).expect("capture conductor identity");
        fs::create_dir_all(self.scope()).unwrap();
        fs::write(
            self.scope().join("conductor.pid"),
            format!(
                "{{\"pid\":{},\"repo\":\"acme/widgets\",\"scope\":\"acme_widgets\",\"pgid\":{},\"start_time_ticks\":{}}}\n",
                conductor.id(),
                identity.pgid,
                identity.start_time_ticks,
            ),
        )
        .unwrap();
        conductor
    }

    fn stop_spawned_run(&self) {
        let _ = self.command("stop").arg("--immediate").output();
    }

    fn scope(&self) -> std::path::PathBuf {
        self.root.join("operator/acme_widgets")
    }
    fn stop_flag(&self) -> std::path::PathBuf {
        self.scope().join("stop.flag")
    }
    fn issue_state(&self) -> std::path::PathBuf {
        self.root.join("issue-state")
    }
    fn gh_calls(&self) -> std::path::PathBuf {
        self.root.join("gh-calls")
    }
}

impl Drop for CliResumeFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn snapshot_tree(root: &Path) -> BTreeMap<std::path::PathBuf, Vec<u8>> {
    fn visit(root: &Path, path: &Path, snapshot: &mut BTreeMap<std::path::PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                visit(root, &path, snapshot);
            } else {
                snapshot.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }

    let mut snapshot = BTreeMap::new();
    visit(root, root, &mut snapshot);
    snapshot
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

#[test]
fn launcher_binds_verified_epic_before_any_conductor_spawn_and_supervisor_reuses_it() {
    let source = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands/autonomous.rs"),
    )
    .unwrap();
    let start = source.find("fn start(options:").unwrap();
    let after_start = &source[start..];
    let binding = after_start.find("bind_accountability_epic").unwrap();
    let launch = after_start.find("start_after_lease").unwrap();
    assert!(binding < launch, "epic binding must precede launch");

    let control = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/commands/autonomous/accountability_runtime/control.rs"),
    )
    .unwrap();
    let repair = control.find("fn repair_stopped_conductor").unwrap();
    let repair_body = &control[repair..];
    let verification = repair_body.find("verify_existing_accountability").unwrap();
    let spawn = repair_body.find("spawn_unit(").unwrap();
    assert!(
        verification < spawn,
        "supervisor must verify the inherited epic before respawn"
    );
    assert!(source.contains("\\\"accountability\\\":{}"));
    let foreground = source.find("fn run_foreground(options:").unwrap();
    let foreground_body = &source[foreground..];
    let foreground_binding = foreground_body
        .find("verify_existing_accountability")
        .unwrap();
    let foreground_cycles = foreground_body.find("run_foreground_cycles").unwrap();
    assert!(
        foreground_binding < foreground_cycles,
        "inherited foreground workers must verify accountability before work"
    );
}

#[test]
fn autonomous_accountability_github_managed_project_marker_source_exposes_portfolio_recovery_contracts(
) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let github = fs::read_to_string(root.join("src/commands/managed_project/github.rs")).unwrap();
    let parser =
        fs::read_to_string(root.join("src/commands/managed_project/github/parse.rs")).unwrap();

    for required in [
        "MarkerDisposition::Exact",
        "create_unknown",
        "exact_nonce_title",
        "record_created_project",
    ] {
        assert!(
            github.contains(required),
            "managed Project orchestration must contain {required}"
        );
    }
    for required in [
        "kind: spec_portfolio",
        "recovery-capsule:",
        "parse_project_candidates",
        "totalCount",
    ] {
        assert!(
            parser.contains(required),
            "managed Project marker parser must contain {required}"
        );
    }
}
