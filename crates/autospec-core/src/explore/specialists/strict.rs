use super::lexicon;
use super::{DetectedDomain, FileLineEvidence};

use std::collections::BTreeSet;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
const STRICT_DEPTH: usize = 3;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StrictCollectorOptions {
    pub repo_dir: PathBuf,
    pub max_depth: usize,
}
impl StrictCollectorOptions {
    pub fn new(repo_dir: impl AsRef<Path>) -> Self {
        Self {
            repo_dir: repo_dir.as_ref().to_path_buf(),
            max_depth: STRICT_DEPTH,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StrictCollectorEvidence {
    pub schema_version: u64,
    pub collector_version: String,
    pub canonical_repo_scope: String,
    pub domains: Vec<DetectedDomain>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrictCollectorErrorCode {
    InvalidRoot,
    InvalidCollectorSchema,
    PathEscapesRoot,
    ReadDirectory,
    ReadFile,
    InvalidUtf8,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StrictCollectorError {
    pub code: StrictCollectorErrorCode,
    pub detail: String,
}

/// Collects deterministic local evidence; callers must keep the repository tree stable for the scan snapshot.
/// Paths are revalidated after reads, but this is not atomic no-follow protection.
pub fn collect_strict_domains(
    options: &StrictCollectorOptions,
) -> Result<StrictCollectorEvidence, StrictCollectorError> {
    if options.max_depth != STRICT_DEPTH {
        return Err(error(
            StrictCollectorErrorCode::InvalidCollectorSchema,
            format!("max_depth must equal {STRICT_DEPTH}"),
        ));
    }
    let root = canonical_root(&options.repo_dir)?;
    let mut hits = lexicon::empty_hits();
    let mut selected_files = root_signal_files(&root)?;
    let mut path_signals = BTreeSet::new();
    walk_paths(&root, &root, 0, &mut selected_files, &mut path_signals)?;
    selected_files.sort();
    selected_files.dedup();
    for file in selected_files {
        scan_file(&root, &file, &mut hits)?;
    }
    let repo_name = root
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            error(
                StrictCollectorErrorCode::InvalidRoot,
                "root name is not UTF-8",
            )
        })?;
    lexicon::scan_identifiers(&[repo_name.to_string()], ".", "repo-name", &mut hits);
    lexicon::scan_identifiers(
        &path_signals.into_iter().collect::<Vec<_>>(),
        "",
        "code path",
        &mut hits,
    );

    let mut domains = lexicon::ranked_domains(hits);
    for domain in &mut domains {
        domain.evidence.sort_by(|left, right| {
            left.file
                .cmp(&right.file)
                .then_with(|| left.line.cmp(&right.line))
                .then_with(|| left.r#match.cmp(&right.r#match))
        });
    }

    Ok(StrictCollectorEvidence {
        schema_version: 1,
        collector_version: "strict-local-v1".to_string(),
        canonical_repo_scope: root.to_string_lossy().replace('\\', "/"),
        domains,
    })
}

fn canonical_root(repo_dir: &Path) -> Result<PathBuf, StrictCollectorError> {
    let metadata = fs::symlink_metadata(repo_dir).map_err(|source| {
        error(
            StrictCollectorErrorCode::InvalidRoot,
            format!("cannot inspect {}: {source}", repo_dir.display()),
        )
    })?;
    if metadata.file_type().is_symlink() {
        return Err(error(
            StrictCollectorErrorCode::PathEscapesRoot,
            format!("root {} is a symlink", repo_dir.display()),
        ));
    }
    let root = fs::canonicalize(repo_dir).map_err(|source| {
        error(
            StrictCollectorErrorCode::InvalidRoot,
            format!("cannot canonicalize {}: {source}", repo_dir.display()),
        )
    })?;
    if root.to_str().is_none() {
        return Err(error(
            StrictCollectorErrorCode::InvalidRoot,
            "canonical root is not UTF-8",
        ));
    }
    if !metadata.is_dir() {
        return Err(error(
            StrictCollectorErrorCode::InvalidRoot,
            format!("root {} is not a directory", root.display()),
        ));
    }
    Ok(root)
}

fn root_signal_files(root: &Path) -> Result<Vec<PathBuf>, StrictCollectorError> {
    lexicon::signal_file_names()
        .filter_map(|name| selected_file(root, name).transpose())
        .collect()
}

fn selected_file(root: &Path, name: &str) -> Result<Option<PathBuf>, StrictCollectorError> {
    let path = root.join(name);
    match fs::symlink_metadata(&path) {
        Ok(_) => canonical_file(root, &path).map(Some),
        Err(source) if source.kind() == ErrorKind::NotFound => Ok(None),
        Err(source) => Err(error(
            StrictCollectorErrorCode::ReadFile,
            format!("cannot inspect {}: {source}", path.display()),
        )),
    }
}

fn walk_paths(
    root: &Path,
    current: &Path,
    depth: usize,
    selected_files: &mut Vec<PathBuf>,
    signals: &mut BTreeSet<String>,
) -> Result<(), StrictCollectorError> {
    let entries = fs::read_dir(current).map_err(|source| {
        error(
            StrictCollectorErrorCode::ReadDirectory,
            format!("cannot read {}: {source}", current.display()),
        )
    })?;
    let mut entries = entries.collect::<Result<Vec<_>, _>>().map_err(|source| {
        error(
            StrictCollectorErrorCode::ReadDirectory,
            format!("cannot enumerate {}: {source}", current.display()),
        )
    })?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        visit_entry(root, entry.path(), depth, selected_files, signals)?;
    }
    Ok(())
}

fn visit_entry(
    root: &Path,
    path: PathBuf,
    depth: usize,
    selected_files: &mut Vec<PathBuf>,
    signals: &mut BTreeSet<String>,
) -> Result<(), StrictCollectorError> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            error(
                StrictCollectorErrorCode::ReadDirectory,
                format!("entry name is not UTF-8: {}", path.display()),
            )
        })?;
    let metadata = fs::symlink_metadata(&path).map_err(|source| {
        error(
            StrictCollectorErrorCode::ReadDirectory,
            format!("cannot inspect {}: {source}", path.display()),
        )
    })?;
    if metadata.file_type().is_symlink() {
        return Err(error(
            StrictCollectorErrorCode::PathEscapesRoot,
            format!("symlink is not permitted: {}", path.display()),
        ));
    }
    if metadata.is_dir() {
        visit_directory(root, &path, name, depth, selected_files, signals)
    } else if metadata.is_file() {
        visit_file(root, &path, name, selected_files, signals)
    } else {
        Err(error(
            StrictCollectorErrorCode::ReadFile,
            format!("unsupported file type: {}", path.display()),
        ))
    }
}

