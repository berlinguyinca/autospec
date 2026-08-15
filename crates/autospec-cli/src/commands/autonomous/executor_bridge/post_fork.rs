// executor_bridge/post_fork.rs — the supervisor primitives that run in the forked child.
//
// Everything here executes between fork() and execve, or in the supervisor process the child
// becomes. In that window only async-signal-safe operations are legal: the child inherits a
// copy of the parent's address space but none of its threads, so any lock those threads held
// is held forever, and the allocator, formatting machinery and std::io are all off limits.
// That is why these are raw libc calls behind `unsafe` rather than ordinary Rust, and it is
// the invariant every block below rests on.
//
// Two consequences worth stating once. Every descriptor and buffer these functions touch is
// created and sized before the fork, so no allocation happens after it. And nothing here may
// return to Rust on the child path — the exits go through terminate_post_fork, which issues
// exit_group directly rather than unwinding.
//
// Split out of executor_bridge.rs, which is 24,448 lines; the file may not grow, so the
// invariants below could not be written down where the code used to live.

use super::*;

/// # Safety
///
/// Must run only in a Darwin post-fork child. `descriptor_limit` and every descriptor in
/// `preserved` must have been captured before fork. The implementation performs no allocation
/// or locking and uses only the async-signal-safe `close` syscall.
#[cfg(target_os = "macos")]
pub(super) unsafe fn raw_close_unintended_descriptors(
    descriptor_limit: i32,
    preserved: &[i32],
) {
    for descriptor in nix::libc::STDERR_FILENO + 1..descriptor_limit {
        if !preserved.contains(&descriptor) {
            // SAFETY: this is the isolated child descriptor table; EBADF is harmless.
            unsafe { nix::libc::close(descriptor) };
        }
    }
}

#[cfg(target_os = "linux")]
unsafe fn raw_errno() -> i32 {
    // SAFETY: errno is thread-local process state.
    unsafe { *nix::libc::__errno_location() }
}

#[cfg(target_os = "linux")]
unsafe fn raw_sync(descriptor: i32) -> i32 {
    // SAFETY: descriptor names an owned regular file.
    unsafe { nix::libc::fdatasync(descriptor) }
}

#[cfg(target_os = "macos")]
unsafe fn raw_sync(descriptor: i32) -> i32 {
    // SAFETY: Darwin fsync supplies the durability fence for the owned regular file.
    unsafe { nix::libc::fsync(descriptor) }
}

#[cfg(target_os = "macos")]
unsafe fn raw_errno() -> i32 {
    // SAFETY: __error returns the current thread's errno pointer on Darwin.
    unsafe { *nix::libc::__error() }
}

/// # Safety
///
/// `descriptor` must be a writable file descriptor owned by this process, and
/// `bytes` must remain valid for the duration of the call. Short writes are
/// retried and EINTR is resumed, so the caller sees all-or-nothing.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) unsafe fn raw_pwrite_all(descriptor: i32, mut bytes: &[u8], mut offset: u64) -> bool {
    while !bytes.is_empty() {
        // SAFETY: the caller owns descriptor and the slice remains valid for this syscall.
        let count = unsafe {
            nix::libc::pwrite(
                descriptor,
                bytes.as_ptr().cast(),
                bytes.len(),
                offset as nix::libc::off_t,
            )
        };
        if count < 0 {
            // SAFETY: errno is thread-local process state.
            if unsafe { raw_errno() } == nix::libc::EINTR {
                continue;
            }
            return false;
        }
        if count == 0 {
            return false;
        }
        let count = count as usize;
        bytes = &bytes[count..];
        offset += count as u64;
    }
    true
}

/// # Safety
///
/// `descriptor` must be a writable descriptor whose ring slot layout matches
/// `encode_output_cursor`; the record is fixed-size and written at a slot offset
/// derived from `generation`, so it never runs past the region reserved for it.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) unsafe fn raw_persist_output_cursor(
    descriptor: i32,
    generation: u64,
    total: u64,
    dropped: u64,
) -> bool {
    let cursor = OutputCursor {
        generation,
        total,
        dropped,
    };
    let record = encode_output_cursor(cursor);
    let slot = generation % 2;
    // SAFETY: descriptor and record are owned by the supervisor child.
    (unsafe { raw_pwrite_all(descriptor, &record, slot * OUTPUT_CURSOR_SLOT_BYTES) })
        // SAFETY: fdatasync operates on the same owned regular file descriptor.
        && unsafe { raw_sync(descriptor) } == 0
}

