// executor_bridge tests: reviewer / runtime — 9 cases.
//
// Split out of tests.rs; see the note in that file.

use super::super as bridge;
use super::super::{
    recover_invocation, runtime_session_adapter, BridgePhase, HarnessConfig, HarnessKind,
};
use super::support_base::{
    environment, git, git_stdout, installed_aliases, test_environment, test_root,
    write_alias_table, write_executable, DetachedForkedCleanup, GitFixture,
};
use super::support_invocation::{
    implementation_proof_fixture, persisted_invocation, reviewer_request,
};
use std::ffi::OsString;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

fn commit_normal_risk_reviewer_fixture(state: &bridge::PersistedInvocation) {
    fs::write(
        state.identity.worktree.join("implementation.txt"),
        "implemented\n",
    )
    .expect("write implementation");
    git(&state.identity.worktree, &["add", "implementation.txt"]);
    git(
        &state.identity.worktree,
        &["commit", "-m", "feat: implement reviewer fixture"],
    );
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_dangling_runtime_and_state_links_fail_closed() {
    use std::os::unix::fs::symlink;

    let fixture = GitFixture::new("dangling-links");
    fs::create_dir_all(fixture.repo.join(".autospec")).expect("autospec directory");
    let manifest = fixture.repo.join(".autospec/runtime.yml");
    symlink(fixture.root.join("missing-runtime.yml"), &manifest).expect("runtime symlink");
    let runtime_error = runtime_session_adapter(&fixture.repo)
        .expect_err("dangling runtime manifest must not disable isolation");
    assert!(runtime_error.contains("symlink"), "{runtime_error}");
    assert!(fs::symlink_metadata(&manifest)
        .expect("runtime link remains")
        .file_type()
        .is_symlink());

    let state = fixture.root.join("state/invocation.json");
    fs::create_dir_all(state.parent().expect("state parent")).expect("state directory");
    symlink(fixture.root.join("missing-invocation.json"), &state).expect("state symlink");
    let expected = persisted_invocation().identity;
    let state_error = recover_invocation(&state, &expected)
        .expect_err("dangling invocation symlink must fail closed");
    assert!(state_error.contains("symlink"), "{state_error}");
    assert!(fs::symlink_metadata(&state)
        .expect("state link remains")
        .file_type()
        .is_symlink());
}

#[test]
fn autonomous_executor_bridge_runtime_marker_precedes_alias_path_order() {
    // Break caught: probing the first installed binary before the active runtime marker.
    let root = test_root("marker");
    let table = write_alias_table(&root, installed_aliases());
    let mut env = environment(&table);
    env.insert("CODEX_THREAD_ID".to_string(), OsString::from("thread-1"));

    let config = HarnessConfig::load(&root, &env).expect("load harness config");
    let resolved = config.resolve(&env).expect("resolve active harness");

    assert_eq!(resolved.kind, HarnessKind::Codex);
    assert_eq!(
        resolved.executable,
        fs::canonicalize("/bin/sh").expect("canonical /bin/sh")
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn autonomous_executor_bridge_explicit_override_precedes_runtime_marker() {
    // Break caught: a Codex marker overriding an operator's explicit Claude selection.
    let root = test_root("override");
    let table = write_alias_table(&root, installed_aliases());
    let mut env = environment(&table);
    env.insert("CODEX_THREAD_ID".to_string(), OsString::from("thread-1"));
    env.insert(
        "AUTOSPEC_HANDOFF_DISPATCHER_KIND".to_string(),
        OsString::from("claude"),
    );

    let resolved = HarnessConfig::load(&root, &env)
        .expect("load harness config")
        .resolve(&env)
        .expect("resolve explicit harness");

    assert_eq!(resolved.kind, HarnessKind::Claude);
    assert_eq!(
        resolved.executable,
        fs::canonicalize("/bin/true").expect("canonical /bin/true")
    );
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_resolves_private_external_reviewer_when_override_is_unset() {
    // Break caught: CI completion still requiring a hand-written reviewer command.
    let (fixture, mut state, _snapshot, _) = implementation_proof_fixture("automatic-reviewer");
    commit_normal_risk_reviewer_fixture(&state);
    state.harness = HarnessKind::Claude;
    state.phase = BridgePhase::CiPassed;
    state.head_oid = Some(git_stdout(&state.identity.worktree, &["rev-parse", "HEAD"]));
    let state_path = fixture.root.join("state/invocation.json");
    let request = reviewer_request(&state, state_path.clone());
    let table = write_alias_table(
        &fixture.root,
        &format!(
            "codex\t{}\tautospec-codex\tCodex fixture\n",
            fs::canonicalize("/bin/true")
                .expect("canonical reviewer")
                .display()
        ),
    );
    let mut env = environment(&table);
    env.insert(
        "AUTOSPEC_HANDOFF_DISPATCHER_KIND".to_string(),
        OsString::from("codex"),
    );
    let artifact_root = state_path.with_extension("review-artifacts");

    let reviewer = bridge::resolve_independent_reviewer(&request, &state, &env, &artifact_root)
        .expect("resolve configured reviewer");

    let command = reviewer.plan.commands.first().expect("one reviewer");
    let automatic = reviewer.automatic.expect("automatic reviewer artifacts");
    assert_eq!(reviewer.plan.commands.len(), 1);
    assert_eq!(
        PathBuf::from(command.argv.first().expect("reviewer executable")),
        automatic.normalizer
    );
    let result = automatic.result;
    assert!(result.starts_with(&artifact_root));
    assert!(!result.starts_with(&state.identity.repository_path));
    assert!(!result.starts_with(&state.identity.worktree));
    assert_eq!(
        fs::metadata(&result)
            .expect("review result metadata")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(&automatic.normalizer)
            .expect("normalizer metadata")
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    let normalizer = fs::read_to_string(&automatic.normalizer).expect("normalizer");
    assert!(normalizer.contains("Return exactly one JSON object"));
    assert!(normalizer.contains("harness.stdout"));
    assert!(normalizer.contains("harness.stderr"));
    assert!(
        normalizer.contains("'--sandbox' 'read-only'"),
        "{normalizer}"
    );
    assert!(!normalizer.contains("workspace-write"), "{normalizer}");
}

#[cfg(unix)]
#[test]
fn automatic_reviewer_identity_change_preserves_superseded_normalizer() {
    let (fixture, mut state, _snapshot, _) =
        implementation_proof_fixture("automatic-reviewer-upgrade");
    commit_normal_risk_reviewer_fixture(&state);
    state.harness = HarnessKind::Claude;
    state.phase = BridgePhase::CiPassed;
    state.head_oid = Some(git_stdout(&state.identity.worktree, &["rev-parse", "HEAD"]));
    let state_path = fixture.root.join("state/invocation.json");
    let request = reviewer_request(&state, state_path.clone());
    let table = write_alias_table(
        &fixture.root,
        &format!(
            "codex\t{}\tautospec-codex\tCodex fixture\n",
            fs::canonicalize("/bin/true")
                .expect("canonical reviewer")
                .display()
        ),
    );
    let mut env = environment(&table);
    env.insert(
        "AUTOSPEC_HANDOFF_DISPATCHER_KIND".to_string(),
        OsString::from("codex"),
    );
    let artifact_root = state_path.with_extension("review-artifacts");
    fs::create_dir_all(&artifact_root).expect("review artifact root");
    fs::set_permissions(&artifact_root, fs::Permissions::from_mode(0o700))
        .expect("private review artifact root");
    let legacy = artifact_root.join("review-normalizer.sh");
    write_executable(&legacy, "#!/bin/sh\nulimit -f 2048\nexit 25\n");
    fs::set_permissions(&legacy, fs::Permissions::from_mode(0o700))
        .expect("private legacy normalizer");

    let reviewer = bridge::resolve_independent_reviewer(&request, &state, &env, &artifact_root)
        .expect("resolve upgraded reviewer without deleting legacy evidence");
    let automatic = reviewer.automatic.expect("automatic reviewer artifacts");

    assert_ne!(automatic.normalizer, legacy);
    assert_eq!(
        fs::read_to_string(&legacy).expect("legacy normalizer"),
        "#!/bin/sh\nulimit -f 2048\nexit 25\n"
    );
    assert!(!fs::read_to_string(&automatic.normalizer)
        .expect("upgraded normalizer")
        .contains("ulimit"));
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_codex_reviewer_ignores_host_policy_but_keeps_auth() {
    // Break caught: retained CODEX_HOME loading host MCP, hook, or execpolicy mutation authority.
    let (fixture, mut state, _snapshot, _) =
        implementation_proof_fixture("automatic-codex-host-policy");
    commit_normal_risk_reviewer_fixture(&state);
    state.harness = HarnessKind::Claude;
    state.phase = BridgePhase::CiPassed;
    state.head_oid = Some(git_stdout(&state.identity.worktree, &["rev-parse", "HEAD"]));
    let state_path = fixture.root.join("state/invocation.json");
    let request = reviewer_request(&state, state_path.clone());
    let safe_root = PathBuf::from(std::env::var_os("HOME").expect("HOME for reviewer fixture"))
        .join(format!(".autospec-codex-review-{}", std::process::id()));
    let codex_home = safe_root.join("codex-home");
    let rules = codex_home.join("rules");
    fs::create_dir_all(&rules).expect("hostile Codex rules directory");
    fs::write(
        codex_home.join("auth.json"),
        r#"{"fixture_auth":"reviewer-auth-remains-readable"}"#,
    )
    .expect("Codex auth fixture");
    fs::write(
        codex_home.join("config.toml"),
        "[mcp_servers.hostile]\ncommand = \"/bin/false\"\n",
    )
    .expect("hostile Codex config");
    fs::write(
        rules.join("hostile.rules"),
        "prefix_rule(pattern=[\"/bin/sh\"], decision=\"allow\")\n",
    )
    .expect("hostile Codex rules");
    let harness = safe_root.join("codex-reviewer");
    let review =
        super::support_review::valid_review_json(state.head_oid.as_deref().expect("review commit"));
    write_executable(
        &harness,
        &format!(
            "#!/bin/sh\n\
         set -eu\n\
         ignore_config=0\n\
         ignore_rules=0\n\
         result=\n\
         while [ \"$#\" -gt 0 ]; do\n\
         \tcase \"$1\" in\n\
         \t\t--ignore-user-config) ignore_config=1 ;;\n\
         \t\t--ignore-rules) ignore_rules=1 ;;\n\
         \t\t--output-last-message) shift; result=$1 ;;\n\
         \tesac\n\
         \tshift\n\
         done\n\
         /usr/bin/grep -q reviewer-auth-remains-readable \"$CODEX_HOME/auth.json\"\n\
         /usr/bin/test -s \"$CODEX_HOME/config.toml\"\n\
         /usr/bin/test -s \"$CODEX_HOME/rules/hostile.rules\"\n\
         if [ \"$ignore_config\" -ne 1 ] || [ \"$ignore_rules\" -ne 1 ]; then\n\
         \tprintf '%s\\n' loaded > \"$CODEX_HOME/host-policy-loaded\"\n\
         \texit 72\n\
         fi\n\
         /usr/bin/test -n \"$result\"\n\
         printf '%s\\n' '{}' > \"$result\"\n",
            review
        ),
    );
    let table = write_alias_table(
        &fixture.root,
        &format!(
            "codex\t{}\tautospec-codex\tCodex fixture\n",
            harness.display()
        ),
    );
    let mut env = environment(&table);
    env.insert(
        "AUTOSPEC_HANDOFF_DISPATCHER_KIND".to_string(),
        OsString::from("codex"),
    );
    env.insert(
        "CODEX_HOME".to_string(),
        codex_home.clone().into_os_string(),
    );
    let artifact_root = state_path.with_extension("review-artifacts");

    let reviewer = bridge::resolve_independent_reviewer(&request, &state, &env, &artifact_root)
        .expect("resolve isolated Codex reviewer");
    let automatic = reviewer.automatic.expect("automatic reviewer artifacts");
    let output = Command::new("/bin/sh")
        .arg(&automatic.normalizer)
        .current_dir(&state.identity.worktree)
        .output()
        .expect("execute isolated Codex reviewer");

    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"LGTM\n");
    assert!(
        !codex_home.join("host-policy-loaded").exists(),
        "host Codex policy was loaded"
    );
    let _ = fs::remove_dir_all(safe_root);
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_normalizes_claude_and_opencode_stdout_verdicts() {
    // Break caught: the automatic boundary only supporting Codex's result-file protocol.
    for kind in ["claude", "opencode"] {
        let (fixture, mut state, _snapshot, _) =
            implementation_proof_fixture(&format!("automatic-{kind}-reviewer"));
        commit_normal_risk_reviewer_fixture(&state);
        state.phase = BridgePhase::CiPassed;
        state.head_oid = Some(git_stdout(&state.identity.worktree, &["rev-parse", "HEAD"]));
        let state_path = fixture.root.join("state/invocation.json");
        let request = reviewer_request(&state, state_path.clone());
        let table = write_alias_table(
            &fixture.root,
            &format!(
                "{kind}\t{}\tautospec-{kind}\t{kind} fixture\n",
                fs::canonicalize("/bin/true")
                    .expect("canonical reviewer")
                    .display()
            ),
        );
        let mut env = environment(&table);
        env.insert(
            "AUTOSPEC_HANDOFF_DISPATCHER_KIND".to_string(),
            OsString::from(kind),
        );
        if kind == "opencode" {
            env.insert(
                "AUTOSPEC_OPENCODE_CONTAINMENT_ADAPTER".to_string(),
                fs::canonicalize("/bin/true")
                    .expect("canonical adapter")
                    .into_os_string(),
            );
        }
        let artifact_root = state_path.with_extension("review-artifacts");

        let reviewer = bridge::resolve_independent_reviewer(&request, &state, &env, &artifact_root)
            .expect("resolve configured reviewer");
        let automatic = reviewer.automatic.expect("automatic reviewer artifacts");

        assert_eq!(automatic.result, automatic.inner_stdout, "{kind}");
        assert_ne!(automatic.result, automatic.inner_stderr, "{kind}");
        assert!(automatic.normalizer.starts_with(&artifact_root), "{kind}");
    }
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_clears_stale_codex_verdict_before_retry() {
    // Break caught: an interrupted Codex review's LGTM being reused by a later empty result.
    let root = test_root("automatic-reviewer-stale-result");
    let harness = root.join("stateful-reviewer");
    let launches = root.join("launches");
    write_executable(
        &harness,
        "#!/bin/sh\n\
         set -eu\n\
         launches=$1\n\
         result=$2\n\
         count=0\n\
         [ ! -f \"$launches\" ] || count=$(cat \"$launches\")\n\
         count=$((count + 1))\n\
         printf '%s\\n' \"$count\" > \"$launches\"\n\
         if [ \"$count\" -eq 1 ]; then\n\
         \tprintf '%s\\n' LGTM > \"$result\"\n\
         \texit 1\n\
         fi\n\
         exit 0\n",
    );
    let artifact_root = root.join("review-artifacts");
    bridge::ensure_private_directory(&artifact_root).expect("private review artifact directory");
    let result = artifact_root.join("harness-result.txt");
    let invocation = bridge::ValidatedInvocation {
        program: fs::canonicalize(&harness).expect("canonical stateful reviewer"),
        argv_zero: None,
        args: vec![launches.display().to_string(), result.display().to_string()],
        current_dir: root.clone(),
        environment_overrides: Vec::new(),
    };
    let automatic = bridge::prepare_automatic_reviewer_normalizer(
        bridge::HarnessKind::Codex,
        &invocation,
        &artifact_root,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        false,
    )
    .expect("automatic reviewer normalizer");

    let first = Command::new("/bin/sh")
        .arg(&automatic.normalizer)
        .current_dir(&root)
        .output()
        .expect("first reviewer attempt");
    assert!(!first.status.success());
    assert_eq!(
        fs::read_to_string(&result).expect("first verdict"),
        "LGTM\n"
    );

    let retry = Command::new("/bin/sh")
        .arg(&automatic.normalizer)
        .current_dir(&root)
        .output()
        .expect("retry reviewer attempt");
    assert!(
        !retry.status.success(),
        "retry accepted the stale Codex verdict: {retry:?}"
    );
    assert_eq!(fs::read_to_string(&result).expect("cleared verdict"), "");
    let _ = fs::remove_dir_all(root);
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_automatic_reviewer_normalizer_reaps_setsid_descendant_by_identity() {
    let _environment = test_environment();
    // Break caught: numeric group cleanup missing a session escape or killing an unrelated PID.
    let fixture = GitFixture::new("automatic-reviewer-setsid-descendant");
    let harness = fixture.root.join("forking-reviewer");
    let descendant_identity_path = fixture.root.join("descendant.identity");
    let private_state = fixture.root.join("private-state");
    let artifact_root = fixture.root.join("review-artifacts");
    bridge::ensure_private_directory(&artifact_root).expect("private review artifact directory");
    let result = artifact_root.join("harness-result.txt");
    let setsid = bridge::trusted_reviewer_utility("setsid").expect("trusted setsid");
    write_executable(
        &harness,
        &format!(
            "#!/bin/sh\n\
         set -eu\n\
         head -c 2097152 /dev/zero > \"$3\"\n\
         exec 3>>\"$1\"\n\
         '{}' -f /bin/sh -c 'trap \"\" HUP INT TERM; sid=$(/usr/bin/ps -o sid= -p \"$$\"); printf \"%s %s\\n\" \"$$\" \"$sid\" > \"$1\"; while :; do /usr/bin/sleep 1; done' descendant \"$2\" &\n\
         while [ ! -s \"$2\" ]; do /usr/bin/sleep 0.01; done\n\
         /usr/bin/sleep 0.1\n\
         printf '%s\\n' '{}' > \"$1\"\n",
            setsid.display(),
            super::support_review::valid_review_json("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        ),
    );
    let invocation = bridge::ValidatedInvocation {
        program: fs::canonicalize(&harness).expect("canonical forking reviewer"),
        argv_zero: None,
        args: vec![
            result.display().to_string(),
            descendant_identity_path.display().to_string(),
            private_state.display().to_string(),
        ],
        current_dir: fixture.repo.clone(),
        environment_overrides: Vec::new(),
    };
    let automatic = bridge::prepare_automatic_reviewer_normalizer(
        bridge::HarnessKind::Codex,
        &invocation,
        &artifact_root,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        false,
    )
    .expect("automatic reviewer normalizer");
    let mut unrelated = Command::new("/usr/bin/sleep")
        .arg("30")
        .process_group(0)
        .spawn()
        .expect("spawn unrelated process");
    let _unrelated_cleanup =
        DetachedForkedCleanup::new(unrelated.id()).expect("arm unrelated cleanup");
    let plan = bridge::DirectCommandPlan {
        commands: vec![bridge::DirectCommand::automatic_reviewer(
            vec![automatic.normalizer.display().to_string()],
            &automatic,
        )
        .expect("reviewer capture policy")],
    };
    let observations = bridge::execute_direct_plan(
        &fixture.repo,
        &plan,
        &fixture.root.join("direct-review"),
        None,
        Duration::from_secs(2),
    )
    .expect("reviewer tree is cleaned without changing its successful exit");

    assert_eq!(observations[0].exit_code(), Some(0));
    assert_eq!(
        fs::metadata(private_state).expect("private state").len(),
        2 * bridge::MAX_DIRECT_OUTPUT_BYTES
    );
    bridge::validate_automatic_reviewer_artifacts(&automatic)
        .expect("bounded strict reviewer artifacts");
    let identity = fs::read_to_string(&descendant_identity_path)
        .expect("descendant identity")
        .split_whitespace()
        .map(|value| value.parse::<u32>().expect("numeric identity"))
        .collect::<Vec<_>>();
    let descendant_pid = identity[0];
    assert_eq!(descendant_pid, identity[1], "setsid was not effective");
    let deadline = Instant::now() + Duration::from_secs(2);
    while Path::new(&format!("/proc/{descendant_pid}")).exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        !Path::new(&format!("/proc/{descendant_pid}")).exists(),
        "harness descendant leaked"
    );
    assert!(
        unrelated.try_wait().expect("observe unrelated").is_none(),
        "identity-safe cleanup killed an unrelated process"
    );
}
