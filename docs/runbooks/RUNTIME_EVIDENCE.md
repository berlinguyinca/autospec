# Runtime Evidence

Runtime evidence proves generated target-app shells are reviewable by a person. All commands are local and operator-invoked. Dry-run is the default; `--confirm` is required before launching a process or writing user-facing artifacts.

## Commands

```bash
bash scripts/autospec-detect-app-launch.sh --dry-run
bash scripts/autospec-app-harness.sh --dry-run --profile web-dev-server
bash scripts/autospec-run-playwright-evidence.sh --dry-run
bash scripts/autospec-runtime-evidence-status.sh
```

No dependencies are installed automatically. No GitHub Actions, scheduler, external AI calls, migrations, or auth/security behavior changes are used.
