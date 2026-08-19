use super::*;

#[cfg(unix)]
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HeartbeatPublicationDurability {
    Unconfirmed,
    Durable,
}

#[cfg(unix)]
#[allow(dead_code)]
#[derive(Debug)]
pub(super) struct HeartbeatPublication {
    pub(super) file: fs::File,
    pub(super) device: u64,
    pub(super) inode: u64,
    pub(super) durability: HeartbeatPublicationDurability,
}

#[cfg(unix)]
#[derive(Debug)]
pub(super) struct PreparedHeartbeat {
    file: fs::File,
    identity: (u64, u64),
    #[cfg(not(target_os = "linux"))]
    temporary_name: String,
}

#[cfg(unix)]
#[allow(dead_code)]
#[derive(Debug)]
pub(super) enum HeartbeatPublicationFailure {
    PreCommit(CommandFailure),
    PostCommit {
        publication: HeartbeatPublication,
        error: CommandFailure,
    },
}

#[cfg(unix)]
pub(super) fn validate_heartbeat_final_name(
    final_name: &str,
) -> Result<(), HeartbeatPublicationFailure> {
    use std::path::Component;

    let mut components = Path::new(final_name).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(name)), None) if name == std::ffi::OsStr::new(final_name) => Ok(()),
        _ => Err(HeartbeatPublicationFailure::PreCommit(
            CommandFailure::diagnostic("heartbeat final name must be one normal component"),
        )),
    }
}

#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HeartbeatFinalBinding {
    Missing,
    Exact,
    Other,
}

#[cfg(unix)]
pub(super) fn heartbeat_final_binding(
    file: &fs::File,
    directory: &impl std::os::fd::AsFd,
    final_name: &str,
    identity: (u64, u64),
) -> Result<(HeartbeatFinalBinding, u64), CommandFailure> {
    use nix::fcntl::AtFlags;
    use nix::sys::stat::{fstat, fstatat};

    let links = fstat(file)
        .map_err(|error| CommandFailure::diagnostic(format!("heartbeat fstat: {error}")))?
        .st_nlink;
    let binding = match fstatat(directory, final_name, AtFlags::AT_SYMLINK_NOFOLLOW) {
        Err(nix::errno::Errno::ENOENT) => HeartbeatFinalBinding::Missing,
        Ok(stat) if (stat.st_dev as u64, stat.st_ino as u64) == identity => {
            HeartbeatFinalBinding::Exact
        }
        Ok(_) => HeartbeatFinalBinding::Other,
        Err(error) => {
            return Err(CommandFailure::diagnostic(format!(
                "could not inspect heartbeat final binding: {error}"
            )))
        }
    };
    Ok((binding, links as u64))
}

#[cfg(unix)]
pub(super) fn post_commit_failure(
    file: fs::File,
    identity: (u64, u64),
    durability: HeartbeatPublicationDurability,
    error: CommandFailure,
) -> HeartbeatPublicationFailure {
    HeartbeatPublicationFailure::PostCommit {
        publication: HeartbeatPublication {
            file,
            device: identity.0,
            inode: identity.1,
            durability,
        },
        error,
    }
}

#[cfg(target_os = "linux")]
pub(super) fn prepare_private_heartbeat_file(
    directory: &impl std::os::fd::AsFd,
    document: &[u8],
    role: &str,
    boundary: &mut impl FnMut(&str, &str) -> Result<(), CommandFailure>,
) -> Result<PreparedHeartbeat, HeartbeatPublicationFailure> {
    use nix::fcntl::{openat, OFlag};
    use nix::sys::stat::{fchmod, fstat, Mode};

    let pre = HeartbeatPublicationFailure::PreCommit;
    private_heartbeat_directory_identity(directory, "publication").map_err(pre)?;
    let descriptor = openat(
        directory,
        ".",
        OFlag::O_TMPFILE | OFlag::O_RDWR | OFlag::O_CLOEXEC,
        Mode::from_bits_truncate(0o600),
    )
    .map_err(|error| {
        pre(CommandFailure::diagnostic(format!(
            "anonymous heartbeat staging is unavailable: {error}"
        )))
    })?;
    let mut file = fs::File::from(descriptor);
    let fail = |message| pre(CommandFailure::diagnostic(message));
    boundary(role, "chmod").map_err(pre)?;
    fchmod(&file, Mode::from_bits_truncate(0o600))
        .map_err(|error| fail(format!("heartbeat chmod: {error}")))?;
    boundary(role, "write").map_err(pre)?;
    file.write_all(document)
        .map_err(|error| fail(format!("heartbeat write: {error}")))?;
    boundary(role, "file-fsync").map_err(pre)?;
    file.sync_all()
        .map_err(|error| fail(format!("heartbeat fsync: {error}")))?;
    let stat = fstat(&file).map_err(|error| fail(format!("heartbeat fstat: {error}")))?;
    boundary(role, "before-link").map_err(pre)?;
    Ok(PreparedHeartbeat {
        file,
        identity: (stat.st_dev as u64, stat.st_ino as u64),
    })
}

