//! I/O coverage: every function that moves bytes needs at least one test
//! that moves those bytes (issue #4385).
//!
//! The incident: `parsePrometheus` — a pure parser — had five tests and was
//! correct. `fetchProgress`, the function that performs the HTTP GET and
//! reads the body, had none. It read the response with
//! `io.CopyN(&sb, resp.Body, 1<<20)`, and `io.CopyN` returns `io.EOF` when
//! the source is *shorter* than the requested count. The body is always far
//! shorter than a 1 MiB cap, so every fetch returned an error, the caller
//! fell back to its safe default, and the feature silently never ran —
//! found only by reading logs for a symptom that had not changed.
//!
//! The parser tests could not have caught it: the parser was never called.
//! The shape recurs. The interesting logic is pure and easy to test, so it
//! gets the tests; the adapter around it is "obviously trivial" so it gets
//! none — and the adapter is where the API misuse lives, because that is
//! the only part that touches an API.
//!
//! The reviewer's question is checkable, and these primitives are the
//! answer to it — "which of these functions makes a syscall, and does any
//! test execute it?":
//!
//! 1. **The answer, per I/O function** ([`classify`]). If no test
//!    performs the function's I/O, that is the finding regardless of how
//!    well the pure core is covered ([`Verdict::Untested`]).
//! 2. **Not a mock of the transport: the transport.** ([`IoKind`]) A test
//!    must move bytes through `httptest`, a temp file, or a pipe. A mock
//!    of the transport proves the logic; the bytes never moved
//!    ([`Verdict::Mocked`]).
//! 3. **The boring case is where stdlib off-by-semantics surface**
//!    ([`Payload::Boring`]). A small body, an empty list, a single row —
//!    `io.CopyN`'s EOF is exactly that: a body shorter than the cap
//!    ([`Verdict::BoringCaseMissing`]).
//! 4. **A pure function extracted from an I/O function does not inherit
//!    its coverage** ([`ExtractedFunction`]). The extraction is good
//!    practice; counting it as coverage is the error.
//!
//! [`read_bounded`] is the read the incident needed, and it is kept here
//! because its own tests follow the invariant: it performs I/O, and its
//! tests perform that I/O through a real pipe and a real temp file,
//! including the boring cases (an empty body, a body shorter than the cap).

use std::io::Read;

/// The transport a function moves bytes through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    /// An HTTP request/response. The test uses `httptest` or a live server.
    Http,
    /// A file on disk. The test uses a temp file.
    File,
    /// A pipe between processes or file descriptors. The test uses a real
    /// pipe.
    Pipe,
    /// A TCP or Unix domain socket.
    Socket,
}

impl Transport {
    /// The label used in report lines.
    pub fn label(self) -> &'static str {
        match self {
            Self::Http => "HTTP",
            Self::File => "file",
            Self::Pipe => "pipe",
            Self::Socket => "socket",
        }
    }
}

/// A function that performs I/O: it makes the syscall and moves bytes
/// through one or more transports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IoFunction {
    /// The function's name.
    pub name: String,
    /// The transports it moves bytes through.
    pub transports: Vec<Transport>,
}

/// A pure function extracted from an I/O function — a parser pulled out of
/// a fetcher. Its tests cover it, and only it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedFunction {
    /// The pure function's name.
    pub name: String,
    /// The I/O function it was extracted from.
    pub extracted_from: String,
}

/// How a test moves bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IoKind {
    /// The test performs the real I/O through the named transport —
    /// `httptest`, a temp file, a pipe.
    Real(Transport),
    /// The test stands up a fake in place of the transport. A mock of the
    /// transport is not the transport: the logic may be exercised, the
    /// bytes never moved.
    Mocked(Transport),
    /// The test moves no bytes: it exercises pure logic.
    None,
}

/// What a test fed the code under test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Payload {
    /// The boring case: a small body, an empty list, a single row.
    /// Off-by-semantics in stdlib calls surface exactly here —
    /// `io.CopyN`'s `io.EOF` is a body shorter than the cap.
    Boring,
    /// A large, representative payload.
    Representative,
}

/// A test, in the terms the invariant is asked about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Test {
    /// The test's name.
    pub name: String,
    /// The functions the test calls directly.
    pub calls: Vec<String>,
    /// How the test moves bytes.
    pub io: IoKind,
    /// The payloads the test exercises.
    pub payloads: Vec<Payload>,
}

/// The answer to "does any test execute this function's I/O?" for one
/// function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// At least one test calls the function, performs the real I/O through
    /// one of its transports, and exercises the boring case.
    Covered {
        /// A covering test.
        test: String,
    },
    /// A test performs the function's real I/O, but none exercises the
    /// boring case — the payload class where stdlib off-by-semantics
    /// surface.
    BoringCaseMissing {
        /// The tests that perform the real I/O, none boring.
        tests: Vec<String>,
    },
    /// Every test that calls the function mocks the transport. The logic
    /// may be proven; the bytes never moved.
    Mocked {
        /// The mock-based tests.
        tests: Vec<String>,
    },
    /// No test performs the function's I/O. The `fetchProgress` shape:
    /// correct where the logic lived, defective where the bytes moved.
    Untested {
        /// Tests that call a function extracted from this one (and not
        /// this one): the coverage the fold would have counted for the
        /// adapter.
        pure_tests: usize,
    },
}

/// The verdict for one I/O function, with the function it belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionCoverage {
    /// The I/O function the verdict is about.
    pub function: IoFunction,
    /// The verdict.
    pub verdict: Verdict,
}

