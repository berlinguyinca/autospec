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

## Remote-isolation naming

`agent_<environment-id>` is only a non-executing naming convention for a
local environment identity. It does not connect to a remote system or authorize
any external operation. Keep companion manifests limited to the mode command
and non-sensitive environment values required by the local profile.
