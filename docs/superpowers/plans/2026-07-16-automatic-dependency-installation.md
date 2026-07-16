# Automatic Dependency Installation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Install and verify AutoSpec's required local tools before the Rust runtime build while preserving best-effort optional tooling and sudo-capable system package installation.

**Architecture:** Extend the existing `ensure-tool.sh` package table instead of adding another resolver. Split `install.sh` into required, harness, and recommended phases; strictness comes from post-install `command -v` verification, while the shared helper retains its compatible always-zero contract. Keep the pre-clone Unix Git bootstrap small and independently tested with fake package-manager executables.

**Tech Stack:** Bash 3.2-compatible shell, Bats, fake-PATH integration tests, Homebrew/APT/DNF/YUM/Pacman/APK/winget/Chocolatey/Scoop.

## Global Constraints

- Work only in `/tmp/wt-feat-automatic-dependency-installation` on `feat/automatic-dependency-installation` for issue #2099.
- Do not add dependencies or run real package managers, network installers, or `sudo` from tests.
- Keep `ensure-tool.sh` always-zero for compatibility; `install.sh` owns required-tool failure.
- `AUTOSPEC_SKIP_SYSTEM_TOOLS=1` skips install attempts but not required verification.
- Dry-run performs no writes, privilege prompts, installation, or host-dependent failure.
- At least one of `codex`, `claude`, or `opencode` must exist after harness installation attempts.
- Optional and feature-specific tools remain non-blocking.
- Shell code must remain Bash 3.2-compatible and pass `bash -n`.

---

### Task 1: Add Rust toolchain and sudo coverage to the shared installer

**Files:**
- Modify: `skills/autospec-shared/scripts/ensure-tool.sh:1-264`
- Test: `skills/autospec-shared/tests/unit/ensure-tool.bats:1-266`

**Interfaces:**
- Consumes: existing `_sudo_cmd` and `_try_<manager>` helpers.
- Produces: `ensure-tool.sh cargo` and `ensure-tool.sh rustc`, both using the existing always-zero contract.

- [ ] **Step 1: Write failing Cargo and sudo tests**

Add fake identity and sudo helpers plus these cases to `ensure-tool.bats`:

```bash
mk_id() {
  local uid="$1"
  cat > "$BIN/id" <<SHIM
#!/usr/bin/env bash
[ "\${1:-}" = "-u" ] && printf '%s\n' '$uid'
SHIM
  chmod +x "$BIN/id"
}

mk_sudo() {
  cat > "$BIN/sudo" <<SHIM
#!/usr/bin/env bash
echo "sudo \$*" >> "$LOG"
exec "\$@"
SHIM
  chmod +x "$BIN/sudo"
}

@test "cargo absent + apt + non-root uses sudo to install cargo and rustc" {
  mk_id 1000
  mk_sudo
  mk_installer apt-get
  run_ensure_isolated cargo
  [ "$status" -eq 0 ]
  grep -q "sudo apt-get install -y cargo rustc" "$LOG"
  grep -q "apt-get install -y cargo rustc" "$LOG"
}

@test "cargo absent + apt + root installs without sudo" {
  mk_id 0
  mk_installer apt-get
  run_ensure_isolated cargo
  [ "$status" -eq 0 ]
  grep -q "apt-get install -y cargo rustc" "$LOG"
  ! grep -q '^sudo ' "$LOG"
}
```

- [ ] **Step 2: Run the tests and confirm the missing Cargo mapping**

Run: `bats skills/autospec-shared/tests/unit/ensure-tool.bats`

Expected: the new Cargo tests fail because the `case "$TOOL"` table has no `cargo|rustc` arm.

- [ ] **Step 3: Add the minimal package mappings**

Add `cargo` and `rustc` to the supported-tools comment and add this case before `curl)`:

```bash
  cargo|rustc)
    _try_brew rust || _try_apt cargo rustc || _try_dnf cargo rust || _try_yum cargo rust \
      || _try_pacman rust || _try_apk cargo rust || _try_winget Rustlang.Rustup \
      || _try_choco rustup.install || _try_scoop rustup || true
    ;;
```

- [ ] **Step 4: Run focused verification**

Run:

