# Host-local changes through ambiguous names (issue #3683)

A name in ssh config may be a round-robin alias over several machines, each
with its own crontab, and `ssh name` lands on a different one every time. A
host-local change (crontab, systemd unit, `/etc` file, installed binary) made
through such a name has a scope question before it has a content question:

- **Resolve the name first** (`dig +short` / `getent hosts`): more than one
  address is a host set, not a host.
- **Address the change to a specific host** and record which host in the
  change's own note — `hostname` at the start of a host-modifying session.
- **Verify on every host in the set.** A read-back through the alias is not
  verification: the read may reach a different machine than the write, with
  nothing in the output distinguishing the two. Never trust a read-back that
  could have come from somewhere else.
- **Diff the hosts before assuming they are copies** — "identical" is
  observed, never presumed.

Checkable in `autospec_core::host_set` (`resolve`, `AddressedChange::record`,
`VerificationLedger`, `diff_hosts`).
