### Added

- `autospec-core::reader_fidelity` — primitives for the invariants that a fact
  about the system is read from the value the system reads, never a path or
  field inferred: a value's source is its producer's own path (the script that
  writes the file, the flag that sets the port, the API that serves the list),
  not one inferred from a sibling directory, a log line, or a naming
  convention (`Inference`, `Source`); the path is confirmed against its writer
  before it is trusted — the cheapest confirmation is `grep -rl <thing>` to
  find who writes the value, and naming a path is not confirmation
  (`Confirmation::matches`); an empty or zero result from an unverified source
  is unverified, not a finding, because an empty result and a wrong path are
  indistinguishable at the call site (while an empty result from a verified
  source is a genuine zero) (`Read`, `verdict`, `ReadVerdict`); and a dramatic
  conclusion rests on a verified read or it is a candidate to check, and the
  report says so (`Finding`, `finding_verdict`, `FindingVerdict`) (#3682,
  2026-09-11).