#[cfg(all(unix, not(target_os = "linux")))]
pub(super) fn prepare_private_heartbeat_file(
    directory: &impl std::os::fd::AsFd,
    document: &[u8],
    role: &str,
    boundary: &mut impl FnMut(&str, &str) -> Result<(), CommandFailure>,
) -> Result<PreparedHeartbeat, HeartbeatPublicationFailure> {
    use nix::fcntl::{openat, OFlag};
    use nix::sys::stat::{fchmod, fstat, Mode, SFlag};
    use nix::unistd::{fsync, unlinkat, UnlinkatFlags};

    let pre = HeartbeatPublicationFailure::PreCommit;
    private_heartbeat_directory_identity(directory, "publication").map_err(pre)?;
    let temporary_name = format!(
        ".autospec-heartbeat-stage-{}-{}",
        std::process::id(),
        UNIQUE_ID_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let descriptor = openat(
        directory,
        temporary_name.as_str(),
        OFlag::O_CREAT | OFlag::O_EXCL | OFlag::O_RDWR | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
        Mode::from_bits_truncate(0o600),
    )
    .map_err(|error| {
        pre(CommandFailure::diagnostic(format!(
            "could not create private heartbeat staging file: {error}"
        )))
    })?;
    let mut file = fs::File::from(descriptor);
    let cleanup = |error: CommandFailure| {
        let cleanup = unlinkat(
            directory,
            temporary_name.as_str(),
            UnlinkatFlags::NoRemoveDir,
        )
        .and_then(|()| fsync(directory));
        pre(match cleanup {
            Ok(()) => error,
            Err(cleanup) => CommandFailure::diagnostic(format!(
                "{error}; could not durably remove heartbeat staging file: {cleanup}"
            )),
        })
    };
    let prepared = (|| {
        boundary(role, "chmod")?;
        fchmod(&file, Mode::from_bits_truncate(0o600))
            .map_err(|error| CommandFailure::diagnostic(format!("heartbeat chmod: {error}")))?;
        boundary(role, "write")?;
        file.write_all(document)
            .map_err(|error| CommandFailure::diagnostic(format!("heartbeat write: {error}")))?;
        boundary(role, "file-fsync")?;
        file.sync_all()
            .map_err(|error| CommandFailure::diagnostic(format!("heartbeat fsync: {error}")))?;
        let stat = fstat(&file)
            .map_err(|error| CommandFailure::diagnostic(format!("heartbeat fstat: {error}")))?;
        if SFlag::from_bits_truncate(stat.st_mode) & SFlag::S_IFMT != SFlag::S_IFREG
            || stat.st_uid != nix::unistd::geteuid().as_raw()
            || stat.st_mode & 0o7777 != 0o600
            || stat.st_nlink != 1
        {
            return Err(CommandFailure::diagnostic(
                "heartbeat staging file identity is not private and regular",
            ));
        }
        boundary(role, "before-link")?;
        Ok((stat.st_dev as u64, stat.st_ino as u64))
    })();
    match prepared {
        Ok(identity) => Ok(PreparedHeartbeat {
            file,
            identity,
            temporary_name,
        }),
        Err(error) => Err(cleanup(error)),
    }
}

#[cfg(target_os = "linux")]
pub(super) fn publish_prepared_heartbeat_file(
    directory: &impl std::os::fd::AsFd,
    final_name: &str,
    prepared: PreparedHeartbeat,
    role: &str,
    boundary: &mut impl FnMut(&str, &str) -> Result<(), CommandFailure>,
) -> Result<HeartbeatPublication, HeartbeatPublicationFailure> {
    use nix::fcntl::{AtFlags, AT_FDCWD};
    use nix::unistd::{fsync, linkat};
    use std::os::fd::AsRawFd;

    validate_heartbeat_final_name(final_name)?;
    let PreparedHeartbeat { file, identity } = prepared;
    let mut link_error = linkat(&file, "", directory, final_name, AtFlags::AT_EMPTY_PATH).err();
    let (mut binding, mut link_count) =
        match heartbeat_final_binding(&file, directory, final_name, identity) {
            Ok(state) => state,
            Err(error) => {
                return Err(post_commit_failure(
                    file,
                    identity,
                    HeartbeatPublicationDurability::Unconfirmed,
                    error,
                ))
            }
        };
    let unavailable = matches!(
        link_error,
        Some(
            nix::errno::Errno::EPERM
                | nix::errno::Errno::EINVAL
                | nix::errno::Errno::ENOENT
                | nix::errno::Errno::EOPNOTSUPP
        )
    );
    if unavailable && binding == HeartbeatFinalBinding::Missing && link_count == 0 {
        let proc_path = format!("/proc/self/fd/{}", file.as_raw_fd());
        link_error = linkat(
            AT_FDCWD,
            proc_path.as_str(),
            directory,
            final_name,
            AtFlags::AT_SYMLINK_FOLLOW,
        )
        .err();
        (binding, link_count) =
            match heartbeat_final_binding(&file, directory, final_name, identity) {
                Ok(state) => state,
                Err(error) => {
                    return Err(post_commit_failure(
                        file,
                        identity,
                        HeartbeatPublicationDurability::Unconfirmed,
                        error,
                    ))
                }
            };
    }
    if (binding, link_count) != (HeartbeatFinalBinding::Exact, 1) {
        let error = CommandFailure::diagnostic(format!(
            "heartbeat link failed or lost identity: {}",
            link_error.map_or_else(
                || "final binding changed".to_string(),
                |error| error.to_string()
            )
        ));
        if link_error.is_none() || binding == HeartbeatFinalBinding::Exact || link_count > 0 {
            return Err(post_commit_failure(
                file,
                identity,
                HeartbeatPublicationDurability::Unconfirmed,
                error,
            ));
        }
        return Err(HeartbeatPublicationFailure::PreCommit(error));
    }

    if let Err(error) = boundary(role, "directory-fsync").and_then(|()| {
        fsync(directory).map_err(|error| CommandFailure::diagnostic(error.to_string()))
    }) {
        return Err(post_commit_failure(
            file,
            identity,
            HeartbeatPublicationDurability::Unconfirmed,
            error,
        ));
    }
    if let Err(error) = boundary(role, "revalidate").and_then(|()| {
        (heartbeat_final_binding(&file, directory, final_name, identity)?
            == (HeartbeatFinalBinding::Exact, 1))
            .then_some(())
            .ok_or_else(|| CommandFailure::diagnostic("heartbeat final identity changed"))
    }) {
        return Err(post_commit_failure(
            file,
            identity,
            HeartbeatPublicationDurability::Unconfirmed,
            error,
        ));
    }
    Ok(HeartbeatPublication {
        file,
        device: identity.0,
        inode: identity.1,
        durability: HeartbeatPublicationDurability::Durable,
    })
}

#[cfg(all(unix, not(target_os = "linux")))]
pub(super) fn publish_prepared_heartbeat_file(
    directory: &impl std::os::fd::AsFd,
    final_name: &str,
    prepared: PreparedHeartbeat,
    role: &str,
    boundary: &mut impl FnMut(&str, &str) -> Result<(), CommandFailure>,
) -> Result<HeartbeatPublication, HeartbeatPublicationFailure> {
    use nix::fcntl::AtFlags;
    use nix::unistd::{fsync, linkat, unlinkat, UnlinkatFlags};

    validate_heartbeat_final_name(final_name)?;
    let PreparedHeartbeat {
        file,
        identity,
        temporary_name,
    } = prepared;
    let expected = file
        .try_clone()
        .and_then(read_regular_file)
        .map_err(|error| {
            HeartbeatPublicationFailure::PreCommit(CommandFailure::diagnostic(format!(
                "could not read prepared heartbeat: {error}"
            )))
        })?;
    let link_result = linkat(
        directory,
        temporary_name.as_str(),
        directory,
        final_name,
        AtFlags::empty(),
    );
    let linked = link_result.is_ok();
    let cleanup_temporary = || {
        unlinkat(
            directory,
            temporary_name.as_str(),
            UnlinkatFlags::NoRemoveDir,
        )
        .map_err(|error| {
            CommandFailure::diagnostic(format!(
                "could not remove private heartbeat staging file: {error}"
            ))
        })?;
        fsync(directory).map_err(|error| {
            CommandFailure::diagnostic(format!("could not sync heartbeat staging cleanup: {error}"))
        })
    };
    if !linked {
        let existing = inspect_heartbeat_target(directory, final_name, &expected.document);
        let cleanup = cleanup_temporary();
        return match (link_result, existing, cleanup) {
            (Err(nix::errno::Errno::EEXIST), Ok(Some(publication)), Ok(())) => {
                let binding = heartbeat_final_binding(
                    &publication.file,
                    directory,
                    final_name,
                    (publication.device, publication.inode),
                );
                let content = publication.file.try_clone().and_then(read_regular_file);
                if binding.ok() == Some((HeartbeatFinalBinding::Exact, 1))
                    && content.is_ok_and(|snapshot| {
                        same_startup_heartbeat_generation(&snapshot.document, &expected.document)
                    })
                {
                    Ok(publication)
                } else {
                    Err(HeartbeatPublicationFailure::PreCommit(
                        CommandFailure::diagnostic(
                            "concurrent heartbeat publication changed during inspection",
                        ),
                    ))
                }
            }
            (error, _, Err(cleanup)) => Err(HeartbeatPublicationFailure::PreCommit(
                CommandFailure::diagnostic(format!(
                    "heartbeat publish failed ({error:?}); {cleanup}"
                )),
            )),
            (Err(error), _, Ok(())) => Err(HeartbeatPublicationFailure::PreCommit(
                CommandFailure::diagnostic(format!("heartbeat link failed: {error}")),
            )),
            _ => unreachable!(),
        };
    }
    if let Err(error) = cleanup_temporary() {
        return Err(post_commit_failure(
            file,
            identity,
            HeartbeatPublicationDurability::Unconfirmed,
            error,
        ));
    }
    if let Err(error) = boundary(role, "directory-fsync").and_then(|()| {
        fsync(directory).map_err(|error| CommandFailure::diagnostic(error.to_string()))
    }) {
        return Err(post_commit_failure(
            file,
            identity,
            HeartbeatPublicationDurability::Unconfirmed,
            error,
        ));
    }
    if let Err(error) = boundary(role, "revalidate").and_then(|()| {
        (heartbeat_final_binding(&file, directory, final_name, identity)?
            == (HeartbeatFinalBinding::Exact, 1))
            .then_some(())
            .ok_or_else(|| CommandFailure::diagnostic("heartbeat final identity changed"))
    }) {
        return Err(post_commit_failure(
            file,
            identity,
            HeartbeatPublicationDurability::Unconfirmed,
            error,
        ));
    }
    Ok(HeartbeatPublication {
        file,
        device: identity.0,
        inode: identity.1,
        durability: HeartbeatPublicationDurability::Durable,
    })
}

#[cfg(target_os = "linux")]
#[allow(dead_code)]
pub(super) fn publish_private_heartbeat_file(
    directory: &impl std::os::fd::AsFd,
    final_name: &str,
    document: &[u8],
    role: &str,
    boundary: &mut impl FnMut(&str, &str) -> Result<(), CommandFailure>,
) -> Result<HeartbeatPublication, HeartbeatPublicationFailure> {
    validate_heartbeat_final_name(final_name)?;
    let prepared = prepare_private_heartbeat_file(directory, document, role, boundary)?;
    publish_prepared_heartbeat_file(directory, final_name, prepared, role, boundary)
}

#[cfg(all(unix, not(target_os = "linux")))]
#[allow(dead_code)]
pub(super) fn publish_private_heartbeat_file(
    directory: &impl std::os::fd::AsFd,
    final_name: &str,
    document: &[u8],
    role: &str,
    boundary: &mut impl FnMut(&str, &str) -> Result<(), CommandFailure>,
) -> Result<HeartbeatPublication, HeartbeatPublicationFailure> {
    validate_heartbeat_final_name(final_name)?;
    let prepared = prepare_private_heartbeat_file(directory, document, role, boundary)?;
    publish_prepared_heartbeat_file(directory, final_name, prepared, role, boundary)
}
