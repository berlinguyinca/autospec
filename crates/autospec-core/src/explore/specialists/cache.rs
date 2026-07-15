use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::roster_json::parse_roster_json;
use super::SpecialistRoster;

pub(crate) fn load(repo_dir: &Path) -> io::Result<Option<SpecialistRoster>> {
    let path = cache_path(repo_dir);
    if !path.is_file() {
        return Ok(None);
    }
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(_) => return Ok(None),
    };
    Ok(parse_roster_json(&contents).ok())
}

pub(crate) fn store(repo_dir: &Path, roster: &SpecialistRoster) -> io::Result<()> {
    let path = cache_path(repo_dir);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, roster.to_json_pretty())
}

fn cache_path(repo_dir: &Path) -> PathBuf {
    repo_dir.join(".autospec/explore-specialists.json")
}
