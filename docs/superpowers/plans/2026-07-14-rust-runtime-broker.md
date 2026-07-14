# Rust Runtime Broker Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace `scripts/agent-env.sh` with a Rust `autospec runtime env` command family and remove every live dependency on the shell broker.

**Architecture:** `autospec-core::runtime_env` owns the constrained v1 manifest grammar, deterministic environment identity, sourceable environment-file serialization, and state-file parsing. `autospec-cli::commands::runtime::env` owns command-line parsing, filesystem changes, and typed child-process execution. Installers and skill prompts invoke the Rust binary directly after the installer has built and installed it.

**Tech Stack:** Rust 2021 standard library, existing Cargo workspace, Bats compatibility fixtures, Bash installer wrappers.

## Global Constraints

- Add no dependencies; the manifest parser implements only the documented v1 mapping/list/scalar subset already accepted by the shell broker.
- Preserve `.autospec/runtime.yml` precedence over `.agent-runtime.yml`, canonical repository paths, POSIX `cksum` environment IDs, ordered output, existing exit statuses, and sourceable `export KEY='value'` env files.
- Direct child commands use `Command` argument vectors; only trusted manifest `command` and `down` strings retain `sh -c` because that is the v1 manifest language.
- Update every multi-harness autospec-run body in lock-step before committing.
- Do not retain `scripts/agent-env.sh` after the final reachability gate; thin installer launchers may invoke only the installed `autospec` binary.
- Use `cargo run -q -p autospec-cli -- validate --fast` as the sole validation entry point.

---

## File Structure

- Create: `crates/autospec-core/src/runtime_env.rs` — typed v1 manifest, environment context, state document, parser, deterministic slug/hash, and env-file formatting/parsing.
- Modify: `crates/autospec-core/src/lib.rs` — export `runtime_env`.
- Create: `crates/autospec-core/tests/runtime_env.rs` — parser, precedence, identity, escaping, and state tests.
- Create: `crates/autospec-cli/src/commands/runtime/env.rs` — all six runtime broker subcommands and typed process/file adapters.
- Modify: `crates/autospec-core/src/runtime_env.rs` — explicit replacement of generated state values with caller-supplied legacy environment overrides.
- Modify: `crates/autospec-cli/src/commands/runtime.rs` — dispatch `runtime env` without changing classify/audit behavior.
- Modify: `crates/autospec-cli/src/commands/mod.rs`, `crates/autospec-cli/src/main.rs` — preserve explicit child exit statuses instead of flattening them to exit `2`.
- Modify: `crates/autospec-cli/tests/runtime_commands.rs` — CLI behavior and exit-code integration tests.
- Modify: `tests/agent-env.bats`, `tests/agent-env-install.bats`, `tests/autospec-run-agent-env-contract.bats` — run the Rust command and reject the shell authority.
- Modify: `install.sh` — install the release `autospec` binary and generate direct Rust command launchers.
- Modify: `skills/autospec-run/{SKILL.md,codex/prompt.md,opencode/agent.md}` — lock-step runtime setup/cleanup commands.
- Modify: `docs/cli-reference.md`, `docs/runbooks/agent-runtime-manifest.md` — document the public broker and v1 manifest grammar.
- Delete: `scripts/agent-env.sh` — former runtime authority after parity proof.

## Task 1: Freeze the manifest and environment-state contract

**Files:**
- Create: `crates/autospec-core/src/runtime_env.rs`
- Modify: `crates/autospec-core/src/lib.rs`
- Create: `crates/autospec-core/tests/runtime_env.rs`
- Test: `tests/agent-env.bats`

**Interfaces:**
- Produces: `RuntimeManifest::read_from_repo(&Path) -> Result<Self, RuntimeEnvError>` and `RuntimeContext::new(manifest, repo, requested_mode, state_root) -> Result<Self, RuntimeEnvError>`.
- Produces: `RuntimeState::render_env_file(&self) -> String` and `RuntimeState::from_env_file(&str) -> Result<Self, RuntimeEnvError>`.
- Consumes: canonical repository paths and v1 manifest text; no process execution.

- [ ] **Step 1: Repair the existing canonical-path fixture and add failing Rust parser tests**

