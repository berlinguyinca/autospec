use super::CommandFailure;
use std::fs;
use std::path::Path;

#[allow(dead_code)]
pub(super) fn open_heartbeat_directory_beneath(
    trusted_parent: &fs::File,
    descendant: &Path,
) -> Result<fs::File, CommandFailure> {
    open_heartbeat_directory_beneath_with_hook(trusted_parent, descendant, || {})
}

#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct HeartbeatDirectoryIdentity {
    pub(super) device: u64,
    pub(super) inode: u64,
    pub(super) owner: u32,
    pub(super) mode: u32,
}

#[cfg(unix)]
pub(super) fn private_heartbeat_directory_identity(
    directory: &impl std::os::fd::AsFd,
    role: &str,
) -> Result<HeartbeatDirectoryIdentity, CommandFailure> {
    use nix::sys::stat::{fstat, SFlag};

    let stat = fstat(directory).map_err(|error| {
        CommandFailure::diagnostic(format!("could not inspect heartbeat {role}: {error}"))
    })?;
    if !SFlag::from_bits_truncate(stat.st_mode).contains(SFlag::S_IFDIR)
        || stat.st_uid != nix::unistd::geteuid().as_raw()
        || stat.st_mode & 0o7777 != 0o700
    {
        return Err(CommandFailure::diagnostic(format!(
            "heartbeat {role} must be owned by the effective user with mode 0700"
        )));
    }
    Ok(HeartbeatDirectoryIdentity {
        device: stat.st_dev as u64,
        inode: stat.st_ino as u64,
        owner: stat.st_uid as u32,
        mode: (stat.st_mode & 0o7777) as u32,
    })
}

#[cfg(unix)]
pub(super) fn private_heartbeat_name_identity(
    parent: &impl std::os::fd::AsFd,
    name: &Path,
    role: &str,
) -> Result<HeartbeatDirectoryIdentity, CommandFailure> {
    use nix::fcntl::AtFlags;
    use nix::sys::stat::{fstatat, SFlag};

    let stat = fstatat(parent, name, AtFlags::AT_SYMLINK_NOFOLLOW).map_err(|error| {
        CommandFailure::diagnostic(format!("could not inspect heartbeat {role}: {error}"))
    })?;
    if !SFlag::from_bits_truncate(stat.st_mode).contains(SFlag::S_IFDIR)
        || stat.st_uid != nix::unistd::geteuid().as_raw()
        || stat.st_mode & 0o7777 != 0o700
    {
        return Err(CommandFailure::diagnostic(format!(
            "heartbeat {role} must be owned by the effective user with mode 0700"
        )));
    }
    Ok(HeartbeatDirectoryIdentity {
        device: stat.st_dev as u64,
        inode: stat.st_ino as u64,
        owner: stat.st_uid as u32,
        mode: (stat.st_mode & 0o7777) as u32,
    })
}

#[cfg(target_os = "linux")]
pub(super) fn heartbeat_openat2_resolve_flags() -> nix::fcntl::ResolveFlag {
    use nix::fcntl::ResolveFlag;

    ResolveFlag::RESOLVE_BENEATH
        | ResolveFlag::RESOLVE_NO_SYMLINKS
        | ResolveFlag::RESOLVE_NO_MAGICLINKS
        | ResolveFlag::RESOLVE_NO_XDEV
}

#[cfg(unix)]
fn validate_portable_descendant(descendant: &Path) -> Result<(), CommandFailure> {
    use std::path::Component;

    if matches!(
        descendant.components().collect::<Vec<_>>().as_slice(),
        [Component::Normal(_)]
    ) {
        Ok(())
    } else {
        Err(CommandFailure::diagnostic(
            "portable heartbeat descendant must be exactly one normal component",
        ))
    }
}

#[cfg(unix)]
fn open_heartbeat_directory_portable_unix_after_hook(
    trusted_parent: &fs::File,
    descendant: &Path,
    parent_identity: HeartbeatDirectoryIdentity,
    expected_binding: HeartbeatDirectoryIdentity,
) -> Result<fs::File, CommandFailure> {
    use nix::fcntl::{openat, OFlag};
    use nix::sys::stat::Mode;

    validate_portable_descendant(descendant)?;
    let directory = openat(
        trusted_parent,
        descendant,
        OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| {
        CommandFailure::diagnostic(format!(
            "portable heartbeat directory resolution failed: {error}"
        ))
    })?;
    validate_opened_directory(
        trusted_parent,
        descendant,
        &directory,
        parent_identity,
        expected_binding,
    )?;
    Ok(fs::File::from(directory))
}

