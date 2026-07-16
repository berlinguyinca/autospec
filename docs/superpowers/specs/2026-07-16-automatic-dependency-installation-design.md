# Automatic Dependency Installation Design

**Issue:** #2099

**Status:** approved by the user on 2026-07-16

## Goal

Make a fresh AutoSpec installation install and verify its core local tools
before any installer step consumes them, while keeping optional capability
tools best-effort and supporting privilege escalation for system packages.

## Problem

The root installer already loops over a broad tool list through
`skills/autospec-shared/scripts/ensure-tool.sh`, but that loop runs after
`install_autospec_runtime_binary`. A fresh machine without Cargo therefore
fails at the Rust build before dependency installation begins. Cargo is not in
the shared installer's tool table, and Python 3 is supported by that table but
omitted from the root default list. The current best-effort contract also
returns success when a required tool remains absent, so a nominally successful
installation can leave AutoSpec unusable.

Linux package-manager helpers already select `sudo` for a non-root user when it
is available. That behavior is not covered by tests, and failures are hidden by
redirected output and the always-zero result contract.

## Scope

This change covers the Unix bootstrap and the shared Bash installer used by
macOS, Linux, and Windows Git Bash. It classifies dependencies, moves core
installation ahead of the Rust build, adds Cargo support, verifies the final
state, and documents the behavior.

It does not automatically install feature-specific heavyweight tools such as
Docker, browsers, Playwright browser bundles, CAD solvers, target-language
runtimes, or every security scanner. Those remain owned by the feature that
uses them.

## Dependency contract

| Class | Commands | Completion rule |
| --- | --- | --- |
| Bootstrap | `bash`, `git` | Required before the repository can be cloned. The running shell satisfies `bash`; `bootstrap.sh` attempts to install `git`. |
| Core | `git`, `bash`, `curl`, `cargo`, `python3`, `gh`, `jq` | Install before the Rust build and fail the installation if any command remains absent. |
| Harness | `codex`, `claude`, `opencode` | Attempt the requested/default harness installs; at least one harness command must be present. |
| Recommended | Existing default optional tools, including `yq`, `bun`, `bats`, `ajv`, `mempalace`, and peer ecosystem CLIs | Warn and continue when installation is unavailable or fails. |
| Feature-specific | Scanners, containers, browsers, solvers, and target-project runtimes | Installed or diagnosed by their owning feature, not the root installer. |

`AUTOSPEC_SKIP_SYSTEM_TOOLS=1` disables automatic package-manager changes but
does not disable verification. Required missing commands still produce one
aggregated failure report. Per-tool `AUTOSPEC_SKIP_ENSURE_TOOL_<TOOL>=1`
continues to suppress that tool's install attempt without converting a core
requirement into an optional one.

## Architecture and flow

### Unix bootstrap

`bootstrap.sh` keeps its small pre-clone responsibility. When `git` is absent,
it tries the first available supported manager: Homebrew, APT, DNF, YUM,
Pacman, or APK. Linux managers run directly for root and through `sudo` for a
non-root user when `sudo` exists. If Git remains absent, bootstrap exits with a
diagnostic naming the attempted manager and a manual recovery command.

The script cannot install `curl` for the documented curl-pipe invocation,
because curl has already been used to obtain the script. A local execution of
`bootstrap.sh` still treats curl as a core installer requirement after clone.

### Root installer

`install.sh` separates tool names into required core, harness, and recommended
sets. The flow becomes:

1. Copy installer/runtime assets needed by subsequent steps.
2. Attempt installation of every core command through `ensure-tool.sh`.
3. Refresh process-local PATH state for newly installed tools, including
   `~/.cargo/bin` and `~/.autospec/bin`, without requiring a new shell.
4. Verify all core commands and aggregate missing names.
5. Build and atomically install the Rust `autospec` binary.
6. Attempt harness and recommended dependency installation.
7. Require at least one supported harness command.
8. Continue the existing skill, peer ecosystem, and optional module setup.
9. Print a dependency summary before the existing suite summary.

