use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
struct TrustedExecutableIdentity {
    dev: u64,
    ino: u64,
    uid: u32,
    gid: u32,
    mode: u32,
    nlink: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::commands::autonomous::executor_bridge) struct TrustedExecutable {
    path: PathBuf,
    identity: TrustedExecutableIdentity,
    content_sha256: String,
    purpose: &'static str,
}

pub(in crate::commands::autonomous::executor_bridge) fn trusted_executable_owner_allowed(
    owner: u32,
    effective_user: u32,
) -> bool {
    owner == 0 || owner == effective_user
}

impl TrustedExecutable {
    pub(in crate::commands::autonomous::executor_bridge) fn resolve(
        path: &Path,
        environment: &BTreeMap<String, OsString>,
        worktree: &Path,
        purpose: &'static str,
    ) -> Result<Self, String> {
        let path = safe_executable(path, environment)
            .map_err(|error| format!("resolve {purpose}: {error}"))?;
        if path.starts_with(worktree) {
            return Err(format!(
                "{purpose} is writable through the executor worktree"
            ));
        }
        let (file, identity) = Self::open_validated(&path, purpose)?;
        let content_sha256 = Self::content_sha256(file, purpose)?;
        Ok(Self {
            path,
            identity,
            content_sha256,
            purpose,
        })
    }

    fn open_validated(
        path: &Path,
        purpose: &'static str,
    ) -> Result<(File, TrustedExecutableIdentity), String> {
        let file = File::open(path)
            .map_err(|error| format!("open {purpose} {}: {error}", path.display()))?;
        let metadata = file
            .metadata()
            .map_err(|error| format!("inspect {purpose} {}: {error}", path.display()))?;
        if !metadata.file_type().is_file() {
            return Err(format!("{purpose} must resolve to a regular file"));
        }
        if metadata.nlink() != 1 {
            return Err(format!("{purpose} must have a single link"));
        }
        if metadata.mode() & 0o111 == 0 {
            return Err(format!("{purpose} must be executable"));
        }
        if metadata.mode() & 0o022 != 0 {
            return Err(format!("{purpose} must not be group/world writable"));
        }
        let effective_user = nix::unistd::geteuid().as_raw();
        if !trusted_executable_owner_allowed(metadata.uid(), effective_user) {
            return Err(format!(
                "{purpose} owner must be root or effective user {effective_user}"
            ));
        }
        Ok((
            file,
            TrustedExecutableIdentity {
                dev: metadata.dev(),
                ino: metadata.ino(),
                uid: metadata.uid(),
                gid: metadata.gid(),
                mode: metadata.mode(),
                nlink: metadata.nlink(),
            },
        ))
    }

    fn content_sha256(file: File, purpose: &'static str) -> Result<String, String> {
        sha256_reader_hex(file)
            .map_err(|error| format!("read {purpose} for content validation: {error}"))
    }

    pub(in crate::commands::autonomous::executor_bridge) fn path(&self) -> &Path {
        &self.path
    }

    /// A same-user writer can still race after this final snapshot and before exec; removing that
    /// residual window requires descriptor-based execution for both native binaries and scripts.
    pub(in crate::commands::autonomous::executor_bridge) fn revalidate(
        &self,
    ) -> Result<(), String> {
        let (file, observed) = Self::open_validated(&self.path, self.purpose)?;
        if observed != self.identity {
            return Err(format!(
                "{} identity changed before contained hook execution",
                self.purpose
            ));
        }
        if Self::content_sha256(file, self.purpose)? != self.content_sha256 {
            return Err(format!(
                "{} content changed before contained hook execution",
                self.purpose
            ));
        }
        Ok(())
    }
}
