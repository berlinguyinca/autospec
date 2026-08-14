use super::{
    heartbeat_root, heartbeat_session_key, parse_startup_heartbeat, ClaimMutationIdentity,
    StartupHeartbeatEvidence,
};
use crate::commands::autonomous::drain::repository_progress_key;
use crate::commands::CommandFailure;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static TEMPORARY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(super) fn process_identity(pid: u32) -> Result<(String, String, String), CommandFailure> {
    let host = hostname()
        .map_err(|error| CommandFailure::diagnostic(format!("read heartbeat host: {error}")))?;
    let (boot_id, process_start) = super::super::autonomous::process_birth_identity(pid)
        .map_err(|error| CommandFailure::diagnostic(format!("read heartbeat process: {error}")))?
        .ok_or_else(|| CommandFailure::diagnostic("heartbeat process identity disappeared"))?;
    if host.is_empty() || boot_id.is_empty() || process_start.is_empty() {
        return Err(CommandFailure::diagnostic(
            "heartbeat process identity is incomplete",
        ));
    }
    Ok((host, boot_id, process_start))
}

#[cfg(unix)]
pub(super) fn hostname() -> Result<String, String> {
    let mut buffer = [0_u8; 256];
    // SAFETY: buffer is writable for its full declared length.
    if unsafe { nix::libc::gethostname(buffer.as_mut_ptr().cast(), buffer.len()) } != 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let end = buffer
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(buffer.len());
    String::from_utf8(buffer[..end].to_vec()).map_err(|error| error.to_string())
}

#[cfg(windows)]
pub(super) fn hostname() -> Result<String, String> {
    #[link(name = "Kernel32")]
    unsafe extern "system" {
        fn GetComputerNameW(buffer: *mut u16, size: *mut u32) -> i32;
    }
    let mut buffer = [0_u16; 256];
    let mut length = buffer.len() as u32;
    // SAFETY: both pointers refer to writable values with the declared capacity.
    if unsafe { GetComputerNameW(buffer.as_mut_ptr(), &mut length) } == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    String::from_utf16(&buffer[..length as usize]).map_err(|error| error.to_string())
}

pub(super) fn publish(
    root: &Path,
    repo: &str,
    issue: u64,
    session_id: Option<&str>,
    document: &[u8],
) -> Result<(), CommandFailure> {
    let expected = parse_startup_heartbeat(document)
        .ok_or_else(|| CommandFailure::diagnostic("startup heartbeat document is malformed"))?;
    ensure_private_directory(root)?;
    let repo_dir = open_or_create_private_directory(root, &repository_progress_key(repo))?;
    publish_exact(&repo_dir, &format!("{issue}.json"), &expected, document)?;
    if let Some(session_id) = session_id {
        let sessions = open_or_create_private_directory(&repo_dir, "sessions")?;
        publish_exact(
            &sessions,
            &format!("{}.json", heartbeat_session_key(session_id)),
            &expected,
            document,
        )?;
    }
    Ok(())
}

fn open_or_create_private_directory(
    parent: &Path,
    name: &str,
) -> Result<std::path::PathBuf, CommandFailure> {
    if name.is_empty() || Path::new(name).is_absolute() || Path::new(name).components().count() != 1
    {
        return Err(CommandFailure::diagnostic(
            "heartbeat directory name must be one normal component",
        ));
    }
    ensure_private_directory(parent)?;
    let path = parent.join(name);
    match fs::create_dir(&path) {
        Ok(()) => set_private_directory_permissions(&path)?,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => {
            return Err(CommandFailure::diagnostic(format!(
                "could not create heartbeat directory: {error}"
            )))
        }
    }
    ensure_private_directory(&path)?;
    Ok(path)
}

fn ensure_private_directory(path: &Path) -> Result<(), CommandFailure> {
    match fs::create_dir(path) {
        Ok(()) => set_private_directory_permissions(path)?,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => {
            return Err(CommandFailure::diagnostic(format!(
                "could not create heartbeat directory: {error}"
            )))
        }
    }
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        CommandFailure::diagnostic(format!("could not inspect heartbeat directory: {error}"))
    })?;
    #[cfg(windows)]
    validate_windows_path_components(path)?;
    if !metadata.file_type().is_dir() || !private_directory_metadata(&metadata) {
        return Err(CommandFailure::diagnostic(
            "heartbeat publication directory is not private",
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn set_private_directory_permissions(path: &Path) -> Result<(), CommandFailure> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|error| {
        CommandFailure::diagnostic(format!("could not secure heartbeat directory: {error}"))
    })
}

#[cfg(windows)]
fn set_private_directory_permissions(_path: &Path) -> Result<(), CommandFailure> {
    Ok(())
}

#[cfg(unix)]
fn private_directory_metadata(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    metadata.uid() == unsafe { nix::libc::geteuid() }
        && metadata.permissions().mode() & 0o7777 == 0o700
}

#[cfg(windows)]
fn validate_windows_path_components(path: &Path) -> Result<(), CommandFailure> {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

    let mut current = std::path::PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        if matches!(
            component,
            std::path::Component::Prefix(_) | std::path::Component::RootDir
        ) {
            continue;
        }
        let metadata = fs::symlink_metadata(&current).map_err(|error| {
            CommandFailure::diagnostic(format!(
                "could not inspect heartbeat path component: {error}"
            ))
        })?;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(CommandFailure::diagnostic(
                "heartbeat path component is a reparse point",
            ));
        }
    }
    Ok(())
}

