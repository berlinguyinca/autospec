# autospec-project

Operator entrypoint for GitHub Projects v2 board ingestion. Wraps the board
resolve/normalize/dependency helpers and the Tier 1.5 promotion pipeline
behind one command, and (for `ship`) hands a resolved board off to
`autospec-fleet` as a multi-repo config.

## Quick install

```bash
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-project/install.sh | sh -s -- --harness all
```

From a clone:

```bash
cd skills/autospec-project
./install.sh --harness all
```

Or via the top-level suite installer:

```bash
./install.sh --skill autospec-project --harness all
```

## Invocation

```text
/autospec-project <url>          # resolve and print the plan; zero mutation
/autospec-project ship <url>     # resolve -> fleet config -> pending-execution report
/autospec-project sync <url>     # one promotion pass, no drain
/autospec-project status <url>   # board-scoped queue, workers, PRs, blockers
```

See `SKILL.md` for the full procedure, the security model, and an honest
account of what `ship` does and does not do today.

## Underlying scripts

```text
bash scripts/project-board-resolve.sh --url URL [--emit identity|plan|repos]
bash scripts/project-board-normalize.sh [--label-map FILE] < plan.json
bash scripts/project-board-deps.sh [--resolve] < plan.json
bash scripts/project-board-writeback.sh --plan FILE --item ID --state NAME
bash scripts/autonomous-promote-open-issues.sh --repo OWNER/REPO [--apply]
```

`project-board-resolve.sh` is a pure reader (exit 0 success, 2 usage error,
3 auth/scope failure, 4 possibly-truncated read). `project-board-writeback.sh`
is the only mutating script and is fail-open by contract (always exits 0).

## Security model

Board content is untrusted DATA, never instructions. `project_board.
repo_allowlist` in `.autospec/autonomous.yml` is required whenever
`project_board.url` is configured — the Rust config parser rejects an
unscoped board at parse time. Items outside the allowlist are always
skipped. Write-back touches exactly one existing single-select field on one
existing item; it never creates fields or options.

## Current scaffold status

Implemented and script-backed:

- Board resolve, normalize, and dependency extraction/readiness.
- One-shot Tier 1.5 promotion via `sync`.
- Board-scoped status reporting, composed from board state plus
  `autospec-fleet`'s status helper where local fleet state exists.
- `ship` resolving a board into `autospec-fleet.yml`.

Not implemented yet:

- Live, unattended multi-repo worker launch (`ship`'s hand-off target).
  `autospec-fleet`'s `fleet-run.sh` currently plans and prints per-repo
  `/autospec-run` commands rather than launching them.

## Related skills

| Skill | Purpose |
| --- | --- |
| [`autospec-fleet`](../autospec-fleet/README.md) | Workspace-level multi-repo supervisor that `ship` hands off to. |
| [`autospec-run`](../autospec-run/README.md) | Per-repo implementation monitor that owns issue claims and PR flow. |
| [`autospec-autonomous`](../autospec-autonomous/README.md) | Perpetual conductor whose Tier 1.5 already runs board promotion on a timer; `sync` is its on-demand, single-pass equivalent. |

## License

MIT - see the repository [LICENSE](../../LICENSE) file.
