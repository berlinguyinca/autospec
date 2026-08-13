// executor_bridge tests: commit / rust — 12 cases.
//
// Split out of tests.rs; see the note in that file.

use crate::commands::autonomous::executor_bridge as bridge;
use super::support_base::{git, git_stdout};
use super::support_invocation::{commit_implementation, implementation_proof_fixture};
use std::collections::BTreeMap;
use std::fs;
#[cfg(unix)]
use std::net::{TcpListener, TcpStream};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(unix)]
#[test]
fn rust_commit_rejects_worktree_signing_program_without_executing_it() {
    let (fixture, state, _snapshot, _closeout) =
        implementation_proof_fixture("rust-commit-signing-program");
    fs::write(
        state.identity.worktree.join("implementation.txt"),
        "recoverable diff\n",
    )
    .expect("write sandboxed diff");
    let marker = fixture.root.join("signer-escaped-sandbox");
    let signer = state.identity.worktree.join("signer.sh");
    fs::write(
        &signer,
        format!(
            "#!/bin/sh\nprintf compromised > {}\nexit 1\n",
            marker.display()
        ),
    )
    .expect("write model-controlled signer");
    fs::set_permissions(&signer, fs::Permissions::from_mode(0o755))
        .expect("make model-controlled signer executable");
    git(&fixture.repo, &["config", "commit.gpgSign", "true"]);
    git(
        &fixture.repo,
        &[
            "config",
            "gpg.program",
            signer.to_str().expect("signer path"),
        ],
    );

    let error = bridge::commit_sandboxed_executor_diff(&state, "test: blocked signer escape", "")
        .expect_err("worktree signing programs must fail closed");

    assert!(error.contains("sign"), "{error}");
    assert!(!marker.exists(), "signing program escaped its sandbox");
}

#[cfg(unix)]
#[test]
fn rust_commit_rejects_ssh_default_key_command_without_executing_it() {
    let (fixture, state, _snapshot, _closeout) =
        implementation_proof_fixture("rust-commit-default-key-command");
    fs::write(
        state.identity.worktree.join("implementation.txt"),
        "recoverable diff\n",
    )
    .expect("write sandboxed diff");
    let marker = fixture.root.join("key-command-escaped-sandbox");
    let key_command = state.identity.worktree.join("key-command.sh");
    fs::write(
        &key_command,
        format!(
            "#!/bin/sh\nprintf compromised > {}\nprintf 'key::invalid\\n'\n",
            marker.display()
        ),
    )
    .expect("write model-controlled key command");
    fs::set_permissions(&key_command, fs::Permissions::from_mode(0o755))
        .expect("make model-controlled key command executable");
    git(&fixture.repo, &["config", "commit.gpgSign", "true"]);
    git(&fixture.repo, &["config", "gpg.format", "ssh"]);
    git(
        &fixture.repo,
        &[
            "config",
            "gpg.ssh.defaultKeyCommand",
            key_command.to_str().expect("key command path"),
        ],
    );

    let error =
        bridge::commit_sandboxed_executor_diff(&state, "test: blocked key command escape", "")
            .expect_err("SSH default key commands must fail closed");

    assert!(
        error.contains("sign") || error.contains("key command"),
        "{error}"
    );
    assert!(
        !marker.exists(),
        "SSH default key command escaped its sandbox"
    );
}

#[test]
fn rust_commit_rejects_unregistered_worktree_git_metadata() {
    let (_fixture, state, _snapshot, _closeout) =
        implementation_proof_fixture("rust-commit-unregistered-gitdir");
    fs::write(
        state.identity.worktree.join("implementation.txt"),
        "recoverable diff\n",
    )
    .expect("write sandboxed diff");
    let local_git = state.identity.worktree.join(".autospec/local-git");
    fs::create_dir_all(local_git.parent().expect("local git parent")).expect("local git parent");
    git(
        &state.identity.worktree,
        &[
            "init",
            "--bare",
            local_git.to_str().expect("local git path"),
        ],
    );
    fs::write(
        state.identity.worktree.join(".git"),
        format!("gitdir: {}\n", local_git.display()),
    )
    .expect("replace worktree git pointer");

    let error = bridge::commit_sandboxed_executor_diff(&state, "test: blocked metadata escape", "")
        .expect_err("unregistered Git metadata must fail closed");

    assert!(
        error.contains("gitdir") || error.contains("Git metadata"),
        "{error}"
    );
    assert!(state.identity.worktree.join("implementation.txt").exists());
}