Make `tests/agent-env.bats` compare the init announcement with `cd "$repo" && pwd -P` so the fixture locks the existing canonical-path behavior on macOS `/var` → `/private` symlinks. Add tests that fail because `runtime_env` does not exist:

```rust
struct TempRepo { root: PathBuf }

impl TempRepo {
    fn with_files(files: &[(&str, &str)]) -> Self {
        let root = std::env::temp_dir().join(format!("autospec-runtime-env-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for (relative, content) in files {
            let path = root.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, content).unwrap();
        }
        Self { root }
    }
    fn path(&self) -> &Path { &self.root }
}

impl Drop for TempRepo {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.root); }
}

const VALID_AUTOSPEC_MANIFEST: &str = "version: 1\nname: sample-app\ndefault_mode: e2e-local-db\nmodes:\n  e2e-local-db:\n    command: sh -c 'true'\n";
const VALID_AGENT_MANIFEST: &str = "version: 1\nname: fallback\ndefault_mode: local\nmodes:\n  local:\n    command: sh -c 'true'\n";

#[test]
fn manifest_prefers_autospec_path_and_preserves_mode_order() {
    let fixture = TempRepo::with_files(&[
        (".autospec/runtime.yml", VALID_AUTOSPEC_MANIFEST),
        (".agent-runtime.yml", VALID_AGENT_MANIFEST),
    ]);
    let manifest = RuntimeManifest::read_from_repo(fixture.path()).unwrap();
    assert_eq!(manifest.path(), fixture.path().join(".autospec/runtime.yml"));
    assert_eq!(manifest.selected_mode("auto").unwrap().name(), "e2e-local-db");
}

#[test]
fn manifest_rejects_lowercase_environment_names() {
    let error = RuntimeManifest::parse("version: 1\nmodes:\n  local:\n    env:\n      lowercase_port: 1\n").unwrap_err();
    assert!(error.to_string().contains("invalid environment name"));
}
```

- [ ] **Step 2: Run the focused tests and confirm the intended red failure**

Run: `cargo test -p autospec-core --test runtime_env`

Expected: compilation failure naming missing `autospec_core::runtime_env` symbols; `bats tests/agent-env.bats` reports the canonical-path assertion as fixed while retaining the current shell behavior.

- [ ] **Step 3: Implement the constrained v1 core model and parser**

Define only the data and operations needed by the current manifest language:

```rust
pub struct RuntimeManifest {
    path: PathBuf,
    name: Option<String>,
    default_mode: Option<String>,
    modes: Vec<RuntimeMode>,
}

pub struct RuntimeMode {
    name: String,
    command: Option<String>,
    down: Option<String>,
    env: Vec<(String, String)>,
}

pub struct RuntimeContext {
    pub repo: PathBuf,
    pub manifest: RuntimeManifest,
    pub mode: RuntimeMode,
    pub environment_id: String,
    pub environment_dir: PathBuf,
    pub env_file: PathBuf,
}
```

Require `version: 1` when it is present, at least one mode, unique mode names, unique environment names, and `^[A-Z_][A-Z0-9_]*$` environment names. Select `default_mode`, otherwise the first declared mode, preserving compatibility with existing manifests. Canonicalize paths with `std::fs::canonicalize`; compute the state ID by invoking `cksum` over `"<canonical-repo>:<mode>"` and applying the existing ASCII slug rules to the manifest name. Format the fixed nine environment values followed by mode environment pairs in manifest order, quoting every value for POSIX `.` consumption.

- [ ] **Step 4: Run focused core tests and the shell compatibility suite**

Run: `cargo test -p autospec-core --test runtime_env && bats tests/agent-env.bats`

Expected: all parser/context tests pass; the shell suite remains green except for behavior explicitly not yet redirected to Rust.

- [ ] **Step 5: Commit the pure contract**

```bash
git add crates/autospec-core/src/lib.rs crates/autospec-core/src/runtime_env.rs crates/autospec-core/tests/runtime_env.rs tests/agent-env.bats
git commit -m "feat: model isolated runtime manifests in Rust"
```

## Task 2: Add typed CLI outcomes and `up|status|down`

