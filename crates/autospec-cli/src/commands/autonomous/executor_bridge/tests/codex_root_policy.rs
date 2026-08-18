// executor_bridge tests: Codex permission-profile root policy — 2 cases.

use super::support_base::test_root;
use crate::commands::autonomous::executor_bridge as bridge;
use std::ffi::OsString;
use std::fs;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

fn permission_filesystem_argument(arguments: &[String]) -> &str {
    arguments
        .windows(2)
        .find_map(|pair| {
            (pair[0] == "-c"
                && pair[1].starts_with("permissions.autospec-network-executor.filesystem="))
            .then_some(pair[1].as_str())
        })
        .expect("permission-profile filesystem policy")
}

#[test]
fn autonomous_executor_bridge_codex_permission_profile_denies_lexical_and_canonical_roots() {
    let root = test_root("codex-permission-root-aliases");
    let canonical_home = root.join("canonical-home");
    let lexical_home = root.join("lexical-home");
    let custom_codex_home = root.join("missing-\"codex\\home\n");
    fs::create_dir_all(&canonical_home).expect("canonical HOME");
    symlink(&canonical_home, &lexical_home).expect("symlinked HOME");

    let arguments = bridge::CodexSandboxPolicy::NetworkPermissionProfile
        .permission_profile_args_for_roots(
            Path::new("/usr/local/bin/codex"),
            &lexical_home,
            Some(&custom_codex_home),
        )
        .expect("permission profile with validated roots");
    let filesystem = permission_filesystem_argument(&arguments);

    for denied in [
        lexical_home.join(".aws"),
        canonical_home.join(".aws"),
        lexical_home.join(".codex/auth.json"),
        canonical_home.join(".codex/auth.json"),
    ] {
        assert!(
            filesystem.contains(&format!("\"{}\"=\"deny\"", denied.display())),
            "missing absolute alias {}: {filesystem}",
            denied.display()
        );
    }
    let escaped_custom = custom_codex_home
        .join("auth.json")
        .display()
        .to_string()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n");
    assert!(
        filesystem.contains(&format!("\"{escaped_custom}\"=\"deny\"")),
        "missing TOML-escaped custom CODEX_HOME alias: {filesystem}"
    );
    assert_eq!(
        filesystem
            .matches(&format!(
                "\"{}\"=\"deny\"",
                canonical_home.join(".aws").display()
            ))
            .count(),
        1,
        "canonical aliases must be deduplicated: {filesystem}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn autonomous_executor_bridge_codex_permission_profile_rejects_unsafe_roots() {
    let policy = bridge::CodexSandboxPolicy::NetworkPermissionProfile;
    let codex = Path::new("/usr/local/bin/codex");
    for (home, expected) in [
        (PathBuf::from("relative-home"), "HOME must be absolute"),
        (
            PathBuf::from("/safe/../escaped-home"),
            "HOME must not contain parent-directory components",
        ),
        (
            PathBuf::from(OsString::from_vec(b"/tmp/non-utf8-\xff".to_vec())),
            "HOME must be UTF-8",
        ),
    ] {
        let error = policy
            .permission_profile_args_for_roots(codex, &home, None)
            .expect_err("unsafe HOME must fail closed");
        assert!(error.contains(expected), "unexpected error: {error}");
    }

    let root = test_root("codex-permission-root-symlink-loop");
    let looped_home = root.join("home-loop");
    fs::create_dir_all(&root).expect("symlink-loop parent");
    symlink(&looped_home, &looped_home).expect("HOME symlink loop");
    let error = policy
        .permission_profile_args_for_roots(codex, &looped_home, None)
        .expect_err("unexpected canonicalization errors must fail closed");
    assert!(
        error.contains("canonicalize HOME"),
        "unexpected error: {error}"
    );
    let _ = fs::remove_dir_all(root);
}
