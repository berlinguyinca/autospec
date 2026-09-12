### Added

- `autospec_core::io_coverage`: every function that moves bytes needs at
  least one test that moves those bytes (issue #4385: `parsePrometheus`
  had five tests and was correct; `fetchProgress` had none, and read its
  body with `io.CopyN(&sb, resp.Body, 1<<20)`, which returns `io.EOF` when
  the source is shorter than the requested count — so every fetch
  returned an error, the caller fell back to its safe default, and the
  feature silently never ran). `classify` answers the reviewer's question
  per I/O function — "which of these functions makes a syscall, and does
  any test execute it?": `Verdict::Covered` (a test calls the function,
  performs the real I/O through one of its transports, and exercises the
  boring case), `Verdict::BoringCaseMissing` (real I/O, no small body /
  empty list / single row — the payload class where stdlib
  off-by-semantics surface), `Verdict::Mocked` (a mock of the transport
  is not the transport), and `Verdict::Untested` (the `fetchProgress`
  shape, naming the tests on the extracted pure core that "cover the
  core, not the adapter"). `ExtractedFunction` encodes the fold: a pure
  function extracted from an I/O function does not inherit its coverage.
  `read_bounded` is the read the incident needed (`take(cap)` +
  `read_to_end`: a source shorter than the cap is a complete read, not an
  error) and its tests follow the invariant — a real pipe and a real temp
  file, boring payloads first.
