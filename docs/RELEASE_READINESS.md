# Autospec Constitution MVP Release Readiness

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