#[test]
fn executor_commit_subject_requires_a_conventional_description() {
    assert_eq!(
        bridge::executor_commit_subject(42, "fix:"),
        "chore: implement autospec issue #42"
    );
    assert_eq!(
        bridge::executor_commit_subject(42, "fix:no-space"),
        "chore: implement autospec issue #42"
    );
    assert_eq!(
        bridge::executor_commit_subject(42, "fix(parser): reject invalid input"),
        "fix(parser): reject invalid input"
    );
}

#[cfg(unix)]
#[test]
fn trusted_git_inventory_accepts_external_validation_hook() {
    let (_fixture, state, _snapshot, _closeout) =
        implementation_proof_fixture("rust-commit-trusted-hook-inventory");
    let common_dir = PathBuf::from(git_stdout(
        &state.identity.worktree,
        &["rev-parse", "--git-common-dir"],
    ));
    let common_dir = if common_dir.is_absolute() {
        common_dir
    } else {
        state.identity.worktree.join(common_dir)
    };
    let hook = common_dir.join("hooks/pre-commit");
    fs::write(&hook, "#!/bin/sh\nexit 0\n").expect("write validation hook");
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755))
        .expect("make validation hook executable");

    let binding = bridge::trusted_worktree_git(&state)
        .expect("trusted common-directory validation hook must be inventoried");

    assert_eq!(binding.active_hooks, vec![hook.canonicalize().unwrap()]);
}

#[cfg(unix)]
#[test]
fn trusted_git_inventory_rejects_post_hook() {
    let (_fixture, state, _snapshot, _closeout) =
        implementation_proof_fixture("rust-commit-post-hook");
    let common_dir = PathBuf::from(git_stdout(
        &state.identity.worktree,
        &["rev-parse", "--git-common-dir"],
    ));
    let common_dir = if common_dir.is_absolute() {
        common_dir
    } else {
        state.identity.worktree.join(common_dir)
    };
    let hook = common_dir.join("hooks/post-index-change");
    fs::write(&hook, "#!/bin/sh\nexit 0\n").expect("write post hook");
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755))
        .expect("make post hook executable");

    let error = bridge::trusted_worktree_git(&state).expect_err("post hook must fail closed");

    assert!(error.contains("unsupported active Git hook"), "{error}");
}

#[cfg(unix)]
#[test]
fn contained_hook_rejects_codex_selected_from_writable_worktree() {
    let environment = std::env::vars_os()
        .filter_map(|(key, value)| key.into_string().ok().map(|key| (key, value)))
        .collect::<BTreeMap<_, _>>();
    let codex = bridge::safe_executable(Path::new("codex"), &environment)
        .expect("installed Codex executable");
    let binding = bridge::TrustedWorktreeGit {
        active_hooks: Vec::new(),
        common_dir: PathBuf::new(),
        git_dir: PathBuf::new(),
        hooks_dir: PathBuf::new(),
        worktree: codex.parent().expect("Codex parent").to_path_buf(),
    };

    let error = bridge::trusted_codex_executable_from(&binding, &environment)
        .expect_err("worktree-selected Codex must fail closed");

    assert!(error.contains("writable by the implementer"), "{error}");
}

#[cfg(unix)]
#[test]
fn contained_hook_rejects_linter_symlink_into_worktree() {
    let (fixture, state, _snapshot, _closeout) =
        implementation_proof_fixture("contained-hook-linter-symlink");
    let scripts = fixture.root.join("scripts");
    fs::create_dir(&scripts).expect("create scripts directory");
    let payload = state.identity.worktree.join("model-linter.sh");
    fs::write(&payload, "#!/bin/sh\nexit 0\n").expect("write model linter");
    std::os::unix::fs::symlink(&payload, scripts.join("lint-implementation.sh"))
        .expect("link model linter");
    let binding = bridge::trusted_worktree_git(&state).expect("trusted worktree");
    let environment = BTreeMap::from([
        ("HOME".to_string(), fixture.root.as_os_str().to_os_string()),
        (
            "AUTOSPEC_SCRIPTS_DIR".to_string(),
            scripts.as_os_str().to_os_string(),
        ),
    ]);

    let error = bridge::trusted_linter_from(&binding, &environment)
        .expect_err("model-writable linter symlink must fail closed");

    assert!(error.contains("non-symlink"), "{error}");
}

