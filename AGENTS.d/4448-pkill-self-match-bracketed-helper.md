# `pkill -f` matches the shell that runs it: the bracketed helper, not a recalled rule (issue #4448)

`pkill -f <pattern>` matches against full command lines, and the shell
running the `pkill` carries the pattern in *its own* command line. The
pattern matches the invoking shell, which dies with it. The signature is
**exit 144 and no output at all** — including none of the work queued after
the kill in the same command. Diagnose it from the exit code alone: a bare
144 after a process-kill command is this mistake, not a mystery.

A written prohibition already existed ("never `pkill -f` your own argv",
`AGENTS.d/4449-silent-false-negatives-recall-rules-are-not-fixes.md`) and the
failure recurred **four times in one session after the note was written**.
The mechanism is the reason the note cannot work: the failure is silent and
self-inflicted in the same stroke — by the time anything could report the
mistake, the process that would report it is gone. There is no error message
to learn from; only a bare 144 and missing work. Rules that must be recalled
at the moment of writing a routine-looking command do not fire; mechanisms do.

## The mechanism

The match is a regex against the full command line, and "full command line"
includes every process that typed or wrapped the command:

- the running `pkill`'s own argv contains the pattern — the bracket form
  `[c]argo.*test` does not match the text `[c]argo.*test` (the regex expects
  `c` where the text has `[`), which is why the bracketed form is safe for
  the matcher to run;
- the **invoking shell's** argv contains the *plain* pattern — the bracketed
  regex *does* match it — so bracketing alone is not enough; the killer must
  also exclude its own session (itself and every ancestor) from the targets;
- the same self-match is a self-wait, not a self-kill, in the `pgrep -f`
  form (issue #3938): the matcher finds itself and blocks on its own exit.

## Invariant

**No raw `pkill -f` / `pgrep -f` with a plain pattern.** Pattern-based
process termination goes through the helper:

    autospec process-kill <pattern> [--signal <NAME>]

The helper (`autospec_core::process_termination`) brackets the first
character of the pattern automatically, excludes its own session (itself and
every ancestor — the processes whose argv carries the plain pattern), kills
the remaining matches by pid, and reports what it killed, what it excluded,
and — when the match set is empty — that a kill matching nothing is a false
negative, not a clean state. `--signal` defaults to TERM. Exit 0 killed at
least one pid; exit 1 matched none; exit 2 usage or tool failure.

The invariant covers the variants a bare prohibition misses: `pgrep -f`
(self-wait) and any wrapper whose argv carries the search string. Process
control in this repo is pid-based where a pid is known (`autospec_kill_tree`,
`autospec_wait_pid`); `autospec process-kill` is the only pattern-based path.