/// # Safety
///
/// `pipe_fd` and `ring_fd` must be owned readable and writable descriptors. The
/// buffer is stack-allocated here and sized before the fork, so the pump performs
/// no allocation on the post-fork path.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) unsafe fn raw_pump_stream(
    pipe_fd: i32,
    ring_fd: i32,
    cursor_fd: i32,
    total: &mut u64,
    generation: &mut u64,
) -> i32 {
    let mut buffer = [0_u8; 65_536];
    let count = loop {
        // SAFETY: buffer is writable and the pipe descriptor is nonblocking.
        let count = {
            #[cfg(test)]
            if launch_child_failpoint() == LaunchFailpoint::RingReadInterrupted as u8
                && RAW_READ_INTERRUPTED_ONCE.swap(1, Ordering::SeqCst) == 0
            {
                // SAFETY: errno is thread-local process state in this post-fork supervisor.
                #[cfg(target_os = "linux")]
                unsafe {
                    *nix::libc::__errno_location() = nix::libc::EINTR
                };
                #[cfg(target_os = "macos")]
                unsafe {
                    *nix::libc::__error() = nix::libc::EINTR
                };
                -1
            } else {
                // SAFETY: buffer is a live stack array and len() is its exact capacity, so the
                // kernel writes at most that many bytes into memory this frame owns.
                unsafe { nix::libc::read(pipe_fd, buffer.as_mut_ptr().cast(), buffer.len()) }
            }
            #[cfg(not(test))]
            {
                // SAFETY: buffer is a live stack array and len() is its exact capacity, so the
                // kernel writes at most that many bytes into memory this frame owns.
                unsafe { nix::libc::read(pipe_fd, buffer.as_mut_ptr().cast(), buffer.len()) }
            }
        };
        if count >= 0 {
            break count;
        }
        // SAFETY: errno is thread-local process state.
        let error = unsafe { raw_errno() };
        if error == nix::libc::EINTR {
            continue;
        }
        if error == nix::libc::EAGAIN || error == nix::libc::EWOULDBLOCK {
            return -1;
        }
        return -2;
    };
    if count == 0 {
        return 0;
    }
    let count = count as usize;
    let position = *total % OUTPUT_SINK_LIMIT;
    let first = count.min((OUTPUT_SINK_LIMIT - position) as usize);
    // SAFETY: ring offsets are bounded by OUTPUT_SINK_LIMIT and both slices are valid.
    if !unsafe { raw_pwrite_all(ring_fd, &buffer[..first], position) }
        // SAFETY: ring_fd is owned by this supervisor and the slice is a subrange of the
        // live buffer, so raw_pwrite_all receives a descriptor and bytes that outlive it.
        || (first < count && !unsafe { raw_pwrite_all(ring_fd, &buffer[first..count], 0) })
    {
        return -2;
    }
    if launch_child_failpoint() == LAUNCH_FAILPOINT_RING_BEFORE_SYNC
        // SAFETY: the ring descriptor is the preopened private regular file just modified.
        || unsafe { raw_sync(ring_fd) } != 0
    {
        return -2;
    }
    *total = total.saturating_add(count as u64);
    *generation = generation.saturating_add(1);
    let dropped = total.saturating_sub(OUTPUT_SINK_LIMIT);
    // SAFETY: cursor descriptor is a preopened private regular file.
    if !unsafe { raw_persist_output_cursor(cursor_fd, *generation, *total, dropped) } {
        return -2;
    }
    count as i32
}

/// # Safety
///
/// `descriptor` must be a writable descriptor owned by this process and `bytes`
/// must stay valid for the call. Retries on EINTR and partial writes.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) unsafe fn raw_write_all(descriptor: i32, bytes: &[u8]) -> bool {
    let mut offset = 0;
    while offset < bytes.len() {
        // SAFETY: descriptor and the remaining immutable byte slice are valid for write.
        let count = unsafe {
            nix::libc::write(
                descriptor,
                bytes[offset..].as_ptr().cast(),
                bytes.len() - offset,
            )
        };
        if count > 0 {
            offset += count as usize;
        } else if count < 0
            // SAFETY: errno is thread-local process state.
            && unsafe { raw_errno() } == nix::libc::EINTR
        {
            continue;
        } else {
            return false;
        }
    }
    true
}