**Files:**
- Modify: `crates/autospec-core/src/runtime_env.rs`
- Create: `crates/autospec-cli/src/commands/runtime/env.rs`
- Modify: `crates/autospec-cli/src/commands/runtime.rs`
- Modify: `crates/autospec-cli/src/commands/mod.rs`
- Modify: `crates/autospec-cli/src/main.rs`
- Modify: `crates/autospec-cli/tests/runtime_commands.rs`

**Interfaces:**
- Consumes: `RuntimeManifest`, `RuntimeContext`, and `RuntimeState` from Task 1.
- Produces: `RuntimeState::replace_existing_value(&mut self, key: &str, value: String) -> Result<(), RuntimeEnvError>`, `CommandFailure { message: String, exit_code: i32 }`, and `autospec runtime env up|status|down`.
- Later tasks rely on: `run(args) -> Result<(), CommandFailure>` and `env::run(args) -> Result<(), CommandFailure>`.

- [ ] **Step 1: Add failing CLI tests for output, state reuse, inactive status, and a child exit of 42**

```rust
fn runtime_fixture(command: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("autospec-runtime-cli-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join(".autospec")).unwrap();
    std::fs::write(
        root.join(".autospec/runtime.yml"),
        format!("version: 1\nname: sample-app\ndefault_mode: local\nmodes:\n  local:\n    command: {command}\n    down: sh -c 'true'\n"),
    ).unwrap();
    root
}

#[test]
fn runtime_env_up_preserves_manifest_command_exit_status() {
    let fixture = runtime_fixture("sh -c 'exit 42'");
    let output = autospec().args(["runtime", "env", "up", "--repo", fixture.to_str().unwrap()]).output().unwrap();
    assert_eq!(output.status.code(), Some(42));
}

#[test]
fn runtime_env_status_reports_inactive_environment_with_exit_three() {
    let fixture = runtime_fixture("sh -c 'true'");
    let output = autospec().args(["runtime", "env", "status", "--repo", fixture.to_str().unwrap()]).output().unwrap();
    assert_eq!(output.status.code(), Some(3));
}
```

- [ ] **Step 2: Run the focused CLI tests and confirm the intended red failure**

Run: `cargo test -p autospec-cli --test runtime_commands runtime_env_ -- --nocapture`

Expected: `runtime env` is rejected as an unknown command.

- [ ] **Step 3: Introduce exit-code-aware command failures and implement the three lifecycle operations**

Use a CLI-local error type so existing commands continue returning diagnostic exit `2` while the broker can return its public status:

```rust
pub struct CommandFailure {
    pub message: String,
    pub exit_code: i32,
}

impl CommandFailure {
    pub fn diagnostic(message: impl Into<String>) -> Self { Self { message: message.into(), exit_code: 2 } }
    pub fn status(message: impl Into<String>, exit_code: i32) -> Self { Self { message: message.into(), exit_code } }
}
```

Map existing `Result<(), String>` command results with `CommandFailure::diagnostic` at the top-level dispatcher, and make `main` exit `error.exit_code`. In `env.rs`, parse both split and equals option forms; reject malformed options before state changes. `up` writes the state/env file before running a trusted manifest `command`; if a state file already exists, parse and reuse it without rerunning the command. Before a new state is written, use `RuntimeState::replace_existing_value` to preserve non-empty caller values for `AGENT_FRONTEND_PORT`, `AGENT_BACKEND_PORT`, `AGENT_PUBLIC_URL`, `AUTOSPEC_PUBLIC_URL`, and `COMPOSE_PROJECT_NAME`. `status` reads and prints an existing state file or returns `3`. `down` runs optional trusted `down` first, preserves a nonzero child exit, and removes state only after successful/no-op teardown.

- [ ] **Step 4: Run broker and existing runtime tests**

Run: `cargo test -p autospec-cli --test runtime_commands runtime_ -- --nocapture && cargo test -p autospec-cli --test cli_commands`

Expected: `up` emits the ordered protocol; `status` returns `3` when inactive; `down` is idempotent; a child `42` remains `42`; classify/audit tests remain green.

- [ ] **Step 5: Commit the lifecycle command**

