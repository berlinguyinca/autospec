---
name: feedback_worktree_resource_isolation
description: "Prove worktree isolation on a clean real engine, preserve ownership evidence, and fail closed on state-root links"
metadata:
  node_type: memory
  type: feedback
---

The 40-stack acceptance proof exposed two operational facts. First, a shared developer Docker
daemon may have exhausted its default network pools even when Autospec names every project
correctly. Never delete unrelated networks or add user-authored fixed subnets to make the test
green. Run scale evidence against an isolated real daemon with sufficient default pools, then
exercise the ordinary manifest and Rust broker unchanged.

Second, evidence must distinguish planned identities from observed resources. Peak containers
comes from label-scoped `docker ps`, not the number of requested worktrees. Record the 40
environment IDs, projects, container/network/volume IDs, host ports, and HTTP statuses before
recovery tests; after cleanup, query those exact ownership labels and report failures instead of
masking teardown errors.

Runtime state is security-sensitive ownership evidence. Unix roots use `0700`, files use
`0600`, and symlinked state/environment/session roots fail with
`RUNTIME_STATE_SYMLINK_REJECTED` before cleanup. Crash tests must reproduce a real lifecycle
checkpoint: an empty Provisioning inventory has no generated env file; TearingDown retains its
authoritative state until cleanup succeeds. Session reference counts must keep the stack alive
until the last lease releases.

Re-run with:

```bash
bats tests/integration/runtime-compose-40-stack.bats
```

Evidence: `reports/runtime-isolation/compose-40-stack.csv` and
`reports/runtime-isolation/compose-40-stack.json`.