```bash
bats skills/autospec-shared/tests/unit/ensure-tool.bats
bash -n skills/autospec-shared/scripts/ensure-tool.sh
```

Expected: all ensure-tool tests pass and `bash -n` exits 0.

- [ ] **Step 5: Commit the tool mapping**

```bash
git add skills/autospec-shared/scripts/ensure-tool.sh skills/autospec-shared/tests/unit/ensure-tool.bats
git commit -m "fix: make the Rust prerequisite installable"
```

---

### Task 2: Enforce required dependencies before the Rust build

**Files:**
- Modify: `install.sh:43-56,539-556,1620-1640`
- Create: `tests/install/test_required_dependencies.sh`
- Modify: `tests/install/test_ecosystem_bootstrap_dry_run.sh:5-25`

**Interfaces:**
- Consumes: `ensure-tool.sh <command>` from Task 1 and `command_present` from `scripts/lib/install-helpers.sh`.
- Produces: `ensure_required_system_tools`, `verify_required_system_tools`, `refresh_dependency_path`, and `verify_harness_tools` shell functions.

- [ ] **Step 1: Write the failing integration test**

Create `tests/install/test_required_dependencies.sh` with an isolated fake HOME/PATH. The test must assert:

```bash
dry_output=$(AUTOSPEC_SKIP_AGENT_ENV_ALIASES=1 bash "$ROOT/install.sh" --dry-run --skill autospec --harness codex 2>&1)
required_line=$(printf '%s\n' "$dry_output" | grep -n 'ensure_required_system_tools' | head -1 | cut -d: -f1)
build_line=$(printf '%s\n' "$dry_output" | grep -n 'install_autospec_runtime_binary: cargo build' | head -1 | cut -d: -f1)
[ "$required_line" -lt "$build_line" ] || fail "required tools were not ensured before cargo build"

set +e
missing_output=$(HOME="$FAKE_HOME" PATH="$ISOLATED_BIN" AUTOSPEC_SKIP_SYSTEM_TOOLS=1 \
  bash "$ROOT/install.sh" --skill autospec --harness codex 2>&1)
missing_status=$?
set -e
[ "$missing_status" -ne 0 ] || fail "missing required tools did not fail installation"
for tool in git curl cargo python3 gh jq; do
  case "$missing_output" in *"$tool"*) ;; *) fail "missing report omitted $tool" ;; esac
done
case "$missing_output" in *install_autospec_runtime_binary*) fail "cargo build ran after required verification failed" ;; esac
```

Build `ISOLATED_BIN` using the established `tests/install/test_codex_check.sh` pattern: symlink host commands except `git`, `curl`, `cargo`, `python3`, `gh`, `jq`, `codex`, `claude`, and `opencode`; keep `bash`, coreutils, and text utilities available. Clean the temporary home and bin with one EXIT trap.

- [ ] **Step 2: Run the integration test and confirm both failures**

Run: `bash tests/install/test_required_dependencies.sh`

Expected: FAIL because `ensure_required_system_tools` does not exist and the existing installer reaches Cargo before strict verification.

- [ ] **Step 3: Split dependency sets and add strict verification**

Replace the single default list with:

```bash
AUTOSPEC_REQUIRED_SYSTEM_TOOLS="${AUTOSPEC_REQUIRED_SYSTEM_TOOLS:-git bash curl cargo python3 gh jq}"
AUTOSPEC_HARNESS_TOOLS="${AUTOSPEC_HARNESS_TOOLS:-codex claude opencode}"
AUTOSPEC_SYSTEM_TOOLS="${AUTOSPEC_SYSTEM_TOOLS:-yq node npm bun bats omx omc oh-my-opencode mempalace ajv}"
```

Add functions that reuse the shared installer and aggregate failures. Maintain
space-delimited `DEPENDENCIES_PRESENT`, `DEPENDENCIES_INSTALLED`, and
`DEPENDENCIES_OPTIONAL_MISSING` values by checking each command before and
after its install attempt:

