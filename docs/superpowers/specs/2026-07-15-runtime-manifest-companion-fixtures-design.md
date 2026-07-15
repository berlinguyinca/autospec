# Runtime manifest companion-fixtures design

## Decision

Document companion development profiles as declarative v1 runtime manifests and validate the fixtures with the Rust `RuntimeManifest` parser. The work does not provision software, connect to a database, or invoke a shell broker.

## Alternatives considered

1. Restore a shell fixture harness for the former broker. Rejected because the Rust control plane owns runtime-manifest validation.
2. Put generated runtime values such as `AGENT_FRONTEND_PORT` in fixture `env` mappings. Rejected because the Rust broker reserves those keys and emits them itself.
3. Use a generic prose-only example. Rejected because parseable fixtures protect the documented manifest grammar.

## Components

- `tests/fixtures/runtime-manifests/lc-binbase-scheduler.yml` declares a local scheduler/Playwright profile with no credentials, hosts, or database instructions.
- `tests/fixtures/runtime-manifests/companion-stack.yml` declares selectable `go-modules` and `flasheic` profiles.
- `crates/autospec-core/tests/runtime_env.rs` parses each fixture through `RuntimeManifest::parse` and checks its default selection.
- `docs/runbooks/agent-runtime-companion-stacks.md` explains `.autospec/runtime.yml` precedence, the Rust CLI entry point, broker-generated values, and non-executing remote-isolation naming.

## Invariants

- `default_mode` names a mode declared in each fixture.
- Fixture `modes.*.env` mappings do not define broker-owned keys such as `AGENT_FRONTEND_PORT`, `AGENT_BACKEND_PORT`, `AUTOSPEC_PUBLIC_URL`, or `COMPOSE_PROJECT_NAME`.
- Documentation describes remote isolation only as a naming convention and prohibits connection, provisioning, migration, deletion, and deployment instructions.
- The only implementation authority is `autospec runtime env`; shell, Python, and Bats are not introduced as a parser or provisioning path.

## Verification

Run `cargo test -p autospec-core --test runtime_env` and `cargo run -q -p autospec-cli -- validate --fast`.
