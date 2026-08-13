use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::time::SystemTime;

use super::optional_json_string;

pub(super) struct ToolchainFreshness {
    installed_version: Option<String>,
    remote_version: Option<String>,
    installed_age_secs: Option<u64>,
    last_update_failure_path: Option<String>,
}

impl ToolchainFreshness {
    pub(super) fn load() -> Self {
        let root = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".autospec");
        let installed_path = root.join("installed-version");
        let failure_path = root.join("last-update-failure.json");
        Self {
            installed_version: read_trimmed_file(&installed_path),
            remote_version: read_trimmed_file(&root.join("remote-version")),
            installed_age_secs: fs::metadata(&installed_path)
                .and_then(|metadata| metadata.modified())
                .ok()
                .and_then(|modified| SystemTime::now().duration_since(modified).ok())
                .map(|age| age.as_secs()),
            last_update_failure_path: failure_path
                .is_file()
                .then(|| failure_path.display().to_string()),
        }
    }

    pub(super) fn to_json(&self) -> String {
        format!(
            "{{\"installed_version\":{},\"remote_version\":{},\"installed_age_secs\":{},\"last_update_failed\":{},\"last_update_failure_path\":{}}}",
            optional_json_string(self.installed_version.as_deref()),
            optional_json_string(self.remote_version.as_deref()),
            self.installed_age_secs
                .map(|age| age.to_string())
                .unwrap_or_else(|| "null".to_string()),
            self.last_update_failure_path.is_some(),
            optional_json_string(self.last_update_failure_path.as_deref()),
        )
    }

    pub(super) fn print_human(&self) {
        println!(
            "toolchain installed={} remote={} age_secs={} last_update_failed={}",
            self.installed_version.as_deref().unwrap_or("unknown"),
            self.remote_version.as_deref().unwrap_or("unknown"),
            self.installed_age_secs
                .map(|age| age.to_string())
                .unwrap_or_else(|| "unknown".to_string()),
            self.last_update_failure_path.is_some()
        );
    }

    pub(super) fn warn_if_failed(&self) {
        if let Some(path) = &self.last_update_failure_path {
            eprintln!(
                "WARN: autospec self-update failed; continuing on installed version; record: {path}"
            );
        }
    }
}

fn read_trimmed_file(path: &Path) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}