fn visit_directory(
    root: &Path,
    path: &Path,
    name: &str,
    depth: usize,
    selected_files: &mut Vec<PathBuf>,
    signals: &mut BTreeSet<String>,
) -> Result<(), StrictCollectorError> {
    if lexicon::should_skip_dir(name) || depth >= STRICT_DEPTH {
        return Ok(());
    }
    let directory = canonical_directory(root, path)?;
    if depth == 0 {
        signals.insert(name.to_string());
    }
    signals.insert(format!("{}/", relative_path(root, &directory)?));
    walk_paths(root, &directory, depth + 1, selected_files, signals)
}

fn visit_file(
    root: &Path,
    path: &Path,
    name: &str,
    selected_files: &mut Vec<PathBuf>,
    signals: &mut BTreeSet<String>,
) -> Result<(), StrictCollectorError> {
    let file = canonical_file(root, path)?;
    if name.ends_with(".csproj") {
        selected_files.push(file.clone());
    }
    signals.insert(relative_path(root, &file)?);
    Ok(())
}

fn scan_file(
    root: &Path,
    file: &Path,
    hits: &mut [Vec<FileLineEvidence>],
) -> Result<(), StrictCollectorError> {
    let bytes = fs::read(file).map_err(|source| {
        error(
            StrictCollectorErrorCode::ReadFile,
            format!("cannot read {}: {source}", file.display()),
        )
    })?;
    revalidate_read_file(root, file, file)?;
    let document = String::from_utf8(bytes).map_err(|source| {
        error(
            StrictCollectorErrorCode::InvalidUtf8,
            format!("invalid UTF-8 in {}: {source}", file.display()),
        )
    })?;
    let relative = relative_path(root, file)?;
    for (index, line) in document.lines().enumerate() {
        lexicon::scan_line(&relative, index + 1, line, hits);
    }
    Ok(())
}

fn canonical_directory(root: &Path, path: &Path) -> Result<PathBuf, StrictCollectorError> {
    canonical_path(root, path, true)
}

fn canonical_file(root: &Path, path: &Path) -> Result<PathBuf, StrictCollectorError> {
    canonical_path(root, path, false)
}