#[cfg(unix)]
fn validate_opened_directory(
    trusted_parent: &fs::File,
    descendant: &Path,
    directory: &impl std::os::fd::AsFd,
    parent_identity: HeartbeatDirectoryIdentity,
    expected_binding: HeartbeatDirectoryIdentity,
) -> Result<(), CommandFailure> {
    if private_heartbeat_directory_identity(trusted_parent, "parent")? != parent_identity {
        return Err(CommandFailure::diagnostic(
            "heartbeat parent descriptor identity drift after descendant open",
        ));
    }
    let opened = private_heartbeat_directory_identity(directory, "descendant")?;
    if opened != expected_binding
        || private_heartbeat_name_identity(trusted_parent, descendant, "descendant binding")?
            != opened
    {
        return Err(CommandFailure::diagnostic(
            "heartbeat descendant name binding changed during open",
        ));
    }
    Ok(())
}

#[cfg(unix)]
#[allow(dead_code)]
pub(super) fn open_heartbeat_directory_portable_unix_with_hook(
    trusted_parent: &fs::File,
    descendant: &Path,
    before_open: impl FnOnce(),
) -> Result<fs::File, CommandFailure> {
    validate_portable_descendant(descendant)?;
    let parent_identity = private_heartbeat_directory_identity(trusted_parent, "parent")?;
    let expected_binding =
        private_heartbeat_name_identity(trusted_parent, descendant, "descendant binding")?;
    before_open();
    if private_heartbeat_directory_identity(trusted_parent, "parent")? != parent_identity {
        return Err(CommandFailure::diagnostic(
            "heartbeat parent descriptor identity drift before descendant open",
        ));
    }
    open_heartbeat_directory_portable_unix_after_hook(
        trusted_parent,
        descendant,
        parent_identity,
        expected_binding,
    )
}

#[cfg(target_os = "linux")]
fn open_heartbeat_directory_beneath_with_openat2(
    trusted_parent: &fs::File,
    descendant: &Path,
    before_open: impl FnOnce(),
    secure_open: impl FnOnce(&fs::File, &Path) -> Result<std::os::fd::OwnedFd, nix::errno::Errno>,
) -> Result<fs::File, CommandFailure> {
    use std::path::Component;

    if descendant.as_os_str().is_empty()
        || descendant.is_absolute()
        || descendant
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(CommandFailure::diagnostic(
            "heartbeat descendant must be relative, non-empty, and contain no '..'",
        ));
    }
    let parent_identity = private_heartbeat_directory_identity(trusted_parent, "parent")?;
    let expected_binding =
        private_heartbeat_name_identity(trusted_parent, descendant, "descendant binding")?;
    before_open();
    if private_heartbeat_directory_identity(trusted_parent, "parent")? != parent_identity {
        return Err(CommandFailure::diagnostic(
            "heartbeat parent descriptor identity drift before descendant open",
        ));
    }
    let directory = match secure_open(trusted_parent, descendant) {
        Ok(directory) => directory,
        Err(nix::errno::Errno::ENOSYS) => {
            return open_heartbeat_directory_portable_unix_after_hook(
                trusted_parent,
                descendant,
                parent_identity,
                expected_binding,
            );
        }
        Err(error) => {
            return Err(CommandFailure::diagnostic(format!(
                "heartbeat directory secure resolution failed: {error}"
            )));
        }
    };
    validate_opened_directory(
        trusted_parent,
        descendant,
        &directory,
        parent_identity,
        expected_binding,
    )?;
    Ok(fs::File::from(directory))
}

#[cfg(target_os = "linux")]
#[allow(dead_code)]
pub(super) fn open_heartbeat_directory_beneath_with_hook(
    trusted_parent: &fs::File,
    descendant: &Path,
    before_open: impl FnOnce(),
) -> Result<fs::File, CommandFailure> {
    use nix::fcntl::{openat2, OFlag, OpenHow};

    open_heartbeat_directory_beneath_with_openat2(
        trusted_parent,
        descendant,
        before_open,
        |parent, path| {
            let how = OpenHow::new()
                .flags(OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC)
                .resolve(heartbeat_openat2_resolve_flags());
            openat2(parent, path, how)
        },
    )
}

