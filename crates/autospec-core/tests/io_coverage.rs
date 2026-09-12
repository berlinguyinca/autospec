//! I/O coverage (issue #4385).
//!
//! The incident: `parsePrometheus` — a pure parser — had five tests and
//! was correct. `fetchProgress`, which performs the HTTP GET and reads the
//! body, had none. It read the response with `io.CopyN(&sb, resp.Body,
//! 1<<20)`, and `io.CopyN` returns `io.EOF` when the source is shorter
//! than the requested count; the body is always far shorter than a 1 MiB
//! cap, so every fetch returned an error, the caller fell back to its safe
//! default, and the feature silently never ran.
//!
//! The regression tests run in the configuration the bug required: a
//! fetcher whose only tests live on the extracted pure parser. The last
//! section follows the invariant itself: `read_bounded` is an I/O
//! function, so its tests perform that I/O — a real pipe and a real temp
//! file, boring payloads first.

use std::io::Write;

use autospec_core::io_coverage::{
    classify, read_bounded, ExtractedFunction, IoFunction, IoKind, Payload, Test, Transport,
    Verdict,
};

/// The incident's adapter: does the HTTP GET and reads the body.
fn fetcher() -> IoFunction {
    IoFunction {
        name: "fetchProgress".into(),
        transports: vec![Transport::Http],
    }
}

/// The incident's extracted pure parser.
fn parser() -> ExtractedFunction {
    ExtractedFunction {
        name: "parsePrometheus".into(),
        extracted_from: "fetchProgress".into(),
    }
}

/// One of the five parser tests: correct, pure, and useless for the
/// fetcher.
fn pure_test(n: usize) -> Test {
    Test {
        name: format!("TestParsePrometheus{n}"),
        calls: vec!["parsePrometheus".into()],
        io: IoKind::None,
        payloads: vec![Payload::Boring],
    }
}

fn parser_tests() -> Vec<Test> {
    (1..=5).map(pure_test).collect()
}

/// The incident: five tests on the parser, none on the fetcher. The
/// answer to the reviewer's question is the finding.
#[test]
fn incident_parser_tests_do_not_cover_the_fetcher() {
    let report = classify(&[fetcher()], &[parser()], &parser_tests());
    assert!(report.any_uncovered());
    let entry = &report.entries()[0];
    assert_eq!(entry.verdict, Verdict::Untested { pure_tests: 5 });
    let line = entry.line();
    assert!(
        line.contains("no test performs the HTTP I/O"),
        "line: {line}"
    );
    assert!(
        line.contains("cover the core, not the adapter"),
        "the fold must be named: {line}"
    );
}

/// The same five parser tests plus one `httptest`-based test on the
/// fetcher with a small body: covered.
#[test]
fn a_httptest_test_with_a_small_body_covers_the_fetcher() {
    let mut tests = parser_tests();
    tests.push(Test {
        name: "TestFetchProgressHttptest".into(),
        calls: vec!["fetchProgress".into()],
        io: IoKind::Real(Transport::Http),
        payloads: vec![Payload::Boring, Payload::Representative],
    });
    let report = classify(&[fetcher()], &[parser()], &tests);
    assert!(!report.any_uncovered());
    assert_eq!(
        report.entries()[0].verdict,
        Verdict::Covered {
            test: "TestFetchProgressHttptest".into()
        }
    );
}

/// A test that stands up a fake in place of the transport: the logic is
/// exercised, the bytes never moved.
#[test]
fn a_mock_of_the_transport_is_not_the_transport() {
    let tests = vec![
        Test {
            name: "TestFetchProgressMockClient".into(),
            calls: vec!["fetchProgress".into()],
            io: IoKind::Mocked(Transport::Http),
            payloads: vec![Payload::Boring],
        },
        pure_test(1),
    ];
    let report = classify(&[fetcher()], &[parser()], &tests);
    let entry = &report.entries()[0];
    assert_eq!(
        entry.verdict,
        Verdict::Mocked {
            tests: vec!["TestFetchProgressMockClient".into()]
        }
    );
    let line = entry.line();
    assert!(
        line.contains("a mock of the transport is not the transport"),
        "line: {line}"
    );
}

/// The transport is real, the payload is not boring: the class where
/// `io.CopyN`'s EOF lives is unexercised, so it is still a finding.
#[test]
fn real_io_without_the_boring_case_is_a_finding() {
    let tests = vec![Test {
        name: "TestFetchProgressLargeBody".into(),
        calls: vec!["fetchProgress".into()],
        io: IoKind::Real(Transport::Http),
        payloads: vec![Payload::Representative],
    }];
    let report = classify(&[fetcher()], &[parser()], &tests);
    assert_eq!(
        report.entries()[0].verdict,
        Verdict::BoringCaseMissing {
            tests: vec!["TestFetchProgressLargeBody".into()]
        }
    );
}

