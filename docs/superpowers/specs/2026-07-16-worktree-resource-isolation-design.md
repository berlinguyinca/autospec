# Worktree Resource Isolation Design

**Status:** approved for specification and decomposition

**Source issue:** [#2103](https://github.com/berlinguyinca/autospec/issues/2103)

## Goal

Make every Autospec feature worktree a collision-free local environment for Maven 4 builds, Docker Compose stacks, host ports, networks, volumes, and agent sessions without duplicating the shared Maven download cache.

## Problem

Autospec already creates linked Git worktrees and derives a runtime environment ID from the canonical repository path and mode. It also emits two dynamic ports and a Compose project name. That isolates Git edits and some conventional runtime names, but it does not own the actual Maven or Compose resources.

Concurrent worktrees can therefore:

- overwrite locally installed Maven artifacts with the same GAV coordinates in one shared local repository;
- race on Maven metadata and partial downloads across Maven processes;
- bind the same fixed Compose host ports;
- share containers, networks, or volumes through explicit global names;
- tear down a stack still used by another agent session;
- leak Docker resources after a crash; and
- reuse stale state when a new worktree is created at an old filesystem path.

A separate `.m2` repository for every worktree is not acceptable because shared dependency caches can occupy hundreds of gigabytes.

## Decisions

1. The Rust `autospec runtime env` control plane remains the single resource authority.
2. Maven 4 is the minimum supported Maven version. No Maven 3 compatibility path is required.
3. Maven downloads remain shared; locally installed artifacts are isolated by worktree environment ID.
4. Compose isolation activates automatically when Autospec detects a supported Compose file.
5. Unsafe Compose resources fail closed. Autospec does not guess about ambiguous infrastructure.
6. Only host ports declared as runtime exports are published.
7. Broker-owned Docker volumes are removed on `down` by default; explicitly preserved volumes survive.
8. Multiple agent sessions in the same worktree and mode share one Maven namespace and Compose stack through session reference counting.
9. Safe stale-resource collection runs during `up`, `status`, and worktree cleanup.
10. Codex, Claude, and OpenCode aliases use one generated, lock-step broker contract.
11. A transparent internal Compose-normalization skill performs deterministic, tracked, one-time migrations when every required change is safe.
12. The completed system must prove 40 concurrent worktree stacks without resource collisions.

## Non-goals

- Supporting Maven 3 or legacy Resolver implementations.
- Copying, hard-linking, overlay-mounting, or manually editing Maven repository content.
- Replacing Docker Compose with Kubernetes or a custom container scheduler.
- Automatically modifying external networks, external volumes, host networking, fixed IP layouts, secrets, credentials, or privileged host mounts.
- Isolating deliberately shared remote services.
- Providing hostile-process containment; a process that deliberately clears broker variables or invokes Docker outside the broker is an explicit bypass.

## Existing foundation

`RuntimeContext::new` canonicalizes the repository path and hashes it with the selected mode. `RuntimeState` emits `AGENT_ENV_ID`, two ports, public URLs, and `COMPOSE_PROJECT_NAME`. Autospec-run provisions this state after entering an issue worktree, and the installed `agent-env` and `autospec-env` commands delegate to the Rust CLI.

This design extends those types and commands. `worktree-guard.sh` remains responsible only for Git checkout safety; it does not become a runtime resource manager.

## Architecture

### Worktree identity

The broker resolves the Git worktree root even when invoked from a subdirectory. It creates a random worktree-generation token in the worktree-specific Git metadata directory, not in the checked-out files. The token does not dirty the worktree and survives branch commits for the life of that linked worktree.

The environment identity includes:

- canonical worktree root;
- worktree-generation token; and
- selected runtime mode.

Recreating a worktree at the same path therefore produces a different owner identity and cannot adopt stale state merely because the path matches.

Non-Git directories retain the current canonical-path identity but cannot claim the full worktree-isolation guarantee.

### Resource planning

At session start, the broker builds one typed `ResourcePlan`:

1. Detect Maven from `pom.xml` or `.mvn/`.
2. Detect standard Compose filenames using Docker Compose discovery order.
3. Parse optional `.autospec/runtime.yml`, falling back to `.agent-runtime.yml`.
4. Apply manifest refinements and explicit opt-outs.
5. Resolve and validate the effective Compose model.
6. Compare the plan with existing state under the environment lock.

The plan is persisted before external resources are created. Persisted state includes a plan version and digest so configuration changes cannot silently reuse incompatible resources.

### State layout

Each environment uses:

```text
${AGENT_ENV_STATE_ROOT:-$HOME/.autospec/envs}/<environment-id>/
  owner.json
  plan.json
  env
  inventory.json
  lease.lock
  sessions/
```

`owner.json` records the repository, worktree-generation token, mode, host, creation time, and manifest digest. `plan.json` contains the validated desired resources. `inventory.json` contains actual Maven prefixes, Compose files and project, container IDs, network IDs, volume IDs, and resolved host ports. State files are written atomically.

Every mutating lifecycle operation holds `lease.lock`. Two simultaneous `up` calls for the same environment cannot double-provision resources.

## Maven 4 isolation

Autospec preserves the effective local repository root selected by the user's Maven settings. It does not set `maven.repo.local` and does not create a second repository tree.

The broker merges these Maven Resolver 2.x properties into `MAVEN_ARGS`:

```text
-Daether.lrm.enhanced.split=true
-Daether.lrm.enhanced.remotePrefix=cached
-Daether.lrm.enhanced.localPrefix=autospec/<AGENT_ENV_ID>
-Daether.system.named.factory=file-lock
```

Existing Maven arguments, JVM options, settings, mirrors, credentials, and custom repository roots are preserved. A conflicting preexisting isolation or lock property fails startup with a diagnostic rather than silently choosing one value. Maven 4's Resolver-provided file-lock implementation and default file-based name mapping coordinate access to the shared repository across local Maven processes; Autospec does not replace Maven's lock protocol.

The fixed `cached` prefix is shared across worktrees. The `autospec/<AGENT_ENV_ID>` prefix contains only artifacts installed locally by that worktree. Concurrent builds of identical GAV coordinates therefore resolve different locally installed bytes without duplicating downloaded dependencies.

The installed-artifact prefix survives ordinary agent-session shutdown for fast restarts. It is removed by:

- worktree cleanup after the owner is proven stale;
- explicit `down --purge-maven`; or
- an operator-approved cache policy added in a later design.

Cleanup first asks Maven for the effective local-repository root under the same settings and arguments, then derives exactly `<root>/autospec/<AGENT_ENV_ID>`. The Rust implementation refuses symlinks, path traversal, an identity mismatch, or any target outside that boundary; acquires the environment lease; confirms no live session remains; and deletes only that environment-owned prefix. It never scans or rewrites Maven coordinate directories and never removes the shared `cached` prefix.

References:

- [Maven Resolver split local repository](https://maven.apache.org/resolver/local-repository.html)
- [Maven Resolver configuration options](https://maven.apache.org/resolver/configuration.html)
- [Maven Resolver named locks](https://maven.apache.org/components/resolver-archives/resolver-LATEST/maven-resolver-named-locks/index.html)
- [Maven configuration and `MAVEN_ARGS`](https://maven.apache.org/configure)

## Docker Compose isolation

### Discovery and project ownership

The standard filenames `compose.yaml`, `compose.yml`, `docker-compose.yaml`, and `docker-compose.yml` are discovered automatically in Docker Compose precedence order. Nonstandard or multiple files are listed in the runtime manifest. The broker always supplies `--project-name <COMPOSE_PROJECT_NAME>` so a top-level `name:` cannot override the worktree namespace.

The broker resolves the effective model with Docker Compose before startup. Validation operates on the resolved model, not raw YAML text.

### Fail-closed rules

The model is rejected when it contains an unapproved instance of:

- a fixed or undeclared published host port;
- `container_name`;
- `network_mode: host`;
- an explicit global network or volume name;
- a fixed subnet, IP address, or overlapping address pool;
- a writable absolute bind mount outside the worktree;
- an external network or volume without a declared shared-resource exception; or
- another setting that bypasses Compose project scoping.

Diagnostics carry stable rule IDs, the service/resource path, the resolved unsafe value, and either a deterministic normalizer action or an operator remediation.

### Explicit exports

Only ports declared under `resources.compose.exports` are published to the host:

```yaml
version: 2
resources:
  compose:
    files:
      - compose.yaml
    exports:
      - service: web
        target: 8080
        protocol: http
        env: AUTOSPEC_PUBLIC_URL
    preserve_volumes: [] # Compose logical volume keys, not engine-global names
```

Autospec generates an environment-state override that binds the declared target to loopback without choosing a fixed host port. Docker allocates the host port atomically. After `up`, the broker queries `docker compose port`, records the actual mapping, and writes the requested environment variable or URL.

`protocol: http` and `protocol: https` write a URL. `protocol: tcp` and `protocol: udp` write the numeric host port. The manifest may additionally request a host-and-port value. A duplicate `env` name or an unsupported protocol is rejected during planning.

Runtime-only generated Compose content is restricted to declared port exports and broker ownership labels. Application services, commands, networks, and volumes are never silently redefined at runtime.

### Inventory and teardown

After successful startup, the broker records actual resource IDs discovered through Compose and Docker ownership labels. It does not rely only on predicted names.

When the last live session releases an environment, `down` removes:

- project containers and orphans;
- project networks;
- broker-owned named and anonymous volumes, except entries in `preserve_volumes`; and
- environment port mappings and session records.

External resources are never deleted. Cleanup verifies that recorded owned resources are gone before removing the state directory. A failed teardown preserves state for a later retry.

References:

- [Docker Compose project names](https://docs.docker.com/compose/how-tos/project-name/)
- [Docker Compose networking](https://docs.docker.com/compose/how-tos/networking/)
- [Docker Compose port syntax](https://docs.docker.com/reference/compose-file/services/#ports)
- [`docker compose port`](https://docs.docker.com/reference/cli/docker/compose/port/)
- [`docker compose down`](https://docs.docker.com/reference/cli/docker/compose/down/)

## Transparent Compose normalization

### Authority boundary

The internal `autospec-compose-normalize` skill orchestrates a deterministic Rust transformer. The model may classify an ambiguous finding or explain a failure, but it does not rewrite YAML.

The same Rust validation and transformation library is used by the broker and the skill. There is no second parser or policy implementation in prompt text.

### Eligible transformations

The transformer may:

- remove fixed host-port bindings and add equivalent manifest exports;
- remove `container_name` when service discovery uses the Compose service name;
- remove explicit network or volume names that only duplicate project-scoped defaults;
- add required runtime environment interpolation; and
- preserve unrelated Compose fields, ordering, comments, and formatting.

It must be idempotent: a second run produces no byte changes.

For a migrated port, the deterministic default environment name is `AUTOSPEC_COMPOSE_<SERVICE>_<TARGET>_<PROTOCOL>`, with each component normalized to uppercase shell-identifier form. When exactly one HTTP or HTTPS export exists, the transformer also maps it to `AUTOSPEC_PUBLIC_URL`. Multiple candidate public URLs are ambiguous and remain fail-closed until the manifest identifies the canonical export.

It may not transform external infrastructure, host networking, fixed IP designs, privileged host mounts, secrets, or any case whose intent is not mechanically provable.

### Transparent prerequisite workflow

When safe normalization is required:

1. The broker emits a versioned migration plan and configuration fingerprint.
2. Autospec searches for an open or merged migration issue/PR with that fingerprint.
3. If none exists, it atomically claims the Compose and runtime-manifest files.
4. It creates one dedicated migration issue, worktree, and branch from current `main`.
5. The internal skill applies the deterministic transformations and validates the result with Docker Compose and isolation linting.
6. Normal CI and review gates merge the prerequisite PR.
7. Concurrent feature worktrees reuse the same migration result, update from `main`, and resume.

This prevents many feature branches from independently producing the same repository migration. Direct Codex, Claude, and OpenCode alias sessions execute the same preflight and cannot bypass a required or in-progress migration. An Autospec-managed session completes the prerequisite workflow automatically; a direct unmanaged session reports the migration state and refuses to claim isolation.

## Sessions and harness aliases

The installer generates Codex, Claude, and OpenCode aliases from one harness table:

```text
autospec-env session -- <harness> <harness-specific permission flags>
```

Every alias:

1. resolves the Git worktree root;
2. runs automatic Maven/Compose detection and normalization preflight;
3. acquires or joins the worktree+mode environment;
4. registers a session reference;
5. starts the harness with the same resource environment; and
6. releases only its own reference on exit or signal.

Session records include PID, process-start identity, harness, host, start time, and heartbeat. Process-start identity prevents PID reuse from reviving an old reference. One agent exiting cannot tear down resources while another live session remains.

Alias source, installed output, documentation, and tests remain in lockstep. A change to one harness entry regenerates all derived alias surfaces.

## Port policy

Compose host ports are assigned atomically by Docker and then discovered.

For direct non-Compose development servers, Autospec uses a host-wide, file-locked port registry. It verifies loopback availability, atomically claims the port for one environment, launches the configured command, and waits for the declared bind or health condition. A bind collision triggers bounded reallocation and restart. Port claims are released during teardown or stale-owner collection.

Caller overrides remain possible, but a fixed override must pass the same host-wide collision check and becomes part of the recorded plan.

## Commands and manifest compatibility

Runtime manifest v2 adds optional resources while preserving automatic detection:

```yaml
version: 2
name: sample-app
default_mode: local
resources:
  maven:
    isolation: split-local
  compose:
    files: [compose.yaml]
    exports: []
    preserve_volumes: []
modes:
  local:
    command: ./dev-start.sh
    down: ./dev-stop.sh
```

Repositories without a manifest still receive detected Maven and Compose isolation. The manifest refines nonstandard files, exports, preserved volumes, and explicit opt-outs.

For Maven, omission means automatic detection, `split-local` explicitly selects the managed strategy, and `off` opts out. For Compose, omission means automatic detection and `off` opts out; no alternate lifecycle strategy is accepted. The one-invocation opt-outs are `AUTOSPEC_MAVEN_ISOLATION=off` and `AUTOSPEC_COMPOSE_ISOLATION=off`. Any other environment value, or any unsupported manifest value, is rejected. The existing `AUTOSPEC_ENV_DISABLE=1` remains the whole-environment emergency bypass.

V1 manifests remain accepted. If a v1 command appears to manage Compose while the broker also detects Compose resources, startup fails with a migration diagnostic rather than running two lifecycle authorities.

A nonempty resource plan may provision without a custom mode `command`; this is the normal path for an automatically detected Compose-only repository. When the plan has no managed resources, the existing requirement for a mode command remains.

The command surface is:

- `up`: detect, validate, lock, provision, inventory, and print the sourceable environment;
- `status`: reconcile desired state, inventory, sessions, Maven prefix, Compose resources, and ports;
- `down`: refuse while live sessions remain, then remove ephemeral resources;
- `exec`: run one command inside the environment;
- `session`: reference-count a long-lived harness process;
- `gc`: explicitly run the conservative stale-resource collector; and
- `down --purge-maven`: additionally delete the environment's installed-artifact prefix.

Automatic garbage collection also runs during `up`, `status`, and Autospec worktree cleanup.

## Crash recovery and garbage collection

Garbage collection reclaims an environment only after all of these checks succeed:

1. The worktree-generation token no longer matches or the worktree is gone.
2. Every recorded session process identity is dead.
3. Docker resources match the recorded Compose project and ownership labels.
4. The inventory contains no resource owned by a different live environment.

Ambiguous evidence blocks deletion and prints an explicit recovery command. A stale path alone is never sufficient evidence.

`up` reconciles partial provisioning. It either finishes an idempotent safe step or tears down the partial resources recorded in inventory before retrying. It does not discard failed state before cleanup succeeds.

## Opt-outs and proof semantics

Automatic Maven or Compose isolation may be disabled for one invocation or explicitly in the manifest. Opt-out is never silent:

- the environment exports `AUTOSPEC_ISOLATION_BYPASSED=1`;
- status and closeout evidence identify the disabled resource class; and
- Autospec cannot label Maven, Compose, or runtime isolation as verified.

An opt-out does not authorize deletion or mutation of shared resources.

## Error handling

The broker fails before launching the harness when it encounters:

- Maven older than 4;
- conflicting Maven Resolver properties;
- malformed or unsupported runtime manifests;
- an unsafe Compose rule without a deterministic migration;
- a failed or ambiguous migration prerequisite;
- resource ownership conflicts;
- corrupt state or inventory; or
- failed partial-resource reconciliation.

Diagnostics include a stable code, environment ID, affected resource, evidence, and one rerunnable recovery command. External command exit statuses remain intact. Cleanup is idempotent, and state remains available when cleanup fails.

## Verification

### Unit and contract tests

- Worktree-generation identity changes when a worktree path is reused.
- Runtime manifest v1 compatibility and v2 resource parsing.
- Maven argument merging and conflicting-property rejection.
- Compose resolved-model rule detection with stable rule IDs.
- Resource plan, inventory, atomic state, and session-reference transitions.
- Normalizer idempotence and preservation of unrelated YAML content.
- Codex, Claude, and OpenCode alias generation and installed lockstep.

### Maven integration

Using real Maven 4 and no repository mocks:

1. Create two worktrees of one fixture project.
2. Install different artifact bytes under the same GAV in each worktree.
3. Resolve the GAV from a consumer in each environment.
4. Prove each consumer receives only its worktree's artifact.
5. Resolve a remote dependency concurrently and prove the downloaded cache is shared and uncorrupted.

### Forty-stack Compose proof

Using real Docker Compose and a locally built lightweight fixture image:

1. Create 40 linked worktrees and start their stacks concurrently.
2. Give each stack a service, project network, named volume, and declared host export.
3. Assert 40 distinct environment IDs, Compose projects, network IDs, volume IDs, and host ports.
4. Reach every service through its generated URL.
5. Start multiple sessions in selected worktrees and prove one exit does not tear down shared resources.
6. Kill sessions and broker operations at provisioning and teardown boundaries.
7. Run safe garbage collection.
8. Tear down all worktrees and assert zero owned containers, networks, volumes, port claims, or session records remain.

The proof records peak container count, startup duration, teardown duration, collisions, retries, and leaked-resource count.

### Repository gates

Every implementation PR runs relevant focused tests plus:

- Rust formatting, strict Clippy, build, and workspace tests;
- Bats installer and alias contracts;
- skill adapter lockstep and generated goldens;
- Autospec validation; and
- Docker/Maven integration gates where the required engines are available.

The final consolidation gate requires the real Maven and 40-stack proofs. A skipped engine-dependent proof cannot satisfy completion.

## Decomposition

Issue #2103 is the umbrella. Implementation should be decomposed into linked issues in this order:

1. Worktree-generation identity, locked resource plan, sessions, and inventory.
2. Maven 4 split-local isolation and cleanup.
3. Compose v2 manifest, resolved-model linter, dynamic exports, and inventory.
4. Deterministic Compose transformer and internal `autospec-compose-normalize` skill.
5. Transparent prerequisite migration orchestration.
6. Codex, Claude, and OpenCode alias lockstep.
7. Crash recovery, safe garbage collection, and worktree-cleanup integration.
8. Documentation, observability, Maven integration, and 40-stack proof.

Each child issue must be small enough for the repository's implementation-agent budget and must preserve a passing baseline for the next child.

## Completion definition

The feature is complete when Maven 4 worktrees can concurrently install the same GAV without cross-resolution, 40 real Compose stacks run with distinct owned resources and reachable dynamic exports, multiple harness sessions safely share one worktree environment, crashes leave no unrecoverable resource ambiguity, stale resources are conservatively reclaimed, all three aliases use the same broker contract, unsafe Compose definitions are either deterministically migrated once or rejected, and the complete repository validation suite passes without skipping the Maven or 40-stack proof.
