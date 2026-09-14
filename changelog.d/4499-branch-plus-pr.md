- `convert`: a branch without an open or merged PR is no longer read as an
  attempt. The push-before-PR window (#4499) — a run killed between the push
  and `pr create` — left orphan branches that the pass counted as done and
  never re-offered. Such candidates are now reported as a distinct
  `interrupted` category (selection line, `INTERRUPTED` line, and the
  `--json` selection), re-offered, and their `--apply` redo overwrites the
  orphan branch with `--force-with-lease` (refusing if the branch moved since
  the fetch, e.g. a PR opened on it). `--apply` also prints `START  #N` and
  `DONE   #N: <outcome>` per issue, so a killed run is diagnosable from its
  output. `Unknown` liveness fails closed.
