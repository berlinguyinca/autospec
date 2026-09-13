//! Gate toolchain preconditions (issue #4589).
//!
//! A pass that judges work must first verify that the tools its gate runs
//! exist. A missing tool is an environment failure, not a verdict: reported
//! as one `HELD` record per patch it is indistinguishable in the durable
//! ledger from patches that genuinely failed their gate, and a later pass
//! could treat never-gated work as settled. One named `FATAL` before any
//! judging is the only correct output for a broken host — and the pass must
//! check it itself, because a scheduled wrapper inherits none of a login
//! shell's environment.

use std::path::{Path, PathBuf};

/// The tools the conversion gate requires, in the order the `FATAL` names
/// them: the gate's own stages run `cargo`, the pass branches and applies
/// with `git`, and it checks liveness and opens PRs with `gh`.
pub const GATE_TOOLS: &[&str] = &["cargo", "git", "gh"];

/// Which of `required` are absent from a `PATH`-formatted string, in the
/// order `required` names them.
///
/// A tool is present when any `PATH` directory holds a regular executable
/// file with its name. Both `:` and `;` separators are accepted, and an
/// empty `PATH` (or an unset one) yields every tool missing: a gate that
/// cannot find its tools must refuse, not guess where they might be.
pub fn missing_tools(required: &[&str], path_value: &str) -> Vec<String> {
    let dirs: Vec<PathBuf> = path_value
        .split([':', ';'])
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .collect();
    required
        .iter()
        .filter(|tool| !dirs.iter().any(|dir| is_executable_file(&dir.join(tool))))
        .map(|tool| tool.to_string())
        .collect()
}

fn is_executable_file(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        matches!(
            std::fs::metadata(path),
            Ok(meta) if meta.is_file() && meta.permissions().mode() & 0o111 != 0
        )
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_with_every_tool_present_reports_nothing_missing() {
        let dir = std::env::temp_dir().join(format!("autospec-preflight-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for tool in GATE_TOOLS {
            let path = dir.join(tool);
            std::fs::write(&path, "#!/bin/sh\n").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = std::fs::metadata(&path).unwrap().permissions();
                perms.set_mode(0o755);
                std::fs::set_permissions(&path, perms).unwrap();
            }
        }
        assert_eq!(
            missing_tools(GATE_TOOLS, &dir.to_string_lossy()),
            Vec::<String>::new()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_tool_is_named_in_the_order_required() {
        let dir =
            std::env::temp_dir().join(format!("autospec-preflight-ordered-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("git"), "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(dir.join("git")).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(dir.join("git"), perms).unwrap();
        }
        // Only `git` exists: the other two come back in required order.
        assert_eq!(
            missing_tools(&["cargo", "git", "gh"], &dir.to_string_lossy()),
            vec!["cargo", "gh"]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_empty_path_reports_every_tool_missing() {
        assert_eq!(missing_tools(GATE_TOOLS, ""), vec!["cargo", "git", "gh"]);
    }

    #[test]
    fn semicolon_separators_are_accepted_like_colons() {
        let dir =
            std::env::temp_dir().join(format!("autospec-preflight-semi-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("cargo"), "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(dir.join("cargo")).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(dir.join("cargo"), perms).unwrap();
        }
        assert_eq!(
            missing_tools(&["cargo", "git"], &dir.to_string_lossy()),
            vec!["git"]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_non_executable_file_does_not_count_as_present() {
        let dir =
            std::env::temp_dir().join(format!("autospec-preflight-noexec-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("cargo"), "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(dir.join("cargo")).unwrap().permissions();
            perms.set_mode(0o644);
            std::fs::set_permissions(dir.join("cargo"), perms).unwrap();
            assert_eq!(
                missing_tools(&["cargo"], &dir.to_string_lossy()),
                vec!["cargo"]
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_directory_with_a_tools_name_does_not_count_as_present() {
        let dir =
            std::env::temp_dir().join(format!("autospec-preflight-isdir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        std::fs::create_dir_all(dir.join("bin").join("cargo")).unwrap();
        assert_eq!(
            missing_tools(&["cargo"], &dir.join("bin").to_string_lossy()),
            vec!["cargo"]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