No later step may run when core verification fails. Recommended-tool failures
remain warnings and do not alter the final exit status.

### Shared tool installer

`ensure-tool.sh` remains the sole package-name and package-manager mapping for
post-clone tools. It gains `cargo` and `rustc` mappings for Homebrew, APT, DNF,
YUM, Pacman, APK, winget, Chocolatey, and Scoop. Cargo installation uses native
packages; this design does not add an unverified curl-to-shell Rust installer.

The helper keeps its public always-zero, best-effort interface so existing
optional callers do not change behavior. Strictness belongs to the root
installer's post-attempt verification, which observes commands rather than
trusting a package-manager exit code.

### Privilege behavior

System package managers use no elevation when `id -u` is zero. Otherwise they
prefix the install command with `sudo` when available. A missing `sudo`, denied
authentication, or a failed package transaction is surfaced by final required
verification with a concise recovery message. User-scoped installers such as
`pip --user` remain unprivileged; npm's existing global behavior is unchanged.

Dry-run mode prints the manager-independent intent and the required verification
set without prompting, elevating, installing, or failing because the current
host lacks a command.

## Error handling

Required failures are collected rather than failing at the first missing tool.
The final error contains:

- every missing required command;
- whether automatic installation was disabled;
- whether no supported package manager or no usable `sudo` path was available;
- the re-runnable installer command after manual recovery.

Package-manager output stays quiet on success. A failed required installation
must emit enough context to identify the manager and tool; secrets and full
environment dumps are never printed. Offline optional installs warn and
continue as they do today.

## Testing

All install tests use isolated temporary homes and fake executables. They must
not invoke a real package manager, network request, or sudo binary.

Focused coverage includes:

- a failing test proving required dependency setup precedes `cargo build`;
- Cargo and Python package mappings for representative Unix and Windows
  managers;
- root execution without sudo and non-root execution through a fake sudo;
- aggregation of multiple missing core commands;
- `AUTOSPEC_SKIP_SYSTEM_TOOLS=1` skipping mutation but retaining verification;
- optional dependency failure remaining non-blocking;
- at-least-one-harness success and zero-harness failure;
- Unix bootstrap installing missing Git through a fake package manager;
- dry-run producing no writes or privilege prompts.

The primary inner-loop command is:

```bash
bash tests/install/test_required_dependencies.sh
```

Before completion, run the ensure-tool Bats suite, install tests, shell syntax
checks, `autospec validate --fast`, and the full repository validation required
by the branch. The untouched branch currently has four reproducible failures in
`autonomous_drain_commands` related to process-group termination; those are a
recorded pre-existing baseline exception and are outside issue #2099.

## Documentation

`README.md` and the contributor setup instructions will distinguish:

- commands automatically installed as core requirements;
- the at-least-one-harness rule;
- optional and feature-specific tooling;
- when and why sudo may prompt;
- dry-run and automatic-install opt-outs;
- recovery when a required dependency cannot be installed.

## Rejected alternatives

1. **Only add Cargo to the existing list.** Rejected because ordering and
   always-success semantics would still allow broken fresh installs.
2. **Create a new dependency manifest consumed by shell and Rust doctor.**
   Rejected for this issue because it adds parsing and synchronization work
   beyond the broken installation path. A later doctor enhancement can consume
   an extracted contract after this behavior is proven.
3. **Move installation into the Rust CLI.** Rejected because Cargo is the first
   missing dependency on a fresh machine, so the CLI cannot bootstrap itself.
4. **Install every feature-specific dependency by default.** Rejected because
   it would add heavyweight, surprising system changes unrelated to the
   operator's selected capabilities.
5. **Install Rust through an unauthenticated curl-to-shell fallback.** Rejected
   in favor of the package managers already trusted by the installer.

## Completion criteria

The work is complete when a simulated fresh host installs core dependencies
before the Rust build, a sudo-capable Linux path is covered without real
elevation, missing requirements fail with one actionable report, optional tools
remain non-blocking, and the documented focused and repository validation gates
pass apart from explicitly reported pre-existing baseline failures.
