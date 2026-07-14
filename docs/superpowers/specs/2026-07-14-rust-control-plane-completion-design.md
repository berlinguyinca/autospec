# Rust Control-Plane Completion Design

**Status:** approved for autonomous execution

## Goal

Move Autospec's stateful orchestration and deterministic policy execution into Rust, then remove the shell and Python implementations that duplicate those runtime authorities.

## Scope and non-goals

This is a control-plane migration, not a rewrite of every executable file. Rust owns persisted state, manifest parsing, policy evaluation, leases, scheduling, process orchestration, and machine-readable reporting. Thin POSIX/PowerShell install and launch bridges remain only where they start the Rust binary or a supported external tool. Bats, Python, Cargo, Node, `gh`, `git`, `tmux`, and `omx` remain external integrations invoked through typed Rust adapters; they are not legacy fallback authorities.

R2 helpers, generated files, fixtures, and R4 CAD/FAB integrations are excluded unless a concrete runtime path requires them.

## Architecture

`autospec` becomes the sole executable authority for each migrated vertical. Its Rust core exposes typed, side-effect-free models and policies; the CLI owns argument parsing, structured output, and subprocess boundaries. Compatibility scripts first become one-line delegators to the equivalent Rust subcommand, then are removed along with all prompt and installer references after parity is proven.

Every cutover follows the same gate:

1. Freeze the shell/Python contract in fixtures, including output, exit codes, state transitions, unsafe-input handling, and cleanup behavior.
2. Add a failing Rust test for each frozen behavior.
3. Implement the Rust command through typed filesystem and subprocess adapters.
4. Run the existing shell/Python tests against the delegator or a shared fixture corpus.
5. Observe the Rust path with explicit rollback only where an external process lifecycle requires it.
6. Replace every live caller, delete the former authority, and add a static test that rejects its reintroduction.

## Migration order

### 1. Runtime manifest and isolated-agent broker

Implement a typed `.autospec/runtime.yml` model with `.agent-runtime.yml` fallback, then add `autospec runtime env init|up|status|down|exec|session`. Preserve manifest precedence, dynamic-port allocation, generated environment-file variables, command exit status (including `42`), idempotent teardown, and per-repository compose naming. Update installed `agent-env`/`autospec-env` launchers and all lock-step autospec-run prompts to invoke the Rust command. Delete `scripts/agent-env.sh` only when the Bats contract passes through the Rust command.

### 2. Deterministic lint and lease APIs

Expand Rust issue and implementation lint modules until every deterministic rule, documented opt-out, stable RULE_ID, JSON record, directive, and exit status has fixture parity with the shell gates. Expose `autospec lint issue` and `autospec lint implementation`. Port file/skill lease acquire, assert, refresh, release, and status behavior into `autospec claim`, retaining atomic writes, heartbeat TTL, conflict reporting, and repository scoping. Replace installed scripts and pre-commit hooks only after their existing Bats suites pass against Rust.

### 3. Run coordination and autonomous conductor

Use the existing Rust queue, state, evidence, and agent-contract models as the basis for an execution coordinator. Add typed adapters for GitHub claim comments, git worktrees, tmux/agent launch, CI sentinels, watchdog liveness, and stop state. First run the coordinator in dry-run and recorded-event modes; then replace the shell conductor and run-state/watchdog authority after proving no duplicate claims, correct server-time stale reclamation, crash recovery, and stop-boundary behavior. Rust may launch external agents but must own the lifecycle decision and persistent run record.

### 4. Context-monitor driver

Retain the existing Rust threshold engine and move the Python driver's adapter, hook-installation, transcript injection, handoff validation, telemetry, and daemon lifecycle responsibilities into a Rust `autospec context-monitor` command. Preserve the Python action sequence and invalid-input contract. Make Python an explicit one-release escape hatch while collecting installation, process-count, latency, and handoff-success parity evidence. Remove the Python package and wrapper only after that observation gate succeeds.

## Error handling and safety

Commands reject malformed paths and manifests before touching state, serialize external command failures without flattening their exit codes, write state atomically, and make cleanup idempotent. Rust adapters pass arguments directly instead of using shell interpolation. Network and destructive operations remain guarded by the existing safe-mode policy and explicit user/operator configuration.

## Verification

Each vertical adds focused Rust unit and CLI tests, preserves its current Bats/Python regression suite, and contributes a static reachability test proving live installers, prompts, hooks, and wrappers no longer call the deleted authority. The final gate is `cargo test --workspace` plus `autospec validate`; no migration is declared complete while the runtime audit reports the replaced implementation as an active R1 authority.

## Completion definition

The conversion is complete when the runtime audit identifies no production R1 shell/Python authority for the four verticals above, `autospec` owns their persisted state and decisions, all live callers use it, their legacy implementations are deleted, and the full repository validation suite passes.