/// # Safety
///
/// Must run in the forked supervisor, which is the subreaper for the process
/// group it is polling; waitpid is called with WNOHANG so it never blocks.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) unsafe fn raw_children_quiescent() -> i32 {
    loop {
        let mut status = 0_i32;
        // SAFETY: this dedicated subreaper owns every remaining adopted child.
        let waited = unsafe { nix::libc::waitpid(-1, &mut status, nix::libc::WNOHANG) };
        if waited > 0 {
            continue;
        }
        if waited == 0 {
            return 0;
        }
        // SAFETY: errno is thread-local process state.
        let error = unsafe { raw_errno() };
        if error == nix::libc::EINTR {
            continue;
        }
        return if error == nix::libc::ECHILD { 1 } else { -1 };
    }
}

/// # Safety
///
/// Must run in the forked supervisor. `harness_pid` is the one child excluded
/// from adoption, so its dedicated waitpid path keeps the exit status this
/// reaper would otherwise consume.
#[cfg(target_os = "linux")]
pub(super) unsafe fn raw_reap_adopted_children(harness_pid: nix::libc::pid_t) -> i32 {
    loop {
        // WNOWAIT lets the exact harness retain its dedicated waitpid path while terminated
        // grandchildren adopted by this subreaper are collected during a long-running command.
        // SAFETY: an all-zero siginfo_t is a valid initial value; waitid fills it before
        // any field is read, and the struct contains no references or niches.
        let mut info: nix::libc::siginfo_t = unsafe { std::mem::zeroed() };
        // SAFETY: info is writable and these flags observe only terminal child state.
        let observed = unsafe {
            nix::libc::waitid(
                nix::libc::P_ALL,
                0,
                std::ptr::addr_of_mut!(info),
                nix::libc::WEXITED | nix::libc::WNOHANG | nix::libc::WNOWAIT,
            )
        };
        if observed < 0 {
            // SAFETY: errno is thread-local process state.
            let error = unsafe { raw_errno() };
            if error == nix::libc::EINTR {
                continue;
            }
            return if error == nix::libc::ECHILD { 0 } else { -1 };
        }
        // SAFETY: waitid initialized the SIGCHLD pid field for WEXITED observations.
        let pid = unsafe { info.si_pid() };
        if pid == 0 || pid == harness_pid {
            return 0;
        }
        let mut status = 0_i32;
        // SAFETY: pid was returned by waitid as a terminal child and is not the harness.
        let waited = unsafe { nix::libc::waitpid(pid, &mut status, nix::libc::WNOHANG) };
        if waited == pid {
            continue;
        }
        if waited < 0 {
            // SAFETY: errno is thread-local process state.
            let error = unsafe { raw_errno() };
            if error == nix::libc::EINTR || error == nix::libc::ECHILD {
                continue;
            }
        }
        return -1;
    }
}

