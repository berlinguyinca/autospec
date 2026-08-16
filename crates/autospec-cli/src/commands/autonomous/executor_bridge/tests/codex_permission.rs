// executor_bridge tests: codex / permission — 9 cases.
//
// Split out of tests.rs; see the note in that file.

use super::super::{BridgePhase, HarnessConfig, HarnessKind};
use super::support_base::{
    environment, git, installed_aliases, test_environment, test_root, write_alias_table,
    write_executable,
};
use crate::commands::autonomous::executor_bridge as bridge;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[test]
fn autonomous_executor_bridge_alias_table_parses_all_four_columns() {
    // Break caught: hard-coded harness binaries or discarded approval/display metadata.
    let aliases =
        HarnessConfig::parse_alias_table(installed_aliases()).expect("parse installed alias table");

    assert_eq!(aliases.len(), 3);
    assert_eq!(aliases[0].kind, HarnessKind::Claude);
    assert_eq!(aliases[0].binary, "true");
    assert_eq!(aliases[0].approval_alias, "--dangerously-skip-permissions");
    assert_eq!(aliases[0].display_name, "Claude Code");
    assert_eq!(aliases[2].kind, HarnessKind::OpenCode);
    assert_eq!(aliases[2].approval_alias, "");
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_rejects_temporary_dispatcher_paths() {
    // Break caught: launching a PATH-shadowed dispatcher from an attacker-writable temp root.
    use std::os::unix::fs::PermissionsExt;

    let root = std::env::temp_dir().join(format!(
        "autospec-unsafe-dispatcher-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ));
    fs::create_dir_all(&root).expect("create unsafe dispatcher root");
    let dispatcher = root.join("codex");
    fs::write(&dispatcher, "#!/bin/sh\nexit 0\n").expect("write dispatcher");
    fs::set_permissions(&dispatcher, fs::Permissions::from_mode(0o755))
        .expect("make dispatcher executable");
    let table = write_alias_table(
        &root,
        "codex\tcodex\t--yolo\tCodex CLI\n\
         claude\ttrue\t--dangerously-skip-permissions\tClaude Code\n\
         opencode\tfalse\t\tOpenCode\n",
    );
    let mut env = environment(&table);
    env.insert("PATH".to_string(), root.as_os_str().to_os_string());
    env.insert(
        "TMPDIR".to_string(),
        std::env::temp_dir().as_os_str().to_os_string(),
    );
    env.insert("CODEX_THREAD_ID".to_string(), OsString::from("thread-1"));

    let error = HarnessConfig::load(&root, &env)
        .expect("load harness config")
        .resolve(&env)
        .expect_err("temporary dispatcher must fail closed");

    assert!(error.contains("temporary"), "unexpected error: {error}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn autonomous_executor_bridge_builds_exact_codex_arguments() {
    // Break caught: Codex launching without workspace-write containment or a result artifact.
    let root = test_root("codex-argv");
    let table = write_alias_table(&root, installed_aliases());
    let mut env = environment(&table);
    env.insert("CODEX_THREAD_ID".to_string(), OsString::from("thread-1"));
    let resolved = HarnessConfig::load(&root, &env)
        .expect("load harness config")
        .resolve(&env)
        .expect("resolve Codex");
    let worktree = Path::new("/safe/worktree");
    let artifact = Path::new("/safe/state/last-message.txt");

    let invocation = resolved
        .invocation(worktree, artifact, "implement issue 42")
        .expect("build Codex invocation");

    assert_eq!(invocation.program, resolved.executable);
    assert_eq!(invocation.current_dir, worktree);
    assert_eq!(
        invocation.args,
        vec![
            "exec",
            "-C",
            "/safe/worktree",
            "--sandbox",
            "workspace-write",
            "--ephemeral",
            "--output-last-message",
            "/safe/state/last-message.txt",
            "implement issue 42",
        ]
    );
    assert!(!invocation.requires_mutation_snapshots);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn autonomous_executor_bridge_codex_sandbox_success_selects_network_permission_profile() {
    let root = test_root("codex-sandbox-profile");
    let log = root.join("codex.log");
    let codex = root.join("codex");
    write_executable(
        &codex,
        &format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nexit 0\n",
            log.display()
        ),
    );

    let policy =
        bridge::preflight_codex_sandbox(&codex).expect("validated network permission profile");
    let harness = bridge::ResolvedHarness {
        kind: HarnessKind::Codex,
        executable: codex,
        opencode_adapter: None,
        codex_sandbox: policy,
        opencode_model: None,
        opencode_variant: None,
    };
    let invocation = harness
        .invocation(
            Path::new("/safe/worktree"),
            Path::new("/safe/worktree/result.txt"),
            "implement issue 42",
        )
        .expect("fallback invocation");

    assert!(!invocation
        .args
        .windows(2)
        .any(|pair| pair == ["--sandbox", "workspace-write"]));
    assert!(invocation
        .args
        .windows(2)
        .any(|pair| { pair == ["-c", "default_permissions=\"autospec-network-executor\"",] }));
    assert!(invocation.args.windows(2).any(|pair| {
        pair == [
            "-c",
            "permissions.autospec-network-executor.network.enabled=true",
        ]
    }));
    let filesystem = invocation
        .args
        .windows(2)
        .find_map(|pair| {
            (pair[0] == "-c"
                && pair[1].starts_with("permissions.autospec-network-executor.filesystem="))
            .then_some(pair[1].as_str())
        })
        .expect("permission-profile filesystem policy");
    for denied in [
        "\"~/.aws\"=\"deny\"",
        "\"~/.codex/archived_sessions\"=\"deny\"",
        "\"~/.codex/auth.json\"=\"deny\"",
        "\"~/.codex/config.toml\"=\"deny\"",
        "\"~/.codex/history.jsonl\"=\"deny\"",
        "\"~/.codex/sessions\"=\"deny\"",
        "\"~/.codex/shell_snapshots\"=\"deny\"",
        "\"~/.config/containers\"=\"deny\"",
        "\"~/.config/gh\"=\"deny\"",
        "\"~/.config/pip\"=\"deny\"",
        "\"~/.docker\"=\"deny\"",
        "\"~/.gnupg\"=\"deny\"",
        "\"~/.gradle\"=\"deny\"",
        "\"~/.git-credentials\"=\"deny\"",
        "\"~/.kube\"=\"deny\"",
        "\"~/.m2\"=\"deny\"",
        "\"~/.netrc\"=\"deny\"",
        "\"~/.npmrc\"=\"deny\"",
        "\"~/.ssh\"=\"deny\"",
        "\"~/.terraform.d\"=\"deny\"",
        "\"~/.vault-token\"=\"deny\"",
    ] {
        assert!(
            filesystem.contains(denied),
            "missing {denied}: {filesystem}"
        );
    }
    assert!(
        !filesystem.contains("\"~/.codex\"=\"deny\""),
        "Codex package roots must remain readable: {filesystem}"
    );
    assert!(filesystem.contains(&format!("\"{}\"=\"read\"", harness.executable.display())));
    assert!(invocation
        .args
        .windows(2)
        .any(|pair| { pair == ["-c", "shell_environment_policy.inherit=\"all\""] }));
    assert!(invocation.args.windows(2).any(|pair| {
        pair == [
            "-c",
            "shell_environment_policy.ignore_default_excludes=false",
        ]
    }));
    assert!(!invocation
        .args
        .iter()
        .any(|argument| argument.contains("danger-full-access")));
    assert_eq!(
        fs::read_to_string(log).expect("probe log").lines().count(),
        1
    );
    let _ = fs::remove_dir_all(root);
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_codex_sandbox_allows_executable_inside_real_codex_home() {
    let _environment = test_environment();
    let root = test_root("codex-permission-profile-real-home");
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).expect("workspace");
    let env = std::env::vars_os()
        .filter_map(|(key, value)| key.into_string().ok().map(|key| (key, value)))
        .collect::<BTreeMap<_, _>>();
    let codex = bridge::safe_executable(Path::new("codex"), &env)
        .expect("Codex CLI is required for its permission-profile regression");
    let home = env.get("HOME").map(PathBuf::from).expect("real HOME");
    if !codex.starts_with(home.join(".codex")) {
        let _ = fs::remove_dir_all(root);
        return;
    }

    let previous_codex_home = std::env::var_os("CODEX_HOME");
    std::env::remove_var("CODEX_HOME");
    let profile_args = bridge::CodexSandboxPolicy::NetworkPermissionProfile
        .permission_profile_args(&codex)
        .expect("permission profile arguments");
    let output = Command::new(&codex)
        .arg("sandbox")
        .arg("-C")
        .arg(&workspace)
        .arg("-P")
        .arg(bridge::CODEX_NETWORK_PERMISSION_PROFILE)
        .args(&profile_args)
        .args(["--", "/bin/true"])
        .output()
        .expect("run Codex permission-profile sandbox from its real installation");
    match previous_codex_home {
        Some(value) => std::env::set_var("CODEX_HOME", value),
        None => std::env::remove_var("CODEX_HOME"),
    }

    assert!(
        output.status.success(),
        "permission profile must allow its own executable at {}: stdout={} stderr={}",
        codex.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let _ = fs::remove_dir_all(root);
}

#[cfg(target_os = "linux")]
#[test]
fn autonomous_executor_bridge_codex_sandbox_permission_profile_denies_credential_files() {
    let _environment = test_environment();
    let root = test_root("codex-permission-profile");
    let home = root.join("home");
    let codex_home = root.join("custom-codex-home");
    let workspace = root.join("workspace");
    fs::create_dir_all(home.join(".aws")).expect("AWS credential directory");
    for path in [
        ".codex/archived_sessions",
        ".codex/sessions",
        ".codex/shell_snapshots",
    ] {
        fs::create_dir_all(home.join(path)).expect("Codex state directory");
    }
    fs::create_dir_all(home.join(".config/gh")).expect("GitHub credential directory");
    fs::create_dir_all(home.join(".ssh")).expect("SSH credential directory");
    for path in ["archived_sessions", "sessions", "shell_snapshots"] {
        fs::create_dir_all(codex_home.join(path)).expect("custom Codex state directory");
    }
    fs::create_dir_all(&workspace).expect("workspace");
    fs::write(home.join(".aws/credentials"), "aws-secret\n").expect("AWS credential");
    fs::write(home.join(".codex/auth.json"), "codex-secret\n").expect("Codex credential");
    fs::write(
        home.join(".codex/config.toml"),
        "model_reasoning_effort = \"high\"\n",
    )
    .expect("Codex config");
    fs::write(home.join(".codex/history.jsonl"), "history-secret\n").expect("Codex history");
    fs::write(
        home.join(".codex/archived_sessions/session.jsonl"),
        "archive-secret\n",
    )
    .expect("Codex archived session");
    fs::write(
        home.join(".codex/sessions/session.jsonl"),
        "session-secret\n",
    )
    .expect("Codex session");
    fs::write(
        home.join(".codex/shell_snapshots/snapshot.sh"),
        "snapshot-secret\n",
    )
    .expect("Codex shell snapshot");
    fs::write(home.join(".config/gh/hosts.yml"), "gh-secret\n").expect("GitHub credential");
    fs::write(home.join(".ssh/id_ed25519"), "ssh-secret\n").expect("SSH credential");
    fs::write(codex_home.join("auth.json"), "custom-codex-secret\n")
        .expect("custom Codex credential");
    fs::write(
        codex_home.join("config.toml"),
        "model_reasoning_effort = \"high\"\n",
    )
    .expect("custom Codex config");
    fs::write(codex_home.join("history.jsonl"), "custom-history-secret\n")
        .expect("custom Codex history");
    fs::write(
        codex_home.join("archived_sessions/session.jsonl"),
        "custom-archive-secret\n",
    )
    .expect("custom Codex archived session");
    fs::write(
        codex_home.join("sessions/session.jsonl"),
        "custom-session-secret\n",
    )
    .expect("custom Codex session");
    fs::write(
        codex_home.join("shell_snapshots/snapshot.sh"),
        "custom-snapshot-secret\n",
    )
    .expect("custom Codex shell snapshot");
    git(&workspace, &["init"]);
    let env = std::env::vars_os()
        .filter_map(|(key, value)| key.into_string().ok().map(|key| (key, value)))
        .collect::<BTreeMap<_, _>>();
    let codex = bridge::safe_executable(Path::new("codex"), &env)
        .expect("Codex CLI is required for its permission-profile regression");
    let policy = bridge::CodexSandboxPolicy::NetworkPermissionProfile;
    let previous_codex_home = std::env::var_os("CODEX_HOME");
    std::env::set_var("CODEX_HOME", &codex_home);
    let profile_args = policy
        .permission_profile_args(&codex)
        .expect("permission profile arguments");
    let mut command = Command::new(&codex);
    command
        .arg("sandbox")
        .arg("-C")
        .arg(&workspace)
        .arg("-P")
        .arg(bridge::CODEX_NETWORK_PERMISSION_PROFILE)
        .args(&profile_args)
        .args([
            "--",
            "/bin/sh",
            "-c",
            "set -eu\nprintf written > workspace-proof\nfor path in \"$HOME/.aws/credentials\" \"$HOME/.codex/auth.json\" \"$HOME/.codex/config.toml\" \"$HOME/.codex/history.jsonl\" \"$HOME/.codex/archived_sessions/session.jsonl\" \"$HOME/.codex/sessions/session.jsonl\" \"$HOME/.codex/shell_snapshots/snapshot.sh\" \"$HOME/.config/gh/hosts.yml\" \"$HOME/.ssh/id_ed25519\" \"$CODEX_HOME/auth.json\" \"$CODEX_HOME/config.toml\" \"$CODEX_HOME/history.jsonl\" \"$CODEX_HOME/archived_sessions/session.jsonl\" \"$CODEX_HOME/sessions/session.jsonl\" \"$CODEX_HOME/shell_snapshots/snapshot.sh\"; do if /bin/cat \"$path\" >/dev/null 2>&1; then exit 41; fi; done\ntest -z \"${CODEX_API_KEY:-}\"\ntest -z \"${OPENAI_API_KEY:-}\"",
        ])
        .env("HOME", &home)
        .env("CODEX_HOME", &codex_home)
        .env("CODEX_API_KEY", "host-only-codex-key")
        .env("OPENAI_API_KEY", "host-only-openai-key");
    let output = command
        .output()
        .expect("run Codex permission-profile sandbox");
    match previous_codex_home {
        Some(value) => std::env::set_var("CODEX_HOME", value),
        None => std::env::remove_var("CODEX_HOME"),
    }

    assert!(
        output.status.success(),
        "permission profile failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_to_string(workspace.join("workspace-proof")).expect("workspace write"),
        "written"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn autonomous_executor_bridge_codex_sandbox_blocks_bwrap_host_setup_failures() {
    let root = test_root("codex-sandbox-loopback-blocked");
    let codex = root.join("codex");
    for (stderr, expected) in [
        (
            "bwrap: loopback: Failed RTM_NEWADDR: Operation not permitted",
            "RTM_NEWADDR",
        ),
        (
            "bwrap: setting up uid map: Permission denied",
            "setting up uid map",
        ),
        (
            "thread 'main' panicked at sandbox.rs:1\nbwrap: setting up uid map: Operation not permitted\nnote: run with backtrace",
            "setting up uid map",
        ),
    ] {
        write_executable(
            &codex,
            &format!("#!/bin/sh\nprintf '%s\\n' '{}' >&2\nexit 1\n", stderr),
        );

        let error = bridge::preflight_codex_sandbox(&codex)
            .expect_err("bwrap host setup failure must block");

        assert!(error.contains("host_setup_required"), "{error}");
        assert!(error.contains("#2619"), "{error}");
        assert!(error.contains(expected), "{error}");
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn autonomous_executor_bridge_codex_sandbox_blocks_non_loopback_probe_failures() {
    let root = test_root("codex-sandbox-unsupported");
    let codex = root.join("codex");
    write_executable(
        &codex,
        "#!/bin/sh\nprintf '%s\\n' 'bwrap: creating new namespace failed: Permission denied' >&2\nexit 1\n",
    );

    let error = bridge::preflight_codex_sandbox(&codex)
        .expect_err("unsupported sandbox failures must block");

    assert!(
        error.contains("executor_codex_sandbox_unavailable"),
        "{error}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn autonomous_executor_bridge_codex_sandbox_cleanup_skips_failing_probe() {
    let mut harness = bridge::ResolvedHarness {
        kind: HarnessKind::Codex,
        executable: PathBuf::from("/missing/codex"),
        opencode_adapter: None,
        codex_sandbox: bridge::CodexSandboxPolicy::Default,
        opencode_model: None,
        opencode_variant: None,
    };
    let called = std::cell::Cell::new(false);
    bridge::configure_codex_sandbox_for_phase(&mut harness, BridgePhase::CleanupPending, |_| {
        called.set(true);
        Err("probe must not run during cleanup".to_string())
    })
    .expect("cleanup phase must not depend on Codex sandbox probing");
    assert!(!called.get());
    assert_eq!(harness.codex_sandbox, bridge::CodexSandboxPolicy::Default);
}