#[cfg(windows)]
fn private_directory_metadata(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0
}

fn publish_exact(
    directory: &Path,
    name: &str,
    expected: &StartupHeartbeatEvidence,
    document: &[u8],
) -> Result<(), CommandFailure> {
    ensure_private_directory(directory)?;
    let destination = directory.join(name);
    if existing_generation(&destination, expected)? {
        return Ok(());
    }

    let temporary = directory.join(format!(
        ".autospec-heartbeat-{}-{}",
        std::process::id(),
        TEMPORARY_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut file = create_private_file(&temporary)?;
        file.write_all(document)
            .map_err(|error| CommandFailure::diagnostic(format!("heartbeat write: {error}")))?;
        file.sync_all()
            .map_err(|error| CommandFailure::diagnostic(format!("heartbeat fsync: {error}")))?;
        drop(file);
        atomic_rename_exclusive(&temporary, &destination).or_else(|error| {
            if existing_generation(&destination, expected)? {
                Ok(())
            } else {
                Err(error)
            }
        })?;
        sync_directory(directory)?;
        if !existing_generation(&destination, expected)? {
            return Err(CommandFailure::diagnostic(
                "heartbeat publication target conflicts",
            ));
        }
        Ok(())
    })();
    let _ = fs::remove_file(&temporary);
    result
}

fn existing_generation(
    path: &Path,
    expected: &StartupHeartbeatEvidence,
) -> Result<bool, CommandFailure> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(_) => {
            return Err(CommandFailure::diagnostic(
                "heartbeat publication target conflicts",
            ))
        }
    };
    if !metadata.file_type().is_file() || !private_file_metadata(&metadata) {
        return Err(CommandFailure::diagnostic(
            "heartbeat publication target conflicts",
        ));
    }
    let document = read_file_no_follow(path)
        .map_err(|_| CommandFailure::diagnostic("heartbeat publication target conflicts"))?;
    let Some(observed) = parse_startup_heartbeat(&document) else {
        return Err(CommandFailure::diagnostic(
            "heartbeat publication target conflicts",
        ));
    };
    if same_generation(&observed, expected) {
        Ok(true)
    } else {
        Err(CommandFailure::diagnostic(
            "heartbeat publication target conflicts",
        ))
    }
}

fn same_generation(left: &StartupHeartbeatEvidence, right: &StartupHeartbeatEvidence) -> bool {
    let mut left = left.clone();
    let mut right = right.clone();
    for evidence in [&mut left, &mut right] {
        evidence.ts = 0;
        evidence.pid = 0;
        evidence.process_start.clear();
    }
    left == right
}

#[cfg(unix)]
fn create_private_file(path: &Path) -> Result<fs::File, CommandFailure> {
    use std::os::unix::fs::OpenOptionsExt;
    use std::os::unix::fs::PermissionsExt;
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_CLOEXEC)
        .open(path)
        .map_err(|error| CommandFailure::diagnostic(format!("heartbeat stage create: {error}")))?;
    file.set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|error| CommandFailure::diagnostic(format!("heartbeat stage chmod: {error}")))?;
    Ok(file)
}

#[cfg(windows)]
fn create_private_file(path: &Path) -> Result<fs::File, CommandFailure> {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|error| CommandFailure::diagnostic(format!("heartbeat stage create: {error}")))
}

#[cfg(unix)]
fn read_file_no_follow(path: &Path) -> std::io::Result<Vec<u8>> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_CLOEXEC | nix::libc::O_NONBLOCK)
        .open(path)?;
    let mut document = Vec::new();
    file.read_to_end(&mut document)?;
    Ok(document)
}

#[cfg(windows)]
fn read_file_no_follow(path: &Path) -> std::io::Result<Vec<u8>> {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    let mut document = Vec::new();
    file.read_to_end(&mut document)?;
    Ok(document)
}