impl FunctionCoverage {
    /// One line for a review or a gate.
    pub fn line(&self) -> String {
        let transports = self
            .function
            .transports
            .iter()
            .copied()
            .map(Transport::label)
            .collect::<Vec<_>>()
            .join("/");
        match &self.verdict {
            Verdict::Covered { test } => format!(
                "OK: {} — test '{test}' performs the {transports} I/O with the boring case",
                self.function.name
            ),
            Verdict::BoringCaseMissing { tests } => format!(
                "FAIL: {} — {} perform(s) the {transports} I/O, but none exercises the boring case (small body, empty list, single row), where stdlib off-by-semantics surface",
                self.function.name,
                list(tests)
            ),
            Verdict::Mocked { tests } => format!(
                "FAIL: {} — {} mock(s) the {transports} transport; a mock of the transport is not the transport",
                self.function.name,
                list(tests)
            ),
            Verdict::Untested { pure_tests } => {
                if *pure_tests > 0 {
                    format!(
                        "FAIL: {} — no test performs the {transports} I/O ({pure_tests} test(s) on its extracted pure core cover the core, not the adapter)",
                        self.function.name
                    )
                } else {
                    format!(
                        "FAIL: {} — no test performs the {transports} I/O",
                        self.function.name
                    )
                }
            }
        }
    }
}

fn list(tests: &[String]) -> String {
    tests
        .iter()
        .map(|t| format!("'{t}'"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The reviewer's question, answered for every I/O function at once.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IoCoverageReport {
    entries: Vec<FunctionCoverage>,
}

impl IoCoverageReport {
    /// One entry per I/O function, in input order.
    pub fn entries(&self) -> &[FunctionCoverage] {
        &self.entries
    }

    /// One line per function.
    pub fn lines(&self) -> Vec<String> {
        self.entries.iter().map(FunctionCoverage::line).collect()
    }

    /// The functions whose I/O no test executes (or mocks), in input order.
    pub fn findings(&self) -> Vec<FunctionCoverage> {
        self.entries
            .iter()
            .filter(|e| !matches!(e.verdict, Verdict::Covered { .. }))
            .cloned()
            .collect()
    }

    /// Whether any I/O function is not covered by a test that performs its
    /// I/O.
    pub fn any_uncovered(&self) -> bool {
        self.entries
            .iter()
            .any(|e| !matches!(e.verdict, Verdict::Covered { .. }))
    }
}

/// Answer the reviewer's question for every I/O function.
///
/// A test covers an I/O function only if it calls the function itself,
/// performs the real I/O through one of the function's transports, and
/// exercises the boring case. Each weaker combination is its own verdict,
/// because each is a different failure: a mock proves logic without moving
/// bytes; real I/O without the boring case leaves the payload class where
/// stdlib off-by-semantics surface unexercised; tests on an extracted pure
/// function cover the core, not the adapter.
pub fn classify(
    functions: &[IoFunction],
    extracted: &[ExtractedFunction],
    tests: &[Test],
) -> IoCoverageReport {
    let entries = functions
        .iter()
        .map(|f| {
            let callers: Vec<&Test> = tests
                .iter()
                .filter(|t| t.calls.iter().any(|c| c == &f.name))
                .collect();
            let real: Vec<&Test> = callers
                .iter()
                .copied()
                .filter(|t| matches!(t.io, IoKind::Real(tr) if f.transports.contains(&tr)))
                .collect();
            let boring: Vec<&Test> = real
                .iter()
                .copied()
                .filter(|t| t.payloads.contains(&Payload::Boring))
                .collect();
            let mock: Vec<&Test> = callers
                .iter()
                .copied()
                .filter(|t| matches!(t.io, IoKind::Mocked(tr) if f.transports.contains(&tr)))
                .collect();

            let verdict = if let Some(t) = boring.first().copied() {
                Verdict::Covered {
                    test: t.name.clone(),
                }
            } else if !real.is_empty() {
                Verdict::BoringCaseMissing {
                    tests: real.iter().map(|t| t.name.clone()).collect(),
                }
            } else if !mock.is_empty() {
                Verdict::Mocked {
                    tests: mock.iter().map(|t| t.name.clone()).collect(),
                }
            } else {
                let pure_tests = tests
                    .iter()
                    .filter(|t| {
                        !t.calls.iter().any(|c| c == &f.name)
                            && t.calls.iter().any(|c| {
                                extracted
                                    .iter()
                                    .any(|e| e.name == *c && e.extracted_from == f.name)
                            })
                    })
                    .count();
                Verdict::Untested { pure_tests }
            };

            FunctionCoverage {
                function: f.clone(),
                verdict,
            }
        })
        .collect();

    IoCoverageReport { entries }
}

/// Read at most `cap` bytes from `reader` and return them.
///
/// A source shorter than the cap is a complete read, not an error. The
/// stdlib calls that invert this — Go's `io.CopyN`, Rust's `ReadExact` —
/// report `EOF` when the source is shorter than the requested count; since
/// a real body is usually far shorter than an arbitrary cap, every read
/// fails. That is the `fetchProgress` defect. `take(cap)` + `read_to_end`
/// is the shape with the right semantics: it stops at the cap, and it
/// stops when the source ends, in either order, without an error.
pub fn read_bounded<R: Read>(reader: &mut R, cap: u64) -> std::io::Result<Vec<u8>> {
    let mut out = Vec::new();
    reader.take(cap).read_to_end(&mut out)?;
    Ok(out)
}
