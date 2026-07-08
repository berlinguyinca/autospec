# Autospec Local Validation Foundation Release Readiness

## Policy source readiness

- [ ] `bash scripts/autospec-validate-policy-sources.sh`
- [ ] `bash scripts/autospec-lock-policy-sources.sh`

## Baseline readiness

- [ ] `bash scripts/autospec-baseline-compose.sh`

## Engine readiness

- [ ] `bash scripts/autospec-command-audit.sh`
- [ ] `bash scripts/autospec-preflight.sh`

## Digital Twin readiness

- [ ] `bash scripts/autospec-build-digital-twin.sh`
- [ ] Confirm `.autospec/state/api-surface.json`, `.autospec/state/ui-surface.json`, `.autospec/state/ai-capabilities.json`, `.autospec/state/knowledge-graph.json`, and `.autospec/state/digital-twin.json` were generated or intentionally refreshed for the target repository.

## Rule audit readiness

- [ ] `bash scripts/autospec-constitution-audit.sh`

## Backlog publishing readiness

- [ ] `bash scripts/autospec-audit-to-backlog.sh --dry-run`

## Worker readiness

- [ ] `bash scripts/autospec-worker-v1.sh --dry-run`

## Verifier readiness

- [ ] `bash scripts/autospec-verify-worker-pr.sh --dry-run --work-item <path>`

## Supervisor readiness

- [ ] `bash scripts/autospec-supervisor-cycle.sh --dry-run --next`

## Onboarding readiness

- [ ] `bash scripts/autospec-onboard-existing-repo.sh --dry-run`

## Bootstrap readiness

- [ ] `bash scripts/autospec-bootstrap-new-project.sh --dry-run --name example --profiles web --application-type web`

## AI/NLAI scaffold readiness

- [ ] `bash scripts/autospec-generate-ai-nlai-scaffold.sh --dry-run`

## Safety guarantees

- No GitHub Actions are installed by Autospec.
- No scheduled automation is enabled.
- Dry-run is the default.
- Confirmed writes require explicit `--confirm`.
- Autospec does not merge or approve its own PRs.
- Promotion/verifier/supervisor reports must keep `approved: false`, `merged: false`, and no self-approval side effects.

## Release-candidate closure decisions

| Requirement | Decision | Evidence |
| --- | --- | --- |
| `digital_twin.surfaces` | Reclassified as implemented engine support; generated state must be refreshed per repo. | `scripts/autospec-build-digital-twin.sh`, `scripts/autospec-digital-twin.py`, generated surface state files. |
| `digital_twin.knowledge_graph` | Reclassified as implemented engine support; generated state must be refreshed per repo. | `scripts/autospec-build-digital-twin.sh`, `scripts/autospec-digital-twin.py`, generated knowledge graph and Digital Twin summary. |
| `docs.drift_detection` | Reclassified as implemented heuristic validator. | `scripts/autospec-metadata-drift.sh`, `scripts/autospec-digital-twin.py`, metadata drift reports. |
| `autonomy.no_self_approval` | Reclassified as implemented safety gate. | `scripts/autospec-promote-pr.sh`, `scripts/autospec-verify-worker-pr.sh`, `scripts/autospec-supervisor-cycle.sh`, release readiness safety guarantees. |

## Known limitations

- See [KNOWN_LIMITATIONS.md](KNOWN_LIMITATIONS.md).

## Manual release steps

```bash
bash scripts/autospec-preflight.sh
bash scripts/autospec-mvp-smoke.sh --dry-run
bash scripts/autospec-sensitive-output-audit.sh
bash scripts/autospec-command-audit.sh
bash scripts/autospec-validate-state.sh
bash scripts/autospec-mvp-status.sh
bash scripts/autospec-release-candidate-gate.sh --dry-run
bash scripts/autospec-dogfood-rc.sh --dry-run
```