#[cfg(unix)]
#[test]
fn rust_commit_runs_trusted_validation_hook_inside_containment() {
    let (fixture, state, _snapshot, _closeout) =
        implementation_proof_fixture("rust-commit-contained-hook");
    let home = fixture.root.join("contained-hook-home");
    fs::create_dir_all(home.join(".config/gh")).expect("create credential fixture");
    fs::write(
        home.join(".config/gh/hosts.yml"),
        "known-credential-sentinel\n",
    )
    .expect("write credential fixture");
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind local network fixture");
    let listener_address = listener.local_addr().expect("local listener address");
    let outside = TcpStream::connect(listener_address)
        .expect("local listener must be reachable outside containment");
    let (accepted, _) = listener.accept().expect("accept outside preflight");
    drop((outside, accepted));
    let mut environment = std::env::vars_os()
        .filter_map(|(key, value)| key.into_string().ok().map(|key| (key, value)))
        .collect::<BTreeMap<_, _>>();
    environment.insert("HOME".to_string(), home.into_os_string());
    environment.insert(
        "AUTOSPEC_SCRIPTS_DIR".to_string(),
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scripts")
            .into_os_string(),
    );
    let hook_context = bridge::TrustedHookContext {
        environment,
        autospec: fs::canonicalize(std::env::current_exe().expect("current test executable"))
            .expect("canonical test executable"),
    };
    let implementation = state.identity.worktree.join("implementation.txt");
    fs::write(&implementation, "contained hook proof\n").expect("write implementation");
    let common_dir = PathBuf::from(git_stdout(
        &state.identity.worktree,
        &["rev-parse", "--git-common-dir"],
    ));
    let common_dir = if common_dir.is_absolute() {
        common_dir
    } else {
        state.identity.worktree.join(common_dir)
    };
    let hook = common_dir.join("hooks/pre-commit");
    fs::write(
        &hook,
        format!(
            "#!/bin/sh\nset -eu\n[ \"$PWD\" = {} ]\n[ -r \"$AUTOSPEC_SCRIPTS_DIR/lint-implementation.sh\" ]\n[ -x \"$AUTOSPEC_BIN\" ]\ngrep -qx 'offline contained evidence' \"$AUTOSPEC_LINT_ISSUE_BODY_FILE\"\n[ ! -r \"$HOME/.config/gh/hosts.yml\" ]\n! /bin/bash -c 'exec 3<>/dev/tcp/127.0.0.1/{}'\ngit diff --cached --name-only | grep -qx implementation.txt\nchmod 600 implementation.txt\n",
            bridge::posix_shell_quote(state.identity.worktree.to_string_lossy().as_ref()),
            listener_address.port(),
        ),
    )
    .expect("write validation hook");
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755))
        .expect("make validation hook executable");

    assert!(bridge::commit_sandboxed_executor_diff_with_hook_context(
        &state,
        "test: contained hook",
        "offline contained evidence\n",
        &hook_context,
    )
    .expect("contained validation hook commit"));

    assert_eq!(
        fs::metadata(implementation)
            .expect("implementation metadata")
            .permissions()
            .mode()
            & 0o777,
        0o600,
        "the trusted validation hook did not execute"
    );
}