#[cfg(all(unix, not(target_os = "linux")))]
#[allow(dead_code)]
pub(super) fn open_heartbeat_directory_beneath_with_hook(
    trusted_parent: &fs::File,
    descendant: &Path,
    before_open: impl FnOnce(),
) -> Result<fs::File, CommandFailure> {
    open_heartbeat_directory_portable_unix_with_hook(trusted_parent, descendant, before_open)
}

#[cfg(not(unix))]
#[allow(dead_code)]
pub(super) fn open_heartbeat_directory_beneath_with_hook(
    _trusted_parent: &fs::File,
    _descendant: &Path,
    _before_open: impl FnOnce(),
) -> Result<fs::File, CommandFailure> {
    Err(CommandFailure::diagnostic(
        "secure heartbeat directory resolution requires Unix descriptor operations",
    ))
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use nix::fcntl::{open, OFlag};
    use nix::sys::stat::{fstat, Mode};
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use std::sync::atomic::{AtomicU64, Ordering};

    static FIXTURE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn private_fixture(label: &str) -> std::path::PathBuf {
        let sequence = FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "autospec-open-beneath-{label}-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("create private fixture");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
            .expect("protect private fixture");
        path
    }

    fn private_child(parent: &Path) -> std::path::PathBuf {
        let child = parent.join("heartbeat");
        fs::create_dir(&child).expect("create private child");
        fs::set_permissions(&child, fs::Permissions::from_mode(0o700))
            .expect("protect private child");
        child
    }

    fn open_parent(parent: &Path) -> fs::File {
        fs::File::from(
            open(
                parent,
                OFlag::O_PATH | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
                Mode::empty(),
            )
            .expect("open trusted parent"),
        )
    }

    #[test]
    fn enosys_uses_handle_relative_portable_opener() {
        let parent_path = private_fixture("enosys");
        let child = private_child(&parent_path);
        let parent = open_parent(&parent_path);

        let opened = open_heartbeat_directory_beneath_with_openat2(
            &parent,
            Path::new("heartbeat"),
            || {},
            |_, _| Err(nix::errno::Errno::ENOSYS),
        )
        .expect("ENOSYS falls back to the portable descriptor-relative opener");

        assert_eq!(
            fstat(&opened).expect("opened metadata").st_ino,
            fs::metadata(&child).expect("child metadata").ino()
        );
        fs::remove_dir_all(parent_path).expect("remove fixture");
    }

    #[test]
    fn errors_other_than_enosys_remain_fail_closed() {
        let parent_path = private_fixture("eperm");
        private_child(&parent_path);
        let parent = open_parent(&parent_path);

        let error = open_heartbeat_directory_beneath_with_openat2(
            &parent,
            Path::new("heartbeat"),
            || {},
            |_, _| Err(nix::errno::Errno::EPERM),
        )
        .expect_err("EPERM must not select the portable fallback");

        assert!(error.message.contains("secure resolution failed"));
        fs::remove_dir_all(parent_path).expect("remove fixture");
    }

    #[test]
    fn enosys_fallback_rejects_a_changed_name_binding() {
        let parent_path = private_fixture("race-parent");
        let child = private_child(&parent_path);
        let replacement_path = private_fixture("race-replacement");
        let replacement = private_child(&replacement_path);
        let displaced = parent_path.join("displaced");
        let parent = open_parent(&parent_path);

        let result = open_heartbeat_directory_beneath_with_openat2(
            &parent,
            Path::new("heartbeat"),
            || {
                fs::rename(&child, &displaced).expect("displace original child");
                fs::rename(&replacement, &child).expect("install replacement child");
            },
            |_, _| Err(nix::errno::Errno::ENOSYS),
        );

        assert!(result.is_err(), "changed name binding was accepted");
        fs::remove_dir_all(parent_path).expect("remove parent fixture");
        fs::remove_dir_all(replacement_path).expect("remove replacement fixture");
    }
}
