// executor_bridge tests: proxy / direct — 10 cases.
//
// Split out of tests.rs; see the note in that file.

use super::super::{HarnessKind, MutationSnapshot, SupervisionOutcome};
use super::support_base::{git_stdout, test_environment, write_executable, GitFixture};
use super::support_invocation::{session_record_ids, supervision_config, supervision_state};
use crate::commands::autonomous::executor_bridge as bridge;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[test]
fn autonomous_executor_bridge_runtime_adapter_binds_attempt_to_exact_session() {
    let _environment = test_environment();
    let fixture = GitFixture::new("runtime-qa-prefix");
    fs::create_dir_all(fixture.repo.join(".autospec")).expect("runtime manifest directory");
    fs::write(
        fixture.repo.join("runtime-up.py"),
        "import http.server, os\npid=os.fork()\nif not pid:\n http.server.HTTPServer(('127.0.0.1', int(os.environ['AGENT_FRONTEND_PORT'])), http.server.SimpleHTTPRequestHandler).serve_forever()\nelse:\n open('runtime.pid','w').write(str(pid))\n",
    )
    .expect("runtime up executable");
    fs::write(
        fixture.repo.join("runtime-down.py"),
        "import os, signal\npid=int(open('runtime.pid').read())\nos.kill(pid, signal.SIGTERM)\nos.remove('runtime.pid')\n",
    )
    .expect("runtime down executable");
    fs::write(
        fixture.repo.join(".autospec/runtime.yml"),
        "version: 1\ndefault_mode: local\nmodes:\n  local:\n    command: python3 runtime-up.py\n    down: python3 runtime-down.py\n",
    )
    .expect("runtime manifest");
    let state_root = fixture.root.join("runtime-state");
    let previous = std::env::var_os("AGENT_ENV_STATE_ROOT");
    std::env::set_var("AGENT_ENV_STATE_ROOT", &state_root);
    let runtime = bridge::DirectRuntimeAdapter::prepare(&fixture.repo).expect("runtime adapter");
    let session_id = runtime.session_id().to_string();
    let plan = bridge::parse_direct_command_plan("/usr/bin/printf ok").expect("direct plan");

    let observed = bridge::execute_direct_plan(
        &fixture.repo,
        &plan,
        &fixture.root.join("evidence"),
        Some(&runtime),
        Duration::from_secs(5),
    )
    .expect("runtime execution");

    assert_eq!(
        observed[0].runtime_session_id.as_deref(),
        Some(session_id.as_str())
    );
    assert_eq!(
        observed[0].process_executable,
        PathBuf::from("/usr/bin/printf").canonicalize().unwrap()
    );
    assert_eq!(
        observed[0].process_argv,
        vec!["/usr/bin/printf".to_string(), "ok".to_string()]
    );
    assert_eq!(session_record_ids(&state_root), vec![session_id]);
    runtime.close_verified().expect("verified close");
    assert!(session_record_ids(&state_root).is_empty());
    match previous {
        Some(value) => std::env::set_var("AGENT_ENV_STATE_ROOT", value),
        None => std::env::remove_var("AGENT_ENV_STATE_ROOT"),
    }
}