#[cfg(unix)]
fn private_file_metadata(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    metadata.uid() == unsafe { nix::libc::geteuid() }
        && metadata.permissions().mode() & 0o7777 == 0o600
        && metadata.nlink() == 1
}

#[cfg(windows)]
fn private_file_metadata(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0
}

#[cfg(target_os = "macos")]
fn atomic_rename_exclusive(source: &Path, destination: &Path) -> Result<(), CommandFailure> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    unsafe extern "C" {
        fn renamex_np(
            source: *const nix::libc::c_char,
            destination: *const nix::libc::c_char,
            flags: u32,
        ) -> i32;
    }
    const RENAME_EXCL: u32 = 0x0000_0004;
    let source = CString::new(source.as_os_str().as_bytes())
        .map_err(|_| CommandFailure::diagnostic("heartbeat stage path contains NUL"))?;
    let destination = CString::new(destination.as_os_str().as_bytes())
        .map_err(|_| CommandFailure::diagnostic("heartbeat destination path contains NUL"))?;
    // SAFETY: both strings are NUL-terminated and remain alive for the call.
    if unsafe { renamex_np(source.as_ptr(), destination.as_ptr(), RENAME_EXCL) } == 0 {
        Ok(())
    } else {
        Err(CommandFailure::diagnostic(format!(
            "heartbeat atomic rename: {}",
            std::io::Error::last_os_error()
        )))
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn atomic_rename_exclusive(source: &Path, destination: &Path) -> Result<(), CommandFailure> {
    fs::hard_link(source, destination)
        .and_then(|()| fs::remove_file(source))
        .map_err(|error| CommandFailure::diagnostic(format!("heartbeat atomic publish: {error}")))
}

#[cfg(windows)]
fn atomic_rename_exclusive(source: &Path, destination: &Path) -> Result<(), CommandFailure> {
    fs::rename(source, destination)
        .map_err(|error| CommandFailure::diagnostic(format!("heartbeat atomic rename: {error}")))
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), CommandFailure> {
    use std::os::unix::fs::OpenOptionsExt;
    let directory = OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_DIRECTORY | nix::libc::O_NOFOLLOW | nix::libc::O_CLOEXEC)
        .open(path)
        .map_err(|error| {
            CommandFailure::diagnostic(format!("heartbeat directory open: {error}"))
        })?;
    directory
        .sync_all()
        .map_err(|error| CommandFailure::diagnostic(format!("heartbeat directory fsync: {error}")))
}

#[cfg(windows)]
fn sync_directory(path: &Path) -> Result<(), CommandFailure> {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    let directory = OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|error| {
            CommandFailure::diagnostic(format!("heartbeat directory open: {error}"))
        })?;
    match directory.sync_all() {
        Ok(()) => Ok(()),
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::InvalidInput
            ) =>
        {
            Ok(())
        }
        Err(error) => Err(CommandFailure::diagnostic(format!(
            "heartbeat directory flush: {error}"
        ))),
    }
}

pub(super) fn retire_released(identity: ClaimMutationIdentity<'_>) -> Result<(), CommandFailure> {
    let root = heartbeat_root()?;
    retire_released_at(&root, identity)
}

fn retire_released_at(
    root: &Path,
    identity: ClaimMutationIdentity<'_>,
) -> Result<(), CommandFailure> {
    let repo = root.join(repository_progress_key(identity.repo));
    let issue = repo.join(format!("{}.json", identity.issue));
    let Some(evidence) = matching_retirement_evidence(&issue, identity)? else {
        return Ok(());
    };
    if let Some(session_id) = evidence.session_id.as_deref() {
        let sessions = repo.join("sessions");
        let session = sessions.join(format!("{}.json", heartbeat_session_key(session_id)));
        remove_if_matching(&session, identity)?;
        if sessions.exists() {
            sync_directory(&sessions)?;
        }
    }
    remove_if_matching(&issue, identity)?;
    sync_directory(&repo)?;
    Ok(())
}

fn matching_retirement_evidence(
    path: &Path,
    identity: ClaimMutationIdentity<'_>,
) -> Result<Option<StartupHeartbeatEvidence>, CommandFailure> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(CommandFailure::diagnostic(format!(
                "could not inspect released heartbeat: {error}"
            )))
        }
        Ok(metadata) if metadata.file_type().is_file() && private_file_metadata(&metadata) => {}
        Ok(_) => return Ok(None),
    }
    let document = read_file_no_follow(path)
        .map_err(|error| CommandFailure::diagnostic(format!("read released heartbeat: {error}")))?;
    let Some(evidence) = parse_startup_heartbeat(&document) else {
        return Ok(None);
    };
    Ok(exact_retirement_identity(&evidence, identity).then_some(evidence))
}

