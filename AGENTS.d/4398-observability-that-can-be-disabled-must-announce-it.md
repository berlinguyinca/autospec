# Observability that can be disabled must announce it, and default on (issue #4398)

The gateway's telemetry path was a flag defaulting to `""`, documented as
"empty disables telemetry". The launcher never passed it. So
`newTelemetryStore("")` made every `Record()` a silent no-op, and the entire
stream — request rows, the fleet-utilisation sampler, and the worker-health
and slot-clamp rows added *specifically* to make a fault visible — went
nowhere, for as long as the gateway has been deployed.

An empty dashboard is not read as "nothing is being recorded". It is read as
**"nothing is happening"**, which looks like good news. That is strictly worse
than a missing dashboard, because it actively argues against investigating.

## The second half: buffered writes that never flush

Even once enabled, rows did not reach the disk. The writer wrapped the file in
a `bufio.Writer` and only flushed when the channel closed — so rows
accumulated until the 4 KiB buffer filled or the process shut down.

The perverse part: a worker-health row describing a **stuck** worker is
exactly the row a low-traffic period produces. The rows that matter most were
the least likely to be written.

## Why both survived review

Only the record **shape** was tested — which JSON keys a row carries. Shape
tests pass whether or not a row ever reaches a file. Nothing exercised the
write path end to end. (Same root as the `io.CopyN` defect and the two issues
already filed on untested I/O boundaries.)

## The invariants

- **A capability that can be disabled must announce it when disabled.** Not
  at debug level, and not only in documentation: at startup, at WARN, naming
  the flag or variable that turns it on. Silence must never be a valid
  representation of "off".
- **Prefer on-by-default for observability.** The failure mode of telemetry
  accidentally on is a file that grows; the failure mode of it accidentally off
  is every investigation starting from a false premise.
- **A buffered writer needs a time bound, not only a size bound.** Any
  append-only stream a *different* process reads must flush on an interval, or
  the reader cannot distinguish "quiet" from "buffered".
- **Test that a row reaches the sink**, not only that it has the right fields.
  A test that never touches the filesystem cannot detect a stream that was
  never enabled.

## For specs

A spec that introduces telemetry must state where it is written, who enables
it, what happens when it is not enabled, and how a reader distinguishes an
empty stream from a disabled one. "Emit a metric" is not a specification —
it is the part that was already obvious.

Checkable as a spec/lint predicate (no `autospec_core` module yet): the
telemetry flag's default is non-empty (or the disable is explicit and logged
at WARN at startup); the writer has a flush interval, not only a buffer size;
and there is a test that writes a row and asserts the sink file received it.
A shape-only test (JSON keys, no filesystem) is the named coverage gap.
