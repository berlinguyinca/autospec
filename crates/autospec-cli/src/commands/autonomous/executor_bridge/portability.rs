use super::*;

#[cfg(any(target_os = "macos", target_os = "freebsd", windows))]
mod direct_attempt;
mod portable_runtime;

#[cfg(any(target_os = "macos", target_os = "freebsd", windows))]
pub(super) use direct_attempt::*;
pub(super) use portable_runtime::*;

#[cfg(test)]
mod autonomous_runtime_support_tests {
    use crate::commands::autonomous;
    use std::ffi::OsString;
    use std::path::Path;
    use std::sync::Mutex;

    static ENVIRONMENT: Mutex<()> = Mutex::new(());

    struct Environment {
        previous: Vec<(&'static str, Option<OsString>)>,
    }

    impl Environment {
        fn set(values: &[(&'static str, &Path)]) -> Self {
            let previous = values
                .iter()
                .map(|(key, value)| {
                    let previous = std::env::var_os(key);
                    // SAFETY: these command-artifact tests serialize all environment mutation.
                    unsafe { std::env::set_var(key, value) };
                    (*key, previous)
                })
                .collect();
            Self { previous }
        }
    }

    impl Drop for Environment {
        fn drop(&mut self) {
            for (key, value) in self.previous.drain(..).rev() {
                // SAFETY: these command-artifact tests serialize all environment mutation.
                unsafe {
                    match value {
                        Some(value) => std::env::set_var(key, value),
                        None => std::env::remove_var(key),
                    }
                }
            }
        }
    }

    fn launch_arguments(command: &str, repo_dir: &Path, dry_run: bool) -> Vec<String> {
        let mut arguments = vec![
            command.to_string(),
            "--repo".to_string(),
            "owner/repo".to_string(),
            "--repo-dir".to_string(),
            repo_dir.display().to_string(),
        ];
        if dry_run {
            arguments.push("--dry-run".to_string());
        }
        arguments
    }

    fn artifact_snapshot(root: &Path) -> Vec<String> {
        fn visit(root: &Path, path: &Path, entries: &mut Vec<String>) {
            let mut children = std::fs::read_dir(path)
                .expect("read artifact snapshot directory")
                .map(|entry| entry.expect("read artifact snapshot entry"))
                .collect::<Vec<_>>();
            children.sort_by_key(|entry| entry.file_name());
            for child in children {
                let child_path = child.path();
                let relative = child_path
                    .strip_prefix(root)
                    .expect("relative artifact path");
                let metadata = std::fs::symlink_metadata(&child_path)
                    .expect("inspect artifact snapshot entry");
                let entry = if metadata.is_dir() {
                    format!("dir:{}", relative.display())
                } else if metadata.is_file() {
                    format!(
                        "file:{}:{:?}",
                        relative.display(),
                        std::fs::read(&child_path).expect("read artifact snapshot file")
                    )
                } else {
                    format!("other:{}", relative.display())
                };
                entries.push(entry);
                if metadata.is_dir() {
                    visit(root, &child_path, entries);
                }
            }
        }

        let mut entries = Vec::new();
        visit(root, root, &mut entries);
        entries
    }

    #[cfg(any(
        target_os = "linux",
        target_os = "macos",
        target_os = "freebsd",
        windows
    ))]
    #[test]
    fn supported_run_foreground_dry_run_is_an_exact_artifact_free_preview() {
        let _serial = ENVIRONMENT.lock().expect("lock environment");
        let root = std::env::temp_dir().join(format!(
            "autospec-native-foreground-dry-run-{}",
            std::process::id()
        ));
        let repo_dir = root.join("repo");
        std::fs::create_dir_all(&repo_dir).expect("create valid repository directory");
        std::fs::write(repo_dir.join("sentinel"), b"unchanged").expect("seed artifact snapshot");
        let operator = root.join("operator");
        let logs = root.join("logs");
        let claims = root.join("claims");
        let heartbeats = root.join("heartbeats");
        let _environment = Environment::set(&[
            ("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator),
            ("AUTOSPEC_AUTONOMOUS_LOG_DIR", &logs),
            ("AUTOSPEC_CLAIM_GIT_STATE_DIR", &claims),
            ("AUTOSPEC_HEARTBEAT_DIR", &heartbeats),
        ]);
        let baseline = artifact_snapshot(&root);

        autonomous::run(&launch_arguments("run-foreground", &repo_dir, true))
            .expect("native foreground dry-run must be a pure preview");

        assert_eq!(
            artifact_snapshot(&root),
            baseline,
            "run-foreground --dry-run"
        );
        std::fs::remove_dir_all(root).expect("remove native dry-run fixture");
    }

    #[cfg(not(any(
        target_os = "linux",
        target_os = "macos",
        target_os = "freebsd",
        windows
    )))]
    #[test]
    fn unsupported_platform_rejects_mutating_launches_before_artifact_creation() {
        let _serial = ENVIRONMENT.lock().expect("lock environment");
        let root = std::env::temp_dir().join(format!(
            "autospec-unsupported-runtime-{}",
            std::process::id()
        ));
        let repo_dir = root.join("repo");
        std::fs::create_dir_all(&repo_dir).expect("create valid repository directory");
        std::fs::write(repo_dir.join("sentinel"), b"unchanged").expect("seed artifact snapshot");
        let operator = root.join("operator");
        let logs = root.join("logs");
        let claims = root.join("claims");
        let heartbeats = root.join("heartbeats");
        let _environment = Environment::set(&[
            ("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator),
            ("AUTOSPEC_AUTONOMOUS_LOG_DIR", &logs),
            ("AUTOSPEC_CLAIM_GIT_STATE_DIR", &claims),
            ("AUTOSPEC_HEARTBEAT_DIR", &heartbeats),
        ]);
        let baseline = artifact_snapshot(&root);
        for command in ["start", "restart", "resume"] {
            let error = autonomous::run(&launch_arguments(command, &repo_dir, false))
                .expect_err("unsupported mutating launch must fail");
            assert!(error.message.contains("unsupported on this host"));
            assert_eq!(artifact_snapshot(&root), baseline, "{command}");
        }

        let mut foreground_start = launch_arguments("start", &repo_dir, false);
        foreground_start.push("--foreground".to_string());
        let error = autonomous::run(&foreground_start)
            .expect_err("unsupported foreground start must fail before artifact creation");
        assert!(error.message.contains("unsupported on this host"));
        assert_eq!(artifact_snapshot(&root), baseline, "start --foreground");

        let error = autonomous::run(&launch_arguments("run-foreground", &repo_dir, false))
            .expect_err("unsupported public foreground entry must fail before artifact creation");
        assert!(error.message.contains("unsupported on this host"));
        assert_eq!(artifact_snapshot(&root), baseline, "run-foreground");

        for command in ["start", "restart", "resume"] {
            autonomous::run(&launch_arguments(command, &repo_dir, true))
                .expect("unsupported dry-run preview remains available");
            assert_eq!(artifact_snapshot(&root), baseline, "{command} --dry-run");
        }
        autonomous::run(&launch_arguments("run-foreground", &repo_dir, true))
            .expect("unsupported foreground dry-run remains an artifact-free preview");
        assert_eq!(
            artifact_snapshot(&root),
            baseline,
            "run-foreground --dry-run"
        );
        std::fs::remove_dir_all(root).expect("remove unsupported-platform fixture");
    }
}

#[cfg(test)]
static PORTABLE_LIFECYCLE_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod supported_host_tests;
#[cfg(all(test, windows))]
mod windows_tests;
