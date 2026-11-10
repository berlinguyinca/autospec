# hive convert-cron

`scripts/hive/convert-cron.sh` runs from cron (`5,35 * * * *`) on the hive login node and
converts finished agent patches to PRs with `autospec convert --apply` on a compute node.

Before requesting compute it runs `scripts/hive/convert-precheck.sh`, which uses the pass's
own plan (no `--apply`) and skips the `srun` when no patch is fresh, delivered or closed.
If the check cannot decide it fails open and the pass runs as before.

## Environment variables

| variable | default | meaning |
|---|---|---|
| `CONVERT_LLM_ROOT` | `/quobyte/metabolomicsgrp/it/llm` | llm root (tests override) |
| `CONVERT_PRECHECK` | `1` | `0` bypasses the pre-check |
| `CONVERT_PRECHECK_BIN` | next to the script | alternate pre-check path |
| `CONVERT_FETCH_SLEEP` | `10` | base seconds between git retries (3 tries) |
| `PLAN_TIMEOUT` | `120` | pre-check plan timebox, seconds |
| `BATCH_N` | `4` | newest N patches per pass |
| `ALLOC_WAIT`, `MAX_RUN` | `600`, `7200` | srun allocation wait, run bound |

## git over ssh

Cron has no ssh agent; git uses the account's default key. A fetch failure is retried three
times, then logged as `FATAL: git ... failed 3x as USER@HOST (key fingerprint)` and the pass
is skipped. A lone "Permission denied (publickey)" next to "Connection refused" lines is a
transient GitHub-side ssh failure; check with `ssh -T git@github.com`.
