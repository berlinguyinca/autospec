# Full-suite portability failure repair plan

## Baseline

`cargo test --workspace --no-fail-fast` fails on macOS in executor-bridge and conductor tests. The dominant shared causes are non-portable filesystem aliases, executable paths, process inspection, and Linux-only atomic publication; several later conductor failures cascade from rejecting macOS `/var` as a symlink.

## Tasks

1. Make trusted-path validation recognize only the fixed macOS system aliases (`/tmp`, `/var`, and `/etc`) while continuing to reject arbitrary symlinks, and classify canonical `/private/var/tmp` targets as temporary.
2. Replace hard-coded test executable locations and GNU-only `stat` arguments with portable fixture helpers or shell behavior, preserving the security assertions.
3. Provide a no-replace artifact publication path on non-Linux Unix using create-new plus hard-link publication, matching the existing durable publication contract.
4. Repair draft-release test fixtures so the release receipt is bound before strict-shell access and validate all release crash windows.
5. Correct remaining independent executor contracts: Codex permission-profile selection, trusted Git hook inventory, private artifact cleanup, root-hardening ordering, and macOS process identity.
6. Re-run focused failing tests after each repair, then run `cargo test --workspace --no-fail-fast` and the repository validation scripts.

## Constraints

- Do not weaken rejection of user-controlled symlinks or temporary executables.
- Do not skip platform tests merely to obtain a green suite.
- Preserve durable no-replace and private-file guarantees on every supported platform.
- Keep production changes paired with regression coverage.