fn exact_retirement_identity(
    evidence: &StartupHeartbeatEvidence,
    identity: ClaimMutationIdentity<'_>,
) -> bool {
    evidence.repo == identity.repo
        && evidence.issue == identity.issue.to_string()
        && evidence.worker_id == identity.worker_id
        && evidence.branch == identity.branch
        && evidence.claim_id == identity.claim_id
}

fn remove_if_matching(
    path: &Path,
    identity: ClaimMutationIdentity<'_>,
) -> Result<(), CommandFailure> {
    if matching_retirement_evidence(path, identity)?.is_none() {
        return Ok(());
    }
    fs::remove_file(path)
        .map_err(|error| CommandFailure::diagnostic(format!("remove released heartbeat: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::claim::ClaimMutationIdentity;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

    struct Fixture {
        root: std::path::PathBuf,
    }

    impl Fixture {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "autospec-heartbeat-portable-{name}-{}-{}",
                std::process::id(),
                NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&root).expect("private heartbeat root");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
                    .expect("private root permissions");
            }
            Self { root }
        }

        fn document(&self, claim_id: &str, session_id: Option<&str>) -> Vec<u8> {
            let session =
                session_id.map_or_else(String::new, |value| format!(r#","session_id":"{value}""#));
            format!(
                r#"{{"repo":"owner/repo","issue":"42","worker_id":"worker-a","branch":"feat/worker","pr":"","claim_id":"{claim_id}","step":"claimed","ts":1,"ttl_seconds":10,"pid":7,"nonce":"nonce-{claim_id}","host":"host-a","boot_id":"boot-a","process_start":"9"{session}}}"#
            )
            .into_bytes()
        }

        fn issue_path(&self) -> std::path::PathBuf {
            self.root
                .join(crate::commands::autonomous::drain::repository_progress_key(
                    "owner/repo",
                ))
                .join("42.json")
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn publication_is_idempotent_but_rejects_another_generation() {
        let fixture = Fixture::new("generation");
        let first = fixture.document("claim-a", Some("session-a"));
        publish(&fixture.root, "owner/repo", 42, Some("session-a"), &first)
            .expect("initial publication");
        publish(&fixture.root, "owner/repo", 42, Some("session-a"), &first)
            .expect("idempotent replay");

        let conflict = fixture.document("claim-b", Some("session-b"));
        let error = publish(
            &fixture.root,
            "owner/repo",
            42,
            Some("session-b"),
            &conflict,
        )
        .expect_err("generation conflict");

        assert_eq!(error.message, "heartbeat publication target conflicts");
    }

    #[cfg(unix)]
    #[test]
    fn publication_rejects_a_final_symlink() {
        use std::os::unix::fs::{symlink, PermissionsExt};

        let fixture = Fixture::new("symlink");
        let repo = fixture.issue_path().parent().unwrap().to_path_buf();
        std::fs::create_dir(&repo).expect("repo directory");
        std::fs::set_permissions(&repo, std::fs::Permissions::from_mode(0o700))
            .expect("repo permissions");
        let outside = fixture.root.join("outside");
        std::fs::write(&outside, b"caller-owned").expect("outside file");
        symlink(&outside, fixture.issue_path()).expect("final symlink");

        let error = publish(
            &fixture.root,
            "owner/repo",
            42,
            None,
            &fixture.document("claim-a", None),
        )
        .expect_err("final symlink conflict");

        assert_eq!(error.message, "heartbeat publication target conflicts");
        assert_eq!(std::fs::read(outside).unwrap(), b"caller-owned");
    }

    #[test]
    fn retirement_removes_only_the_exact_generation() {
        let fixture = Fixture::new("retirement");
        let document = fixture.document("claim-a", Some("session-a"));
        publish(
            &fixture.root,
            "owner/repo",
            42,
            Some("session-a"),
            &document,
        )
        .expect("heartbeat");

        retire_released_at(
            &fixture.root,
            ClaimMutationIdentity {
                repo: "owner/repo",
                issue: 42,
                worker_id: "worker-a",
                branch: "feat/worker",
                claim_id: "claim-b",
            },
        )
        .expect("mismatch is not retired");
        assert!(fixture.issue_path().exists());

        retire_released_at(
            &fixture.root,
            ClaimMutationIdentity {
                repo: "owner/repo",
                issue: 42,
                worker_id: "worker-a",
                branch: "feat/worker",
                claim_id: "claim-a",
            },
        )
        .expect("exact retirement");
        assert!(!fixture.issue_path().exists());
    }
}
