# autospec-e2e-clone

Clone environment provisioner for autospec-test Mode II E2E testing.

Provisions an isolated, scaled-down, anonymized clone of a production environment.
Outputs a routable URL to `.autospec/clone-url.txt` for autospec-test to consume.

## Install

```bash
bash <(curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-e2e-clone/install.sh) --harness all
```

## Contract

Create `.autospec/clone.yml` in your target repository:

```yaml
sources:
  - kind: postgres
    dsn_env: PROD_DB_URL

expose:
  kind: docker_compose
  compose_file: deploy/docker-compose.clone.yml
  url_template: "http://localhost:8080"
```

See [design spec](../../docs/specs/2026-05-22-autospec-e2e-clone-design.md) for full contract reference.

## Components

C1 (this PR) — Skill scaffold + contract loader + JSON Schema.
C2–C10 — Snapshot drivers, anonymize, scale-down, seed, expose, teardown (pending).

### Edge-case seeding

`scripts/edge-case-seed.mjs` seeds required snapshot shapes from the configured
catalog. It evaluates predicates with `sqlite3` when available and falls back to
the Python standard-library `sqlite3` module on minimal developer installations.

Anonymization writes through a temporary file and replaces the source with
platform-safe backup/restore semantics. Interrupted runs remove stale `.anon`
files and restore a leftover `.anonymize-backup` when the source is missing;
the backup is removed only after the source exists. This is recovery-safe, not a
transactional filesystem guarantee.
