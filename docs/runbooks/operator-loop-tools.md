# Operator-loop tools index

The single index of the tools the operator loop runs, each with the host
it lives on (issue #3978). A tool that is not in this index is not part
of the loop; a row whose entry point does not resolve is a broken row, and
the row's runbook says where the tool actually is.

Entry points are documented per tool so that a location claim never loses
its scope qualifier: a path that is not resolvable from the repository
root is a deployment path and is named as such.

- **Repo-relative entry points** resolve from the repository checkout on
  the local (merge) host.
- **Deployment paths** (`<llm>/...`) live in the deployment directory on
  the named host, not in this repository.

| Tool | Entry point | Host | Cadence | Role | Runbook |
|---|---|---|---|---|---|
| refresh-queue | `scripts/refresh-queue.sh` | local workspace (authenticated) | */10 | regenerates the dispatch queue from the live tracker; stages missing specs | [refresh-queue-sweep](refresh-queue-sweep.md) |
| topup | `<llm>/topup.sh` | deployment host (authenticated) | */10 | consumes the queue artifact, produces dispatch requests | — |
| dispatch-agent | `<llm>/dispatch-gw.sh` | shared cluster (no credential) | */10 | dispatches agents from dispatch requests | — |
| needs-classify sweep | `skills/autospec-classify/SKILL.md` | local workspace (authenticated) | 0 3 * * * | promotes `needs-classify` issues onto the implementation queue | [needs-classify-sweep](needs-classify-sweep.md) |

Per-hop liveness is reported by `autospec dispatch status`
(`docs/cli-reference.md`); the heartbeat rules the scheduled hops follow
are in `docs/runbooks/log-status-observation.md`.
