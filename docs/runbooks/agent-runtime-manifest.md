# Agent runtime manifests

`autospec runtime env` is the supported interface for manifest-driven local runtime
environments. The installed `agent-env` and `autospec-env` aliases delegate to this Rust
command family.

## Commands

```text
autospec runtime env init [--repo PATH] [--manifest agent|autospec] [--force]
autospec runtime env up [--repo PATH] [--mode MODE]
autospec runtime env status [--repo PATH] [--mode MODE]
autospec runtime env down [--repo PATH] [--mode MODE] [--purge-maven]
autospec runtime env exec [--repo PATH] [--mode MODE] -- COMMAND [ARGS...]
autospec runtime env session [--repo PATH] [--mode MODE] [--keep-alive] -- COMMAND [ARGS...]
autospec runtime env gc [--repo PATH] [--mode MODE]
autospec runtime env normalize-compose --repo PATH --check|--apply [--fingerprint SHA256]
```

All commands default to `--repo .`. The environment-selecting commands (`up`,
`status`, `down`, `exec`, and `session`) default to `--mode auto`; `init` does
not select a mode. `init` creates `.agent-runtime.yml` by default; use
`--manifest autospec` to create
`.autospec/runtime.yml`. It refuses to replace either existing manifest unless `--force`
is supplied.

## V1 manifest grammar

The parser accepts a constrained YAML mapping. It requires at least one mode; if `version`
is supplied it must be `1`. Scalar values may be single- or double-quoted. Mode environment
names must be uppercase shell-style names (`[A-Z_][A-Z0-9_]*`) and cannot repeat within a
mode. The broker-owned generated names listed below are reserved and cannot appear in a
mode's `env` mapping.

```yaml
version: 1
name: sample-app                 # optional; defaults to the repository basename
default_mode: local              # optional; must name a declared mode; otherwise first declared mode
modes:
  local:
    command: sh -c 'start-local' # required when provisioning a new environment
    down: sh -c 'stop-local'     # optional teardown command
    readiness: bound             # optional; bound (default) or deferred
    env:
      E2E_USE_HARNESS: "1"
      FEATURE_FLAG: enabled
```

`init` also emits `ports` and `public_url_env` compatibility declarations. They are retained
in v1 manifests for interoperability, but v1 runtime allocation derives frontend and backend
ports itself. Unknown top-level and mode keys are ignored by the constrained v1 parser.

Direct modes default to `readiness: bound`: after `command` succeeds, the broker requires the
generated `AGENT_FRONTEND_PORT` to bind before the environment becomes active. Use explicit
deferred readiness only when the session child starts the server later:

```yaml
modes:
  playwright:
    command: sh -c 'true'
    readiness: deferred
```

Deferred readiness runs `command` once and activates the environment without proving that a
listener exists. It is intended for child-owned startup that consumes the generated runtime
environment; it must not replace bound readiness for a command that claims to start a server.
Both manifest versions reject readiness values other than `bound` and `deferred`.

## V2 resource grammar and ownership

Manifest `version: 2` retains `modes` and adds typed resources:

```yaml
version: 2
default_mode: local
modes:
  local:
    command: sh -c 'true'
resources:
  maven:
    isolation: split-local
  compose:
    files: [compose.yaml]
    exports:
      - service: web
        target: 8080
        protocol: http
        env: AUTOSPEC_PUBLIC_URL
        value: url
    preserve_volumes: [developer-cache]
    shared_resources:
      networks: [company-vpn]
      volumes: [maven-cache]
```

Maven isolation requires Maven 4 and owns only the effective local-repository prefix
`autospec/<AGENT_ENV_ID>`. Normal `down` preserves it; `down --purge-maven` canonicalizes the
Maven-reported root, requires zero live sessions, and removes only that exact prefix.

Compose owns one unique project plus every declared container, network, volume, export, and
published port. Labels bind each resource to its environment, owner key, and plan digest.
External resources are accepted only when their exact logical keys appear in
`shared_resources`. `normalize-compose --check` emits a migration and fingerprint;
`normalize-compose --apply --fingerprint SHA256` applies that exact plan transactionally.

The opt-outs `AUTOSPEC_MAVEN_ISOLATION=off`, `AUTOSPEC_COMPOSE_ISOLATION=off`, and
`AUTOSPEC_ENV_DISABLE=1` export `AUTOSPEC_ISOLATION_BYPASSED=1`. Evidence produced under an
opt-out must downgrade its isolation claim from verified.

## Manifest and mode selection

The command canonicalizes the repository path before selecting state. It chooses manifests in
this order:

1. `.autospec/runtime.yml`
2. `.agent-runtime.yml`

An explicit `--mode` wins. With `--mode auto`, the command uses `default_mode`; when that is
absent it uses the first declared mode. A declared `default_mode` that is not present in
`modes` is rejected while the manifest is parsed. The selected environment has a deterministic
ID from the canonical repository path, manifest name (or repository basename), and mode, so the
same repository/mode pair reuses its state.

