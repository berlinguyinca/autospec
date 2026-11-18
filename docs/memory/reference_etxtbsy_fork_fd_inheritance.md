---
name: reference_etxtbsy_fork_fd_inheritance
description: ETXTBSY on a write-then-exec test shim is caused by fork() fd inheritance, not path collision; rename(2) does not fix it — measured
metadata:
  type: reference
---

A recurring `Text file busy (os error 26)` on a write-then-exec test shim (write an
executable fixture, chmod it, then `Command::spawn` it) is **not** a path-uniqueness
problem, even when every writer already derives a unique path (pid + atomic sequence,
the same pattern verified correct elsewhere in this codebase — see
[[feedback_per_session_worktree_isolation]]).

**Real mechanism (confirmed empirically on autospec, issue #3495/PR #3501):**
`execve(2)` refuses a file while *any* open file description still has that inode open
for writing. `Command::spawn` forks first; the child inherits a duplicate of every fd
the parent holds at fork time, because `O_CLOEXEC` only fires at `execve`, not at
`fork`. So a shim written with `fs::write` can have its write descriptor pinned open
inside a completely unrelated child process spawned by another concurrent test — the
two tests need not touch the same file, or even know about each other. This is why
unique-per-test paths don't help: the hazard is fd inheritance across `fork`, not
inode collision across tests.

**`rename(2)` alone does NOT fix it** — measured, not assumed:

| strategy | ETXTBSY / 3200 trials |
|---|---:|
| `fs::write` + `set_permissions` | 933 |
| in-process write + `rename` into place | 598 |
| out-of-process staging + `rename` | **0** |

Renaming changes the published *name*, not the fact that the parent process still has
the inode open for writing at the moment a sibling test's child forks. The only fix
that reached zero was staging the write in a **separate process** (so the writing fd
never exists in the parent that later forks test-runner children), then renaming.

**How to apply:** if a test suite hits `ETXTBSY` on a shim/fixture binary under
parallel `cargo test`, do not reach for unique paths or a bare `rename()` — verify the
fd-inheritance mechanism first (does anything in the same process spawn children after
opening the shim for write?), and fix by moving the write out-of-process. Centralize in
one `write_executable`-style helper other call sites already import, so hardening it
once covers every site — do not fix call sites one at a time; audit for siblings
first (this codebase had ~10 more instances of the same anti-pattern, filed as #3500
rather than fixed inline).
