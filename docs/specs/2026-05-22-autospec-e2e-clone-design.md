# Autospec E2E Clone Provisioner — Skill C

**Status:** Draft design (2026-05-22)
**Scope:** Closes tracker #423. Major new skill `autospec-e2e-clone` providing the clone-environment provisioning that autospec-test v1 Mode II + v2 edge-case-seeds require. Without Skill C, Mode II is theoretical; with it, the autospec-test family runs end-to-end against production clones.

## 1. Goal & non-goals

### Goal
Provision an isolated, scaled-down, anonymized clone of a production environment that autospec-test can run E2E tests against. The clone exposes a routable URL the autospec-test contract consumes via `e2e.clone_url_env`. The provisioner handles snapshot capture, PII anonymization, edge-case data seeding (consumes the `edge_case_seeds.require_shapes` from autospec-test contracts), and URL exposure.

### Non-goals
- Cloud-provider-specific managed services (this is operator-driven; we provide the orchestration not the infra)
- Real-time mirroring of production state (clones are point-in-time)
- Cross-region replication
- Operating the clone for the operator (we provision; operator runs)

## 2. Architecture

New top-level skill `autospec-e2e-clone` (sibling of `autospec-test`). Same SKILL.md / codex/prompt.md / opencode/agent.md pattern. Lives at `skills/autospec-e2e-clone/`.

Pipeline:

```
1. Snapshot          → capture DB + filesystem + S3 state
2. Anonymize         → redact PII per declarative rules
3. Scale down        → sample large tables (foreign-key-aware)
4. Edge-case seed    → consume edge_case_seeds.require_shapes from .autospec/test.yml
5. Expose URL        → provision routable URL + credentials
6. Health check      → verify URL responds; matrix of expected endpoints
7. Output            → writes .autospec/clone-url.txt for autospec-test gate to read
```

Contract file: `.autospec/clone.yml` in target repo. Declarable:

```yaml
sources:
  - kind: postgres
    dsn_env: PROD_DB_URL
    tables_full: [users, products]            # full clone
    tables_sample: { events: 10000, logs: 5000 }  # sampled
  - kind: s3
    bucket: prod-assets
    sample_prefix_count: 100                  # first 100 objects per prefix
  - kind: filesystem
    paths: [/var/data/uploads]
    sample_files_per_dir: 50

anonymize:
  rules:
    - table: users
      columns:
        email: hash_with_domain        # x@example.com → <sha256>@example.com
        ssn: redact                    # always replace with NULL
        phone: scrub_to_country_code   # +49 ... → +49-XXXXXXX
        name: replace_with_faker       # uses faker locale en_US
      pii_assertions:
        - "SELECT COUNT(*) FROM users WHERE email LIKE '%.com' AND email NOT LIKE '<sha256>@%' = 0"
    - table: events
      columns:
        ip_address: scrub_to_subnet    # 192.168.1.5 → 192.168.1.0
  reversible_map_path: .autospec/anonymize-map.<sha>.json

scale_down:
  default_sample: 10000
  foreign_key_aware: true              # rows referenced by sampled rows get included

edge_case_seed:
  consume_from: .autospec/test.yml     # reads edge_case_seeds.require_shapes
  seed_shapes_catalog: $AUTOSPEC_SCRIPTS_DIR/seed-shapes/catalog.yml

expose:
  kind: docker_compose                 # or: k8s_ephemeral_namespace | dedicated_staging_slot | custom_cmd
  compose_file: deploy/docker-compose.clone.yml
  url_template: "http://localhost:8080"
  health_endpoint: /health
  ready_wait_secs: 60
```

## 3. Component 1 — Snapshot drivers

Per-source-kind adapter at `skills/autospec-e2e-clone/scripts/snapshot/`:

| Kind | Adapter | Notes |
|---|---|---|
| postgres | `pg.sh` | uses `pg_dump --schema-only` + `pg_dump --data-only --table=` per table |
| mysql | `mysql.sh` | uses `mysqldump` per table |
| sqlite | `sqlite.sh` | `cp source.db snapshot.db` |
| s3 | `s3.sh` | `aws s3 sync --include-prefix=` with sampling cap |
| filesystem | `fs.sh` | `rsync` with per-dir file count cap |
| custom_cmd | `custom.sh` | operator-provided commands |

Snapshot output: `.autospec/clone-snapshots/<source-id>/<timestamp>/` (gitignored).

## 4. Component 2 — Anonymize

`scripts/anonymize.mjs` reads contract rules + snapshot data + writes anonymized output:

```js
// per row, per column:
switch (rule) {
  case 'hash_with_domain': value = `${sha256(value.split('@')[0])}@${value.split('@')[1]}`;
  case 'redact':           value = null;
  case 'scrub_to_country_code': value = scrubPhone(value);
  case 'replace_with_faker': value = faker[rule.params.kind]();  // name, address, etc.
  case 'scrub_to_subnet':  value = ipToSubnet(value);
}
```

Reversible mappings stored at `.autospec/anonymize-map.<contract-sha>.json` so test runs can join across tables without leaking PII.

`pii_assertions` run AFTER anonymization — verify no original PII survives (e.g., count of non-hashed emails = 0). Fail-closed.

## 5. Component 3 — Scale-down with foreign-key reachability

`scripts/scale-down.mjs`:
1. Read foreign-key constraints from the DB schema
2. For each `tables_sample[<name>]: N` declaration:
   - `SELECT * FROM <name> ORDER BY RANDOM() LIMIT N` (Postgres) / equivalents
   - Capture sampled row IDs
