# Installation

## Ubuntu AppArmor support for Codex

On Linux, the installer probes the network-enabled Codex permission profile
used by autonomous executors before it installs skills. Most hosts need no
change. On Ubuntu systems where
`kernel.apparmor_restrict_unprivileged_userns=1` blocks Bubblewrap with the
known loopback or UID-map errors, the installer adds this targeted profile:

```text
abi <abi/4.0>,

include <tunables/global>

/usr/bin/bwrap flags=(unconfined) {
  userns,

  include if exists <local/usr.bin.bwrap>
}
```

The probe mirrors the autonomous executor's credential denies, secret
environment exclusions, resolved Codex executable grant, and custom
`CODEX_HOME` denies. Codex versions whose `sandbox` subcommand supports
`--ignore-user-config` receive that flag. Older versions run the probe with an
empty temporary `CODEX_HOME`, which provides the same no-user-config property
without hiding the real Codex paths from the deny policy.

The helper creates a root-owned candidate inside `/etc/apparmor.d`, validates
that inode with `apparmor_parser -Q -K` (compile without loading it or writing
the parser cache), atomically links the same inode as
`/etc/apparmor.d/usr.bin.bwrap` with owner `root:root` and mode `0644`, reloads
it, and re-runs the Codex probe. Creation through reload happens inside one
privileged boundary, and trusted ancestors, inode identity, and exact content
are rechecked after validation. The root shell starts through
`/usr/bin/env -i` with only `PATH=/usr/sbin:/usr/bin:/sbin:/bin`, using
`/bin/bash --noprofile --norc -p`, so caller `PATH`, `BASH_ENV`, and exported
functions cannot cross the boundary. It never replaces a profile whose content
differs.

If reload or the post-install probe fails, a newly created profile is unloaded
and removed after another identity/content check. A pre-existing identical
profile is preserved.
Set `AUTOSPEC_SKIP_SYSTEM_TOOLS=1` to disable all privileged profile writes;
installation then stops with `codex_sandbox_host_setup_required` when the host
still blocks Codex.

The `flags=(unconfined) { userns, }` rule is deliberately narrow by executable
path and capability, but it allows `/usr/bin/bwrap` to run unconfined after
creating a user namespace. This trades some AppArmor confinement for the
Bubblewrap sandbox that protects executor credentials and the host filesystem.

If an operator-managed `/etc/apparmor.d/usr.bin.bwrap` already exists, reconcile
it manually instead of overwriting it. To revert an Autospec-installed profile,
first confirm that its content exactly matches the block above, then unload and
remove it:

```bash
sudo apparmor_parser -R /etc/apparmor.d/usr.bin.bwrap
sudo rm /etc/apparmor.d/usr.bin.bwrap
```

Run `bash scripts/ensure-codex-sandbox.sh` to probe and reinstall the profile
after resolving the host policy.

If removal must preserve another Bubblewrap policy, edit that policy manually
and validate it before reloading:

```bash
sudo apparmor_parser -Q -K /etc/apparmor.d/usr.bin.bwrap
sudo apparmor_parser -r /etc/apparmor.d/usr.bin.bwrap
```
