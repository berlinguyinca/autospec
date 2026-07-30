# Runtime session terminal foreground design

## Problem

`autospec runtime env session` starts both its worker and the requested harness in
new Unix process groups. The invoking process group remains the terminal foreground
owner, so an interactive harness such as Codex is stopped by `SIGTTIN` when it reads
stdin. The supervisor then waits indefinitely for a stopped descendant.

## Design

Before spawning a grouped child, capture the current foreground process group when
stdin is a controlling terminal owned by the caller. In the child setup path:

1. create the child's process group;
2. temporarily block `SIGTTOU`;
3. make that group the terminal foreground owner;
4. restore the original signal mask; and
5. execute the child.

Doing the handoff before `exec` makes nested ownership deterministic: the outer
supervisor hands the terminal to the worker, then the worker hands it to the
harness. After each child exits, its parent restores the process group it captured.
Non-TTY commands retain the existing process-group behavior without a terminal
handoff.

Foreground restoration is explicit on normal paths and best-effort in `Drop` on
early returns. Existing process groups remain intact for signal forwarding and
descendant cleanup.

## Verification

A Unix integration test runs a non-interactive shell inside a real pseudo-terminal.
The shell starts an Autospec runtime session whose child reads one line, then the
shell itself reads a second line after Autospec exits. Both reads must complete:
the first proves the harness owned the terminal and the second proves ownership was
restored.
