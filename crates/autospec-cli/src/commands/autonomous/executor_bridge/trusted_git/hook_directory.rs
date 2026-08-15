use super::*;

pub(super) fn resolve_hooks_directory(path: &Path) -> Result<(PathBuf, bool), String> {
    match fs::canonicalize(path) {
        Ok(path) => Ok((path, true)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let name = path
                .file_name()
                .ok_or_else(|| "executor Git hook directory has no final component".to_string())?;
            let parent = path
                .parent()
                .ok_or_else(|| "executor Git hook directory has no parent".to_string())?;
            let parent = fs::canonicalize(parent).map_err(|error| {
                format!("canonicalize executor Git hook directory parent: {error}")
            })?;
            let resolved = parent.join(name);
            match fs::canonicalize(&resolved) {
                Ok(path) => Ok((path, true)),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok((resolved, false)),
                Err(error) => Err(format!(
                    "canonicalize executor Git hook directory after race: {error}"
                )),
            }
        }
        Err(error) => Err(format!("canonicalize executor Git hook directory: {error}")),
    }
}