```bash
refresh_dependency_path() {
    for dependency_bin in "$HOME/.cargo/bin" "$HOME/.autospec/bin"; do
        [ -d "$dependency_bin" ] || continue
        case ":$PATH:" in *":$dependency_bin:"*) ;; *) PATH="$dependency_bin:$PATH" ;; esac
    done
    export PATH
    hash -r 2>/dev/null || true
}

verify_required_system_tools() {
    missing_required=""
    for tool in $AUTOSPEC_REQUIRED_SYSTEM_TOOLS; do
        command_present "$tool" || missing_required="$missing_required $tool"
    done
    [ -z "$missing_required" ] && return 0
    err "required AutoSpec commands remain missing:$missing_required"
    [ "${AUTOSPEC_SKIP_SYSTEM_TOOLS:-0}" = "1" ] && err "automatic installation was disabled by AUTOSPEC_SKIP_SYSTEM_TOOLS=1"
    err "install the missing commands and rerun: bash install.sh --skill $SKILL_ARG --harness $HARNESS_ARG"
    return 1
}

ensure_required_system_tools() {
    if [ "$DRY_RUN" -eq 1 ]; then
        info "[dry-run] ensure_required_system_tools: would ensure and verify $AUTOSPEC_REQUIRED_SYSTEM_TOOLS"
        return 0
    fi
    if [ "${AUTOSPEC_SKIP_SYSTEM_TOOLS:-0}" != "1" ]; then
        for tool in $AUTOSPEC_REQUIRED_SYSTEM_TOOLS; do bash "$ensure_tool" "$tool"; done
    fi
    refresh_dependency_path
    verify_required_system_tools
}
```

Resolve `ensure_tool` once from `skills/autospec-shared/scripts/ensure-tool.sh`. Extend the optional loop to include `$AUTOSPEC_HARNESS_TOOLS $AUTOSPEC_SYSTEM_TOOLS`. Add `verify_harness_tools` that returns success when any harness is present, reports all three names otherwise, and no-ops during dry-run.

Add `print_dependency_summary` before the suite summary. It prints the three
tracked categories, plus `required missing: none` on success. On strict failure,
`verify_required_system_tools` prints the aggregated required-missing category
before returning nonzero. Extend the integration test to assert the successful
dry-run mentions all dependency phases and the failure output contains
`required missing:`.

- [ ] **Step 4: Reorder the root flow**

After copying runtime assets and establishing `~/.autospec/bin`, call `ensure_required_system_tools`; only then call `install_autospec_runtime_binary`. Run optional/harness installation afterward, then call `verify_harness_tools` before peer ecosystems and Turbo.

- [ ] **Step 5: Run focused install tests**

Run:

```bash
bash tests/install/test_required_dependencies.sh
bash tests/install/test_ecosystem_bootstrap_dry_run.sh
bash tests/install/test_install_dry_run.sh
bash -n install.sh
```

Expected: every command prints `PASS` where applicable and exits 0.

- [ ] **Step 6: Commit strict root verification**

```bash
git add install.sh tests/install/test_required_dependencies.sh tests/install/test_ecosystem_bootstrap_dry_run.sh
git commit -m "fix: fail before build when core tools are unavailable"
```

---

### Task 3: Bootstrap missing Git with native privilege handling

**Files:**
- Modify: `bootstrap.sh:23-42`
- Create: `tests/install/test_bootstrap_dependencies.sh`

**Interfaces:**
- Consumes: native package managers on PATH and the same root-versus-sudo policy as `ensure-tool.sh`.
- Produces: `ensure_bootstrap_git`, which guarantees `git` exists or exits before clone/update.

- [ ] **Step 1: Write a fake-APT bootstrap test**

Create a fake `id`, `sudo`, and `apt-get` under a temporary PATH. `id -u` prints `1000`; `sudo` records and executes its arguments; `apt-get install -y git` creates an executable fake Git command. Prepare `$AUTOSPEC_HOME/repo/.git` and a fake `$AUTOSPEC_HOME/repo/install.sh` that records invocation, then assert:

```bash
PATH="$BIN:$CORE" AUTOSPEC_HOME="$HOME_DIR/.autospec" bash "$ROOT/bootstrap.sh" --dry-run
grep -q '^sudo apt-get install -y git$' "$LOG"
grep -q '^install --dry-run$' "$LOG"
```