#[cfg(unix)]
#[test]
fn rust_commit_failure_preserves_sandboxed_executor_diff_without_remote_mutation() {
    let (fixture, state, _snapshot, _closeout) =
        implementation_proof_fixture("rust-commit-sandboxed-failure");
    let remote_before = git_stdout(
        &fixture.root,
        &[
            "--git-dir",
            fixture.root.join("remote.git").to_str().unwrap(),
            "show-ref",
        ],
    );
    fs::write(
        state.identity.worktree.join("implementation.txt"),
        "recoverable diff\n",
    )
    .expect("write sandboxed diff");
    let head_before = git_stdout(&state.identity.worktree, &["rev-parse", "HEAD"]);
    let common_dir = PathBuf::from(git_stdout(
        &state.identity.worktree,
        &["rev-parse", "--git-common-dir"],
    ));
    let common_dir = if common_dir.is_absolute() {
        common_dir
    } else {
        state.identity.worktree.join(common_dir)
    };
    let hook = common_dir.join("hooks/pre-commit");
    let escaped = fixture.root.join("hook-escaped-containment");
    let hook_body = format!(
        "#!/bin/sh\nset +e\nchmod 600 implementation.txt\nprintf 'hook mutation\\n' > implementation.txt\ngit add implementation.txt\ngit update-ref refs/heads/hook-escape HEAD\ngit config autospec.hook-escape true\nprintf compromised > {}\nprintf escaped > {}\nexit 1\n",
        bridge::posix_shell_quote(hook.to_string_lossy().as_ref()),
        bridge::posix_shell_quote(escaped.to_string_lossy().as_ref()),
    );
    fs::write(&hook, &hook_body).expect("write escaping hook");
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755))
        .expect("make rejecting hook executable");

    let error = bridge::commit_sandboxed_executor_diff(&state, "test: blocked commit", "")
        .expect_err("hook must block Rust-owned commit");

    assert!(error.contains("commit"), "{error}");
    assert!(!escaped.exists(), "validation hook escaped containment");
    assert_eq!(
        fs::metadata(state.identity.worktree.join("implementation.txt"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600,
        "hook execution receipt is missing"
    );
    assert_eq!(
        git_stdout(&state.identity.worktree, &["rev-parse", "HEAD"]),
        head_before
    );
    assert_eq!(
        git_stdout(&state.identity.worktree, &["show", ":implementation.txt"]),
        "recoverable diff",
        "contained hook rewrote the staged index"
    );
    assert_eq!(fs::read_to_string(&hook).unwrap(), hook_body);
    for args in [
        ["show-ref", "--verify", "refs/heads/hook-escape"],
        ["config", "--get", "autospec.hook-escape"],
    ] {
        assert!(!Command::new("git")
            .current_dir(&state.identity.worktree)
            .args(args)
            .status()
            .unwrap()
            .success());
    }
    assert!(git_stdout(
        &state.identity.worktree,
        &["status", "--porcelain=v1", "--untracked-files=all"]
    )
    .contains("implementation.txt"));
    assert_eq!(
        git_stdout(
            &fixture.root,
            &[
                "--git-dir",
                fixture.root.join("remote.git").to_str().unwrap(),
                "show-ref",
            ]
        ),
        remote_before
    );
}

#[test]
fn autonomous_executor_bridge_rejects_dirty_implementation_before_remote_mutation() {
    // Break caught: an uncommitted harness edit escaping the exact pushed commit.
    let (fixture, mut state, snapshot, closeout) = implementation_proof_fixture("proof-dirty");
    commit_implementation(&state);
    fs::write(state.identity.worktree.join("dirty.txt"), "dirty\n").expect("dirty path");
    let remote_before = git_stdout(
        &fixture.root,
        &[
            "--git-dir",
            fixture.root.join("remote.git").to_str().unwrap(),
            "show-ref",
        ],
    );

    let error = bridge::prove_implementation(
        &fixture.root.join("state/invocation.json"),
        &mut state,
        &snapshot,
        &closeout,
    )
    .expect_err("dirty implementation must fail closed");

    assert!(error.contains("clean"), "{error}");
    assert_eq!(
        git_stdout(
            &fixture.root,
            &[
                "--git-dir",
                fixture.root.join("remote.git").to_str().unwrap(),
                "show-ref"
            ]
        ),
        remote_before
    );
}

#[test]
fn autonomous_executor_bridge_rejects_foreign_branch_before_remote_mutation() {
    // Break caught: proof accepting a clean commit from a branch outside the claim identity.
    let (fixture, mut state, snapshot, closeout) =
        implementation_proof_fixture("proof-foreign-branch");
    git(
        &state.identity.worktree,
        &["checkout", "-b", "foreign/issue-42"],
    );
    commit_implementation(&state);
    let remote_before = git_stdout(
        &fixture.root,
        &[
            "--git-dir",
            fixture.root.join("remote.git").to_str().unwrap(),
            "show-ref",
        ],
    );

    let error = bridge::prove_implementation(
        &fixture.root.join("state/invocation.json"),
        &mut state,
        &snapshot,
        &closeout,
    )
    .expect_err("foreign branch must fail closed");

    assert!(error.contains("branch"), "{error}");
    assert_eq!(
        git_stdout(
            &fixture.root,
            &[
                "--git-dir",
                fixture.root.join("remote.git").to_str().unwrap(),
                "show-ref"
            ]
        ),
        remote_before
    );
}
