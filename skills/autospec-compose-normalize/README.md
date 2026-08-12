# autospec-compose-normalize

Orchestrates a single fingerprinted prerequisite migration when Autospec runtime
provisioning detects Docker Compose files that require migration for isolated worktree
environments. Every YAML decision is delegated and each edit is fingerprinted so the
migration is idempotent and auditable.

## Install

```bash
./install.sh --harness all
```