The fake Git command must return success for `fetch`, `checkout`, `merge`, and `symbolic-ref` so no network or repository mutation occurs.

- [ ] **Step 2: Run the test and confirm bootstrap rejects missing Git**

Run: `bash tests/install/test_bootstrap_dependencies.sh`

Expected: FAIL with the current `git is required but not on PATH` diagnostic.

- [ ] **Step 3: Implement minimal Git installation in `bootstrap.sh`**

Add root/sudo selection and a manager chain before `require git`:

```bash
install_bootstrap_git() {
    sudo_cmd=""
    if [ "$(id -u 2>/dev/null || printf '1')" != "0" ] && command -v sudo >/dev/null 2>&1; then
        sudo_cmd="sudo"
    fi
    if command -v brew >/dev/null 2>&1; then brew install git
    elif command -v apt-get >/dev/null 2>&1; then $sudo_cmd apt-get install -y git
    elif command -v dnf >/dev/null 2>&1; then $sudo_cmd dnf install -y git
    elif command -v yum >/dev/null 2>&1; then $sudo_cmd yum install -y git
    elif command -v pacman >/dev/null 2>&1; then $sudo_cmd pacman -Sy --noconfirm git
    elif command -v apk >/dev/null 2>&1; then $sudo_cmd apk add --no-cache git
    else return 1
    fi
}

if ! command -v git >/dev/null 2>&1; then
    info "git not found; attempting installation"
    install_bootstrap_git || true
fi
require git
```

Do not alter the Bash requirement: the script is already executing under Bash.

- [ ] **Step 4: Verify bootstrap behavior and syntax**

Run:

```bash
bash tests/install/test_bootstrap_dependencies.sh
bash -n bootstrap.sh
```

Expected: the test prints `PASS`; syntax check exits 0.

- [ ] **Step 5: Commit Unix bootstrap support**

```bash
git add bootstrap.sh tests/install/test_bootstrap_dependencies.sh
git commit -m "fix: bootstrap Git through native package managers"
```

---

### Task 4: Document behavior and run release-level verification

**Files:**
- Modify: `README.md:150-215`
- Modify: `CONTRIBUTING.md:15-27`

**Interfaces:**
- Consumes: final behavior and environment variable names from Tasks 1-3.
- Produces: operator and contributor documentation with exact recovery and opt-out commands.

- [ ] **Step 1: Update public installation documentation**

Document this exact split in README and CONTRIBUTING:

```text
Automatically required: Bash, Git, curl, Cargo/Rust, Python 3, GitHub CLI, jq.
Harness requirement: at least one of Codex CLI, Claude Code, or OpenCode.
Linux system packages may prompt through sudo; root installs do not use sudo.
AUTOSPEC_SKIP_SYSTEM_TOOLS=1 skips package changes but still verifies requirements.
Optional capability tools warn without failing installation.
```

Include the recovery command `bash install.sh --skill all --harness all` after manually installing anything listed in the aggregated error.

- [ ] **Step 2: Run formatting and focused suites**

Run:

```bash
git diff --check
bash -n bootstrap.sh install.sh skills/autospec-shared/scripts/ensure-tool.sh
bats skills/autospec-shared/tests/unit/ensure-tool.bats
bash tests/install/test_required_dependencies.sh
bash tests/install/test_bootstrap_dependencies.sh
for test_file in tests/install/*.sh; do bash "$test_file"; done
```

Expected: every command exits 0 with no skipped test hidden from the report.

- [ ] **Step 3: Run repository validation**

Run:

```bash
autospec validate --fast
bash scripts/validate-launch-readiness.sh
cargo test --workspace
```

Expected: dependency and validation checks pass. If the four untouched `autonomous_drain_commands` process-group failures recur, record them separately as the known baseline exception; do not claim the full workspace suite passed.

- [ ] **Step 4: Commit documentation and validation adjustments**

```bash
git add README.md CONTRIBUTING.md
git commit -m "docs: make dependency bootstrap behavior explicit"
```

- [ ] **Step 5: Final branch review**

Run:

```bash
git status --short
git diff origin/main...HEAD --check
git log --oneline origin/main..HEAD
```

Expected: clean status, no whitespace errors, design plus focused implementation commits only.