```bash
git add crates/autospec-cli/src/main.rs crates/autospec-cli/src/commands/mod.rs crates/autospec-cli/src/commands/runtime.rs crates/autospec-cli/src/commands/runtime/env.rs crates/autospec-cli/tests/runtime_commands.rs
git commit -m "feat: add Rust runtime environment lifecycle commands"
```

## Task 3: Complete `init|exec|session` without shell authority

**Files:**
- Modify: `crates/autospec-cli/src/commands/runtime/env.rs`
- Modify: `crates/autospec-cli/tests/runtime_commands.rs`
- Modify: `crates/autospec-core/tests/runtime_env.rs`

**Interfaces:**
- Consumes: the lifecycle operations from Task 2.
- Produces: all six documented public subcommands and normal-exit session cleanup.

- [ ] **Step 1: Add failing tests for init protection, direct exec, session bypass, auto-init, and cleanup**

```rust
#[test]
fn runtime_env_session_removes_state_after_child_completion() {
    let fixture = runtime_fixture("sh -c 'true'");
    let state_root = fixture.join("state");
    let output = autospec().env("AGENT_ENV_STATE_ROOT", &state_root).args(["runtime", "env", "session", "--repo", fixture.to_str().unwrap(), "--", "sh", "-c", "exit 0"]).output().unwrap();
    assert!(output.status.success());
    let status = autospec().env("AGENT_ENV_STATE_ROOT", &state_root).args(["runtime", "env", "status", "--repo", fixture.to_str().unwrap()]).output().unwrap();
    assert_eq!(status.status.code(), Some(3));
}
```

- [ ] **Step 2: Run the focused session tests and confirm the intended red failure**

Run: `cargo test -p autospec-cli --test runtime_commands runtime_env_session -- --nocapture`

Expected: failures because `session` is not implemented.

- [ ] **Step 3: Implement the remaining subcommands**

`init` writes the exact conservative v1 manifest, refuses an existing selected manifest with exit `4`, and supports only `agent|autospec`. `exec` provisions/reuses state and launches its trailing command with direct argument vectors in the canonical repository. `session` bypasses when `AUTOSPEC_ENV_DISABLE=1`, passes through unchanged when no manifest exists, auto-initializes only when `AUTOSPEC_ENV_AUTO_INIT=1`, records a session file while the child is running, and removes it plus tears down after normal child completion unless `--keep-alive` or `AUTOSPEC_ENV_KEEP_ALIVE=1` is set. Keep child exit codes unchanged.

For Unix interruption, install a minimal signal handler that only records `SIGINT`/`SIGTERM` in an atomic flag; the parent then terminates the direct child, removes the session record, runs teardown, and exits `130`/`143`. Test this with a spawned sleep child and an explicit signal only on Unix.

- [ ] **Step 4: Run all focused broker tests**

Run: `cargo test -p autospec-core --test runtime_env && cargo test -p autospec-cli --test runtime_commands runtime_env_ -- --nocapture`

Expected: all six commands preserve their documented state, output, and exit behavior.

- [ ] **Step 5: Commit the complete broker**

```bash
git add crates/autospec-core/tests/runtime_env.rs crates/autospec-cli/src/commands/runtime/env.rs crates/autospec-cli/tests/runtime_commands.rs
git commit -m "feat: complete Rust isolated runtime broker"
```

## Task 4: Build/install the Rust binary and replace live callers

**Files:**
- Modify: `install.sh`
- Modify: `tests/agent-env.bats`
- Modify: `tests/agent-env-install.bats`
- Modify: `tests/autospec-run-agent-env-contract.bats`
- Modify: `skills/autospec-run/SKILL.md`
- Modify: `skills/autospec-run/codex/prompt.md`
- Modify: `skills/autospec-run/opencode/agent.md`

**Interfaces:**
- Consumes: installed `~/.autospec/bin/autospec` and the Task 3 command family.
- Produces: `agent-env` and `autospec-env` aliases that execute `autospec runtime env` and no direct legacy script references.

- [ ] **Step 1: Add failing installer and prompt-contract tests**

Make the existing static tests require an installer function that builds `cargo build --release -p autospec-cli`, atomically installs `target/release/autospec` to `$HOME/.autospec/bin/autospec`, and writes wrappers containing exactly:

```bash
exec "${AUTOSPEC_BIN:-$HOME/.autospec/bin/autospec}" runtime env "$@"
```

