# Autospec Command Index

All commands are operator-invoked local commands. No GitHub Actions, cron, or scheduler is used.

| Command | Purpose | Dry-run | Confirm | Writes local files? | Writes GitHub? | Primary reports | Next command |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `scripts/autospec-start.sh` | Recommend the right entry flow | yes | accepted | reports only | no | `.autospec/reports/start-plan.md` | onboarding or bootstrap |
| `scripts/autospec-preflight.sh` | Check local environment readiness | yes/default | n/a | reports | no | `.autospec/reports/preflight.md` | MVP smoke |
| `scripts/autospec-mvp-smoke.sh` | Run safe local MVP smoke checks | yes/default | n/a | reports | no | `.autospec/reports/mvp-smoke.md` | fix blockers or MVP status |
| `scripts/autospec-command-audit.sh` | Audit command consistency | yes/default | n/a | reports | no | `.autospec/reports/command-audit.md` | update command docs |
| `scripts/autospec-report-index.sh` | Index generated reports | yes/default | n/a | reports | no | `.autospec/reports/REPORT_INDEX.md` | MVP status |
| `scripts/autospec-validate-state.sh` | Validate generated state/report artifacts | yes/default | n/a | reports | no | `.autospec/reports/state-validation.md` | sensitive audit |
| `scripts/autospec-sensitive-output-audit.sh` | Scan generated Autospec outputs for secrets | yes/default | n/a | reports | no | `.autospec/reports/sensitive-output-audit.md` | fix leaks |
| `scripts/autospec-recovery-status.sh` | Show locks, runs, stuck handovers, and recovery path | yes/default | n/a | reports | no | `.autospec/reports/recovery-status.md` | resume/cleanup/status |
| `scripts/autospec-clean-generated-reports.sh` | Clean generated report artifacts only | yes | yes | reports | no | `.autospec/reports/clean-generated-reports.md` | rerun reports |
| `scripts/autospec-onboard-existing-repo.sh` | Read-first existing repo onboarding | yes | yes | metadata/reports | no | `.autospec/reports/onboarding-result.md` | `autospec-constitution-audit.sh` |
| `scripts/autospec-bootstrap-new-project.sh` | Metadata-first new project bootstrap | yes | yes | metadata/specs | no | `.autospec/reports/bootstrap-result.md` | scaffold generators |
| `scripts/autospec-constitution-audit.sh` | Structured policy audit | yes/default | n/a | reports/state | no | `.autospec/reports/constitution-audit.md` | `autospec-audit-to-backlog.sh` |
| `scripts/autospec-audit-to-backlog.sh` | Convert audit output to v3 backlog plan/publishing | yes | yes | reports/ledger | confirm may call GitHub issue publishing | `.autospec/reports/audit-to-backlog-result.md` | `autospec-autonomy-status.sh` |
| `scripts/autospec-generate-ai-nlai-scaffold.sh` | Generate AI/NLAI specs and v3 issue drafts | yes | yes | specs/issues | no | `.autospec/reports/ai-nlai-scaffold-result.md` | audit-to-backlog |
| `scripts/autospec-generate-product-baseline-scaffold.sh` | Generate product baseline specs/issues | yes | yes | specs/issues | no | `.autospec/reports/product-baseline-scaffold-result.md` | audit-to-backlog |
| `scripts/autospec-supervisor-cycle.sh` | Run one bounded autonomous cycle | yes | yes | reports/state/worker branch as configured | confirm may use GitHub | `.autospec/reports/supervisor-cycle-result.md` | verifier |
| `scripts/autospec-verify-worker-pr.sh` | Independently verify worker output | yes | yes | reports/state | confirm may comment only | `.autospec/reports/verifier-report.md` | promotion gate |
| `scripts/autospec-promote-pr.sh` | Mark verifier-passed PR ready for human review | yes | yes | reports/state | confirm may label/comment | `.autospec/reports/promotion-result.md` | human review |
| `scripts/autospec-autonomy-status.sh` | Summarize autonomy state | yes/default | n/a | reports | no | `.autospec/reports/autonomy-status.md` | recommended command |
| `scripts/autospec-mvp-status.sh` | Summarize MVP readiness | yes/default | n/a | reports | no | `.autospec/reports/mvp-status.md` | hardening |
| `scripts/autospec-spec-coverage.sh` | Map original Constitution vision to implementation/scaffold/validation/deferred evidence | yes | yes | reports/spec coverage backlog | no | `.autospec/reports/spec-coverage.md` | fix gaps or release smoke |

Dry-run remains the default where a command has side effects. Confirm is required for GitHub writes.
