#[cfg(windows)]
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ResolvedDirectExecutable {
    pub(super) program: PathBuf,
    pub(super) argv_zero: String,
    #[cfg(windows)]
    pub(super) command_script: Option<PathBuf>,
}

impl ResolvedDirectExecutable {
    pub(super) fn invocation_argv(&self, declared: &[String]) -> Result<Vec<String>, String> {
        #[cfg(windows)]
        if let Some(script) = &self.command_script {
            return windows_command_script_argv(&self.program, script, &declared[1..]);
        }
        let mut argv = declared.to_vec();
        argv[0] = self.argv_zero.clone();
        Ok(argv)
    }
}

pub(super) fn resolve_direct_executable(
    worktree: &Path,
    executable: &str,
) -> Result<ResolvedDirectExecutable, String> {
    if executable.is_empty() || executable.starts_with('-') {
        return Err("executor direct command executable is invalid".to_string());
    }
    let path = Path::new(executable);
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else if path.components().count() > 1 {
        let candidate = worktree.join(path);
        if !candidate.starts_with(worktree) {
            return Err("executor direct command executable escapes the worktree".to_string());
        }
        candidate
    } else {
        direct_path_candidates(path)
            .ok_or_else(|| format!("executor direct command executable is missing: {executable}"))?
    };
    #[cfg(windows)]
    let candidate = windows_candidate_with_pathext(candidate);
    let canonical = fs::canonicalize(&candidate).map_err(|error| {
        format!(
            "canonicalize direct command executable {}: {error}",
            candidate.display()
        )
    })?;
    if !canonical.is_file() {
        return Err(format!(
            "executor direct command executable is not a regular file: {}",
            canonical.display()
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if fs::metadata(&canonical)
            .map_err(|error| format!("inspect direct command executable: {error}"))?
            .permissions()
            .mode()
            & 0o111
            == 0
        {
            return Err(format!(
                "executor direct command executable is not executable: {}",
                canonical.display()
            ));
        }
    }
    let argv_zero = candidate
        .into_os_string()
        .into_string()
        .map_err(|_| "executor direct command proxy path is not UTF-8".to_string())?;
    #[cfg(windows)]
    if matches!(
        canonical.extension().and_then(OsStr::to_str),
        Some(extension) if extension.eq_ignore_ascii_case("cmd") || extension.eq_ignore_ascii_case("bat")
    ) {
        let command_processor = windows_command_processor()?;
        return Ok(ResolvedDirectExecutable {
            argv_zero: command_processor.display().to_string(),
            program: command_processor,
            command_script: Some(canonical),
        });
    }
    Ok(ResolvedDirectExecutable {
        program: canonical,
        argv_zero,
        #[cfg(windows)]
        command_script: None,
    })
}

fn direct_path_candidates(path: &Path) -> Option<PathBuf> {
    let search_path = std::env::var_os("PATH").unwrap_or_default();
    let directories =
        std::env::split_paths(&search_path).filter(|directory| directory.is_absolute());
    #[cfg(not(windows))]
    {
        directories
            .map(|directory| directory.join(path))
            .find(|candidate| candidate.is_file())
    }
    #[cfg(windows)]
    {
        let extensions = windows_pathext(path);
        for directory in directories {
            for extension in &extensions {
                let candidate = directory.join(format!("{}{}", path.display(), extension));
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
        None
    }
}

#[cfg(windows)]
fn windows_candidate_with_pathext(candidate: PathBuf) -> PathBuf {
    if candidate.is_file() || candidate.extension().is_some() {
        return candidate;
    }
    windows_pathext(&candidate)
        .into_iter()
        .map(|extension| PathBuf::from(format!("{}{}", candidate.display(), extension)))
        .find(|path| path.is_file())
        .unwrap_or(candidate)
}

#[cfg(windows)]
fn windows_pathext(path: &Path) -> Vec<String> {
    if path.extension().is_some() {
        return vec![String::new()];
    }
    std::env::var("PATHEXT")
        .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string())
        .split(';')
        .filter_map(|extension| {
            let extension = extension.trim();
            (!extension.is_empty()
                && extension.starts_with('.')
                && extension[1..]
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric()))
            .then(|| extension.to_string())
        })
        .collect()
}

#[cfg(windows)]
fn windows_command_processor() -> Result<PathBuf, String> {
    let system_root = std::env::var_os("SystemRoot")
        .ok_or_else(|| "executor cannot resolve cmd.exe without SystemRoot".to_string())?;
    let candidate = PathBuf::from(system_root).join("System32").join("cmd.exe");
    let canonical = fs::canonicalize(&candidate)
        .map_err(|error| format!("canonicalize Windows command processor: {error}"))?;
    if !canonical.is_file() {
        return Err(format!(
            "Windows command processor is not a regular file: {}",
            canonical.display()
        ));
    }
    Ok(canonical)
}

#[cfg(windows)]
fn windows_command_script_argv(
    command_processor: &Path,
    script: &Path,
    arguments: &[String],
) -> Result<Vec<String>, String> {
    let script = script
        .to_str()
        .ok_or_else(|| "executor command wrapper path is not UTF-8".to_string())?;
    if script
        .chars()
        .any(|character| matches!(character, '\r' | '\n' | '\0' | '%' | '!' | '^'))
    {
        return Err(
            "executor command wrapper path contains unsafe cmd.exe metacharacters".to_string(),
        );
    }
    let mut tokens = vec![format!("\"{script}\"")];
    for argument in arguments {
        if argument.chars().any(|character| {
            matches!(
                character,
                '\r' | '\n' | '\0' | '"' | '&' | '|' | '<' | '>' | '^' | '%' | '!' | '(' | ')'
            )
        }) {
            return Err(
                "executor command wrapper argument contains unsafe cmd.exe metacharacters"
                    .to_string(),
            );
        }
        if argument.is_empty() || argument.chars().any(char::is_whitespace) {
            tokens.push(format!("\"{argument}\""));
        } else {
            tokens.push(argument.clone());
        }
    }
    let command = format!("\"{}\"", tokens.join(" "));
    Ok(vec![
        command_processor.display().to_string(),
        "/d".to_string(),
        "/s".to_string(),
        "/c".to_string(),
        command,
    ])
}
