# autospec-monitor

Read-only observability into a live `autospec-autonomous` conductor. Summarizes the
typed status, timeline, launch, heartbeat, tier, and discovery artifact surfaces so an
operator can see what a run is doing without mutating repository or GitHub state.

## Install

```bash
./install.sh --harness all
./install.sh --harness all --update   # idempotent re-install over an existing copy
```

## Usage

- `autospec autonomous monitor --once` — concise operator snapshot
- `autospec autonomous status --json` — typed status detail
- `autospec autonomous timeline --lines N` — recent cycle history
