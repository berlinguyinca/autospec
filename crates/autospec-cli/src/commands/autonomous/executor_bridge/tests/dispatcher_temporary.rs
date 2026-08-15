// executor_bridge tests: dispatcher / temporary — 7 cases.
//
// Split out of tests.rs; see the note in that file.

use super::super::{
    HarnessConfig, CLAUDE_BUILTIN_TOOLS, CLAUDE_FORBIDDEN_TOOLS, CLAUDE_LOCAL_TOOLS,
};
use super::support_base::{
    environment, installed_aliases, test_root, write_alias_table, TEST_SEQUENCE,
};
use crate::commands::autonomous::executor_bridge as bridge;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

#[test]
fn autonomous_executor_bridge_builds_exact_claude_arguments() {
    // Break caught: Claude buffering progress until exit, being unable to test/commit locally,
    // or inheriting remote Git permissions from user/project settings.
    let root = test_root("claude-argv");
    let table = write_alias_table(&root, installed_aliases());
    let mut env = environment(&table);
    env.insert(
        "AUTOSPEC_HANDOFF_DISPATCHER_KIND".to_string(),
        OsString::from("claude"),
    );
    let resolved = HarnessConfig::load(&root, &env)
        .expect("load harness config")
        .resolve(&env)
        .expect("resolve Claude");

    let invocation = resolved
        .invocation(
            Path::new("/safe/worktree"),
            Path::new("/safe/state/unused.txt"),
            "implement issue 42",
        )
        .expect("build Claude invocation");

    assert_eq!(
        invocation.args,
        vec![
            "-p",
            "--tools",
            CLAUDE_BUILTIN_TOOLS,
            "--safe-mode",
            "--disable-slash-commands",
            "--setting-sources",
            "",
            "--strict-mcp-config",
            "--mcp-config",
            r#"{"mcpServers":{}}"#,
            "--permission-mode",
            "acceptEdits",
            "--allowedTools",
            CLAUDE_LOCAL_TOOLS,
            "--disallowedTools",
            CLAUDE_FORBIDDEN_TOOLS,
            "--no-session-persistence",
            "--verbose",
            "--include-partial-messages",
            "--output-format",
            "stream-json",
            "implement issue 42",
        ]
    );
    assert!(invocation.requires_mutation_snapshots);
    assert!(!invocation
        .args
        .iter()
        .any(|arg| arg == "--dangerously-skip-permissions"));
    assert!(
        !invocation.args.iter().any(|arg| arg == "--bare"),
        "Claude invocation must preserve normal OAuth/keychain authentication"
    );
    assert_eq!(
        CLAUDE_BUILTIN_TOOLS, "Read,Edit,Write,Glob,Grep,Bash",
        "Claude must expose only the six local built-ins needed for implementation"
    );
    for isolation_flag in [
        "--safe-mode",
        "--disable-slash-commands",
        "--strict-mcp-config",
    ] {
        assert!(
            invocation.args.iter().any(|arg| arg == isolation_flag),
            "Claude invocation must include {isolation_flag}"
        );
    }
    let strict_mcp = invocation
        .args
        .windows(2)
        .find(|pair| pair[0] == "--mcp-config")
        .expect("explicit MCP configuration");
    assert_eq!(strict_mcp[1], r#"{"mcpServers":{}}"#);
    let setting_sources = invocation
        .args
        .windows(2)
        .find(|pair| pair[0] == "--setting-sources")
        .expect("explicit setting source isolation");
    assert_eq!(setting_sources[1], "");
    let reviewer = resolved
        .review_invocation(
            Path::new("/safe/worktree"),
            Path::new("/safe/state/review.txt"),
            "review issue 42",
        )
        .expect("build Claude reviewer invocation");
    assert!(reviewer
        .args
        .windows(2)
        .any(|pair| pair == ["--output-format", "text"]));
    assert!(!reviewer
        .args
        .iter()
        .any(|arg| arg == "--include-partial-messages"));
    for required in [
        "Read",
        "Edit",
        "Write",
        "Glob",
        "Grep",
        "Bash(git status)",
        "Bash(git status *)",
        "Bash(git diff)",
        "Bash(git diff *)",
        "Bash(git add *)",
        "Bash(git commit *)",
        "Bash(cargo test)",
        "Bash(cargo test *)",
        "Bash(npm test)",
        "Bash(npm test *)",
        "Bash(pnpm test *)",
        "Bash(yarn test *)",
        "Bash(go test *)",
        "Bash(pytest *)",
        "Bash(python -m pytest *)",
        "Bash(bats)",
        "Bash(bats *)",
        "Bash(shellcheck)",
        "Bash(shellcheck *)",
        "Bash(bash -n)",
        "Bash(bash -n *)",
        "Bash(./scripts/validate-all.sh)",
        "Bash(./scripts/validate-all.sh *)",
    ] {
        assert!(
            CLAUDE_LOCAL_TOOLS.split(',').any(|tool| tool == required),
            "Claude local-only policy must allow {required}"
        );
    }
    for overly_broad in [
        "Bash",
        "Bash(git *)",
        "Bash(cargo *)",
        "Bash(npm *)",
        "Bash(pnpm *)",
        "Bash(yarn *)",
        "Bash(bash *)",
        "Bash(bash -c *)",
    ] {
        assert!(
            !CLAUDE_LOCAL_TOOLS
                .split(',')
                .any(|tool| tool == overly_broad),
            "Claude local-only policy must not broadly allow {overly_broad}"
        );
    }
    for forbidden in [
        "Bash(git push)",
        "Bash(git push *)",
        "Bash(git fetch)",
        "Bash(git fetch *)",
        "Bash(git pull)",
        "Bash(git pull *)",
        "Bash(git remote)",
        "Bash(git remote *)",
        "Bash(git worktree)",
        "Bash(git worktree *)",
        "Bash(git branch)",
        "Bash(git branch *)",
        "Bash(git checkout)",
        "Bash(git checkout *)",
        "Bash(git switch)",
        "Bash(git switch *)",
        "Bash(git merge *)",
        "Bash(git rebase *)",
    ] {
        assert!(
            CLAUDE_FORBIDDEN_TOOLS
                .split(',')
                .any(|tool| tool == forbidden),
            "Claude local-only policy must explicitly deny {forbidden}"
        );
    }
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_rejects_dot_path_dispatcher_directory() {
    // Break caught: PATH=. selecting an executable from the conductor's mutable cwd.
    use std::os::unix::fs::PermissionsExt;

    let dispatcher_name = format!(
        "autospec-relative-path-dispatcher-{}",
        TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let dispatcher = std::env::current_dir()
        .expect("current directory")
        .join(&dispatcher_name);
    fs::write(&dispatcher, "#!/bin/sh\nexit 0\n").expect("write relative dispatcher");
    fs::set_permissions(&dispatcher, fs::Permissions::from_mode(0o755))
        .expect("make relative dispatcher executable");
    let env = BTreeMap::from([("PATH".to_string(), OsString::from("."))]);

    let error = bridge::safe_executable(Path::new(&dispatcher_name), &env)
        .expect_err("relative PATH directories must be ignored");

    assert!(
        error.contains("not found on PATH"),
        "unexpected error: {error}"
    );
    let _ = fs::remove_file(dispatcher);
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_rejects_empty_path_dispatcher_directory() {
    // Break caught: an empty PATH component selecting from the conductor's mutable cwd.
    use std::os::unix::fs::PermissionsExt;

    let dispatcher_name = format!(
        "autospec-empty-path-dispatcher-{}",
        TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let dispatcher = std::env::current_dir()
        .expect("current directory")
        .join(&dispatcher_name);
    fs::write(&dispatcher, "#!/bin/sh\nexit 0\n").expect("write empty-path dispatcher");
    fs::set_permissions(&dispatcher, fs::Permissions::from_mode(0o755))
        .expect("make empty-path dispatcher executable");
    let env = BTreeMap::from([("PATH".to_string(), OsString::from(":"))]);

    let error = bridge::safe_executable(Path::new(&dispatcher_name), &env)
        .expect_err("empty PATH directories must be ignored");

    assert!(
        error.contains("not found on PATH"),
        "unexpected error: {error}"
    );
    let _ = fs::remove_file(dispatcher);
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_rejects_every_temporary_dispatcher_root() {
    // Break caught: the Rust bridge accepting a dispatcher that the shared shell resolver
    // rejects, especially /var/tmp and an operator-supplied TMPDIR.
    use bridge::temporary_path;

    let mut env = BTreeMap::new();
    env.insert(
        "TMPDIR".to_string(),
        OsString::from("/safe/operator-temporary"),
    );

    for denied in [
        "/tmp/codex",
        "/private/tmp/codex",
        "/var/tmp/codex",
        "/var/folders/session/codex",
        "/safe/operator-temporary/codex",
    ] {
        assert!(
            temporary_path(Path::new(denied), &env),
            "{denied} must be treated as temporary"
        );
    }
    for allowed in [
        "/tmp-safe/codex",
        "/private/tmp-safe/codex",
        "/var/tmp-safe/codex",
        "/var/folders-safe/codex",
        "/safe/operator-temporary-safe/codex",
    ] {
        assert!(
            !temporary_path(Path::new(allowed), &env),
            "{allowed} must not match a temporary prefix accidentally"
        );
    }
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_checks_temporary_candidate_before_canonicalization() {
    // Break caught: a symlink placed in temporary storage being accepted because its target
    // is an otherwise safe executable.
    use std::os::unix::fs::{symlink, PermissionsExt};

    let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let safe_root = PathBuf::from(std::env::var_os("HOME").expect("HOME for test fixture"))
        .join(".autospec/test-fixtures")
        .join(format!(
            "executor-safe-target-{}-{sequence}",
            std::process::id()
        ));
    fs::create_dir_all(&safe_root).expect("create safe target root");
    let safe_target = safe_root.join("codex");
    fs::write(&safe_target, "#!/bin/sh\nexit 0\n").expect("write safe target");
    fs::set_permissions(&safe_target, fs::Permissions::from_mode(0o755))
        .expect("make safe target executable");

    let var_tmp_root = PathBuf::from("/var/tmp").join(format!(
        "autospec-executor-candidate-{}-{sequence}",
        std::process::id()
    ));
    fs::create_dir_all(&var_tmp_root).expect("create /var/tmp candidate root");
    let var_tmp_link = var_tmp_root.join("codex");
    symlink(&safe_target, &var_tmp_link).expect("link temporary candidate to safe target");

    let custom_tmp = safe_root.join("operator-tmp");
    fs::create_dir_all(&custom_tmp).expect("create supplied TMPDIR");
    let custom_tmp_link = custom_tmp.join("claude");
    symlink(&safe_target, &custom_tmp_link).expect("link TMPDIR candidate to safe target");
    let env = BTreeMap::from([("TMPDIR".to_string(), custom_tmp.as_os_str().to_os_string())]);

    for candidate in [&var_tmp_link, &custom_tmp_link] {
        let error = bridge::safe_executable(candidate, &env)
            .expect_err("temporary candidate must fail before canonicalization");
        assert!(error.contains("temporary"), "unexpected error: {error}");
    }

    let _ = fs::remove_dir_all(var_tmp_root);
    let _ = fs::remove_dir_all(safe_root);
}

#[cfg(unix)]
#[test]
fn autonomous_executor_bridge_checks_temporary_target_after_canonicalization() {
    // Break caught: a safe-looking configured path resolving to an executable in /var/tmp.
    use std::os::unix::fs::{symlink, PermissionsExt};

    let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let safe_root = PathBuf::from(std::env::var_os("HOME").expect("HOME for test fixture"))
        .join(".autospec/test-fixtures")
        .join(format!(
            "executor-safe-candidate-{}-{sequence}",
            std::process::id()
        ));
    fs::create_dir_all(&safe_root).expect("create safe candidate root");

    let var_tmp_root = PathBuf::from("/var/tmp").join(format!(
        "autospec-executor-target-{}-{sequence}",
        std::process::id()
    ));
    fs::create_dir_all(&var_tmp_root).expect("create /var/tmp target root");
    let temporary_target = var_tmp_root.join("codex");
    fs::write(&temporary_target, "#!/bin/sh\nexit 0\n").expect("write temporary target");
    fs::set_permissions(&temporary_target, fs::Permissions::from_mode(0o755))
        .expect("make temporary target executable");
    let var_tmp_link = safe_root.join("codex");
    symlink(&temporary_target, &var_tmp_link).expect("link safe candidate to /var/tmp target");

    let custom_tmp_root =
        PathBuf::from(std::env::var_os("HOME").expect("HOME for supplied TMPDIR fixture"))
            .join(".autospec/test-fixtures")
            .join(format!(
                "executor-custom-tmp-target-{}-{sequence}",
                std::process::id()
            ));
    fs::create_dir_all(&custom_tmp_root).expect("create supplied TMPDIR target root");
    let custom_tmp_target = custom_tmp_root.join("claude");
    fs::write(&custom_tmp_target, "#!/bin/sh\nexit 0\n").expect("write supplied TMPDIR target");
    fs::set_permissions(&custom_tmp_target, fs::Permissions::from_mode(0o755))
        .expect("make supplied TMPDIR target executable");
    let custom_tmp_link = safe_root.join("claude");
    symlink(&custom_tmp_target, &custom_tmp_link)
        .expect("link safe candidate to supplied TMPDIR target");

    let custom_env = BTreeMap::from([(
        "TMPDIR".to_string(),
        custom_tmp_root.as_os_str().to_os_string(),
    )]);
    let empty_env = BTreeMap::new();
    for (candidate, env) in [(&var_tmp_link, &empty_env), (&custom_tmp_link, &custom_env)] {
        let error = bridge::safe_executable(candidate, env)
            .expect_err("temporary canonical target must fail closed");
        assert!(error.contains("temporary"), "unexpected error: {error}");
    }

    let _ = fs::remove_dir_all(safe_root);
    let _ = fs::remove_dir_all(var_tmp_root);
    let _ = fs::remove_dir_all(custom_tmp_root);
}

#[test]
fn autonomous_executor_bridge_loads_installed_config_locations() {
    // Break caught: an installed binary requiring a manual alias-table override.
    let root = test_root("installed-config");
    write_alias_table(&root, installed_aliases());
    let mut env = BTreeMap::from([
        (
            "AUTOSPEC_CONFIG_DIR".to_string(),
            root.as_os_str().to_os_string(),
        ),
        ("PATH".to_string(), OsString::from("/bin:/usr/bin")),
    ]);
    assert_eq!(
        HarnessConfig::load(&root, &env)
            .expect("load AUTOSPEC_CONFIG_DIR aliases")
            .aliases
            .len(),
        3
    );

    env.remove("AUTOSPEC_CONFIG_DIR");
    let home = test_root("installed-home");
    let standard_config = home.join(".autospec/config");
    fs::create_dir_all(&standard_config).expect("create standard config");
    write_alias_table(&standard_config, installed_aliases());
    env.insert("HOME".to_string(), home.as_os_str().to_os_string());
    assert_eq!(
        HarnessConfig::load(&root, &env)
            .expect("load standard aliases")
            .aliases
            .len(),
        3
    );
    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(home);
}
