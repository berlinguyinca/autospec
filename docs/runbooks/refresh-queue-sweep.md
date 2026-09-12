# Runbook — `refresh-queue` sweep

Scheduled regeneration of the Phase 4 dispatch queue from the live GitHub
tracker. It pairs with the queue-liveness hop contract
(`docs/invariants.md`, issue #3800) and the "put existing tools on a timer"
request (issue #3927): this tool runs on a schedule, never on human recall.
The incident it closes is issue #3978 — a negative finding
("refresh-queue.sh does not exist") that lost its scope qualifier and
became a false premise because nobody could say where the tool actually
lives. This runbook is the answer: a scope-qualified location, and a
schedule.

## Where the tool lives

The tool is in the repository checkout, on the operator's local (merge)
host. The entry point is `scripts/refresh-queue.sh`, resolved from the
repository root — a path you can take from this runbook alone. If it does
not resolve in your checkout, the runbook's path is the contract
(issue #3978): check the checkout, do not assume the script lives
somewhere else, and do not conclude the tool does not exist.

- **Entry point:** `scripts/refresh-queue.sh`
- **Tests:** `tests/refresh-queue.bats`
- **Liveness primitives:** `autospec-core`'s `dispatch_pipeline` module
  (`crates/autospec-core/src/dispatch_pipeline.rs`), surfaced as
  `autospec dispatch` (`docs/cli-reference.md`).

## Why it must run off-cluster

`gh` is unauthenticated on the cluster. The refresh step holds the
`gh-token` credential, and a step that holds a credential runs only on a
host the operator controls — never on cluster-shared storage
(`docs/invariants.md`, issue #3800). So the script runs locally and ships
the resulting queue artifact (`~/.autospec/queue.txt`) to the cluster;
the cluster-side `dispatch-agent` hop reads the shipped artifact with no
credential of its own. Any plan to move the refresh step onto the cluster
must first answer where the token would live, and "the cluster" is not an
answer.

## Manual sweep

```bash
cd /path/to/repo
bash scripts/refresh-queue.sh --out "$HOME/.autospec/queue.txt"
autospec dispatch stamp
```

`--out` is deliberate: the script's own default filename is
`~/.autospec/queue.json`, which is **not** the artifact the liveness gate
reads (`~/.autospec/queue.txt`). Name the real artifact explicitly so
`autospec dispatch stamp` stamps the file the consumer checks — the same
"log path the check reads must be the log the process writes" rule from
`docs/runbooks/log-status-observation.md`, applied to the artifact. The
sweep prints one `Repo <owner>/<name>: staged=<n> skipped=<m> -> <out>`
line per repository; a run that found no repositories writes
`No repositories to process.` and exits 1. `--dry-run` reports the
decision without writing; `--repo OWNER/REPO` narrows the sweep to one
repo.

## Sample crontab — alongside `topup`

Both scheduled hops live on the authenticated host and share one cadence
(`*/10` = the topology default `DEFAULT_INTERVAL_SECS` of 600s in
`crates/autospec-core/src/dispatch_pipeline.rs`):

```cron
SHELL=/bin/bash
PATH=/usr/local/bin:/usr/bin:/bin
GH_PAGER=cat

# minute hour dom mon dow user  command
*/10   *    *   *   *   autospec  cd /srv/autospec/<repo> && bash scripts/refresh-queue.sh --out "$HOME/.autospec/queue.txt" && autospec dispatch stamp >> ~/.autospec/logs/refresh-queue.log 2>&1 || echo "refresh-queue failed" >> ~/.autospec/logs/refresh-queue.log
*/10   *    *   *   *   autospec  <llm>/topup.sh >> ~/.autospec/logs/topup.log 2>&1 || echo "topup failed" >> ~/.autospec/logs/topup.log
```

The explicit env-var preamble (`SHELL`, `PATH`, `GH_PAGER`) is required
because cron starts with a near-empty environment. The two lines share a
cadence on purpose: the producer must beat at least as often as the
consumer it feeds, and a silent scheduled hop is a failure —
`autospec dispatch status` reports per-hop liveness and refuses to read
an unstamped or stale queue as "no work" (issue #3800). Liveness is read
from the stamped queue and the heartbeat line, never from a log's mtime
(`docs/runbooks/log-status-observation.md`, issue #3995).

## Failure modes

- **`gh: not authenticated`** — the sweep dies into
  `~/.autospec/logs/refresh-queue.log`; the queue goes stale after three
  intervals and `autospec dispatch check` holds with
  `STAMP_NOT_REFRESHED`. Re-authenticate locally; do not "fix" it by
  moving the step to the cluster.
- **Cron silently does nothing** — the log may exist and be old. That is
  `stale`, never `dead`: assess from the heartbeat/stamp, and prove the
  check is reading the log the process declares.
- **Queue lags the tracker** — `autospec dispatch reconcile
  --admitted-file <tsv>` reports the admitted-but-unschedulable count;
  nonzero means the queue is behind and the refresh hop must repopulate
  it (issue #3927).
- **Filed issues never reached dispatch** — 178 labelled issues, 90
  queued, 75 covered by a branch or PR, and 37 in none of the three sets,
  reported by nothing (issue #4450). `autospec dispatch queue-gap
  --admitted-file <path> --covered-file <path>` prints all four counts
  (`eligible`, `queued`, `has_branch_or_pr`, `missing`) on every run and
  exits 1 on a non-empty gap without appending the missing issues away.
  Declare the implementation the step depends on with
  `--require-step NAME=COMMAND` and an absent one fails the run by name
  (issue #3772).
- **Entry point does not resolve** — `scripts/refresh-queue.sh` missing
  from the checkout is a checkout defect, not a missing tool
  (issue #3978). Re-fetch the repo before drawing any conclusion from the
  absence.
