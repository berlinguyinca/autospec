# End-to-end acceptance scenarios
1. Standalone Pi mission with AutoSpec disabled: risk-based plan, skipped or approved Plannotator, worker spawn, review, evidence and Herdr graph.
2. AutoSpec mission: existing GitHub issue DAG, Pi adapter, multiple worktrees, PR, independent AI reviewer, defect repair, head change invalidates old verdict, CI and guarded merge.
3. One HCN room spans two workspaces on different hosts; parent agent tab spawns two panes; graph and timeline correctly distinguish spawn, handoff and peer consultation; pane closure does not kill worker.
4. Coworker's Claude/Pi on another machine joins via scoped bridge, asks/answers, accepts bounded task, proposes artifact; virtual tab displays status without remote shell access.
5. Human comments on failed review, forks alternative from historical node, changes requirements, compares implementations, selects winner, preserves original evidence and invalidates only affected gates.
6. Replay after restart and dropped events, duplicate events, expired token, stale lease, unauthorized room traversal and guest execution attempts. Fail closed without fake progress or leaked information.
7. Keyboard/mobile/reduced-motion UI, large graph performance, screenshots and visual regression under autospec-design. Test cross-repo issues, Plannotator digest and PR head SHA validity.