fn canonical_path(
    root: &Path,
    path: &Path,
    expect_directory: bool,
) -> Result<PathBuf, StrictCollectorError> {
    let read_code = if expect_directory {
        StrictCollectorErrorCode::ReadDirectory
    } else {
        StrictCollectorErrorCode::ReadFile
    };
    reject_symlink_components(root, path, read_code)?;
    let canonical = fs::canonicalize(path).map_err(|source| {
        error(
            read_code,
            format!("cannot canonicalize {}: {source}", path.display()),
        )
    })?;
    if !canonical.starts_with(root) {
        return Err(error(
            StrictCollectorErrorCode::PathEscapesRoot,
            format!("path escapes root: {}", path.display()),
        ));
    }
    let metadata = fs::metadata(&canonical).map_err(|source| {
        error(
            read_code,
            format!("cannot inspect {}: {source}", canonical.display()),
        )
    })?;
    let expected_type = if expect_directory {
        metadata.is_dir()
    } else {
        metadata.is_file()
    };
    if !expected_type {
        return Err(error(
            read_code,
            format!("unexpected file type: {}", canonical.display()),
        ));
    }
    Ok(canonical)
}

fn reject_symlink_components(
    root: &Path,
    path: &Path,
    read_code: StrictCollectorErrorCode,
) -> Result<(), StrictCollectorError> {
    let relative = path.strip_prefix(root).map_err(|_| {
        error(
            StrictCollectorErrorCode::PathEscapesRoot,
            format!("path escapes root: {}", path.display()),
        )
    })?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        let metadata = fs::symlink_metadata(&current).map_err(|source| {
            error(
                read_code,
                format!("cannot inspect {}: {source}", current.display()),
            )
        })?;
        if metadata.file_type().is_symlink() {
            return Err(error(
                StrictCollectorErrorCode::PathEscapesRoot,
                format!("symlink is not permitted: {}", current.display()),
            ));
        }
    }
    Ok(())
}

fn revalidate_read_file(
    root: &Path,
    file: &Path,
    expected: &Path,
) -> Result<(), StrictCollectorError> {
    let current = canonical_file(root, file)?;
    if current != expected {
        return Err(error(
            StrictCollectorErrorCode::PathEscapesRoot,
            format!("selected file changed after read: {}", file.display()),
        ));
    }
    Ok(())
}

fn relative_path(root: &Path, path: &Path) -> Result<String, StrictCollectorError> {
    let relative = path.strip_prefix(root).map_err(|_| {
        error(
            StrictCollectorErrorCode::PathEscapesRoot,
            format!("path escapes root: {}", path.display()),
        )
    })?;
    relative
        .to_str()
        .map(|value| value.replace('\\', "/"))
        .ok_or_else(|| {
            error(
                StrictCollectorErrorCode::ReadFile,
                format!("path is not UTF-8: {}", path.display()),
            )
        })
}

fn error(code: StrictCollectorErrorCode, detail: impl Into<String>) -> StrictCollectorError {
    StrictCollectorError {
        code,
        detail: detail.into(),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{canonical_file, revalidate_read_file, StrictCollectorErrorCode};

    fn temporary_directory(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let path = PathBuf::from(format!("/tmp/autospec-strict-{name}-{nanos}"));
        fs::create_dir_all(&path).expect("create directory");
        fs::canonicalize(path).expect("canonical directory")
    }

    #[cfg(unix)]
    #[test]
    fn canonical_file_rejects_a_nonregular_unix_socket() {
        use std::os::unix::net::UnixDatagram;

        let root = temporary_directory("socket");
        let socket = root.join("requirements.txt");
        let _listener = UnixDatagram::bind(&socket).expect("bind socket");

        let error = canonical_file(&root, &socket).expect_err("socket must fail");

        assert_eq!(error.code, StrictCollectorErrorCode::ReadFile);
    }

    #[cfg(unix)]
    #[test]
    fn post_read_validation_rejects_a_swapped_symlink() {
        use std::os::unix::fs::symlink;

        let root = temporary_directory("post-read");
        let file = root.join("requirements.txt");
        let outside = temporary_directory("outside").join("requirements.txt");
        fs::write(&file, "ccxt\n").expect("write selected file");
        fs::write(&outside, "stripe\n").expect("write outside file");
        let expected = canonical_file(&root, &file).expect("canonical selected file");
        fs::remove_file(&file).expect("remove selected file");
        symlink(&outside, &file).expect("swap to symlink");

        let error =
            revalidate_read_file(&root, &file, &expected).expect_err("post-read symlink must fail");

        assert_eq!(error.code, StrictCollectorErrorCode::PathEscapesRoot);
    }
}