/// A test that performs real I/O through a different transport does not
/// cover this function: the test must perform *that* I/O.
#[test]
fn real_io_through_the_wrong_transport_does_not_cover() {
    let tests = vec![Test {
        name: "TestFetchProgressViaFile".into(),
        calls: vec!["fetchProgress".into()],
        io: IoKind::Real(Transport::File),
        payloads: vec![Payload::Boring],
    }];
    let report = classify(&[fetcher()], &[parser()], &tests);
    assert_eq!(
        report.entries()[0].verdict,
        Verdict::Untested { pure_tests: 0 }
    );
}

/// A function that moves bytes through more than one transport is covered
/// by a test on any one of them.
#[test]
fn a_multi_transport_function_is_covered_per_transport() {
    let writer = IoFunction {
        name: "loadMetrics".into(),
        transports: vec![Transport::File, Transport::Pipe],
    };
    let tests = vec![Test {
        name: "TestLoadMetricsTempFile".into(),
        calls: vec!["loadMetrics".into()],
        io: IoKind::Real(Transport::File),
        payloads: vec![Payload::Boring],
    }];
    let report = classify(&[writer], &[], &tests);
    assert!(!report.any_uncovered());
}

/// The report answers the reviewer's question for every function at once,
/// one line each, findings first-class.
#[test]
fn report_lines_and_findings() {
    let writer = IoFunction {
        name: "writeProgress".into(),
        transports: vec![Transport::File],
    };
    let tests = parser_tests();
    let report = classify(&[fetcher(), writer], &[parser()], &tests);
    let lines = report.lines();
    assert_eq!(lines.len(), 2);
    assert!(lines[0].starts_with("FAIL: fetchProgress — "));
    assert!(lines[1].starts_with("FAIL: writeProgress — "));
    let findings = report.findings();
    assert_eq!(findings.len(), 2);
}

// ---
// The invariant applied to this module: `read_bounded` performs I/O, so its
// tests perform that I/O — a real pipe and a real temp file. Not a mock of
// the transport: the transport. Boring payloads first, because that is
// where stdlib off-by-semantics surface.

fn temp_file(name: &str, contents: &[u8]) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "autospec-io-coverage-{}-{name}",
        std::process::id()
    ));
    std::fs::write(&path, contents).unwrap();
    path
}

/// The incident's exact shape: a body far shorter than a 1 MiB cap, read
/// through a real pipe. A source shorter than the cap is a complete read,
/// not an error.
#[test]
fn read_bounded_pipe_shorter_than_cap_is_a_complete_read_not_an_error() {
    let (mut rx, mut tx) = std::io::pipe().unwrap();
    let body = b"# HELP up\nup 1\n"; // 16 bytes, far short of the cap
    tx.write_all(body).unwrap();
    drop(tx);
    let got = read_bounded(&mut rx, 1 << 20).unwrap();
    assert_eq!(got, body);
}

/// The emptiest boring case: an empty body through a real pipe.
#[test]
fn read_bounded_pipe_empty_body_is_an_empty_read() {
    let (mut rx, tx) = std::io::pipe().unwrap();
    drop(tx);
    let got = read_bounded(&mut rx, 1 << 20).unwrap();
    assert!(got.is_empty());
}

/// A body exactly as long as the cap: the boundary.
#[test]
fn read_bounded_pipe_body_exactly_at_cap() {
    let body = vec![b'x'; 64];
    let (mut rx, mut tx) = std::io::pipe().unwrap();
    tx.write_all(&body).unwrap();
    drop(tx);
    let got = read_bounded(&mut rx, 64).unwrap();
    assert_eq!(got, body);
}

/// A body longer than the cap through a real pipe: the read stops at the
/// cap, without an error.
#[test]
fn read_bounded_pipe_truncates_at_cap() {
    let body = vec![b'y'; 128];
    let (mut rx, mut tx) = std::io::pipe().unwrap();
    tx.write_all(&body).unwrap();
    drop(tx);
    let got = read_bounded(&mut rx, 64).unwrap();
    assert_eq!(got, &body[..64]);
}

/// The boring cases through a real temp file, not a mock reader: empty,
/// short, and long relative to the cap.
#[test]
fn read_bounded_temp_file_boring_cases() {
    let empty = temp_file("empty", b"");
    let short = temp_file("short", b"single row\n");
    let long = temp_file("long", &vec![b'z'; 256]);
    let result = || {
        (
            {
                let mut f = std::fs::File::open(&empty).unwrap();
                read_bounded(&mut f, 1 << 20).unwrap()
            },
            {
                let mut f = std::fs::File::open(&short).unwrap();
                read_bounded(&mut f, 1 << 20).unwrap()
            },
            {
                let mut f = std::fs::File::open(&long).unwrap();
                read_bounded(&mut f, 4).unwrap()
            },
        )
    };
    let (got_empty, got_short, got_long) = result();
    std::fs::remove_file(&empty).unwrap();
    std::fs::remove_file(&short).unwrap();
    std::fs::remove_file(&long).unwrap();
    assert!(got_empty.is_empty());
    assert_eq!(got_short, b"single row\n");
    assert_eq!(got_long, vec![b'z'; 4]);
}
