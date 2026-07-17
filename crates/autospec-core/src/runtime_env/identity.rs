use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use getrandom::fill;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::RuntimeEnvError;

const GENERATION_FILE: &str = "autospec-runtime-generation";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EnvironmentIdentity {
    pub canonical_repo: PathBuf,
    pub mode: String,
    pub generation: Option<String>,
    pub environment_id: String,
    pub owner_key: String,
}

impl EnvironmentIdentity {
    pub fn resolve(
        repo: &Path,
        mode: &str,
        generation: Option<&str>,
    ) -> Result<Self, RuntimeEnvError> {
        let canonical_repo = resolve_git_path(repo, "--show-toplevel")?
            .map_or_else(|| std::fs::canonicalize(repo), Ok)
            .map_err(|error| {
                RuntimeEnvError::new(format!("repo does not exist: {} ({error})", repo.display()))
            })?;
        let generation = generation.map(str::to_owned);
        let owner_key = identity_hash(&canonical_repo, mode, generation.as_deref())?;
        let name = canonical_repo
            .file_name()
            .and_then(|value| value.to_str())
            .map(slugify)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "agent_env".to_string());
        let environment_id = format!("{name}-{}", &owner_key[..16]);
        Ok(Self {
            canonical_repo,
            mode: mode.to_string(),
            generation,
            environment_id,
            owner_key,
        })
    }
}

pub fn load_generation_token(repo: &Path) -> Result<Option<String>, RuntimeEnvError> {
    let Some(git_dir) = resolve_git_path(repo, "--git-dir")? else {
        return Ok(None);
    };
    let path = git_dir.join(GENERATION_FILE);
    match std::fs::read_to_string(&path) {
        Ok(token) => return Ok(Some(validate_token(token, &path)?)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(RuntimeEnvError::new(format!(
                "could not read runtime generation {}: {error}",
                path.display()
            )));
        }
    }

    let mut random = [0_u8; 16];
    fill(&mut random).map_err(|error| {
        RuntimeEnvError::new(format!(
            "could not generate runtime generation token: {error}"
        ))
    })?;
    let token = random
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    match options.open(&path) {
        Ok(mut file) => {
            file.write_all(token.as_bytes())
                .and_then(|()| file.sync_all())
                .map_err(|error| {
                    RuntimeEnvError::new(format!(
                        "could not write runtime generation {}: {error}",
                        path.display()
                    ))
                })?;
            sync_directory(&git_dir)?;
            Ok(Some(token))
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            std::fs::read_to_string(&path)
                .map_err(|error| {
                    RuntimeEnvError::new(format!(
                        "could not read runtime generation {}: {error}",
                        path.display()
                    ))
                })
                .and_then(|token| validate_token(token, &path).map(Some))
        }
        Err(error) => Err(RuntimeEnvError::new(format!(
            "could not create runtime generation {}: {error}",
            path.display()
        ))),
    }
}

fn resolve_git_path(repo: &Path, selector: &str) -> Result<Option<PathBuf>, RuntimeEnvError> {
    let output = Command::new("git")
        .args(["-C"])
        .arg(repo)
        .args(["rev-parse", "--path-format=absolute", selector])
        .output()
        .map_err(|error| RuntimeEnvError::new(format!("could not run git rev-parse: {error}")))?;
    if !output.status.success() {
        return Ok(None);
    }
    let value = String::from_utf8(output.stdout).map_err(|error| {
        RuntimeEnvError::new(format!("git rev-parse returned non-UTF-8: {error}"))
    })?;
    Ok(Some(PathBuf::from(value.trim())))
}

fn identity_hash(
    repo: &Path,
    mode: &str,
    generation: Option<&str>,
) -> Result<String, RuntimeEnvError> {
    let bytes = serde_json::to_vec(&(repo, mode, generation)).map_err(|error| {
        RuntimeEnvError::new(format!("could not encode runtime identity: {error}"))
    })?;
    Ok(hex_digest(Sha256::digest(bytes).as_slice()))
}

fn validate_token(token: String, path: &Path) -> Result<String, RuntimeEnvError> {
    let token = token.trim().to_string();
    if token.len() == 32 && token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(token)
    } else {
        Err(RuntimeEnvError::new(format!(
            "invalid runtime generation token in {}",
            path.display()
        )))
    }
}

fn hex_digest(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn slugify(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                character.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string()
}

fn sync_directory(path: &Path) -> Result<(), RuntimeEnvError> {
    std::fs::File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| {
            RuntimeEnvError::new(format!(
                "could not synchronize runtime generation directory {}: {error}",
                path.display()
            ))
        })
}
