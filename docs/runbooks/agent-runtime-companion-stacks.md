# Companion stack runtime manifests

The Rust runtime control plane owns companion development profiles. Put a
repository-specific declarative manifest in `.autospec/runtime.yml` and use
`autospec runtime env` to inspect, select, and create its local runtime state.
The companion fixtures demonstrate the `lc-binbase-scheduler` profile and the
`go-modules` and `flasheic` modes.

## Rust-owned environment contract

Only `autospec runtime env` parses the manifest and creates the local runtime
environment. The Rust broker generates these nine reserved values; they must
not appear in a mode's `env` mapping:

- `AGENT_ENV_ID`
- `AGENT_ENV_MODE`
- `AGENT_ENV_REPO`
- `AGENT_ENV_MANIFEST`
- `AGENT_FRONTEND_PORT`
- `AGENT_BACKEND_PORT`
- `AGENT_PUBLIC_URL`
- `AUTOSPEC_PUBLIC_URL`
- `COMPOSE_PROJECT_NAME`

Companion manifests are local declarative input. They contain no credentials,
hosts, or connection strings, and they must not contain provisioning,
migration, deletion, or deployment commands.

## Maven and Compose companions

Use manifest `version: 2` for resource-bearing companions. Maven supports
`resources.maven.isolation: split-local` under Maven 4. Compose declares source files and
exports; the broker generates the project, ownership labels, override, and host ports.
Application commands, networks, and volumes remain repository declarations. External networks
or volumes require exact logical keys under `shared_resources`.

The lifecycle surface is `up`, `status`, `down`, `exec`, `session`, `gc`, and
`normalize-compose`; use `down --purge-maven` for the guarded Maven prefix. Two sessions share
one stack, and the first release cannot tear it down while the other lease is live.

`AUTOSPEC_MAVEN_ISOLATION=off`, `AUTOSPEC_COMPOSE_ISOLATION=off`, and
`AUTOSPEC_ENV_DISABLE=1` export `AUTOSPEC_ISOLATION_BYPASSED=1` and downgrade a verified
isolation claim. Unix state directories/files use `0700`/`0600`;
`RUNTIME_STATE_SYMLINK_REJECTED` prevents cleanup through a linked environment or session root.

## Remote-isolation naming

`agent_<environment-id>` is only a non-executing naming convention for a
local environment identity. It does not connect to a remote system or authorize
any external operation. Keep companion manifests limited to the mode command
and non-sensitive environment values required by the local profile.
