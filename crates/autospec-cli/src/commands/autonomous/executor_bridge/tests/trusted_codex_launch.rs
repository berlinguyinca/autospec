// executor_bridge tests: trusted Codex launch plans — 8 cases.

use super::support_base::{test_root, write_executable};
use super::support_invocation::implementation_proof_fixture;
use crate::commands::autonomous::executor_bridge as bridge;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::symlink;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TRUSTED_TOOL_NONCE: AtomicU64 = AtomicU64::new(0);

struct TrustedToolRoot(PathBuf);

impl Drop for TrustedToolRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn launch_fixture(
    label: &str,
    shebang: &str,
) -> (
    super::support_base::GitFixture,
    TrustedToolRoot,
    bridge::TrustedWorktreeGit,
    BTreeMap<String, OsString>,
    PathBuf,
    PathBuf,
) {
    let (fixture, state, _snapshot, _closeout) = implementation_proof_fixture(label);
    let binding = bridge::trusted_worktree_git(&state).expect("trusted worktree");
    let root = PathBuf::from(std::env::var_os("HOME").expect("HOME"))
        .join(".local/share/autospec-launch-tests")
        .join(format!(
            "{label}-{}-{}",
            std::process::id(),
            TRUSTED_TOOL_NONCE.fetch_add(1, Ordering::Relaxed)
        ));
    let bin = root.join("trusted-tools");
    fs::create_dir_all(&bin).expect("trusted tool directory");
    let node = bin.join("node");
    write_executable(&node, "#!/bin/sh\nexit 0\n");
    let codex = bin.join("codex");
    write_executable(&codex, &format!("{shebang}\nprocess.exit(0);\n"));
    let environment = BTreeMap::from([
        ("PATH".to_string(), bin.as_os_str().to_os_string()),
        ("HOME".to_string(), fixture.root.as_os_str().to_os_string()),
    ]);
    (
        fixture,
        TrustedToolRoot(root),
        binding,
        environment,
        node,
        codex,
    )
}

#[test]
fn trusted_codex_launch_accepts_exact_env_node_shebang() {
    let (_fixture, _tools, binding, environment, node, codex) =
        launch_fixture("trusted-codex-env-node", "#!/usr/bin/env node");
    let script = codex.with_file_name("codex.js");
    fs::rename(&codex, &script).unwrap();
    symlink(&script, &codex).unwrap();

    let launch = bridge::trusted_codex_launch_from(&binding, &environment).expect("node launch");

    assert_eq!(launch.program(), fs::canonicalize(node).unwrap());
    assert_eq!(launch.prefix_args(), &[fs::canonicalize(script).unwrap()]);
    launch
        .revalidate()
        .expect("unchanged node launch identities");
}

#[test]
fn trusted_codex_launch_accepts_exact_absolute_node_shebang() {
    let (fixture, _tools, binding, environment, node, codex) =
        launch_fixture("trusted-codex-absolute-node", "#!/usr/bin/env node");
    fs::write(&codex, format!("#!{}\nprocess.exit(0);\n", node.display())).unwrap();
    fs::set_permissions(&codex, fs::Permissions::from_mode(0o755)).unwrap();

    let launch = bridge::trusted_codex_launch_from(&binding, &environment).expect("node launch");

    assert_eq!(launch.program(), fs::canonicalize(node).unwrap());
    assert_eq!(launch.prefix_args(), &[fs::canonicalize(codex).unwrap()]);
    drop(fixture);
}

#[test]
fn trusted_codex_launch_rejects_shebang_options_assignments_and_other_interpreters() {
    for (index, shebang) in [
        "#!/usr/bin/env node --trace-warnings",
        "#!/usr/bin/env -S node",
        "#!/usr/bin/env NODE_ENV=test node",
        "#!/bin/sh",
        "#!node",
    ]
    .into_iter()
    .enumerate()
    {
        let (_fixture, _tools, binding, environment, _node, _codex) =
            launch_fixture(&format!("trusted-codex-bad-shebang-{index}"), shebang);
        let error = bridge::trusted_codex_launch_from(&binding, &environment)
            .expect_err("non-exact node shebang must fail closed");
        assert!(error.contains("shebang"), "unexpected error: {error}");
    }
}

#[test]
fn trusted_codex_launch_rejects_worktree_temporary_and_weak_files() {
    let (_fixture, _tools, binding, mut environment, _node, codex) =
        launch_fixture("trusted-codex-unsafe-files", "#!/usr/bin/env node");
    fs::set_permissions(&codex, fs::Permissions::from_mode(0o777)).unwrap();
    assert!(bridge::trusted_codex_launch_from(&binding, &environment)
        .expect_err("writable script")
        .contains("group/world writable"));

    fs::set_permissions(&codex, fs::Permissions::from_mode(0o755)).unwrap();
    fs::hard_link(&codex, codex.parent().unwrap().join("codex-hardlink")).unwrap();
    assert!(bridge::trusted_codex_launch_from(&binding, &environment)
        .expect_err("multiply-linked script")
        .contains("single link"));

    let worktree_codex = binding.worktree.join("codex");
    fs::copy(Path::new("/usr/bin/true"), &worktree_codex).unwrap();
    fs::set_permissions(&worktree_codex, fs::Permissions::from_mode(0o755)).unwrap();
    environment.insert("PATH".into(), binding.worktree.as_os_str().to_os_string());
    assert!(bridge::trusted_codex_launch_from(&binding, &environment)
        .expect_err("worktree executable")
        .contains("worktree"));
}

