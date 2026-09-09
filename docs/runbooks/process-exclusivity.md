# Process exclusivity helpers (issue #3967)

Process counts are samples, not guarantees: counting competitors at
startup is a check-then-act race, and a count taken from inside the
process table includes the measurer (issue #3963, #3793). Exclusivity is
a property you **hold**, not a property you observe.

## `scripts/lib/autospec-exclusivity.sh`

Source it (`. scripts/lib/autospec-exclusivity.sh`), then:

| Helper | Contract |
|---|---|
| `autospec_gate <lockfile> [label]` | Take an `flock` on `<lockfile>`, held on a dedicated fd for the lifetime of the shell (released by the kernel on exit, crash included). Returns `0` holding, `1` refusing while a competitor holds it (the holder note — `pid=`/`label=`/`since=` — is printed for observation only), `2` on usage. |
| `autospec_gate_release` | Drop the lock before the shell exits (optional). |
| `autospec_gate_advisory <pattern> [label]` | Warning only, never a gate: reports how many running processes match `<pattern>` as a `WARN:` line. Always returns `0` (`2` on usage). |
| `autospec_spawn <pidfile> <cmd> [args...]` | Run `<cmd>` detached (`setsid`) and write the leader pid to `<pidfile>` (mode `0600`). Prints the pid. The pidfile, not the process table, is the identity a later run consults. |
| `autospec_pidfile_pid <pidfile>` | Print the live pid recorded in `<pidfile>`. Returns `0` with the pid, `1` when the file is missing/unreadable/malformed or the pid is dead, `2` on usage. Compose with `autospec_wait_pid` / `autospec_stop_pid` from `scripts/lib/autospec-process-wait.sh` (#3938). |
| `autospec_match_procs <pattern>` | Pattern-matching fallback, last resort only. Snapshots the process table to a file (a `$(ps ...)` subshell would carry the caller's argv into the table it measures), matches the fixed string against each command line, and **excludes the matcher's whole session** — itself (`BASHPID`, so a `$(...)` subshell still names itself), every ancestor, every live descendant: the wrapper shells and pipeline processes of #3793. Prints every match — `<pid>: <args>` — to stderr **before** any caller can act, and the pids to stdout. Returns `0` with matches, `1` with none, `2` on usage. |

Command-line matchers (`pgrep -f` / `pkill -f`) are rejected in repo
scripts by `scripts/lint-process-matchers.sh` (#3938); this lib is the
approved replacement.

## Tests

`tests/autospec-exclusivity.bats` covers the populated #3793 case in
both directions: a guard called while a competitor is running must
refuse, and a guard whose own argv contains the pattern must not match
itself.
