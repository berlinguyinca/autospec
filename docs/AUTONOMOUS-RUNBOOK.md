# Autonomous executor runbook

## Issues merged through an integration branch

GitHub closes `Closes #...` issues only when a pull request reaches the default
branch. When the autonomous executor targets a non-default integration branch,
it therefore closes the exact source issue explicitly.

The executor performs that close only after it has observed the exact pull
request in GitHub's `MERGED` state and durably persisted `BridgePhase::Merged`
with the merge commit OID. It observes the issue before and after requesting
the close, so replay after a crash is idempotent.

If issue observation or closure fails, finalization stops without advancing to
`BridgePhase::CleanupPending`. The persisted `BridgePhase::Merged` transaction
is the recovery point: rerun the conductor and it will retry the same issue.
Do not edit the bridge phase or delete its state file to force cleanup.

Default-branch pull requests continue to rely on GitHub's native closing-keyword
behavior and do not receive an explicit `gh issue close`.