/// # Safety
///
/// Must be called only on the child side of fork, with every descriptor argument
/// owned and every buffer sized before the fork. It never returns to Rust: all
/// exits go through terminate_post_fork.
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[allow(clippy::too_many_arguments)]
pub(super) unsafe fn raw_supervisor_loop(
    harness_pid: nix::libc::pid_t,
    stdout_pipe: i32,
    stderr_pipe: i32,
    stdout_ring: i32,
    stderr_ring: i32,
    stdout_cursor: i32,
    stderr_cursor: i32,
    exit_status: i32,
    harness_exit: i32,
) -> ! {
    let mut pipe_descriptors = [stdout_pipe, stderr_pipe];
    for descriptor in pipe_descriptors {
        // SAFETY: descriptors are inherited owned pipe reads.
        let flags = unsafe { nix::libc::fcntl(descriptor, nix::libc::F_GETFL) };
        if flags < 0
            // SAFETY: only O_NONBLOCK is added.
            || unsafe {
                nix::libc::fcntl(
                    descriptor,
                    nix::libc::F_SETFL,
                    flags | nix::libc::O_NONBLOCK,
                )
            } < 0
        {
            terminate_post_fork(127);
        }
    }
    let mut totals = [0_u64; 2];
    let mut generations = [1_u64; 2];
    // SAFETY: cursor descriptors are preopened fixed files.
    if !unsafe { raw_persist_output_cursor(stdout_cursor, generations[0], 0, 0) }
        // SAFETY: stderr_cursor is an owned descriptor and the cursor record is fixed
        // size, so the write stays inside the slot reserved for this generation.
        || !unsafe { raw_persist_output_cursor(stderr_cursor, generations[1], 0, 0) }
    {
        terminate_post_fork(127);
    }
    let mut exit_code = None;
    let mut exit_notified = false;
    loop {
        let mut pollfds = [
            nix::libc::pollfd {
                fd: pipe_descriptors[0],
                events: nix::libc::POLLIN | nix::libc::POLLHUP | nix::libc::POLLERR,
                revents: 0,
            },
            nix::libc::pollfd {
                fd: pipe_descriptors[1],
                events: nix::libc::POLLIN | nix::libc::POLLHUP | nix::libc::POLLERR,
                revents: 0,
            },
        ];
        // SAFETY: both pollfd entries are initialized.
        unsafe { nix::libc::poll(pollfds.as_mut_ptr(), pollfds.len() as _, 50) };
        for (index, pollfd) in pollfds.iter().enumerate() {
            if pollfd.revents & (nix::libc::POLLIN | nix::libc::POLLHUP | nix::libc::POLLERR) != 0 {
                loop {
                    // SAFETY: each descriptor tuple is preopened and exclusively owned here.
                    let count = unsafe {
                        if index == 0 {
                            raw_pump_stream(
                                pipe_descriptors[0],
                                stdout_ring,
                                stdout_cursor,
                                &mut totals[0],
                                &mut generations[0],
                            )
                        } else {
                            raw_pump_stream(
                                pipe_descriptors[1],
                                stderr_ring,
                                stderr_cursor,
                                &mut totals[1],
                                &mut generations[1],
                            )
                        }
                    };
                    if count > 0 {
                        continue;
                    }
                    if count == -2 {
                        terminate_post_fork(127);
                    }
                    if count == 0 {
                        nix::libc::close(pipe_descriptors[index]);
                        pipe_descriptors[index] = -1;
                    }
                    break;
                }
            }
        }
        if exit_code.is_none() {
            let mut status = 0_i32;
            // SAFETY: harness_pid is the exact direct child of this supervisor.
            let waited =
                // SAFETY: status is a live local and WNOHANG makes this a non-blocking poll, so
                // the supervisor cannot stall here while holding the harness open.
                unsafe { nix::libc::waitpid(harness_pid, &mut status, nix::libc::WNOHANG) };
            if waited == harness_pid {
                let code = if nix::libc::WIFEXITED(status) {
                    nix::libc::WEXITSTATUS(status)
                } else if nix::libc::WIFSIGNALED(status) {
                    128 + nix::libc::WTERMSIG(status)
                } else {
                    1
                };
                exit_code = Some(code);
            } else if waited < 0
                // SAFETY: errno is thread-local process state.
                && unsafe { raw_errno() } != nix::libc::EINTR
            {
                terminate_post_fork(127);
            }
            #[cfg(target_os = "linux")]
            if exit_code.is_none()
                // SAFETY: the exact harness is excluded from this adopted-child reap.
                && unsafe { raw_reap_adopted_children(harness_pid) } < 0
            {
                terminate_post_fork(127);
            }
        }
        if !exit_notified {
            if let Some(code) = exit_code {
                let code_bytes = code.to_ne_bytes();
                // SAFETY: the conductor owns the read side when attached; a detached
                // conductor is allowed to make this best-effort notification fail.
                let _ = unsafe { raw_write_all(harness_exit, &code_bytes) };
                exit_notified = true;
            }
        }
        if pipe_descriptors[0] < 0 && pipe_descriptors[1] < 0 {
            if let Some(code) = exit_code {
                // SAFETY: this supervisor is the dedicated subreaper for the harness tree.
                let quiescent = unsafe { raw_children_quiescent() };
                if quiescent < 0 {
                    terminate_post_fork(127);
                }
                if quiescent == 0 {
                    continue;
                }
                let mut data = [0_u8; 8];
                data[..4].copy_from_slice(&code.to_ne_bytes());
                data[4..].copy_from_slice(b"EXIT");
                let mut commit = [0_u8; 8];
                commit[..4].copy_from_slice(&code.to_ne_bytes());
                commit[4..].copy_from_slice(b"DONE");
                // SAFETY: ordered records are written to the preopened private status file.
                if !unsafe { raw_pwrite_all(exit_status, &data, 0) }
                    // SAFETY: exit_status is an owned descriptor; fdatasync only flushes it.
                    || unsafe { raw_sync(exit_status) } != 0
                    // SAFETY: exit_status is owned and commit is a live 8-byte local.
                    || !unsafe { raw_pwrite_all(exit_status, &commit, 8) }
                    // SAFETY: exit_status is an owned descriptor; fdatasync only flushes it.
                    || unsafe { raw_sync(exit_status) } != 0
                {
                    terminate_post_fork(127);
                }
                // SAFETY: this is the post-sync completion fence for the attached conductor.
                let _ = unsafe { raw_write_all(harness_exit, b"DONE") };
                // SAFETY: this supervisor exclusively owns the notification descriptor.
                unsafe { nix::libc::close(harness_exit) };
                terminate_post_fork(0);
            }
        }
    }
}
