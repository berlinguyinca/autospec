# Worktree Resource Isolation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give every Autospec worktree a Maven 4 namespace, Docker Compose project, dynamic host exports, owned resources, and reference-counted harness sessions that remain collision-free across 40 concurrent stacks.

**Architecture:** Extend the Rust runtime broker into the single resource authority. `autospec-core` owns typed identity, manifest, plan, inventory, Maven, Compose-policy, and normalization contracts; `autospec-cli` owns Git/Maven/Docker process execution, leases, reconciliation, sessions, and garbage collection. The Autospec skills and generated Codex/Claude/OpenCode aliases only orchestrate that Rust contract.

**Tech Stack:** Rust 2021, Maven 4 with Resolver 2.x split-local repositories, Docker Compose v2, pinned `serde` 1.0.228, `serde_json` 1.0.150, `getrandom` 0.4.3, `sha2` 0.11.0, and `yaml-edit` 0.2.3, Bash/Bats validation, GitHub CLI for prerequisite migration issues and PRs.

**Source:** `docs/superpowers/specs/2026-07-16-worktree-resource-isolation-design.md` and umbrella issue [#2103](https://github.com/berlinguyinca/autospec/issues/2103).

## Global Constraints

- Maven 4 is the minimum supported Maven version; do not add a Maven 3 path.
- Preserve one effective Maven local-repository root, the fixed remote prefix `cached`, and per-environment local prefix `autospec/<AGENT_ENV_ID>`.
- Never copy, hard-link, overlay, or manually rewrite Maven coordinate directories.
- Docker Compose resources must be project-scoped and broker-labeled; ambiguous or globally named resources fail before startup.
- Only manifest-declared Compose exports may bind host ports, and Docker chooses those ports atomically.
- `down` removes broker-owned volumes by default; `preserve_volumes` contains Compose logical volume keys that survive.
- A worktree-generation token participates in identity so path reuse cannot adopt stale state.
- Multiple sessions in the same worktree and mode share resources; only the last live reference can tear them down.
- Automatic stale-resource collection is conservative: ambiguous ownership blocks deletion.
- Codex, Claude, and OpenCode aliases are generated from one table and remain lock-step tested.
- The Compose normalizer is deterministic and idempotent; an LLM may explain ambiguity but never rewrites YAML.
- Pin new dependency versions, run `cargo audit`, and reject unresolved advisories before merging the dependency-introducing task.
- Each task runs formatting, strict Clippy, focused tests, and `cargo run -q -p autospec-cli -- validate --fast` before commit.
- Create one linked GitHub child issue per task, branch as `feat/worktree-resource-isolation/<task-slug>`, and execute it in a worktree created by `scripts/worktree-guard.sh create`.

---

## File Structure

- Modify: `crates/autospec-core/Cargo.toml`, `Cargo.lock` — pinned serialization and lossless-YAML dependencies.
- Modify: `crates/autospec-core/src/runtime_env.rs` — public runtime exports and compatibility facade.
- Create: `crates/autospec-core/src/runtime_env/identity.rs` — worktree-generation-aware environment identity.
- Create: `crates/autospec-core/src/runtime_env/resources.rs` — resource plan, owner, inventory, and session types.
- Create: `crates/autospec-core/src/runtime_env/diagnostic.rs` — stable code/resource/evidence/recovery error contract.
- Modify: `crates/autospec-core/src/runtime_env/manifest.rs` — v1 compatibility plus v2 resource grammar.
- Create: `crates/autospec-core/src/runtime_env/maven.rs` — Maven 4 split-local argument and cleanup-boundary contracts.
- Create: `crates/autospec-core/src/runtime_env/compose.rs` — resolved-model policy, exports, ownership, and override rendering.
- Create: `crates/autospec-core/src/runtime_env/compose_normalize.rs` — lossless deterministic Compose/manifest edits.
- Modify: `crates/autospec-core/tests/runtime_env.rs` — identity and v1 compatibility coverage.
- Create: `crates/autospec-core/tests/runtime_resources.rs` — plan, manifest v2, Maven, Compose, inventory, and normalization tests.
- Modify: `crates/autospec-cli/src/commands/runtime/env.rs` — thin command parser and lifecycle coordinator.
- Create: `crates/autospec-cli/src/commands/runtime/env/options.rs` — complete command-line grammar.
- Create: `crates/autospec-cli/src/commands/runtime/env/state.rs` — atomic JSON state, file lease, generation token, and reconciliation.
- Create: `crates/autospec-cli/src/commands/runtime/env/maven.rs` — Maven discovery, version/root interrogation, and safe purge.
- Create: `crates/autospec-cli/src/commands/runtime/env/compose.rs` — Docker Compose execution, port discovery, labeling, inventory, and teardown.
- Create: `crates/autospec-cli/src/commands/runtime/env/session.rs` — process-start identity, heartbeat, reference release, and signals.
- Create: `crates/autospec-cli/src/commands/runtime/env/gc.rs` — conservative stale-owner collection and direct-server port registry.
- Modify: `crates/autospec-cli/tests/runtime_commands.rs` — command and failure-semantics coverage.
- Create: `crates/autospec-cli/tests/runtime_resources.rs` — fake-executable boundary tests for Git, Maven, and Docker argument vectors.
- Create: `tests/fixtures/runtime-resources/` — v1/v2 manifests and safe/unsafe Compose fixtures.
- Create: `tests/integration/runtime-maven-isolation.bats` — real Maven 4 same-GAV proof.
- Create: `tests/integration/runtime-compose-isolation.bats` — real 40-stack Compose proof and leak audit.
- Create: `skills/autospec-compose-normalize/SKILL.md`, `skills/autospec-compose-normalize/codex/prompt.md`, `skills/autospec-compose-normalize/opencode/agent.md` — lock-step internal migration workflow.
- Create: `skills/autospec-compose-normalize/install.sh`, `skills/autospec-compose-normalize/uninstall.sh` — installation lifecycle.
- Create: `skills/autospec-compose-normalize/tests/normalize.bats` — skill-to-Rust contract.
- Modify: `skills/autospec-run/SKILL.md`, `skills/autospec-run/codex/prompt.md`, `skills/autospec-run/opencode/agent.md` — unconditional broker preflight, migration handoff, and cleanup.
- Modify: `install.sh`, `tests/agent-env-install.bats` — one generated harness alias table.
- Create: `scripts/autospec-runtime-worktree-cleanup.sh` and `tests/runtime-worktree-cleanup.bats` — broker GC adapter composed before confirmed Git cleanup; `worktree-guard.sh` remains Git-only.
- Modify: `docs/runbooks/agent-runtime-manifest.md`, `docs/runbooks/agent-runtime-companion-stacks.md`, `docs/cli-reference.md` — v2 resources, operations, and recovery.
- Modify: Rust validation catalog checks and generated fixtures/goldens selected by `autospec validate` — new skill presence and trio lockstep.

## Task 1: Establish generation-aware, locked resource state

**Files:**
- Create: `crates/autospec-core/src/runtime_env/identity.rs`
- Create: `crates/autospec-core/src/runtime_env/resources.rs`
- Create: `crates/autospec-core/src/runtime_env/diagnostic.rs`
- Modify: `crates/autospec-core/src/runtime_env.rs`
- Modify: `crates/autospec-core/Cargo.toml`
- Modify: `Cargo.lock`
- Create: `crates/autospec-core/tests/runtime_resources.rs`
- Create: `crates/autospec-cli/src/commands/runtime/env/state.rs`
- Modify: `crates/autospec-cli/src/commands/runtime/env.rs`
- Create: `crates/autospec-cli/src/commands/runtime/env/isolation.rs`
- Modify: `crates/autospec-cli/tests/runtime_commands.rs`

**Interfaces:**
- Produces: `EnvironmentIdentity::resolve(repo: &Path, mode: &str, generation: Option<&str>) -> Result<Self, RuntimeEnvError>`.
- Produces: `ResourcePlan`, `EnvironmentOwner`, `ResourceInventory`, `SessionRecord`, and `IsolationDiagnostic` as `serde` JSON contracts with `schema_version: 1`.
- Produces: `StateLayout::new(root, environment_id)` with exact `owner.json`, `plan.json`, `env`, `inventory.json`, `lease.lock`, and `sessions/` paths.
- Produces: `EnvironmentLease::acquire(environment_dir: &Path) -> Result<Self, CommandFailure>` using `std::fs::File::lock`.
- Produces: `load_generation_token(repo)`, `write_json_atomic(path, value)`, and `read_json(path)` for later tasks.

- [ ] **Step 1: Add failing identity, JSON, and concurrent-lease tests**

Add tests that prove a reused canonical path receives a new ID and that persisted state round-trips without shell evaluation:

```rust
#[test]
fn generation_token_prevents_path_reuse_from_adopting_state() {
    let repo = TempRepo::with_files(&[]);
    let first = EnvironmentIdentity::resolve(repo.path(), "local", Some("gen-a")).unwrap();
    let second = EnvironmentIdentity::resolve(repo.path(), "local", Some("gen-b")).unwrap();
    assert_ne!(first.environment_id, second.environment_id);
    assert_ne!(first.owner_key, second.owner_key);
}

#[test]
fn inventory_json_preserves_resource_ids_and_ports() {
    let inventory = ResourceInventory::fixture("env-a", "compose-a", 49152);
    let encoded = serde_json::to_string(&inventory).unwrap();
    assert_eq!(serde_json::from_str::<ResourceInventory>(&encoded).unwrap(), inventory);
}
```

Add a CLI test that starts two `lease-probe` test processes against one environment directory and asserts the second remains blocked until the first releases the lock. Add a linked-worktree fixture invoked from a nested subdirectory and assert its identity uses the `git rev-parse --show-toplevel` root and worktree-specific Git directory.

- [ ] **Step 2: Run the focused tests and confirm the red failures**

Run: `cargo test -p autospec-core --test runtime_resources && cargo test -p autospec-cli --test runtime_commands runtime_env_lease -- --nocapture`

Expected: compilation fails because `EnvironmentIdentity`, JSON resource types, and `EnvironmentLease` do not exist.

- [ ] **Step 3: Implement the minimal typed state and lease boundary**

Use these public shapes so later tasks do not invent parallel state:

```rust
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EnvironmentIdentity {
    pub canonical_repo: PathBuf,
    pub mode: String,
    pub generation: Option<String>,
    pub environment_id: String,
    pub owner_key: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ResourcePlan {
    pub schema_version: u32,
    pub digest: String,
    pub identity: EnvironmentIdentity,
    pub maven: Option<MavenPlan>,
    pub compose: Option<ComposePlan>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MavenPlan {
    pub isolation: MavenIsolation,
    pub local_prefix: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum MavenIsolation { SplitLocal, Off }

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ComposeIsolation { Managed, Off }

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ExportProtocol { Http, Https, Tcp, Udp }

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ExportValue { Url, Port, HostPort }

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ComposeExport {
    pub service: String,
    pub target: u16,
    pub protocol: ExportProtocol,
    pub env: String,
    pub value: ExportValue,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OwnedVolume { pub logical_key: Option<String>, pub id: String }

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ResolvedExport { pub env: String, pub host: String, pub port: u16 }

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct IsolationDiagnostic {
    pub schema_version: u32,
    pub code: String,
    pub environment_id: String,
    pub resource: String,
    pub evidence: String,
    pub recovery_command: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum EnvironmentLifecycle { Planned, Provisioning, Active, TearingDown, CleanupFailed }

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EnvironmentOwner {
    pub schema_version: u32,
    pub identity: EnvironmentIdentity,
    pub host: String,
    pub created_at_unix_ms: u64,
    pub manifest_digest: String,
    pub lifecycle: EnvironmentLifecycle,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SessionRecord {
    pub schema_version: u32,
    pub session_id: String,
    pub pid: u32,
    pub process_start: String,
    pub harness: String,
    pub host: String,
    pub started_at_unix_ms: u64,
    pub heartbeat_at_unix_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ComposePlan {
    pub isolation: ComposeIsolation,
    pub files: Vec<PathBuf>,
    pub project_name: String,
    pub exports: Vec<ComposeExport>,
    pub preserve_volumes: Vec<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ResourceInventory {
    pub schema_version: u32,
    pub environment_id: String,
    pub compose_project: Option<String>,
    pub containers: Vec<String>,
    pub networks: Vec<String>,
    pub volumes: Vec<OwnedVolume>,
    pub exports: Vec<ResolvedExport>,
    pub maven_local_prefix: Option<PathBuf>,
}
```

Pin `serde = { version = "=1.0.228", features = ["derive"] }`, `serde_json = "=1.0.150"`, `getrandom = "=0.4.3"`, and `sha2 = "=0.11.0"`. Resolve the worktree root and Git directory through typed `git rev-parse` argument vectors. Store the token at `<git-dir>/autospec-runtime-generation`; create it atomically from 128 bits filled by `getrandom::fill`. Non-Git directories pass `None` and cannot claim the full isolation guarantee. Hash canonical identity and a canonical `ResourcePlanContent` value that excludes the `digest` field with SHA-256. Write JSON using a same-directory create-new temporary, file sync, atomic rename, and parent-directory sync. Keep the existing sourceable `env` file for shell callers, but make `owner.json`, `plan.json`, and `inventory.json` the authoritative state. Split state I/O out of the existing 905-line `env.rs`; do not change current command behavior in this task.

- [ ] **Step 4: Run focused and regression tests**

Run: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test -p autospec-core --test runtime_resources && cargo test -p autospec-cli --test runtime_commands`

Expected: generation, JSON, and lease tests pass; every existing runtime command test remains green.

- [ ] **Step 5: Commit the state foundation**

```bash
git add Cargo.lock crates/autospec-core/Cargo.toml crates/autospec-core/src/runtime_env.rs crates/autospec-core/src/runtime_env/identity.rs crates/autospec-core/src/runtime_env/resources.rs crates/autospec-core/src/runtime_env/diagnostic.rs crates/autospec-core/tests/runtime_resources.rs crates/autospec-cli/src/commands/runtime/env.rs crates/autospec-cli/src/commands/runtime/env/state.rs crates/autospec-cli/tests/runtime_commands.rs
git commit -m "feat: make runtime resource ownership generation-aware"
```

## Task 2: Parse manifest v2 and build automatic resource plans

**Files:**
- Modify: `crates/autospec-core/Cargo.toml`
- Modify: `Cargo.lock`
- Modify: `crates/autospec-core/src/runtime_env.rs`
- Modify: `crates/autospec-core/src/runtime_env/manifest.rs`
- Create: `crates/autospec-core/src/runtime_env/manifest_v2.rs`
- Create: `crates/autospec-core/src/runtime_env/resource_plan.rs`
- Create: `crates/autospec-core/src/runtime_env/shell_command.rs`
- Modify: `crates/autospec-core/src/runtime_env/resources.rs`
- Modify: `crates/autospec-core/tests/runtime_env.rs`
- Modify: `crates/autospec-core/tests/runtime_resources.rs`
- Create: `crates/autospec-core/tests/runtime_resource_plan.rs`
- Create: `crates/autospec-core/tests/runtime_manifest_v2.rs`
- Modify: `crates/autospec-cli/src/commands/runtime/env.rs`
- Create: `crates/autospec-cli/src/commands/runtime/env/isolation.rs`
- Modify: `crates/autospec-cli/tests/runtime_commands.rs`
- Modify: `crates/autospec-cli/tests/runtime_resources.rs`
- Modify: `crates/autospec-cli/tests/runtime_commands.rs`
- Create: `tests/fixtures/runtime-resources/manifest-v2.yml`
- Create: `tests/fixtures/runtime-resources/compose.yaml`

**Interfaces:**
- Consumes: `EnvironmentIdentity` and `ResourcePlan` from Task 1.
- Produces: `RuntimeResources { maven: MavenResourceConfig, compose: ComposeResourceConfig }`.
- Produces: `RuntimeManifest::resource_plan_for_repo(repo: &Path, identity: &EnvironmentIdentity) -> Result<ResourcePlan, RuntimeEnvError>`.
- Produces: `ResourcePlan::apply_invocation_overrides(maven_value, compose_value, whole_environment_disabled)` with fail-closed values.
- Consumes and validates the exact Task 1 enums `MavenIsolation::{SplitLocal, Off}` and `ComposeIsolation::{Managed, Off}`.

- [ ] **Step 1: Add failing v1 compatibility, v2 parsing, and auto-detection tests**

```rust
#[test]
fn v2_resources_parse_exports_and_logical_preserved_volumes() {
    let manifest = RuntimeManifest::parse(include_str!("../../../tests/fixtures/runtime-resources/manifest-v2.yml")).unwrap();
    assert_eq!(manifest.resources().maven.isolation, MavenIsolation::SplitLocal);
    assert_eq!(manifest.resources().compose.exports[0].env, "AUTOSPEC_PUBLIC_URL");
    assert_eq!(manifest.resources().compose.preserve_volumes, vec!["postgres-data"]);
}

#[test]
fn resource_detection_finds_pom_and_standard_compose_without_a_manifest() {
    let repo = TempRepo::with_files(&[("pom.xml", "<project/>"), ("compose.yaml", "services: {}\n")]);
    let plan = RuntimeManifest::resource_plan_for_repo(repo.path(), &fixture_identity(repo.path())).unwrap();
    assert!(plan.maven.is_some());
    assert_eq!(plan.compose.unwrap().files, vec![repo.path().join("compose.yaml")]);
}

#[test]
fn v1_command_that_starts_compose_cannot_compete_with_the_broker() {
    let repo = TempRepo::with_files(&[
        (".autospec/runtime.yml", "version: 1\nmodes:\n  local:\n    command: docker compose up\n"),
        ("compose.yaml", "services: {}\n"),
    ]);
    let error = RuntimeManifest::resource_plan_for_repo(repo.path(), &fixture_identity(repo.path())).unwrap_err();
    assert!(error.to_string().contains("RUNTIME_DUAL_COMPOSE_AUTHORITY"));
}

#[test]
fn compose_only_plan_does_not_require_a_mode_command() {
    let repo = TempRepo::with_files(&[("compose.yaml", "services: {}\n")]);
    assert!(RuntimeManifest::resource_plan_for_repo(repo.path(), &fixture_identity(repo.path())).is_ok());
}

#[test]
fn empty_plan_without_a_mode_command_keeps_the_existing_error() {
    let repo = TempRepo::with_files(&[]);
    let error = RuntimeManifest::resource_plan_for_repo(repo.path(), &fixture_identity(repo.path())).unwrap_err();
    assert!(error.to_string().contains("command"));
}
```

Retain the existing tests proving v1 mode order, default-mode selection, and reserved environment names.

- [ ] **Step 2: Run focused tests and confirm v2 is rejected**

Run: `cargo test -p autospec-core --test runtime_env && cargo test -p autospec-core --test runtime_resources -- --nocapture`

Expected: the v2 fixture fails with the current `unsupported runtime manifest version: 2` diagnostic.

- [ ] **Step 3: Implement the constrained v2 grammar and deterministic detection**

Pin `yaml-edit = "=0.2.3"` and use its lossless syntax tree for v2 nested maps/lists; keep v1 behavior byte-compatible. Accept only:

```rust
pub struct MavenResourceConfig {
    pub isolation: MavenIsolation,
}

pub struct ComposeResourceConfig {
    pub isolation: ComposeIsolation,
    pub files: Vec<PathBuf>,
    pub exports: Vec<ComposeExport>,
    pub preserve_volumes: Vec<String>,
    pub shared_networks: Vec<String>,
    pub shared_volumes: Vec<String>,
}
```

Omission means auto-detect. Maven accepts `split-local|off`; Compose accepts `off` when present and otherwise uses `Managed`. Discover `compose.yaml`, `compose.yml`, `docker-compose.yaml`, then `docker-compose.yml`, stopping at the first standard file unless `resources.compose.files` explicitly lists a set. Reject duplicate files, duplicate export environment names, unsupported protocols, unknown resource keys, and invalid `preserve_volumes` entries.

Define shared exceptions as `resources.compose.shared_resources.networks` and `.volumes`, each a list of exact Compose logical keys. `AUTOSPEC_MAVEN_ISOLATION` and `AUTOSPEC_COMPOSE_ISOLATION` accept only `off`; set `AUTOSPEC_ISOLATION_BYPASSED=1` whenever either applies. With `AUTOSPEC_ENV_DISABLE=1`, `up`, `exec`, and `session` skip provisioning and export the bypass marker; `status` remains read-only; `down` and `gc` still reconcile only previously recorded owned resources so bypass cannot create leaks or authorize shared-resource deletion. Reject a v1 mode command containing an executable token sequence `docker compose` or `docker-compose` when Compose detection is active.

- [ ] **Step 4: Run parsing, dependency, and validation gates**

Run: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test -p autospec-core --test runtime_env && cargo test -p autospec-core --test runtime_resources && cargo audit && cargo run -q -p autospec-cli -- validate --fast`

Expected: v1 and v2 tests pass, no unresolved advisory is reported, and Autospec validation reports every required check passed.

- [ ] **Step 5: Commit manifest v2 and detection**

```bash
git add Cargo.lock crates/autospec-core/Cargo.toml crates/autospec-core/src/runtime_env.rs crates/autospec-core/src/runtime_env/manifest.rs crates/autospec-core/src/runtime_env/manifest_v2.rs crates/autospec-core/src/runtime_env/resource_plan.rs crates/autospec-core/src/runtime_env/resources.rs crates/autospec-core/src/runtime_env/shell_command.rs crates/autospec-core/tests/runtime_env.rs crates/autospec-core/tests/runtime_manifest_v2.rs crates/autospec-core/tests/runtime_resource_plan.rs crates/autospec-core/tests/runtime_resources.rs crates/autospec-cli/src/commands/runtime/env.rs crates/autospec-cli/src/commands/runtime/env/isolation.rs crates/autospec-cli/tests/runtime_commands.rs crates/autospec-cli/tests/runtime_resources.rs tests/fixtures/runtime-resources
git commit -m "feat: plan Maven and Compose resources from runtime v2"
```

## Task 3: Make sessions reference-counted and reconcile partial state

**Files:**
- Modify: `crates/autospec-core/src/runtime_env.rs`
- Modify: `crates/autospec-core/src/runtime_env/resources.rs`
- Modify: `crates/autospec-core/tests/runtime_resources.rs`
- Create: `crates/autospec-core/src/runtime_env/session.rs`
- Create: `crates/autospec-core/tests/runtime_session.rs`
- Create: `crates/autospec-cli/src/commands/runtime/env/session.rs`
- Create: `crates/autospec-cli/src/commands/runtime/env/worker.rs`
- Create: `crates/autospec-cli/src/commands/runtime/env/lifecycle.rs`
- Modify: `crates/autospec-cli/src/commands/runtime/env/isolation.rs`
- Modify: `crates/autospec-cli/src/commands/runtime/env/state.rs`
- Modify: `crates/autospec-cli/src/commands/runtime/env.rs`
- Modify: `crates/autospec-cli/tests/runtime_resources.rs`
- Create: `crates/autospec-cli/tests/runtime_sessions.rs`
- Create: `crates/autospec-cli/tests/runtime_session_security.rs`
- Create: `crates/autospec-cli/tests/runtime_state_reconciliation.rs`

**Interfaces:**
- Consumes: `EnvironmentLease`, owner, plan, and inventory from Task 1.
- Produces: `SessionRecord { schema_version, session_id, pid, process_start, harness, host, started_at_unix_ms, heartbeat_at_unix_ms }`.
- Produces: `SessionLease::register`, `SessionLease::heartbeat`, `SessionLease::release`, and `live_sessions`.
- Produces: `SessionSet` and `ReleaseDecision::{KeepActive, TearDown}` as pure reference-count policy.
- Produces: lifecycle states `Planned`, `Provisioning`, `Active`, `TearingDown`, and `CleanupFailed` in `owner.json`.

- [ ] **Step 1: Add failing multi-session, PID-reuse, and partial-provision tests**

```rust
#[test]
fn releasing_one_of_two_live_sessions_keeps_the_environment_active() {
    let mut sessions = SessionSet::default();
    sessions.register(SessionRecord::fixture("session-a", 100, "start-a"));
    sessions.register(SessionRecord::fixture("session-b", 101, "start-b"));
    assert_eq!(sessions.release("session-a"), ReleaseDecision::KeepActive);
    assert_eq!(sessions.release("session-b"), ReleaseDecision::TearDown);
}

#[test]
fn reused_pid_with_a_different_process_start_is_not_live() {
    let recorded = ProcessIdentity { pid: 4242, process_start: "111".into() };
    let observed = ProcessIdentity { pid: 4242, process_start: "222".into() };
    assert!(!recorded.matches(&observed));
}
```

Add CLI tests where two `session` children overlap, the first exits, `status` remains active, and teardown runs exactly once after the second exits. Add a fixture with `owner.json` in `Provisioning` and assert the next `up` reconciles rather than trusting the legacy `env` file.

- [ ] **Step 2: Run focused tests and confirm teardown currently happens too early**

Run: `cargo test -p autospec-core --test runtime_session -- --nocapture && cargo test -p autospec-cli --test runtime_sessions runtime_env_two_sessions -- --nocapture`

Expected: core symbols are missing and the CLI teardown counter records one teardown after the first child exits.

- [ ] **Step 3: Implement session records and lifecycle reconciliation under one lease**

Use a random `process_start` token and hold an exclusive lock on `sessions/<session-id>.lock` for the process lifetime. Store PID plus that token in JSON; a nonblocking lock attempt is the cross-platform liveness authority, so PID reuse cannot revive the record. Prune a record only when its lock is acquirable. `down` returns a stable `RUNTIME_LIVE_SESSIONS` diagnostic while any record remains live.

Construct `RuntimeContext` and `StateLayout` from `ResourcePlan.identity.environment_id`; owner, plan, inventory, env, lease, and sessions must share that generation-aware directory rather than the legacy path hash.

Preserve `session --keep-alive`: final release records zero live sessions but suppresses automatic teardown, allowing an explicit later `down`. Without `--keep-alive`, final release tears down ephemeral resources while retaining the Maven installed-artifact prefix.

Run harnesses under a supervised internal session worker. Keep its one-shot handoff and process-group supervision in `env/worker.rs`; the worker, not the outer CLI process, owns the session lock and heartbeat, so killing the outer process leaves the environment live until the harness exits. Authenticate the internal worker with a random, one-shot handoff that external harnesses and manifest commands never inherit. Monitoring failures terminate and reap the complete harness process group before releasing the session lock. Use the repository's structured `kill -- -PGID` process-group boundary; keep raw signal FFI confined to the existing independently reviewed fixed-signal handler.

Persist owner lifecycle before each external side effect. On `up`, compare the plan digest and inventory: finish an idempotent recorded step or enter `TearingDown` and remove recorded partial resources before retrying. Never treat the presence of `env` as proof that provisioning completed.

Treat any subset of `owner.json`, `plan.json`, and `inventory.json` as partial authoritative state and fail closed. Until the Maven/Compose adapters exist, preserve nonempty or mismatched inventory instead of running the current manifest's cleanup against unverified ownership. Teardown removes owner last, records every failure as `CleanupFailed`, and retains the directory plus `lease.lock` as a stable lock tombstone.

- [ ] **Step 4: Run session, signal, and state regressions**

Run: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test -p autospec-core --test runtime_session && cargo test -p autospec-core --test runtime_resources && cargo test -p autospec-cli --test runtime_sessions -- --nocapture && cargo test -p autospec-cli --test runtime_session_security -- --nocapture && cargo test -p autospec-cli --test runtime_resources`

Expected: overlapping sessions share state, signals preserve child exit semantics, partial state reconciles, and teardown occurs once after the final release.

- [ ] **Step 5: Commit reference-counted sessions**

```bash
git add crates/autospec-core/src/runtime_env.rs crates/autospec-core/src/runtime_env/resources.rs crates/autospec-core/src/runtime_env/session.rs crates/autospec-core/tests/runtime_resources.rs crates/autospec-core/tests/runtime_session.rs crates/autospec-cli/src/commands/runtime/env.rs crates/autospec-cli/src/commands/runtime/env/isolation.rs crates/autospec-cli/src/commands/runtime/env/lifecycle.rs crates/autospec-cli/src/commands/runtime/env/session.rs crates/autospec-cli/src/commands/runtime/env/worker.rs crates/autospec-cli/src/commands/runtime/env/state.rs crates/autospec-cli/tests/runtime_commands.rs crates/autospec-cli/tests/runtime_resources.rs crates/autospec-cli/tests/runtime_sessions.rs crates/autospec-cli/tests/runtime_session_security.rs crates/autospec-cli/tests/runtime_state_reconciliation.rs
git commit -m "feat: reference-count shared worktree runtime sessions"
```

## Task 4: Isolate Maven 4 locally installed artifacts

**Files:**
- Create: `crates/autospec-core/src/runtime_env/maven.rs`
- Modify: `crates/autospec-core/src/runtime_env.rs`
- Modify: `crates/autospec-core/tests/runtime_resources.rs`
- Create: `crates/autospec-cli/src/commands/runtime/env/maven.rs`
- Modify: `crates/autospec-cli/src/commands/runtime/env.rs`
- Modify: `crates/autospec-cli/tests/runtime_resources.rs`
- Create: `tests/integration/runtime-maven-isolation.bats`
- Create: `tests/fixtures/runtime-resources/maven/producer/pom.xml`
- Create: `tests/fixtures/runtime-resources/maven/consumer/pom.xml`

**Interfaces:**
- Consumes: `MavenIsolation::SplitLocal`, environment identity, state lease, and inventory.
- Produces: `MavenPlan::arguments(existing: &str, environment_id: &str) -> Result<MavenArgs, IsolationDiagnostic>`.
- Produces: `MavenArgs::parse`, `append_property`, and `render` so quoted caller arguments round-trip into `MAVEN_ARGS`.
- Produces: `MavenPurgeTarget::new`, plus `MavenAdapter::probe`, `effective_local_repository`, `configure`, and `purge_owned_prefix`.
- Adds: `down --purge-maven` while ordinary `down` retains the installed-artifact prefix.

- [ ] **Step 1: Add failing property-merge, conflict, version, and purge-boundary tests**

```rust
#[test]
fn maven_arguments_share_remote_cache_and_split_local_installs() {
    let args = MavenPlan::arguments("-T 2", "sample-a").unwrap();
    assert!(args.tokens().contains(&OsString::from("-Daether.lrm.enhanced.split=true")));
    assert!(args.tokens().contains(&OsString::from("-Daether.lrm.enhanced.remotePrefix=cached")));
    assert!(args.tokens().contains(&OsString::from("-Daether.lrm.enhanced.localPrefix=autospec/sample-a")));
    assert!(args.tokens().contains(&OsString::from("-Daether.system.named.factory=file-lock")));
}

#[test]
fn purge_rejects_a_prefix_that_escapes_the_effective_repository() {
    let error = MavenPurgeTarget::new(Path::new("/m2"), Path::new("/tmp/elsewhere"), "env-a").unwrap_err();
    assert_eq!(error.code, "MAVEN_PURGE_OUTSIDE_REPOSITORY");
}
```

CLI tests place a fake `mvn` first on `PATH`, return Maven `4.0.0`, capture `MAVEN_ARGS`, answer `help:evaluate -Dexpression=settings.localRepository -q -DforceStdout`, and verify Maven 3, conflicting `aether.*` values, symlinked purge targets, and nonempty live sessions fail before deletion.

- [ ] **Step 2: Run focused tests and confirm Maven is not configured**

Run: `cargo test -p autospec-core --test runtime_resources maven_ -- --nocapture && cargo test -p autospec-cli --test runtime_resources maven_ -- --nocapture`

Expected: the Maven module is absent and the fake Maven invocation receives no split-local arguments.

- [ ] **Step 3: Implement Maven 4 probing, argument injection, and exact-prefix purge**

Tokenize `MAVEN_ARGS` with a small POSIX/Windows quote-aware parser; preserve the original tokens and append exactly:

```text
-Daether.lrm.enhanced.split=true
-Daether.lrm.enhanced.remotePrefix=cached
-Daether.lrm.enhanced.localPrefix=autospec/<AGENT_ENV_ID>
-Daether.system.named.factory=file-lock
```

Reject any preexisting assignment to those keys unless it equals the broker value. Probe `mvn --version` and reject a major version other than `4`. Query the effective repository with the same settings and merged arguments. For purge, acquire the environment lease, prove zero live sessions, canonicalize the Maven-reported root without following a symlinked owned prefix, require the lexical target to equal `<root>/autospec/<AGENT_ENV_ID>`, then remove only that tree. Record the target in inventory before deletion and never touch `<root>/cached`.

- [ ] **Step 4: Run fake-boundary tests and the real same-GAV proof**

Run: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test -p autospec-core --test runtime_resources maven_ -- --nocapture && cargo test -p autospec-cli --test runtime_resources maven_ -- --nocapture && bats tests/integration/runtime-maven-isolation.bats`

Expected: two real Maven 4 worktrees install different bytes at one GAV, each consumer resolves its own bytes, and both resolve one remote dependency from the shared `cached` prefix without corruption.

- [ ] **Step 5: Commit Maven isolation**

```bash
git add crates/autospec-core/src/runtime_env.rs crates/autospec-core/src/runtime_env/maven.rs crates/autospec-core/tests/runtime_resources.rs crates/autospec-cli/src/commands/runtime/env.rs crates/autospec-cli/src/commands/runtime/env/maven.rs crates/autospec-cli/tests/runtime_resources.rs tests/fixtures/runtime-resources/maven tests/integration/runtime-maven-isolation.bats
git commit -m "feat: isolate Maven 4 installs per worktree"
```

## Task 5: Reject unsafe resolved Compose models

**Files:**
- Create: `crates/autospec-core/src/runtime_env/compose.rs`
- Modify: `crates/autospec-core/src/runtime_env.rs`
- Modify: `crates/autospec-core/tests/runtime_resources.rs`
- Create: `tests/fixtures/runtime-resources/compose/safe.yaml`
- Create: `tests/fixtures/runtime-resources/compose/fixed-port.yaml`
- Create: `tests/fixtures/runtime-resources/compose/container-name.yaml`
- Create: `tests/fixtures/runtime-resources/compose/host-network.yaml`
- Create: `tests/fixtures/runtime-resources/compose/global-name.yaml`
- Create: `tests/fixtures/runtime-resources/compose/fixed-ip.yaml`
- Create: `tests/fixtures/runtime-resources/compose/external.yaml`
- Create: `tests/fixtures/runtime-resources/compose/writable-bind.yaml`
- Create: `crates/autospec-cli/src/commands/runtime/env/compose.rs`
- Modify: `crates/autospec-cli/tests/runtime_resources.rs`

**Interfaces:**
- Consumes: `ComposePlan` and declared `ComposeExport` values from Task 2.
- Produces: `ComposePolicy::evaluate(model: &serde_json::Value, plan: &ComposePlan) -> Vec<IsolationDiagnostic>`.
- Produces stable rule IDs: `COMPOSE_FIXED_PORT`, `COMPOSE_UNDECLARED_PORT`, `COMPOSE_CONTAINER_NAME`, `COMPOSE_HOST_NETWORK`, `COMPOSE_GLOBAL_NAME`, `COMPOSE_FIXED_ADDRESS`, `COMPOSE_EXTERNAL_UNDECLARED`, and `COMPOSE_WRITABLE_BIND_OUTSIDE_WORKTREE`.
- Produces: `ComposeAdapter::resolved_model` using `docker compose ... config --format json`.

- [ ] **Step 1: Add one failing test per stable rule and one safe model**

```rust
#[test]
fn fixed_host_port_reports_service_path_and_value() {
    let diagnostics = evaluate_fixture("fixed-port.json", ComposePlan::fixture());
    assert_eq!(diagnostics[0].code, "COMPOSE_FIXED_PORT");
    assert_eq!(diagnostics[0].resource, "services.web.ports[0].published");
    assert_eq!(diagnostics[0].evidence, "8080");
}

#[test]
fn declared_target_without_a_published_port_is_safe() {
    assert!(evaluate_fixture("safe.json", ComposePlan::fixture()).is_empty());
}
```

Add CLI tests that prove all `-f` arguments and `--project-name` precede `config --format json`, nonzero Compose exits are preserved, and validation runs before `up`.

- [ ] **Step 2: Run the policy tests and confirm no resolved-model gate exists**

Run: `cargo test -p autospec-core --test runtime_resources compose_policy_ -- --nocapture && cargo test -p autospec-cli --test runtime_resources compose_config_ -- --nocapture`

Expected: the policy module and `ComposeAdapter` are missing.

- [ ] **Step 3: Implement resolved-model validation without editing source YAML**

Parse the Compose JSON as `serde_json::Value`, walk exact known model paths, and return every violation in deterministic path order. Treat external resources as safe only when the v2 manifest declares their exact logical key in `shared_resources`. Allow read-only binds and writable absolute binds contained by the canonical worktree. Reject unknown port protocols and any published port not generated by the broker. Diagnostics must include code, environment ID, resource path, evidence, and the rerunnable command `autospec runtime env normalize-compose --repo <repo> --check` when normalization is eligible.

- [ ] **Step 4: Run policy, fixture, and repository gates**

Run: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test -p autospec-core --test runtime_resources compose_ -- --nocapture && cargo test -p autospec-cli --test runtime_resources compose_ -- --nocapture && cargo run -q -p autospec-cli -- validate --fast`

Expected: every unsafe fixture has exactly its stable rule ID and path, the safe fixture passes, and no Docker `up` call occurs after a validation failure.

- [ ] **Step 5: Commit the Compose policy gate**

```bash
git add crates/autospec-core/src/runtime_env.rs crates/autospec-core/src/runtime_env/compose.rs crates/autospec-core/tests/runtime_resources.rs crates/autospec-cli/src/commands/runtime/env/compose.rs crates/autospec-cli/tests/runtime_resources.rs tests/fixtures/runtime-resources/compose
git commit -m "feat: fail closed on unsafe Compose resources"
```

## Task 6: Provision and inventory Compose resources with Docker-assigned ports

**Files:**
- Modify: `crates/autospec-core/src/runtime_env/compose.rs`
- Modify: `crates/autospec-core/src/runtime_env/resources.rs`
- Modify: `crates/autospec-core/tests/runtime_resources.rs`
- Modify: `crates/autospec-cli/src/commands/runtime/env/compose.rs`
- Modify: `crates/autospec-cli/src/commands/runtime/env.rs`
- Modify: `crates/autospec-cli/tests/runtime_resources.rs`
- Create: `tests/integration/runtime-compose-isolation.bats`
- Create: `tests/fixtures/runtime-resources/compose-stack/compose.yaml`
- Create: `tests/fixtures/runtime-resources/compose-stack/.autospec/runtime.yml`

**Interfaces:**
- Consumes: validated Compose plan, environment lease, lifecycle state, and inventory.
- Consumes automatic resource-only plans for compose-only repositories without requiring a manifest mode command.
- Produces: `ComposeOverride::render(plan) -> Result<String, IsolationDiagnostic>` containing only export ports and ownership labels.
- Produces: `ComposeAdapter::up`, `discover_inventory`, `resolve_exports`, and `down_owned`.
- Extends `RuntimeState` with only declared export environment values and canonical public URL selection.

- [ ] **Step 1: Add failing override, inventory, preserve-volume, and refcount tests**

```rust
#[test]
fn override_lets_docker_choose_a_loopback_host_port() {
    let rendered = ComposeOverride::render(&ComposePlan::one_http_export("web", 8080)).unwrap();
    assert!(rendered.contains("target: 8080"));
    assert!(rendered.contains("host_ip: 127.0.0.1"));
    assert!(!rendered.contains("published:"));
    assert!(rendered.contains("autospec.environment-id"));
}

#[test]
fn preserved_volume_is_not_in_the_delete_set() {
    let inventory = compose_inventory_with_volumes(&["db-data", "cache-data"]);
    assert_eq!(inventory.deletable_volumes(&[String::from("db-data")]), vec![String::from("cache-data")]);
}
```

CLI tests provide a fake Docker command that returns container/network/volume IDs and `127.0.0.1:49152` from `compose port`; assert `inventory.json` stores actual IDs and `AUTOSPEC_PUBLIC_URL=http://127.0.0.1:49152`.

- [ ] **Step 2: Run focused tests and confirm Compose resources are not started**

Run: `cargo test -p autospec-core --test runtime_resources compose_override_ -- --nocapture && cargo test -p autospec-cli --test runtime_resources compose_lifecycle_ -- --nocapture`

Expected: override and lifecycle methods are missing.

- [ ] **Step 3: Implement lease-ordered Compose lifecycle and exact ownership teardown**

Write the override atomically inside the environment state directory. Run:

```text
docker compose -f <source> -f <override> --project-name <project> up -d --remove-orphans
docker compose -f <source> -f <override> --project-name <project> port <service> <target>/<protocol>
```

Wire `up`, `exec`, and `session` to retain and provision the automatic plan returned by Task 2, including repositories with only a standard Compose file and no runtime manifest.

Add `com.autospec.environment-id`, `com.autospec.owner-key`, and `com.autospec.plan-digest` labels. Inventory actual resource IDs after every successful external step. Before `down`, capture anonymous and named volume mount IDs; run Compose down without `--volumes`; then delete only labeled, recorded, non-preserved volume IDs. Verify containers, networks, and deletable volumes are gone before deleting state. A failed check leaves `CleanupFailed` state and a rerunnable recovery command.

Reject a caller-provided `COMPOSE_PROJECT_NAME`; the broker's generation-aware project name is authoritative. Map HTTP and HTTPS exports to URLs, TCP and UDP exports to numeric ports, and `ExportValue::HostPort` to `127.0.0.1:<port>`. Set `AUTOSPEC_PUBLIC_URL` only for the manifest's unique canonical HTTP(S) export.

- [ ] **Step 4: Run fake-boundary tests and a real two-stack proof**

Run: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test -p autospec-core --test runtime_resources compose_ -- --nocapture && cargo test -p autospec-cli --test runtime_resources compose_ -- --nocapture && bats tests/integration/runtime-compose-isolation.bats`

Expected: two worktrees have distinct project, network, volume, and host-port IDs; both URLs respond; one of two sessions exiting leaves its shared stack running; final teardown leaks no owned resource.

- [ ] **Step 5: Commit Compose lifecycle ownership**

```bash
git add crates/autospec-core/src/runtime_env/compose.rs crates/autospec-core/src/runtime_env/resources.rs crates/autospec-core/tests/runtime_resources.rs crates/autospec-cli/src/commands/runtime/env.rs crates/autospec-cli/src/commands/runtime/env/compose.rs crates/autospec-cli/tests/runtime_resources.rs tests/fixtures/runtime-resources/compose-stack tests/integration/runtime-compose-isolation.bats
git commit -m "feat: own isolated Compose stacks and dynamic exports"
```

## Task 7: Add the deterministic Compose normalizer

**Files:**
- Create: `crates/autospec-core/src/runtime_env/compose_normalize.rs`
- Modify: `crates/autospec-core/src/runtime_env.rs`
- Modify: `crates/autospec-core/tests/runtime_resources.rs`
- Modify: `crates/autospec-cli/src/commands/runtime/env/options.rs`
- Modify: `crates/autospec-cli/src/commands/runtime/env.rs`
- Modify: `crates/autospec-cli/tests/runtime_resources.rs`
- Create: `tests/fixtures/compose-normalize/fixed-port/`
- Create: `tests/fixtures/compose-normalize/container-name/`
- Create: `tests/fixtures/compose-normalize/project-name/`
- Create: `tests/fixtures/compose-normalize/multiple-http/`
- Create: `tests/fixtures/compose-normalize/external/`
- Create: `tests/fixtures/compose-normalize/host-network/`

**Interfaces:**
- Consumes: the exact `ComposePolicy` diagnostics from Task 5 and v2 manifest editor from Task 2.
- Produces: `NormalizationPlan { schema_version, fingerprint, edits, remaining_diagnostics }`.
- Produces: `ComposeNormalizer::plan(repo, compose_files, manifest)`, `apply(plan)`, and `verify(plan)`.
- Adds: `autospec runtime env normalize-compose --repo PATH --check|--apply --fingerprint SHA256`.

- [ ] **Step 1: Add failing safe-transform, ambiguity, byte-preservation, and idempotence tests**

```rust
#[test]
fn fixed_port_becomes_a_manifest_export_without_touching_comments() {
    let fixture = NormalizeFixture::open("fixed-port");
    let plan = ComposeNormalizer::plan(fixture.repo()).unwrap();
    let output = plan.rendered_files().unwrap();
    assert!(output.compose.contains("# keep database explanation"));
    assert!(!output.compose.contains("8080:8080"));
    assert!(output.manifest.contains("env: AUTOSPEC_COMPOSE_WEB_8080_TCP"));
}

#[test]
fn applying_the_same_fingerprint_twice_is_a_byte_noop() {
    let fixture = NormalizeFixture::open("fixed-port");
    let first = fixture.apply_once().unwrap();
    let second = fixture.apply_once().unwrap();
    assert_eq!(first, second);
}
```

Assert host networking, external resources, multiple candidate public URLs, anchors that change edit meaning, and unrecognized fixed-port syntax leave files unchanged with stable fail-closed diagnostics.

- [ ] **Step 2: Run focused tests and confirm the transformer is missing**

Run: `cargo test -p autospec-core --test runtime_resources normalize_ -- --nocapture && cargo test -p autospec-cli --test runtime_resources normalize_ -- --nocapture`

Expected: `ComposeNormalizer` and `normalize-compose` are undefined.

- [ ] **Step 3: Implement lossless, fingerprinted edits shared with the policy gate**

Use `yaml_edit::Document` for source files and SHA-256 over canonical file paths, original bytes, plan schema, and policy version. Eligible edits are exact:

```rust
pub enum NormalizationEdit {
    RemovePublishedPort { service: String, index: usize, export: ComposeExport },
    RemoveRedundantContainerName { service: String },
    RemoveProjectScopedResourceName { kind: ResourceKind, logical_key: String },
    UpsertRuntimeResources { resources: RuntimeResources },
}
```

A fixed port becomes `protocol: tcp` and `AUTOSPEC_COMPOSE_<SERVICE>_<TARGET>_TCP` unless Compose declares `app_protocol: http|https`; only a single declared HTTP(S) candidate also receives `AUTOSPEC_PUBLIC_URL`. Remove `container_name` only when it equals the service name and no resolved-model reference uses that literal. Remove an explicit network/volume name only when it is `${COMPOSE_PROJECT_NAME}_<logical-key>`. Apply all files through same-directory temporary files plus atomic renames after rechecking the fingerprint. Re-run `docker compose config` and `ComposePolicy`; rollback all files if either fails.

- [ ] **Step 4: Run idempotence, CLI, and real Compose-config checks**

Run: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test -p autospec-core --test runtime_resources normalize_ -- --nocapture && cargo test -p autospec-cli --test runtime_resources normalize_ -- --nocapture && docker compose -f tests/fixtures/compose-normalize/fixed-port/compose.yaml config`

Expected: eligible fixtures transform once and validate; the second run is byte-identical; ineligible fixtures remain byte-identical with stable diagnostics.

- [ ] **Step 5: Commit the shared normalizer**

```bash
git add crates/autospec-core/src/runtime_env.rs crates/autospec-core/src/runtime_env/compose_normalize.rs crates/autospec-core/tests/runtime_resources.rs crates/autospec-cli/src/commands/runtime/env.rs crates/autospec-cli/src/commands/runtime/env/options.rs crates/autospec-cli/tests/runtime_resources.rs tests/fixtures/compose-normalize
git commit -m "feat: normalize safe Compose isolation changes deterministically"
```

## Task 8: Add the internal normalizer skill and one-time migration workflow

**Files:**
- Create: `skills/autospec-compose-normalize/SKILL.md`
- Create: `skills/autospec-compose-normalize/codex/prompt.md`
- Create: `skills/autospec-compose-normalize/opencode/agent.md`
- Create: `skills/autospec-compose-normalize/README.md`
- Create: `skills/autospec-compose-normalize/install.sh`
- Create: `skills/autospec-compose-normalize/uninstall.sh`
- Create: `tests/unit/test_autospec_compose_normalize_skill.bats`
- Create: `templates/skill-blocks/runtime-resource-preflight.md`
- Modify: `skills/autospec-run/SKILL.md`, `skills/autospec-run/codex/prompt.md`, `skills/autospec-run/opencode/agent.md`
- Modify: `skills/autospec/SKILL.md`, `skills/autospec/codex/prompt.md`, `skills/autospec/opencode/agent.md`
- Modify: `tests/autospec-run-agent-env-contract.bats`
- Modify: `crates/autospec-core/src/validation/external.rs`
- Create: `tests/fixtures/skill-goldens/autospec-compose-normalize.SKILL.md.sha256`
- Create: `tests/fixtures/skill-goldens/autospec-compose-normalize.codex.prompt.md.sha256`
- Create: `tests/fixtures/skill-goldens/autospec-compose-normalize.opencode.agent.md.sha256`

**Interfaces:**
- Consumes: `normalize-compose --check|--apply --fingerprint` from Task 7.
- Produces: an internal skill that never authors YAML and owns migration issue/worktree/PR orchestration.
- Produces: one shared runtime-preflight prompt block used by both `/autospec` and `/autospec-run` trios.
- Uses: `claim-guard.sh` to atomically claim all Compose files plus the selected runtime manifest.

- [ ] **Step 1: Add failing skill-structure and transparent-preflight tests**

```bash
run grep -F 'autospec runtime env normalize-compose --repo "$PWD" --check' "$REPO_ROOT/skills/autospec-compose-normalize/SKILL.md"
[ "$status" -eq 0 ]
run grep -E 'apply_patch|cat .*compose|yq .* -i' "$REPO_ROOT/skills/autospec-compose-normalize/SKILL.md"
[ "$status" -ne 0 ]
```

Contract tests must require: a complete trio, one deterministic Rust delegation, fingerprint reuse, atomic claim targets, exactly one migration issue/branch, normal CI/review, direct-session refusal while migration is required, and unconditional runtime preflight even when no manifest exists.

- [ ] **Step 2: Run the focused tests and confirm the skill/preflight are absent**

Run: `bats tests/unit/test_autospec_compose_normalize_skill.bats tests/autospec-run-agent-env-contract.bats`

Expected: the normalizer skill is missing and the existing preflight still contains the manifest-only condition.

- [ ] **Step 3: Land the complete skill and shared prerequisite workflow atomically**

The skill sequence is fixed:

```text
check -> read fingerprint -> find matching open/merged migration -> claim files -> create/lint/classify one issue -> create worktree -> apply -> verify -> commit -> PR -> CI/review -> merge -> release claim
```

Use `<!-- autospec-compose-fingerprint: SHA256 -->` in the issue and PR bodies. The issue must satisfy `scripts/lint-issue.sh`, enter through `needs-classify`, and complete deterministic classification before implementation begins. If a matching migration is open, wait/reuse it; if merged, merge current `origin/main` into the feature worktree and retry `up`. A direct unmanaged harness prints the issue/PR and exits before claiming isolation. An Autospec-managed run executes the skill automatically. Add a shared prompt block marker to authoritative `SKILL.md` files, derive both trios, and regenerate goldens; do not hand-edit mirror bodies. The skill installer fails with a precise top-level bootstrap command when the installed Rust binary lacks `normalize-compose`; it never installs a second transformer.

- [ ] **Step 4: Run trio, install, claim, and validation gates**

Run: `bash scripts/derive-trio.sh skills/autospec-compose-normalize --in-place && bash scripts/derive-trio.sh skills/autospec-run --in-place && bash scripts/derive-trio.sh skills/autospec --in-place && bash scripts/gen-skill-goldens.sh autospec-compose-normalize autospec-run autospec && bats tests/unit/test_autospec_compose_normalize_skill.bats tests/autospec-run-agent-env-contract.bats tests/derive-trio.bats tests/gen-skill-goldens.bats tests/block-expansion-gate.bats tests/install-expansion.bats && cargo run -q -p autospec-cli -- validate --fast`

Expected: all three skill bodies are lock-step, generated goldens match, both Phase 4 entry points use automatic preflight, and no prompt contains a second YAML policy implementation.

- [ ] **Step 5: Commit the transparent migration skill**

```bash
git add skills/autospec-compose-normalize skills/autospec-run skills/autospec templates/skill-blocks/runtime-resource-preflight.md tests/unit/test_autospec_compose_normalize_skill.bats tests/autospec-run-agent-env-contract.bats tests/fixtures/skill-goldens crates/autospec-core/src/validation/external.rs
git commit -m "feat: migrate Compose isolation prerequisites transparently"
```

## Task 9: Generate harness aliases and integrate conservative cleanup

**Files:**
- Create: `config/harness-runtime-aliases.tsv`
- Create: `scripts/gen-harness-runtime-aliases.sh`
- Create: `templates/generated/harness-runtime-aliases.sh`
- Create: `templates/generated/harness-runtime-aliases.fish`
- Create: `docs/generated/harness-runtime-aliases.md`
- Modify: `install.sh`
- Modify: `uninstall.sh`
- Modify: `scripts/autospec-session`
- Modify: `scripts/lib/autospec-harness-detect.sh`
- Modify: `tests/agent-env-install.bats`
- Modify: `tests/install-rollover.bats`
- Create: `tests/harness-runtime-alias-generation.bats`
- Create: `crates/autospec-core/src/runtime_env/ports.rs`
- Create: `crates/autospec-cli/src/commands/runtime/env/gc.rs`
- Modify: `crates/autospec-cli/src/commands/runtime/env.rs`
- Modify: `crates/autospec-core/tests/runtime_resources.rs`
- Modify: `crates/autospec-cli/tests/runtime_resources.rs`
- Create: `scripts/autospec-runtime-worktree-cleanup.sh`
- Create: `tests/runtime-worktree-cleanup.bats`
- Modify: `skills/autospec-run/SKILL.md`, `skills/autospec-run/codex/prompt.md`, `skills/autospec-run/opencode/agent.md`
- Modify: `skills/autospec/SKILL.md`, `skills/autospec/codex/prompt.md`, `skills/autospec/opencode/agent.md`
- Modify: `scripts/autospec-watchdog.sh`
- Modify: `tests/resume/test_watchdog_gc.bats`

**Interfaces:**
- Produces: one harness table with rows `claude`, `codex`, and `opencode`, consumed by ordinary aliases and rollover wrappers.
- Produces: `PortRegistry::claim`, `PortClaim::release`, and bounded direct-server bind retry.
- Adds: `autospec runtime env gc`; automatic GC runs during `up`, `status`, and runtime-aware worktree cleanup.
- Produces: `autospec-runtime-worktree-cleanup.sh PATH` as a thin Rust adapter; `worktree-guard.sh` remains Git-only.

- [ ] **Step 1: Add failing alias generation, port collision, and stale-owner tests**

```rust
#[test]
fn fixed_port_claim_conflicts_across_environment_ids() {
    let mut registry = PortRegistry::default();
    registry.claim_fixed("env-a", 42000).unwrap();
    let error = registry.claim_fixed("env-b", 42000).unwrap_err();
    assert_eq!(error.code, "PORT_ALREADY_CLAIMED");
}

#[test]
fn gc_refuses_a_resource_with_an_owner_label_mismatch() {
    let decision = GcPolicy::evaluate(stale_owner(), inventory_with_owner("different-owner"));
    assert_eq!(decision, GcDecision::Ambiguous("RESOURCE_OWNER_MISMATCH"));
}
```

Bats tests require every generated Bash/Zsh/Fish and rollover command to contain `autospec-env session -- <harness>`, reject duplicate table IDs, and prove watchdog cleanup calls the runtime adapter only after existing unpushed/heartbeat/issue safety checks.

- [ ] **Step 2: Run focused tests and confirm duplicated aliases and missing GC**

Run: `cargo test -p autospec-core --test runtime_resources -- --nocapture && bats tests/harness-runtime-alias-generation.bats tests/runtime-worktree-cleanup.bats`

Expected: port/GC symbols and scripts are missing; aliases remain duplicated in `install.sh` and rollover bypasses the broker.

- [ ] **Step 3: Implement the table generator, port registry, and ownership-safe GC**

Use this canonical table grammar:

```text
claude	claude	--dangerously-skip-permissions	Claude Code
codex	codex	--yolo	Codex CLI
opencode	opencode		OpenCode
```

The generator validates four tab-separated fields and emits deterministic POSIX/Fish/docs output. `install.sh` consumes generated files; `autospec-session` enters the same broker session before tmux launch.

Make `scripts/lib/autospec-harness-detect.sh` read the same harness IDs/executables and add a validation assertion that its supported harness set exactly equals the TSV set; permission arguments remain runtime-alias fields and are never duplicated in detection code.

Store direct-server claims under `${AGENT_ENV_STATE_ROOT:-$HOME/.autospec/envs}/ports/registry.json` with `ports/lease.lock`. Lock order is always environment lease, then port-registry lease, and release occurs in reverse order. While holding the registry claim, bind a probe listener, close it immediately before child launch, and wait for the declared bind/health condition; a bind failure releases and reallocates, with at most five attempts. GC requires a missing/mismatched worktree-generation token, zero locked session records, matching Docker ownership labels, and no inventory entry owned by another live environment. Ambiguity returns a stable recovery command without deletion.

The cleanup adapter calls `autospec runtime env gc --repo <worktree>` and propagates failure. Wire it after the watchdog's existing Git safety gates and before Git removal in both Phase 4 trios; remove `|| true` teardown masking. Do not add runtime behavior to `worktree-guard.sh`.

When a worktree-generation token is proven stale, GC also invokes Task 4's guarded Maven-prefix purge. `status` and `up` invoke the same collector before reconciliation. `AUTOSPEC_ENV_DISABLE=1` cannot disable `down` or `gc` for resources already recorded as broker-owned.

- [ ] **Step 4: Run alias, rollover, GC, lock-step, and regression gates**

Run: `bash scripts/gen-harness-runtime-aliases.sh --check && bash -n install.sh scripts/gen-harness-runtime-aliases.sh scripts/autospec-session scripts/autospec-runtime-worktree-cleanup.sh && cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test -p autospec-core --test runtime_resources -- --nocapture && cargo test -p autospec-cli --test runtime_resources -- --nocapture && bats tests/harness-runtime-alias-generation.bats tests/agent-env-install.bats tests/install-rollover.bats tests/runtime-worktree-cleanup.bats tests/resume/test_watchdog_gc.bats tests/autospec-run-agent-env-contract.bats && cargo run -q -p autospec-cli -- validate --fast`

Expected: all harness surfaces derive from one table, rollover cannot bypass the broker, stale owned resources are removed, ambiguous resources remain, and every trio/golden check passes.

- [ ] **Step 5: Commit aliases, ports, and cleanup integration**

```bash
git add config/harness-runtime-aliases.tsv scripts/gen-harness-runtime-aliases.sh templates/generated docs/generated install.sh uninstall.sh scripts/autospec-session scripts/lib/autospec-harness-detect.sh tests/agent-env-install.bats tests/install-rollover.bats tests/harness-runtime-alias-generation.bats crates/autospec-core/src/runtime_env/ports.rs crates/autospec-core/tests/runtime_resources.rs crates/autospec-cli/src/commands/runtime/env.rs crates/autospec-cli/src/commands/runtime/env/gc.rs crates/autospec-cli/tests/runtime_resources.rs scripts/autospec-runtime-worktree-cleanup.sh tests/runtime-worktree-cleanup.bats skills/autospec-run skills/autospec scripts/autospec-watchdog.sh tests/resume/test_watchdog_gc.bats
git commit -m "feat: keep every harness inside owned runtime resources"
```

## Task 10: Document and prove forty concurrent stacks

**Files:**
- Create: `tests/integration/runtime-compose-40-stack.bats`
- Create: `tests/fixtures/runtime-resources/forty-stack/compose.yaml`
- Create: `tests/fixtures/runtime-resources/forty-stack/.autospec/runtime.yml`
- Create: `tests/fixtures/runtime-resources/forty-stack/Dockerfile`
- Modify: `docs/runbooks/agent-runtime-manifest.md`
- Modify: `docs/runbooks/agent-runtime-companion-stacks.md`
- Modify: `docs/cli-reference.md`
- Modify: `docs/CONFIG_REFERENCE.md`
- Modify: `docs/USER_MANUAL.md`
- Modify: `README.md`
- Modify: `AGENTS.md`
- Create: `docs/memory/feedback_worktree_resource_isolation.md`
- Modify: `docs/memory/MEMORY.md`
- Modify: `tests/smoke/test_install_all_skills.bats`
- Modify: `crates/autospec-core/src/validation/external.rs`
- Modify: `crates/autospec-cli/src/commands/runtime/env/state.rs`
- Modify: `crates/autospec-cli/tests/runtime_state_reconciliation.rs`

**Interfaces:**
- Consumes: every broker, skill, alias, and cleanup contract from Tasks 1–9.
- Produces: a rerunnable real-engine proof report containing peak containers, startup/teardown duration, collisions, retries, and leaks.
- Documents: manifest v2, opt-outs, proof downgrades, diagnostics, recovery, and exact Maven/Compose ownership semantics.
- Enforces: private runtime state (`0700` directories and `0600` files on Unix) and rejects symlinked state/session roots before destructive cleanup.

- [ ] **Step 1: Add the failing forty-stack proof and documentation assertions**

The proof creates 40 linked worktrees, starts them concurrently, and records one row per environment:

```text
environment_id,compose_project,container_id,network_id,volume_id,host_port,http_status
```

It asserts 40 unique values in each resource column, HTTP status `200` for every generated URL, zero collision/retry exhaustion, reference-count survival in selected worktrees, crash recovery at provisioning/teardown checkpoints, and zero labeled resources after cleanup. Unix security regressions assert state directories are mode `0700`, authoritative files are `0600`, and symlinked environment/session roots fail closed before cleanup. Documentation tests require every public command, v2 key, opt-out, and stable recovery code.

- [ ] **Step 2: Run the proof before the final wiring and capture any red assertion**

Run: `bats tests/integration/runtime-compose-40-stack.bats`

Expected: at least one final documentation/reporting or 40-stack assertion fails until this task's fixtures and evidence collector are complete; engine absence is a hard failure, not a skip.

- [ ] **Step 3: Complete the proof collector and public documentation**

Build one local lightweight HTTP fixture image, reuse it across all stacks, and write the CSV plus JSON summary to `reports/runtime-isolation/`. Never pull one image per worktree. Document:

```text
up | status | down | exec | session | gc | normalize-compose | down --purge-maven
AUTOSPEC_MAVEN_ISOLATION=off | AUTOSPEC_COMPOSE_ISOLATION=off | AUTOSPEC_ENV_DISABLE=1
```

Explain that opt-outs export `AUTOSPEC_ISOLATION_BYPASSED=1` and downgrade isolation claims from verified. Update generated documentation sources before rendered outputs. Extend install smoke inventory for the new skill.

- [ ] **Step 4: Run the complete release evidence sequence without skipped engine gates**

Run: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && bats tests/integration/runtime-maven-isolation.bats && bats tests/integration/runtime-compose-isolation.bats && bats tests/integration/runtime-compose-40-stack.bats && cargo run -q -p autospec-cli -- validate --fast && cargo run -q -p autospec-cli -- validate && git diff --check`

Expected: all Rust, Bats, lock-step, generated, Maven 4, Docker Compose, 40-stack, permission/symlink hardening, and leak checks pass; the 40-stack JSON reports `collisions: 0`, `retry_exhaustions: 0`, and `leaked_resources: 0`.

- [ ] **Step 5: Commit final proof and documentation**

```bash
git add tests/integration/runtime-compose-40-stack.bats tests/fixtures/runtime-resources/forty-stack docs README.md AGENTS.md tests/smoke/test_install_all_skills.bats crates/autospec-core/src/validation/external.rs reports/runtime-isolation
git commit -m "test: prove forty isolated worktree runtime stacks"
```

## Final consolidation gate

- [ ] Create or confirm the ten linked child issues under #2103 and ensure each merged PR cites its task and source spec.
- [ ] Fetch `origin`, merge current `origin/main` into the consolidation branch without rewriting published commits, and rerun the complete Task 10 evidence sequence.
- [ ] Verify `git status --short` is empty and `git diff --check origin/main...HEAD` passes.
- [ ] Run the repository's implementation linter against the consolidation diff and resolve every blocking RULE_ID.
- [ ] Open the final PR with the Maven and 40-stack reports attached, obtain the required review, and ship through `/gw:merge-it` only when required CI and both real-engine proofs are green.