#[test]
fn trusted_executable_owner_policy_accepts_only_root_or_effective_user() {
    assert!(bridge::trusted_executable_owner_allowed(0, 501));
    assert!(bridge::trusted_executable_owner_allowed(501, 501));
    assert!(!bridge::trusted_executable_owner_allowed(502, 501));
}

#[test]
fn trusted_codex_launch_profile_and_shell_words_are_exact() {
    let (_fixture, tools, binding, mut environment, node, codex) =
        launch_fixture("trusted-codex-'quoted", "#!/usr/bin/env node");
    let quoted_bin = tools.0.join("tools'quoted");
    fs::rename(node.parent().unwrap(), &quoted_bin).unwrap();
    environment.insert("PATH".into(), quoted_bin.as_os_str().to_os_string());
    let launch = bridge::trusted_codex_launch_from(&binding, &environment).expect("node launch");
    let profile = bridge::contained_hook_profile_paths(&launch).expect("profile entries");

    assert_eq!(profile.len(), 2);
    assert!(profile.iter().any(|entry| entry.contains("node")));
    assert!(profile.iter().any(|entry| entry.contains("codex")));
    let words = launch.shell_words();
    assert!(
        words.contains("'\"'\"'"),
        "shell words were not quoted: {words}"
    );
    assert!(!words.contains(codex.to_string_lossy().as_ref()));
}

#[test]
fn trusted_codex_launch_revalidation_detects_identity_replacement() {
    let (_fixture, _tools, binding, environment, _node, codex) =
        launch_fixture("trusted-codex-identity-replacement", "#!/usr/bin/env node");
    let launch = bridge::trusted_codex_launch_from(&binding, &environment).expect("node launch");
    let replacement = codex.parent().unwrap().join("replacement-codex");
    write_executable(&replacement, "#!/usr/bin/env node\nprocess.exit(1);\n");
    fs::rename(replacement, &codex).unwrap();

    assert!(launch
        .revalidate()
        .expect_err("replaced script identity")
        .contains("identity changed"));

    let launch = bridge::trusted_codex_launch_from(&binding, &environment).expect("second launch");
    let node = codex.parent().unwrap().join("node");
    let replacement = codex.parent().unwrap().join("replacement-node");
    write_executable(&replacement, "#!/bin/sh\nexit 0\n");
    fs::rename(replacement, node).unwrap();
    assert!(launch
        .revalidate()
        .expect_err("replaced node identity")
        .contains("identity changed"));
}

#[test]
fn trusted_codex_launch_revalidation_detects_same_inode_same_length_script_overwrite() {
    let (_fixture, _tools, binding, environment, _node, codex) =
        launch_fixture("trusted-codex-content-script", "#!/usr/bin/env node");
    let launch = bridge::trusted_codex_launch_from(&binding, &environment).expect("node launch");
    let before = fs::metadata(&codex).expect("script metadata before overwrite");
    fs::write(&codex, "#!/usr/bin/env node\nprocess.exit(1);\n").expect("overwrite script");
    let after = fs::metadata(&codex).expect("script metadata after overwrite");
    assert_eq!(
        (after.dev(), after.ino(), after.len()),
        (before.dev(), before.ino(), before.len())
    );

    assert!(launch
        .revalidate()
        .expect_err("same-identity script content replacement")
        .contains("content changed"));
}

#[test]
fn trusted_codex_launch_revalidation_detects_same_inode_same_length_node_overwrite() {
    let (_fixture, _tools, binding, environment, node, _codex) =
        launch_fixture("trusted-codex-content-node", "#!/usr/bin/env node");
    let launch = bridge::trusted_codex_launch_from(&binding, &environment).expect("node launch");
    let before = fs::metadata(&node).expect("node metadata before overwrite");
    fs::write(&node, "#!/bin/sh\nexit 1\n").expect("overwrite node");
    let after = fs::metadata(&node).expect("node metadata after overwrite");
    assert_eq!(
        (after.dev(), after.ino(), after.len()),
        (before.dev(), before.ino(), before.len())
    );

    assert!(launch
        .revalidate()
        .expect_err("same-identity node content replacement")
        .contains("content changed"));
}

#[test]
fn trusted_codex_launch_preserves_native_binary_argv() {
    let (fixture, _tools, binding, mut environment, _node, codex) =
        launch_fixture("trusted-codex-native", "#!/usr/bin/env node");
    fs::remove_file(&codex).unwrap();
    fs::copy(Path::new("/usr/bin/true"), &codex).unwrap();
    fs::set_permissions(&codex, fs::Permissions::from_mode(0o755)).unwrap();
    environment.insert(
        "PATH".into(),
        codex.parent().unwrap().as_os_str().to_os_string(),
    );

    let launch = bridge::trusted_codex_launch_from(&binding, &environment).expect("native launch");

    assert_eq!(launch.program(), fs::canonicalize(codex).unwrap());
    assert!(launch.prefix_args().is_empty());
    drop(fixture);
}
