# Installation

## Reference files

A skill body may delegate a phase to a file under `skills/<skill>/references/`,
using a `**MUST** read skills/<skill>/references/<file>` pointer. Those pointers
name repository-relative paths, so they resolve during development but not on a
machine that only has the skill installed — and a run normally happens inside a
target repository, not inside an autospec checkout.

The installers therefore ship each reference file to three places:

| destination | applies to |
|---|---|
| `~/.autospec/skills/<skill>/references/` | always — harness-neutral, and the only copy OpenCode can reach, since it installs a flat `agent/<skill>.md` with no per-skill directory |
| `$CLAUDE_CONFIG_DIR/skills/<skill>/references/` | `--harness claude` or `all` |
| `$CODEX_HOME/skills/<skill>/references/` | `--harness codex` or `all` |

Each skill declares its own list in `skills/<skill>/install.sh` as
`SKILL_REFERENCE_FILES`, and `check_reference_pointer_integrity` fails the build
when a file under `references/` is missing from that list. Hidden entries such as
`.gitkeep` are placeholders and are not shipped.

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

Set `AUTOSPEC_SKIP_RUNTIME_BINARY=1` to make `autospec-runtime-install.sh` a declared
no-op, skipping the `cargo build --release` and the runtime binary install. Intended for tests that drive the interactive prompts
under a substituted `HOME`, where the install receipt cannot hit its cache and a
cold release build would dominate the run. An install that skips it leaves any
previously installed `autospec` binary untouched.

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
