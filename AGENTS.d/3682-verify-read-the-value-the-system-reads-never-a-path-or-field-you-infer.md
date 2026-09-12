# Verify: read the value the system reads, never a path or field you inferred (issue #3682)

Three wrong conclusions in one session, the same cause each time. Each time
the reader needed a fact the system already stores, and each time it
reconstructed the fact instead of reading it from where the system reads it:

- **"The pipeline is blocked — zero endpoints."** The reader listed
  `$L/endpoints/`. The endpoints live in `$L/state/endpoints/`. There were
  ten, all healthy. A total outage was reported that did not exist, and a
  fault was hunted in two healthy scripts.
- **"Four workers are unreachable."** The reader took each worker's port by
  grepping its *log* for a URL. The endpoint files record the real port. The
  registrations went to the wrong port, all failed, and the workers were
  declared dead. Curling the addresses from the endpoint files returned `200`
  for all ten.
- **"iw-30 is hung."** `agent.out` was 0 bytes after 2h21m, so it nearly got
  cancelled. The workers showed nine slots generating with a 61k-token context
  in flight. It was working.

In all three the authoritative value was one file read away, and in all three
the reconstructed value was **plausible** — which is what makes this
expensive. A wrong path returns an empty list, not an error. A wrong port
returns a connection failure, not a warning. Absent output looks exactly like
a stall. Every one presents as a *finding about the system* rather than a
mistake by the reader. Reconstructing is locally cheaper — a guess is one
command, finding who writes the directory is three or four — so under pressure
the guess wins, and the guess is silent when wrong. A fabricated fact does not
fail; it propagates: false "zero endpoints" becomes false "pipeline blocked"
becomes a search for a fault in two healthy scripts. Nothing failed loudly
until the trace of a third script printed the real path.

- **A value's source is the producer's own path, never a guess.** A value has
  a producer — the script that writes the file, the flag that sets the port,
  the API that serves the list. Before reporting a fact about the system,
  name the file or command the system itself uses to obtain it, and read
  that. A path inferred from a sibling directory, a log line, or a naming
  convention is a candidate, not a fact (`Inference`, `Source`).
- **Confirm the path against its writer before trusting it.** The cheapest
  confirmation is usually `grep -rl <thing>` over the scripts to find who
  writes the value, before reading it — the one command the #3682 reader
  skipped three times. Naming the path is not confirmation: a named path and a
  wrong path are the same until the writer says otherwise
  (`Confirmation::matches`).
- **An empty or zero result is unverified, not a finding, until the path is
  confirmed against its writer.** An empty result and a wrong path are
  indistinguishable at the call site, and an empty result is exactly the shape
  a dramatic conclusion takes (`verdict`, `ReadVerdict`). The control case is
  the point: an empty result from a *verified* source is a genuine zero — a
  real finding — which is the case an unverified zero is confused with.
- **A dramatic conclusion requires a verified read.** "Zero endpoints",
  "workers unreachable", "agent hung" are findings only when they rest on a
  read from the producer's own path, confirmed against the writer; otherwise
  they are candidates to check, and the report says so (`Finding`,
  `finding_verdict`).

This is the reader-side twin of #3677 (a mutation that silently fails to
apply) and #3678 (a comment describing behaviour no code performs): in all
three a step that did not really happen is indistinguishable from one that
succeeded.

Checkable in `autospec_core::reader_fidelity` (`Inference`, `Source`,
`Confirmation`, `Read`, `verdict`, `ReadVerdict`, `UnverifiedReason`,
`Finding`, `finding_verdict`, `FindingVerdict` — pure in-memory, no I/O, no
clock, no subprocess, so the shell verification step can adopt them as the
single source of truth). Tests:
`crates/autospec-core/tests/reader_fidelity.rs`, including the regression that
reconstructs all three incidents (the sibling-directory `endpoints/` guess
that read empty, the log-line port that missed, the 0-byte `agent.out`) and
asserts each wrong read is `Unverified`/`Unsupported` while its corrected
read is `Verified`/`Sound`.