Make the three autospec-run bodies require `autospec runtime env up` and `autospec runtime env down`, and reject `agent-env.sh`.

- [ ] **Step 2: Run the static tests and confirm the intended red failure**

Run: `bats tests/agent-env-install.bats tests/autospec-run-agent-env-contract.bats`

Expected: failures because installer wrappers and lock-step prompts still name `agent-env.sh`.

- [ ] **Step 3: Install the runtime binary and update all callers in lock-step**

Add `install_autospec_runtime_binary` before `install_agent_env_commands`. It invokes Cargo from `REPO_ROOT`, writes to a temporary file in `$HOME/.autospec/bin`, makes it executable, then renames it to `autospec`; dry-run only reports the exact build/install actions. The two aliases call the installed binary. Update both setup and cleanup blocks in all three autospec-run files byte-identically after frontmatter. Update Bats to build/use `target/debug/autospec` for source-tree broker compatibility rather than sourcing the deleted script.

- [ ] **Step 4: Run the installer and lock-step verification**

Run: `bats tests/agent-env.bats tests/agent-env-install.bats tests/autospec-run-agent-env-contract.bats && cargo run -q -p autospec-cli -- validate --fast`

Expected: Bats invokes Rust for each broker case, the installer tests find only Rust wrappers, and validation succeeds.

- [ ] **Step 5: Commit the caller cutover**

```bash
git add install.sh tests/agent-env.bats tests/agent-env-install.bats tests/autospec-run-agent-env-contract.bats skills/autospec-run/SKILL.md skills/autospec-run/codex/prompt.md skills/autospec-run/opencode/agent.md
git commit -m "refactor: route isolated runtime callers through Rust"
```

## Task 5: Delete the shell broker and prove it is unreachable

**Files:**
- Delete: `scripts/agent-env.sh`
- Modify: `crates/autospec-cli/tests/runtime_commands.rs`
- Modify: `docs/cli-reference.md`
- Create: `docs/runbooks/agent-runtime-manifest.md`

**Interfaces:**
- Consumes: all Rust command and caller behavior from Tasks 1–4.
- Produces: the public operator documentation and a negative regression gate for the deleted authority.

- [ ] **Step 1: Add a failing negative reachability test**

Add a Rust integration test that reads tracked source paths and fails if `scripts/agent-env.sh` exists or any installed wrapper/prompt/test/documentation command calls it. Allow only the deletion-history text in the migration design, not executable command references.

- [ ] **Step 2: Run the test and confirm it fails before deletion**

Run: `cargo test -p autospec-cli --test runtime_commands legacy_agent_env_authority_is_absent -- --exact`

Expected: failure naming `scripts/agent-env.sh` and one or more remaining live references if Task 4 missed any.

- [ ] **Step 3: Delete the authority and document the Rust contract**

Delete the shell file. Add CLI-reference rows for `runtime env` and a runbook containing the v1 manifest grammar, precedence, generated environment variables, state-root behavior, child command semantics, exact status codes `1`, `2`, `3`, `4`, `42`, and safe cleanup expectations.

- [ ] **Step 4: Run full verification**

Run: `cargo fmt --all --check && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && bats tests/agent-env.bats tests/agent-env-install.bats tests/autospec-run-agent-env-contract.bats && cargo run -q -p autospec-cli -- validate --fast && git diff --check`

Expected: every command succeeds; the reachability test proves no former shell authority remains.

- [ ] **Step 5: Commit the deletion and proof**

```bash
git add -A scripts/agent-env.sh crates/autospec-cli/tests/runtime_commands.rs docs/cli-reference.md docs/runbooks/agent-runtime-manifest.md
git commit -m "refactor: retire the shell agent environment broker"
```

## Plan self-review

- Every broker capability from the existing script is covered: init, up, status, down, exec, session, auto-init, disable, keep-alive, exact child exits, and canonical state identity.
- No task introduces a parser or process dependency.
- The legacy deletion is downstream of fixture parity, installed-binary availability, lock-step caller migration, and a negative reachability test.
- The coordinator, lint/claim, and context-monitor driver are deliberately excluded from this plan and remain scheduled by the approved control-plane design.
