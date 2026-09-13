# Asking a dispatch command what it does is never an action (issue #4568)

`autospec dispatch stamp --help` did not print help. It wrote to the queue —
creating `$HOME/.autospec/queue.txt`, stamping it `refreshed-by
refresh-queue`, and beating the ledger. The shape of the defect: asking a
command what it does performed its side effect.

The cause was structural, not accidental: `stamp` takes no required
arguments, so an unrecognized flag fell through to execution rather than
to an error. The sibling subcommands were safe only because a missing
required argument stopped the run — `guard` and `freshness` were protected
by an accident of their signatures, not by any validation of the flags.
Any future subcommand with no required arguments would have inherited the
same behavior.

And it mattered more than a help string: the stamp is the artifact the
freshness check reads, and a stamp is an assertion that *the producer ran
and this queue is current*. `--help` made that assertion falsely, on a
file that had never been produced at all. The one command whose entire
purpose is to certify provenance was certifying anything typed at it —
the same failure as the endpoint-registry incident, where a timestamp was
treated as evidence of a producer that never ran.

The discipline now lives at the dispatcher, where every subcommand goes
through it and none can opt out:

- **`-h`/`--help` is recognized before any argument interpretation, in
  every subcommand, and prints usage** — exit 0, nothing written. It is
  answered by the shared gate, not re-implemented per subcommand.
- **A flag no dispatch subcommand accepts is an error that names the
  flag** — exit 2, nothing written. Never an empty option set, never a
  silent mutation. The known-flag set covers every subcommand, including
  the spec subcommands, so a flag from another subcommand is still
  rejected: if no subcommand reads it, executing one with it is not what
  was asked.
- **`stamp` requires its evidence.** `--queue <path>` is now a required
  argument, and the file must already exist: a stamp certifies the queue
  the producer wrote, and a queue that was not named is never the one
  being certified. Stamping a non-existent queue into existence is
  refused — the producer writes the artifact, then stamps it.

`dispatch stamp --help` now prints usage and writes nothing;
`dispatch stamp --bogus` exits 2 naming the flag and writes nothing;
`dispatch stamp` with no queue refuses; `dispatch stamp --queue
<missing>` refuses to create it. The same three assertions hold for every
other `dispatch` subcommand, driven from the one shared parser — a test
loops all fourteen subcommands over both invocations and asserts none of
them write the queue or the ledger.

The help text moved with the discipline: `print_help` now lives in the
shared argument module that guarantees `--help` is answered, not in the
command that used to ignore it. `dispatch.rs` shrank by 35 lines for the
change.