3. Recursively pull in rows from OTHER tables that any sampled row references (via foreign keys)
4. Repeat until reachability closure
5. Output a `manifest.json` of rows-included counts per table

Catches the "sampled order_items but didn't include their orders" footgun.

## 6. Component 4 — Edge-case seed

Consumes `.autospec/test.yml`'s `edge_case_seeds.require_shapes` (per autospec-test v2 §5b):

For each declared shape (e.g., `task_done_today`, `task_in_collapsed_foldout`), checks the snapshot has ≥ `count_min` rows matching the shape's predicate. If short, INSERTs synthetic rows matching the predicate (using a template registered in the seed catalog).

Synthetic rows are marked `_autospec_synthetic: true` so they're distinguishable from real data.

## 7. Component 5 — Expose URL

Per `expose.kind` adapter at `skills/autospec-e2e-clone/scripts/expose/`:

| Kind | Adapter | Notes |
|---|---|---|
| docker_compose | `compose.sh` | spins up via `docker compose -f <file> up -d`; tears down via `down` |
| k8s_ephemeral_namespace | `k8s.sh` | creates ns; applies manifests; deletes ns on teardown |
| dedicated_staging_slot | `staging.sh` | swaps to a pre-allocated staging slot; restores on teardown |
| custom_cmd | `custom.sh` | operator-provided up/down commands |

Outputs the final URL to `.autospec/clone-url.txt` which the autospec-test gate reads.

Health check before declaring ready: poll `expose.health_endpoint` until 200 OK or `expose.ready_wait_secs` timeout.

## 8. Component 6 — Teardown

Symmetric `scripts/teardown.sh` reverses the provisioning. Cleans up:
- Docker containers / k8s namespaces / staging slots
- Snapshot artifacts (optional — preserve via `--keep-snapshots`)
- `.autospec/clone-url.txt`

Runs automatically when autospec-test gate completes (success or failure).

## 9. Decomposition (10 phases)

| # | Phase | Size | Deps |
|---|---|---|---|
| C1 | Skill scaffold + .autospec/clone.yml contract + JSON Schema | 1 PR | none |
| C2 | Snapshot drivers: pg + mysql + sqlite | 1-2 PRs | C1 |
| C3 | Snapshot drivers: s3 + filesystem + custom_cmd | 1 PR | C1 |
| C4 | Anonymize engine + reversible map + pii_assertions | 1-2 PRs | C2 |
| C5 | Scale-down with foreign-key reachability | 1 PR | C2 |
| C6 | Edge-case seed consumer + catalog overlay | 1 PR | C4 + C5 |
| C7 | Expose adapter: docker_compose | 1 PR | C6 |
| C8 | Expose adapters: k8s + staging_slot + custom_cmd | 1 PR | C7 |
| C9 | Teardown + autospec-test contract integration | 1 PR | C7 |
| C10 | Synthetic targets + integration tests + dogfood against an autospec-test target | 1-2 PRs | C9 |

All priority:high. C1 root. C2-C3 parallel. C4-C5 parallel after C2. C6 after C4+C5. C7 after C6. C8-C9 parallel after C7. C10 after C9.

## 10. Testing

Per-component bats + integration test against fixtures:
- `target-postgres-fixture/` — small Postgres compose with pre-seeded data
- `target-s3-fixture/` — minio with pre-uploaded objects
- `target-sqlite-fixture/` — file-based sqlite for fastest CI
- `target-mode-ii-real/` — full Mode II target (pg + s3 + scope tokens from autospec-test contract)

Mode II safety: `pii_assertions` must pass post-anonymization or provisioning fails. Snapshot artifacts always go to `.autospec/clone-snapshots/` (gitignored). Source DSNs never logged.

## 11. Dependencies & scope boundaries

| Dependency | Status | Failure mode |
|---|---|---|
| `autospec-test` v1 Mode II contract | live | C6/C9 read its edge_case_seeds field |
| `autospec-test` v2 contract | live | same |
| `seed-shapes/catalog.yml` | live (v2 #348) | C6 consumes it |
| Docker / k8s / pg client / aws CLI | external tools | absent → adapter exits with operator-actionable error |

### Out of scope
- Cloud-provider auto-provisioning (operator picks expose.kind)
- Replication / real-time mirror (clones are point-in-time)
- Test-data generation beyond edge-case seeding (use faker via anonymize rules)
- Cost optimization (operator manages resource lifecycle)

## 12. Decision log

| Q | Decision | Rationale |
|---|---|---|
| Skill C as new family or extend autospec-test? | New family `autospec-e2e-clone` | Clean separation; can ship independently |
| Reversible anonymization? | Yes (mappings stored per-contract-sha) | Tests need joins across tables; one-way hash blocks that |
| Foreign-key-aware sampling default? | Yes | Naive sampling produces broken referential integrity |
| Edge-case seed in C or in autospec-test? | C (provisioner produces; test consumes) | Single source of truth per source spec design |
| Multiple expose.kind? | Yes (4 + custom) | Operators have varied infra; no one-size-fits-all |
| Teardown on test failure? | Yes (auto) | Resource leaks unacceptable; symmetric lifecycle |

## 13. Open follow-ups

- Cloud-provider managed-service adapters (AWS RDS snapshot+restore, GCP Cloud SQL, Azure)
- Differential updates (snapshot diff since last)
- Realistic test-data generation beyond shapes (synthetic users with related events, etc.)
- Multi-region clone for global apps
