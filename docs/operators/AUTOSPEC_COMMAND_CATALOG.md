# AutoSpec Operator Command Catalog

## Safety Classification

| Command | Purpose | Safety |
| --- | --- | --- |
| bash scripts/autospec-v60-status.sh | Inspect V60 freeze status | dry_run_safe |
| bash scripts/autospec-v61-status.sh | Inspect V61 mainline freeze status | dry_run_safe |
| bash scripts/autospec-v61-mainline-acceptance.sh | Write V60 mainline acceptance ledger | local_artifact_write |
| bash scripts/autospec-v61-capability-truth-audit.sh | Audit phase capability truth labels | local_artifact_write |
| bash scripts/autospec-v61-golden-path-build.sh | Write operator golden path docs | local_artifact_write |
| bash scripts/autospec-v61-release-candidate-pack.sh | Write V60 mainline RC packet | local_artifact_write |
| bash scripts/autospec-v61-human-approval-boundary-audit.sh | Audit human approval boundaries | dry_run_safe |
| bash scripts/autospec-v61-remote-write-boundary-audit.sh | Audit remote write boundaries | dry_run_safe |
| future approved canary command with --execute-real-github-write | Human-approved remote write canary | human_approval_required |
| merge or auto-merge | Not provided by V61 | blocked_by_policy |