## Generated environment and state

State is stored at `${AGENT_ENV_STATE_ROOT:-$HOME/.autospec/envs}/<environment-id>/env`; an
empty `AGENT_ENV_STATE_ROOT` is treated the same as an unset value. The file is sourceable POSIX
shell syntax. `up` and `status` print the same protocol, including
`AGENT_ENV_FILE`, so a caller can load it with:

```bash
autospec runtime env up --repo "$PWD" > .autospec-agent-env.out
AGENT_ENV_FILE=$(sed -n 's/^AGENT_ENV_FILE=//p' .autospec-agent-env.out | tail -n 1)
. "$AGENT_ENV_FILE"
```

Every generated state file contains these variables:

- `AGENT_ENV_ID`
- `AGENT_ENV_MODE`
- `AGENT_ENV_REPO`
- `AGENT_ENV_MANIFEST`
- `AGENT_FRONTEND_PORT`
- `AGENT_BACKEND_PORT`
- `AGENT_PUBLIC_URL`
- `AUTOSPEC_PUBLIC_URL`
- `COMPOSE_PROJECT_NAME`

Entries under the selected mode's `env` mapping are included as well. New state allocates
loopback frontend and backend ports dynamically. Non-empty caller values for
`AGENT_FRONTEND_PORT`, `AGENT_BACKEND_PORT`, `AGENT_PUBLIC_URL`,
`AUTOSPEC_PUBLIC_URL`, and `COMPOSE_PROJECT_NAME` are preserved instead. Existing valid state
is reused as-is rather than reprovisioned.

## Child command and session behavior

The manifest `command` and optional `down` command execute through `sh -c` in the canonical
repository directory. A direct child supplied to `exec` or `session` executes in that same
directory with the generated environment. A nonzero direct-child exit code is returned
unchanged.

`session` first checks these controls:

- `AUTOSPEC_ENV_DISABLE=1` runs the direct child unchanged, without reading or creating a
  manifest.
- Without a manifest, `session` also runs the direct child unchanged unless
  `AUTOSPEC_ENV_AUTO_INIT=1`; that setting creates the conservative agent manifest before
  provisioning.
- `--keep-alive` or `AUTOSPEC_ENV_KEEP_ALIVE=1` preserves provisioned state after the child
  exits. Otherwise, normal completion removes the session record and runs teardown.

On Unix, `SIGINT` and `SIGTERM` are recorded by the signal handler; parent code terminates and
waits for the child, removes the session record, runs teardown, and exits `130` or `143`. That
signal status takes precedence if teardown fails; failed teardown leaves its state directory for
a later `down` retry.

## Status codes and safe cleanup

| Code | Meaning |
| --- | --- |
| `0` | The requested operation completed successfully. |
| `1` | A selected mode cannot provision because it has no `command`, or a child terminated without an exit code. |
| `2` | A diagnostic failure: malformed arguments, invalid/missing manifest or state, unavailable repository, allocation, filesystem, or process-spawn error. |
| `3` | `status` found no active environment for the selected repository and mode. |
| `4` | `init` refused to overwrite an existing runtime manifest without `--force`. |
| `42` | Example preserved child or manifest-command exit: any ordinary nonzero child status is returned unchanged. |
| `130` / `143` | Unix `session` interrupted by `SIGINT` / `SIGTERM`; this status wins if teardown also fails. |

Use `autospec runtime env down --repo <path> --mode <mode>` for cleanup. It is safe to repeat:
missing state is not an error, optional `down` is skipped, and state is removed only after the
configured teardown succeeds. Do not delete the state directory manually; doing so can skip a
manifest-defined cleanup command. A `session` without keep-alive performs this cleanup
automatically. For a keep-alive session, call `down` after the dependent work completes.

State directories and session roots are `0700` on Unix; authoritative JSON, env, lease,
registry, and session files are `0600`. `RUNTIME_STATE_SYMLINK_REJECTED` refuses a symlinked
root before destructive cleanup. Other stable recovery diagnostics include
`RUNTIME_PARTIAL_STATE`, `RUNTIME_OWNER_MISMATCH`, `RUNTIME_PLAN_MISMATCH`,
`RUNTIME_LIVE_SESSIONS`, `RESOURCE_OWNER_MISMATCH`, and
`PORT_BIND_HEALTH_RETRIES_EXHAUSTED`. Run the printed recovery command instead of deleting
state manually. `gc` deletes only a stale generation with zero locked sessions and matching
ownership labels.

The real-engine acceptance proof is `bats tests/integration/runtime-compose-40-stack.bats`.
It hard-fails when Docker Compose v2 is absent and writes CSV/JSON evidence under
`reports/runtime-isolation/`.