#[test]
fn autonomous_executor_bridge_direct_proxy_argv_zero_helper() {
    if std::env::var_os("AUTOSPEC_TEST_DIRECT_PROXY_ARGV_ZERO").is_none() {
        return;
    }
    assert_eq!(
        std::env::args_os()
            .next()
            .as_deref()
            .and_then(|arg0| Path::new(arg0).file_name()),
        Some(std::ffi::OsStr::new("cargo-proxy"))
    );
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_direct_proxy_preserves_argv_zero() {
    let _environment = test_environment();
    let fixture = GitFixture::new("direct-proxy-argv-zero");
    let proxy = fixture.root.join("cargo-proxy");
    let executable = std::env::current_exe()
        .expect("current test executable")
        .canonicalize()
        .expect("canonical test executable");
    std::os::unix::fs::symlink(&executable, &proxy).expect("test executable proxy");
    let plan = bridge::parse_direct_command_plan(&format!(
        "{} commands::autonomous::executor_bridge::tests::proxy_direct::autonomous_executor_bridge_direct_proxy_argv_zero_helper --exact --nocapture",
        proxy.display()
    ))
    .expect("proxy command plan");
    let previous = std::env::var_os("AUTOSPEC_TEST_DIRECT_PROXY_ARGV_ZERO");
    std::env::set_var("AUTOSPEC_TEST_DIRECT_PROXY_ARGV_ZERO", "1");

    let result = bridge::execute_direct_plan(
        &fixture.repo,
        &plan,
        &fixture.root.join("proxy-evidence"),
        None,
        Duration::from_secs(5),
    );

    match previous {
        Some(value) => std::env::set_var("AUTOSPEC_TEST_DIRECT_PROXY_ARGV_ZERO", value),
        None => std::env::remove_var("AUTOSPEC_TEST_DIRECT_PROXY_ARGV_ZERO"),
    }
    let observed = result.expect("validated proxy must preserve argv zero");
    assert_eq!(observed[0].executable, executable);
    assert_eq!(observed[0].process_executable, executable);
    assert_eq!(observed[0].argv[0], proxy.display().to_string());
    assert_eq!(observed[0].process_argv[0], proxy.display().to_string());
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_direct_proxy_change_retries_terminal_failure() {
    let _environment = test_environment();
    let fixture = GitFixture::new("direct-proxy-retry");
    let artifact_root = fixture.root.join("proxy-evidence");
    let executable = std::env::current_exe()
        .expect("current test executable")
        .canonicalize()
        .expect("canonical test executable");
    let proxy = fixture.root.join("cargo-proxy");
    std::os::unix::fs::symlink(&executable, &proxy).expect("test executable proxy");
    let arguments = "commands::autonomous::executor_bridge::tests::proxy_direct::autonomous_executor_bridge_direct_proxy_argv_zero_helper --exact --nocapture";
    let canonical_plan =
        bridge::parse_direct_command_plan(&format!("{} {arguments}", executable.display()))
            .expect("canonical command plan");
    let proxy_plan = bridge::parse_direct_command_plan(&format!("{} {arguments}", proxy.display()))
        .expect("proxy command plan");
    let previous = std::env::var_os("AUTOSPEC_TEST_DIRECT_PROXY_ARGV_ZERO");
    std::env::set_var("AUTOSPEC_TEST_DIRECT_PROXY_ARGV_ZERO", "1");

    let first = bridge::execute_direct_plan(
        &fixture.repo,
        &canonical_plan,
        &artifact_root,
        None,
        Duration::from_secs(5),
    );
    assert!(
        first.is_err(),
        "canonical argv zero must reproduce the proxy failure"
    );
    let result = bridge::execute_direct_plan(
        &fixture.repo,
        &proxy_plan,
        &artifact_root,
        None,
        Duration::from_secs(5),
    );

    match previous {
        Some(value) => std::env::set_var("AUTOSPEC_TEST_DIRECT_PROXY_ARGV_ZERO", value),
        None => std::env::remove_var("AUTOSPEC_TEST_DIRECT_PROXY_ARGV_ZERO"),
    }
    let observed = result.expect("proxy correction must archive and retry prior failure");
    assert_eq!(observed[0].terminal, bridge::AttemptTerminal::Exited(0));
    assert_eq!(observed[0].argv[0], proxy.display().to_string());
    assert!(fs::read_dir(&artifact_root)
        .expect("proxy failure archive")
        .flatten()
        .any(|entry| {
            let file_name = entry.file_name();
            let name = file_name.to_string_lossy();
            name.rsplit_once(".archive-").is_some_and(|(_, suffix)| !suffix.is_empty())
        }));
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_direct_proxy_change_retries_terminal_success() {
    let _environment = test_environment();
    let fixture = GitFixture::new("direct-proxy-success-retry");
    let artifact_root = fixture.root.join("proxy-evidence");
    let executable = PathBuf::from("/usr/bin/true")
        .canonicalize()
        .expect("canonical true executable");
    let proxy = fixture.root.join("true-proxy");
    std::os::unix::fs::symlink(&executable, &proxy).expect("true executable proxy");
    let canonical_plan = bridge::parse_direct_command_plan(&executable.display().to_string())
        .expect("canonical command plan");
    let proxy_plan = bridge::parse_direct_command_plan(&proxy.display().to_string())
        .expect("proxy command plan");

    let first = bridge::execute_direct_plan(
        &fixture.repo,
        &canonical_plan,
        &artifact_root,
        None,
        Duration::from_secs(5),
    )
    .expect("canonical command success");
    assert_eq!(first[0].terminal, bridge::AttemptTerminal::Exited(0));
    let observed = bridge::execute_direct_plan(
        &fixture.repo,
        &proxy_plan,
        &artifact_root,
        None,
        Duration::from_secs(5),
    )
    .expect("proxy correction must archive and rerun prior success");

    assert_eq!(observed[0].terminal, bridge::AttemptTerminal::Exited(0));
    assert_eq!(observed[0].argv[0], proxy.display().to_string());
    assert!(fs::read_dir(&artifact_root)
        .expect("proxy success archive")
        .flatten()
        .any(|entry| {
            let file_name = entry.file_name();
            let name = file_name.to_string_lossy();
            name.rsplit_once(".archive-").is_some_and(|(_, suffix)| !suffix.is_empty())
        }));
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_cargo_proxy_dispatches_rustup() {
    let _environment = test_environment();
    let fixture = GitFixture::new("cargo-rustup-proxy");
    let Ok(rustup) = bridge::resolve_direct_executable(&fixture.repo, "rustup") else {
        return;
    };
    let proxy = fixture.root.join("cargo");
    std::os::unix::fs::symlink(&rustup.program, &proxy).expect("Cargo rustup proxy");
    let plan = bridge::parse_direct_command_plan(&format!("{} --version", proxy.display()))
        .expect("Cargo proxy command plan");

    let observed = bridge::execute_direct_plan(
        &fixture.repo,
        &plan,
        &fixture.root.join("cargo-evidence"),
        None,
        Duration::from_secs(5),
    )
    .expect("Cargo proxy dispatch through rustup");

    assert_eq!(observed[0].terminal, bridge::AttemptTerminal::Exited(0));
    assert_eq!(observed[0].executable, rustup.program);
    assert_eq!(observed[0].argv[0], proxy.display().to_string());
    assert!(fs::read_to_string(&observed[0].stdout_path)
        .expect("Cargo version output")
        .starts_with("cargo "));
}

#[test]
fn autonomous_executor_bridge_codex_sandbox_fallback_children_have_no_sensitive_credentials() {
    let _environment = test_environment();
    let fixture = GitFixture::new("credentialless-direct-child");
    let keys = [
        "GH_TOKEN",
        "GH_ENTERPRISE_TOKEN",
        "GITHUB_TOKEN",
        "GITHUB_ENTERPRISE_TOKEN",
        "SSH_AUTH_SOCK",
        "GIT_ASKPASS",
        "AWS_ACCESS_KEY_ID",
        "AWS_SECRET_ACCESS_KEY",
        "NPM_TOKEN",
        "DATABASE_URL",
        "INTERNAL_API_KEY",
        "DOCKER_AUTH_CONFIG",
        "KUBECONFIG",
        "VAULT_TOKEN",
        "OPENAI_API_KEY",
    ];
    let previous = keys
        .iter()
        .map(|key| ((*key).to_string(), std::env::var_os(key)))
        .collect::<Vec<_>>();
    for key in keys {
        std::env::set_var(key, format!("forbidden-{key}"));
    }
    let plan = bridge::parse_direct_command_plan("/usr/bin/env").expect("environment command");
    let observed = bridge::execute_direct_plan(
        &fixture.repo,
        &plan,
        &fixture.root.join("credential-evidence"),
        None,
        Duration::from_secs(5),
    )
    .expect("credentialless child");
    let environment = fs::read_to_string(&observed[0].stdout_path).expect("captured environment");

    for key in keys {
        assert!(
            !environment
                .lines()
                .any(|line| line.starts_with(&format!("{key}="))),
            "{key} leaked to direct child"
        );
    }
    for expected in [
        "GIT_CONFIG_NOSYSTEM=1",
        "GIT_CONFIG_GLOBAL=/dev/null",
        "GIT_TERMINAL_PROMPT=0",
    ] {
        assert!(
            environment.lines().any(|line| line == expected),
            "{expected}"
        );
    }
    assert!(
        environment
            .lines()
            .any(|line| line.starts_with("GH_CONFIG_DIR=")
                && line.ends_with("/credentialless-config"))
    );
    assert!(environment
        .lines()
        .any(|line| line.starts_with("GIT_SSH_COMMAND=/usr/bin/ssh -F /dev/null")));
    for (key, value) in previous {
        match value {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_codex_sandbox_preserves_host_auth_for_codex_only() {
    let _environment = test_environment();
    let fixture = GitFixture::new("codex-host-auth");
    let fake_codex = fixture.root.join("codex");
    write_executable(
        &fake_codex,
        "#!/bin/sh\nprintf 'codex=%s openai=%s\\n' \"${CODEX_API_KEY:-missing}\" \"${OPENAI_API_KEY:-missing}\"\n",
    );
    let mut state = supervision_state(&fixture);
    state.identity.branch = git_stdout(
        &fixture.repo,
        &["symbolic-ref", "--quiet", "--short", "HEAD"],
    );
    let snapshot =
        MutationSnapshot::capture(&fixture.repo, &state.identity.branch).expect("snapshot");
    let state_path = fixture.root.join("state/codex-host-auth.json");
    let event_log = fixture.root.join("log/codex-host-auth.jsonl");
    let artifact = fixture.repo.join(".autospec/executor-closeout.md");
    let harness = bridge::ResolvedHarness {
        kind: HarnessKind::Codex,
        executable: fake_codex.canonicalize().expect("canonical fake Codex"),
        opencode_adapter: None,
        codex_sandbox: bridge::CodexSandboxPolicy::NetworkPermissionProfile,
        opencode_model: None,
        opencode_variant: None,
    };
    let previous_codex = std::env::var_os("CODEX_API_KEY");
    let previous_openai = std::env::var_os("OPENAI_API_KEY");
    let previous_claim = std::env::var_os("AUTOSPEC_TEST_EXACT_EVIDENCE_CLAIM");
    std::env::set_var("CODEX_API_KEY", "codex-host-auth-value");
    std::env::set_var("OPENAI_API_KEY", "openai-host-auth-value");
    std::env::set_var("AUTOSPEC_TEST_EXACT_EVIDENCE_CLAIM", "1");

    let outcome = bridge::supervise_resolved_harness(
        &state_path,
        &event_log,
        &mut state,
        bridge::HarnessLaunch {
            resolved: &harness,
            artifact: &artifact,
            prompt: "prove host auth",
        },
        &snapshot,
        supervision_config(5_000),
    )
    .expect("supervise fake Codex");

    match previous_codex {
        Some(value) => std::env::set_var("CODEX_API_KEY", value),
        None => std::env::remove_var("CODEX_API_KEY"),
    }
    match previous_openai {
        Some(value) => std::env::set_var("OPENAI_API_KEY", value),
        None => std::env::remove_var("OPENAI_API_KEY"),
    }
    match previous_claim {
        Some(value) => std::env::set_var("AUTOSPEC_TEST_EXACT_EVIDENCE_CLAIM", value),
        None => std::env::remove_var("AUTOSPEC_TEST_EXACT_EVIDENCE_CLAIM"),
    }
    assert_eq!(outcome, SupervisionOutcome::Exited { exit_code: 0 });
    let events = fs::read_to_string(&event_log).expect("Codex host output");
    assert!(
        events.contains("codex=codex-host-auth-value openai=openai-host-auth-value"),
        "Codex host auth was stripped: {events}"
    );
}

#[test]
fn autonomous_executor_bridge_resolves_full_suite_in_authoritative_order() {
    let fixture = GitFixture::new("full-suite-order");
    fs::write(
        fixture.repo.join("Cargo.toml"),
        "[package]\nname='fixture'\nversion='0.1.0'\n",
    )
    .expect("write cargo manifest");
    let issue = "### Operator/full verification\n\n```bash\n/usr/bin/printf issue\n```\n";
    let spec = "### Operator full\n\n```\n/usr/bin/printf spec\n```\n";
    let override_env = BTreeMap::from([(
        "AUTOSPEC_FULL_TEST_COMMAND".to_string(),
        OsString::from("/usr/bin/printf override"),
    )]);

    let overridden = bridge::resolve_full_suite(&fixture.repo, issue, &[spec], &override_env)
        .expect("environment override");
    assert_eq!(overridden.source, bridge::FullSuiteSource::Environment);
    assert_eq!(
        overridden.plan.commands[0].argv,
        vec!["/usr/bin/printf", "override"]
    );

    let declared = bridge::resolve_full_suite(&fixture.repo, issue, &[spec], &BTreeMap::new())
        .expect("declared full verification");
    assert_eq!(declared.source, bridge::FullSuiteSource::Declared);
    assert_eq!(declared.plan.commands.len(), 2);

    let fallback = bridge::resolve_full_suite(&fixture.repo, "", &[], &BTreeMap::new())
        .expect("ecosystem fallback");
    assert_eq!(fallback.source, bridge::FullSuiteSource::Ecosystem);
    assert_eq!(
        fallback
            .plan
            .commands
            .iter()
            .map(|command| command.argv.join(" "))
            .collect::<Vec<_>>(),
        vec![
            "cargo fmt --check",
            "cargo clippy --all-targets -- -D warnings",
            "cargo test --all-targets",
            "cargo build --all-targets",
        ]
    );
}

#[test]
fn autonomous_executor_bridge_full_suite_rejects_unknown_or_incomplete_repositories() {
    let fixture = GitFixture::new("full-suite-unknown");
    let error = bridge::resolve_full_suite(&fixture.repo, "", &[], &BTreeMap::new())
        .expect_err("unknown repository must not have a fabricated suite");
    assert!(error.contains("complete full suite"), "{error}");

    fs::write(
        fixture.repo.join("package.json"),
        r#"{"scripts":{"test":"vitest"}}"#,
    )
    .expect("write incomplete package manifest");
    let error = bridge::resolve_full_suite(&fixture.repo, "", &[], &BTreeMap::new())
        .expect_err("package suite missing lint/typecheck/build must fail closed");
    assert!(error.contains("incomplete"), "{error}");
}
