# Every I/O function needs a test that performs the I/O (issue #4385)

`parsePrometheus` — a pure parser — had five tests and was correct.
`fetchProgress`, the function that does the HTTP GET and reads the body, had
none. It read the response with:

```go
io.CopyN(&sb, resp.Body, 1<<20)
```

`io.CopyN` returns `io.EOF` when the source is **shorter** than the
requested count, and the body is always far shorter than a 1 MiB cap. So
every fetch returned an error, the caller fell back to its safe default,
and the feature silently never ran. It reached production and was only
found by reading logs for a symptom that had not changed.

The parser tests could not have caught it: the parser was never called.

The shape recurs. The interesting logic is pure and easy to test, so it
gets the tests; the adapter around it is "obviously trivial" so it gets
none — and the adapter is where the API misuse lives, because that is the
only part that touches an API. Coverage measured where the logic lived was
read as coverage of the function that moves the bytes; the five green
parser tests were the evidence a reviewer would have pointed to.

- **Every function that performs I/O gets at least one test that performs
  that I/O — `httptest`, a temp file, a pipe. Not a mock of the
  transport: the transport.** A mock of the transport proves the logic;
  the bytes never moved, so the API misuse — the only kind of bug the
  adapter can have — is exactly what the mock cannot see.
- **The test must exercise a realistic payload, specifically including the
  boring case** — a small body, an empty list, a single row — because
  off-by-semantics in stdlib calls surface exactly there. `io.CopyN`'s
  EOF is precisely the boring case: a body shorter than the cap. A test
  that feeds only large, representative payloads has not tested the
  boundary where the stdlib inverts "short" into "error".
- **A pure function extracted from an I/O function does not inherit its
  coverage.** The extraction is good practice — the parser deserved its
  five tests; counting them as coverage of the fetcher is the error. The
  reviewer's question separates the two: "Which of these functions makes
  a syscall, and does any test execute it?" If the answer is no, that is
  the finding regardless of how well the pure core is covered.
- **The correct read is the one whose tests do the I/O.** `take(cap)` +
  `read_to_end` stops at the cap and stops when the source ends, in either
  order, without an error. Its tests run through a real pipe and a real
  temp file with the boring payloads first — an I/O function in this
  repository is the invariant's own proof.

Checkable in `autospec_core::io_coverage` (`classify` — the reviewer's
question answered per I/O function, `Verdict::Untested` / `Verdict::Mocked`
/ `Verdict::BoringCaseMissing` / `Verdict::Covered`, `ExtractedFunction` —
the pure core's tests never count for the adapter, `read_bounded` — the
read the incident needed). Tests: `crates/autospec-core/tests/io_coverage.rs`,
including the incident end-to-end: five parser tests and no fetcher test
are the finding, named as "cover the core, not the adapter"; a mocked
transport and a real I/O without the boring case each fail; and
`read_bounded` tested through a real pipe and a real temp file — a body
shorter than the cap, an empty body, and a body at and past the cap — all
complete reads, not errors.
