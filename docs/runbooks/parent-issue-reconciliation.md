# Parent issue reconciliation

Autospec records every decomposed umbrella through the typed CLI before child
classification:

```bash
export AUTOSPEC_PARENT_STATE_ROOT="$HOME/.autospec/parent-state/owner__repo"
autospec parent record --repo owner/repo --parent 10 --children 11,12
```

Use `--quarantined` when the umbrella's authoritative typed safety decision is
`SAFETY_AMBIGUOUS` or `SAFETY_BLOCK`. The command posts one marked decomposition
comment, adds an `<!-- autospec-parent:10 -->` lifecycle comment to each child, and updates
the shared status cache. Lifecycle comments are accepted only from actors in
the issue-safety `trusted_actors` policy. Repeating the command with the same
relationship is safe; a conflicting child list fails closed. Child issue bodies
are never rewritten.

After a child PR merges, the run workflow executes:

```bash
autospec parent reconcile-child --repo owner/repo --child 11
```

GitHub is authoritative. The command reads the child's parent marker, reads the
marked decomposition comment, and queries every child state. It leaves the
parent open while any child is open. Once all children are closed, it posts one
marked completion summary and closes the parent. Repeated reconciliation is
idempotent.

Every monitor scan also repairs children closed manually or by another
workflow:

```bash
autospec parent sweep --repo owner/repo
```

Parent mutations sharing one state root are serialized with a filesystem lock,
so concurrent local worktree sessions cannot lose cache updates or duplicate a
completion summary. The trusted GitHub decomposition comment remains
authoritative and replaces stale local child lists during reconciliation.

If GitHub mutation fails after all children are terminal, the cache may show
`complete but stale`; failures before the remote relationship can be read leave
the state unknown. Point `autospec status` at the same cache to inspect recorded
state, then rerun `sweep` after the remote failure clears:

```bash
AUTOSPEC_PARENT_STATE_ROOT="$HOME/.autospec/parent-state/owner__repo" autospec status
```
