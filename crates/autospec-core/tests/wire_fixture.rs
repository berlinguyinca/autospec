//! A test that invents the wire format can never be wrong, so it can never
//! fail (issue #4243).
//!
//! The incident: a test asserted that `GET /slots` returns an object with a
//! `slots` field, because the test wrote the JSON that way. llama.cpp returns
//! a bare array. The test passed forever — it was checking the parser against
//! the test's own belief about the wire format, and both beliefs were written
//! from the same imagination, so they could never disagree. The first real
//! response arrived in production.
//!
//! Invariants under test:
//! - Invariant 1: captured bytes and authored bytes are different kinds of
//!   evidence, and the difference is visible in the type
//!   ([`FixtureProvenance`], [`WireAssertion`], [`coverage_verdict`]).
//! - Invariant 2: a test file that builds a third-party response out of a
//!   string literal while naming no recorded fixture is a finding
//!   ([`scan_wire_literals`], [`sweep_authored_payloads`]); fixture loading,
//!   comments, plain strings and non-test files are not.
//! - Invariant 3: a fixture is committed data, not a source file parked in a
//!   data directory ([`fixture_is_data_file`]), and it must keep the shape the
//!   regression needs ([`assert_fixture_shape`], [`json_shape`]).
//! - Invariant 4: a fix is proven in both directions over captured bytes —
//!   old code fails, new code passes — and every other combination is
//!   explicitly not evidence ([`fix_evidence`], [`divergence_report`]).
//! - Invariant 5: a spec that cannot name where a real sample comes from is
//!   refused, and the refusal names what would satisfy it
//!   ([`parse_sample_source`]).

use autospec_core::wire_fixture::{
    assert_fixture_shape, coverage_verdict, divergence_report, fix_evidence, fixture_is_data_file,
    is_test_file, is_test_source, json_shape, parse_sample_source, references_recorded_fixture,
    scan_wire_literals, sweep_authored_payloads, sweep_summary, Coverage, DivergenceProof,
    FixEvidence, FixtureProvenance, InlinePayload, JsonShape, ProbeOutcome, SampleSource,
    SampleSourceError, ShapeDrift, SweepFinding, TestFile, WireAssertion, WireFormat,
    FIXTURE_DATA_DIRS,
};
use std::collections::BTreeSet;

/// Bytes `GET /slots` actually sends (llama.cpp server, bare array).
const CAPTURED_SLOTS: &str = r#"[{"id":0,"n_ctx":65536,"is_processing":false},{"id":1,"n_ctx":65536,"is_processing":true},{"id":2,"n_ctx":4096,"is_processing":false}]"#;

/// Bytes the test invented instead (object with a `slots` field).
const AUTHORED_SLOTS: &str = r#"{"n_slots": 3, "slots": [{"id": 0}, {"id": 1}, {"id": 2}]}"#;

fn captured() -> FixtureProvenance {
    FixtureProvenance::Captured {
        source: "http://llama-01:8080/slots".to_string(),
    }
}

fn authored() -> FixtureProvenance {
    FixtureProvenance::Authored
}

fn assertion(endpoint: &str, provenance: FixtureProvenance) -> WireAssertion {
    WireAssertion {
        endpoint: endpoint.to_string(),
        provenance,
    }
}

fn file<'a>(path: &'a str, source: &'a str) -> TestFile<'a> {
    TestFile { path, source }
}

fn old_code_failed() -> ProbeOutcome {
    ProbeOutcome::failed(0, "invalid type: sequence, expected struct SlotsResponse")
}

fn new_code_parsed() -> ProbeOutcome {
    ProbeOutcome::parsed(3)
}

fn proof(
    provenance: FixtureProvenance,
    old_code: ProbeOutcome,
    new_code: ProbeOutcome,
) -> DivergenceProof {
    DivergenceProof {
        endpoint: "GET /slots".to_string(),
        provenance,
        old_code,
        new_code,
    }
}

fn finding_paths(findings: &[SweepFinding]) -> Vec<&str> {
    findings.iter().map(|f| f.path.as_str()).collect()
}

fn finding_lines(findings: &[SweepFinding]) -> Vec<usize> {
    findings.iter().map(|f| f.line).collect()
}

// ---------------------------------------------------------------------------
// The incident, reproduced at runtime.
// ---------------------------------------------------------------------------

#[test]
fn captured_slots_bytes_are_a_bare_array_and_the_authored_belief_is_an_object() {
    assert_eq!(json_shape(CAPTURED_SLOTS), JsonShape::BareArray);
    assert_eq!(json_shape(AUTHORED_SLOTS), JsonShape::Object);
}

#[test]
fn the_invented_shape_fails_to_deserialize_the_real_bytes_and_the_fixed_shape_does_not() {
    #[derive(Debug, serde::Deserialize)]
    #[allow(dead_code)]
    struct SlotsResponse {
        n_slots: usize,
        slots: Vec<serde_json::Value>,
    }
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Slot {
        id: usize,
    }

    // The pre-fix parser, run against the bytes the dependency actually sent.
    let old = serde_json::from_str::<SlotsResponse>(CAPTURED_SLOTS);
    let old_outcome = match &old {
        Ok(_) => ProbeOutcome::parsed(1),
        Err(err) => ProbeOutcome::failed(0, &err.to_string()),
    };
    assert!(old_outcome.failed_to_parse());
    let old_err = old.unwrap_err().to_string();
    // The derived impl reads a JSON array positionally, so the mismatch is
    // reported at the first field: slot 0 (a map) lands on `n_slots`. The
    // point is not the wording, it is that the real bytes are a hard error
    // for the invented shape, and the test never saw it because it never
    // ran over the real bytes.
    assert!(
        old_err.contains("invalid type: map, expected usize"),
        "the real bytes must be rejected with a type error, got: {old_err}"
    );

    // The post-fix parser, same bytes.
    let new = serde_json::from_str::<Vec<Slot>>(CAPTURED_SLOTS);
    let new_outcome = match &new {
        Ok(slots) => ProbeOutcome::parsed(slots.len()),
        Err(err) => ProbeOutcome::failed(0, &err.to_string()),
    };
    assert_eq!(new_outcome, ProbeOutcome::parsed(3));

    // Both directions, over captured bytes: that is the proof.
    let verdict = fix_evidence(&proof(captured(), old_outcome, new_outcome));
    assert_eq!(verdict, FixEvidence::NeededAndFixed);
    assert!(verdict.is_evidence());
}

#[test]
fn the_same_test_and_parser_written_against_the_authored_bytes_agree_perfectly() {
    // This is why the bug survived: the invented bytes match the invented
    // parser, so the suite is green in both directions and says nothing about
    // llama.cpp.
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct SlotsResponse {
        n_slots: usize,
        slots: Vec<serde_json::Value>,
    }

    let belief: SlotsResponse = serde_json::from_str(AUTHORED_SLOTS).expect("authored bytes parse");
    assert_eq!(belief.n_slots, 3);

    // And the proof built from them is not evidence, whatever it shows.
    let verdict = fix_evidence(&proof(
        authored(),
        ProbeOutcome::failed(0, "invalid type: map, expected a sequence"),
        ProbeOutcome::parsed(3),
    ));
    assert_eq!(verdict, FixEvidence::RunOnAuthoredBytes);
    assert!(!verdict.is_evidence());
}

// ---------------------------------------------------------------------------
// Invariant 1: provenance is the distinction, and self-agreement blocks.
// ---------------------------------------------------------------------------

#[test]
fn no_assertions_about_an_external_response_is_not_a_defect() {
    let coverage = coverage_verdict(&[]);
    assert_eq!(coverage, Coverage::NoAssertions);
    assert!(!coverage.blocks());
    assert!(coverage.line().contains("no assertions"));
}

#[test]
fn assertions_backed_only_by_authored_bytes_are_self_agreeing_and_block() {
    let coverage = coverage_verdict(&[
        assertion("GET /slots", authored()),
        assertion("GET /props", authored()),
        assertion("POST /completion", authored()),
    ]);
    assert_eq!(coverage, Coverage::SelfAgreeing { authored: 3 });
    assert!(
        coverage.blocks(),
        "0 captured fixtures means every assertion checks the parser against the test's own \
         belief — the exact #4243 state"
    );
    let line = coverage.line();
    assert!(line.contains("0 captured, 3 authored"), "{line}");
    assert!(line.contains("#4243"), "{line}");
}

#[test]
fn one_captured_assertion_makes_the_suite_recorded_backed() {
    let coverage = coverage_verdict(&[
        assertion("GET /slots", captured()),
        assertion("GET /props", authored()),
    ]);
    assert_eq!(
        coverage,
        Coverage::RecordedBacked {
            captured: 1,
            authored: 1
        }
    );
    assert!(!coverage.blocks());
    assert!(coverage.line().contains("1 captured, 1 authored"));
}

#[test]
fn captured_provenance_names_where_the_bytes_came_from() {
    assert!(captured().captured());
    assert!(!authored().captured());
    assert_eq!(
        captured().label(),
        "captured from http://llama-01:8080/slots"
    );
    assert_eq!(authored().label(), "authored by the test");
}

#[test]
fn provenance_serialises_with_its_kind_tag() {
    let json = serde_json::to_string(&captured()).expect("serialisable");
    assert_eq!(
        json,
        r#"{"kind":"captured","source":"http://llama-01:8080/slots"}"#
    );
    assert_eq!(
        serde_json::to_string(&authored()).expect("serialisable"),
        r#"{"kind":"authored"}"#
    );
    let back: FixtureProvenance = serde_json::from_str(&json).expect("round trips");
    assert_eq!(back, captured());
}

// ---------------------------------------------------------------------------
// Invariant 2: the detection sweep finds authored payloads and nothing else.
// ---------------------------------------------------------------------------

#[test]
fn a_go_test_building_the_response_in_a_raw_literal_is_one_json_finding() {
    let source = r#"package proxy

func TestSlots(t *testing.T) {
	body := `{"n_slots": 3, "slots": [{"id": 0}]}`
	if body == "" {
		t.Fatal("empty")
	}
}
"#;
    let payloads = scan_wire_literals("internal/proxy/slots_test.go", source);
    assert_eq!(payloads.len(), 1, "one payload, not one per brace");
    assert_eq!(payloads[0].format, WireFormat::Json);
    assert_eq!(payloads[0].line, 4);
    assert_eq!(payloads[0].path, "internal/proxy/slots_test.go");
    assert!(
        payloads[0].excerpt.contains("\"n_slots\": 3"),
        "{}",
        payloads[0].excerpt
    );
}

#[test]
fn a_multiline_raw_literal_is_reported_where_the_author_wrote_it_and_lines_keep_counting() {
    let source = r#"func TestSlots(t *testing.T) {
	body := `{
  "n_slots": 3,
  "slots": [{"id": 0}]
}`
	other := `{"status": "ok"}`
	_ = body
	_ = other
}
"#;
    let payloads = scan_wire_literals("internal/proxy/slots_test.go", source);
    assert_eq!(payloads.len(), 2);
    assert_eq!(
        finding_lines(
            &payloads
                .into_iter()
                .map(|p| SweepFinding {
                    path: p.path,
                    line: p.line,
                    format: p.format,
                    excerpt: p.excerpt,
                })
                .collect::<Vec<_>>()
        ),
        vec![2, 6],
        "the multi-line literal is reported at its opening line and the literal after it \
         must not inherit its line count"
    );
}

#[test]
fn a_rust_test_building_the_response_in_a_raw_string_is_found() {
    let source = r##"#[cfg(test)]
mod tests {
    #[test]
    fn slots_parse() {
        let body = r#"{"n_slots": 3, "slots": [{"id": 0}]}"#;
        assert!(parse(body).is_ok());
    }
}
"##;
    let payloads = scan_wire_literals("crates/proxy/src/slots.rs", source);
    assert_eq!(payloads.len(), 1);
    assert_eq!(payloads[0].format, WireFormat::Json);
    assert_eq!(payloads[0].line, 5);
}

#[test]
fn escaped_quotes_inside_a_quoted_string_do_not_end_the_literal_early() {
    let source = "func TestX(t *testing.T) {\n\tbody := \"{\\\"n_slots\\\": 3, \\\"slots\\\": []}\"\n\t_ = body\n}\n";
    let payloads = scan_wire_literals("internal/proxy/x_test.go", source);
    assert_eq!(payloads.len(), 1, "the escaped body is one JSON payload");
    assert_eq!(payloads[0].format, WireFormat::Json);
    assert_eq!(payloads[0].line, 2);
}

#[test]
fn xml_and_protobuf_text_payloads_are_classified_too() {
    let xml = scan_wire_literals(
        "internal/oai/listrecords_test.go",
        "`<srs><version>3.0</version></srs>`\n",
    );
    assert_eq!(xml.len(), 1);
    assert_eq!(xml[0].format, WireFormat::Xml);

    let protobuf = scan_wire_literals(
        "internal/model/params_test.go",
        "`model: \"llama-3\"\nn_ctx: 65536\nbatch: 512\n`\n",
    );
    assert_eq!(protobuf.len(), 1);
    assert_eq!(protobuf[0].format, WireFormat::ProtobufText);
}

#[test]
fn a_json_example_in_a_comment_is_never_a_payload() {
    let rust = "// GET /slots -> {\"n_slots\": 3, \"slots\": []}\nfunc TestX() {}\n";
    assert!(scan_wire_literals("internal/proxy/x_test.rs", rust).is_empty());

    let block = "/* the response is {\"slots\": []} today */\nfn t() {}\n";
    assert!(scan_wire_literals("internal/proxy/x_test.rs", block).is_empty());

    let shell = "# response: {\"n_slots\": 3, \"slots\": []}\necho ok\n";
    assert!(scan_wire_literals("tests/integration/slots.sh", shell).is_empty());

    let python = "# {\"n_slots\": 3}\ndef test_slots():\n    assert True\n";
    assert!(scan_wire_literals("tests/test_slots.py", python).is_empty());
}

#[test]
fn plain_strings_and_test_tables_are_not_payloads() {
    let source = r#"func TestX(t *testing.T) {
	name := "hello"
	cases := []string{"a", "b"}
	empty := ""
	url := "http://llama-01:8080/slots"
	_, _, _, _ = name, cases, empty, url
}
"#;
    assert!(scan_wire_literals("internal/proxy/x_test.go", source).is_empty());

    // An array of bare scalars has no `"key":` and is a table, not a response.
    assert!(scan_wire_literals("internal/proxy/x_test.go", "`[\"a\", \"b\"]`\n").is_empty());
}

#[test]
fn fixture_loading_code_is_not_a_payload() {
    let source = r#"func TestSlots(t *testing.T) {
	raw, err := os.ReadFile("testdata/slots_llamacpp.json")
	if err != nil {
		t.Fatal(err)
	}
	var slots []Slot
	if err := json.Unmarshal(raw, &slots); err != nil {
		t.Fatal(err)
	}
}
"#;
    assert!(scan_wire_literals("internal/proxy/slots_test.go", source).is_empty());
}

#[test]
fn a_test_that_names_a_recorded_fixture_is_skipped_wholesale() {
    assert!(references_recorded_fixture(
        r#"os.ReadFile("testdata/slots.json")"#
    ));
    assert!(references_recorded_fixture(
        r#"filepath.Join("testdata", "slots.json")"#
    ));
    assert!(references_recorded_fixture("cat fixtures/slots.xml"));
    assert!(references_recorded_fixture("path: `test-fixtures`"));
    assert!(!references_recorded_fixture(
        r#"body := `{"n_slots": 3, "slots": []}`"#
    ));
    assert!(
        !references_recorded_fixture("// a fixture of the imagination"),
        "the word 'fixture' alone must not excuse an authored payload"
    );

    let source = r#"func TestSlots(t *testing.T) {
	raw, _ := os.ReadFile("testdata/slots.json")
	authored := `{"n_slots": 3, "slots": []}` // an extra case, next to real bytes
	_, _ = raw, authored
}
"#;
    let findings = sweep_authored_payloads(&[file("internal/proxy/slots_test.go", source)]);
    assert!(
        findings.is_empty(),
        "an authored literal alongside recorded bytes is a complementary case (a malformed or \
         empty response), not the #4243 defect"
    );
}

#[test]
fn test_source_detection_covers_the_common_naming_conventions() {
    for path in [
        "internal/proxy/slots_test.go",
        "src/server/slots.test.ts",
        "src/server/slots.spec.ts",
        "tests/unit/slots.rs",
        "tests/test_slots.py",
        "test/slots.sh",
    ] {
        assert!(is_test_source(path), "{path} must read as a test");
    }
    for path in [
        "internal/proxy/slots.go",
        "crates/autospec-core/src/wire_fixture.rs",
        "src/server/slots.ts",
        "docs/specs/2026-08-01-design.md",
        "testdata/slots.json",
        "tests/testdata/slots.json",
    ] {
        assert!(!is_test_source(path), "{path} must not read as a test");
    }
}

#[test]
fn a_rust_src_file_with_an_inline_test_module_is_a_test_file() {
    let src = file(
        "crates/proxy/src/slots.rs",
        "pub fn parse() {}\n#[cfg(test)]\nmod tests {\n    fn t() {}\n}\n",
    );
    assert!(!is_test_source(src.path));
    assert!(is_test_file(&src));

    let pure = file("crates/proxy/src/slots.rs", "pub fn parse() {}\n");
    assert!(!is_test_file(&pure));
}

#[test]
fn the_sweep_flags_the_incident_file_and_stays_quiet_on_the_fixed_one() {
    let invented = file(
        "internal/proxy/slots_test.go",
        "func TestSlots(t *testing.T) {\n\tbody := `{\"n_slots\": 3, \"slots\": [{\"id\": 0}]}`\n\t_ = body\n}\n",
    );
    let recorded = file(
        "internal/proxy/slots_recorded_test.go",
        "func TestSlots(t *testing.T) {\n\traw, _ := os.ReadFile(\"testdata/slots_llamacpp.json\")\n\t_ = raw\n}\n",
    );
    let production = file(
        "internal/proxy/slots.go",
        "func Parse(body string) {\n\t_ = `{\"n_slots\": 3}`\n}\n",
    );

    let findings = sweep_authored_payloads(&[invented, recorded, production]);
    assert_eq!(
        finding_paths(&findings),
        vec!["internal/proxy/slots_test.go"]
    );
    assert_eq!(findings[0].line, 2);
    assert_eq!(findings[0].format, WireFormat::Json);

    let report = findings[0].report();
    assert!(
        report.starts_with("internal/proxy/slots_test.go:2: "),
        "{report}"
    );
    assert!(
        report.contains("JSON payload built inside the test"),
        "{report}"
    );
    assert!(report.contains("#4243"), "{report}");
    assert!(report.contains("testdata"), "{report} names the fix");
}

#[test]
fn sweep_findings_are_sorted_so_output_is_deterministic() {
    let a = file("b/z_test.go", "x := `{\"a\": 1}`\ny := `{\"b\": 1}`\n");
    let b = file("a/y_test.go", "z := `{\"c\": 1}`\n");
    let forwards = sweep_authored_payloads(&[a, b]);
    let backwards = sweep_authored_payloads(&[b, a]);
    assert_eq!(forwards, backwards);
    assert_eq!(
        finding_paths(&forwards),
        vec!["a/y_test.go", "b/z_test.go", "b/z_test.go"]
    );
    assert_eq!(finding_lines(&forwards), vec![1, 1, 2]);
}

#[test]
fn sweep_summary_distinguishes_clean_from_nothing_to_check() {
    let clean = sweep_summary(&[]);
    assert!(clean.contains("0 findings"), "{clean}");
    assert!(!clean.contains("across"), "{clean}");

    let findings = sweep_authored_payloads(&[
        file("a/y_test.go", "z := `{\"c\": 1}`\n"),
        file("b/z_test.go", "z := `{\"d\": 1}`\n"),
    ]);
    let summary = sweep_summary(&findings);
    assert!(
        summary.contains("2 authored payload(s) across 2 test file(s)"),
        "{summary}"
    );
    assert!(summary.contains("#4243"), "{summary}");
}

// ---------------------------------------------------------------------------
// Invariant 3: a fixture is data, and it must keep its shape.
// ---------------------------------------------------------------------------

#[test]
fn a_fixture_must_be_under_a_data_directory_and_carry_a_data_extension() {
    assert!(fixture_is_data_file(
        "internal/proxy/testdata/slots_llamacpp.json"
    ));
    assert!(fixture_is_data_file("fixtures/srs.xml"));
    assert!(fixture_is_data_file("test_fixtures/slots.yaml"));
    assert!(fixture_is_data_file("test-fixtures/slots.textproto"));

    assert!(
        !fixture_is_data_file("internal/proxy/slots_fixture.go"),
        "a Go file that believes something is not committed data"
    );
    assert!(
        !fixture_is_data_file("internal/proxy/testdata/slots.go"),
        "the same belief parked in the data directory still compiles into the test"
    );
    assert!(
        !fixture_is_data_file("internal/proxy/slots.json"),
        "data-shaped bytes outside a fixture directory are not a recorded fixture"
    );
    assert!(!fixture_is_data_file("testdata/notes"));
    assert!(!fixture_is_data_file("slots.json"));
}

#[test]
fn every_declared_fixture_directory_is_data_recognisable() {
    let dirs: BTreeSet<&str> = FIXTURE_DATA_DIRS.iter().copied().collect();
    for dir in dirs {
        assert!(
            fixture_is_data_file(&format!("{dir}/slots.json")),
            "{dir} must be a fixture directory"
        );
    }
}

#[test]
fn json_shape_separates_the_two_shapes_that_cost_a_production_incident() {
    assert_eq!(json_shape(CAPTURED_SLOTS), JsonShape::BareArray);
    assert_eq!(json_shape(AUTHORED_SLOTS), JsonShape::Object);
    // `[]` is still a bare array: the assertion is about shape, not content.
    assert_eq!(json_shape("[]"), JsonShape::BareArray);
    assert_eq!(json_shape("{}"), JsonShape::Object);
    assert_eq!(json_shape("null"), JsonShape::Scalar);
    assert_eq!(json_shape("  \n\t "), JsonShape::Empty);
    assert_eq!(json_shape("{oops"), JsonShape::Malformed);
    // Trailing garbage is malformed, not "the object part".
    assert_eq!(json_shape("{} {}"), JsonShape::Malformed);
}

#[test]
fn a_bare_array_fixture_asserted_as_bare_array_is_ok() {
    assert_eq!(
        assert_fixture_shape(
            "testdata/slots_llamacpp.json",
            JsonShape::BareArray,
            CAPTURED_SLOTS
        ),
        Ok(JsonShape::BareArray)
    );
}

#[test]
fn a_fixture_that_stopped_being_a_bare_array_fails_the_test_that_needs_one() {
    let drift = assert_fixture_shape(
        "internal/proxy/testdata/slots_llamacpp.json",
        JsonShape::BareArray,
        AUTHORED_SLOTS,
    )
    .expect_err("an object is not a bare array");
    assert_eq!(drift.expected, JsonShape::BareArray);
    assert_eq!(drift.found, JsonShape::Object);
    assert_eq!(drift.fixture, "internal/proxy/testdata/slots_llamacpp.json");
    let line = drift.line();
    assert!(line.contains("fixture is no longer"), "{line}");
    assert!(line.contains("changed shape"), "{line}");
    assert!(line.contains("(found an object)"), "{line}");
    assert_eq!(line, drift.to_string());
}

#[test]
fn a_fixture_that_no_longer_parses_is_reported_as_unusable_not_as_a_pass() {
    let drift = assert_fixture_shape(
        "testdata/slots_llamacpp.json",
        JsonShape::BareArray,
        "[{\"id\": 0},",
    )
    .expect_err("truncated bytes do not parse");
    assert_eq!(drift.found, JsonShape::Malformed);
    assert!(drift.reason.contains("does not parse"), "{}", drift.reason);

    let empty = assert_fixture_shape("testdata/slots_llamacpp.json", JsonShape::BareArray, "\n")
        .expect_err("no bytes");
    assert_eq!(empty.found, JsonShape::Empty);
    assert!(empty.reason.contains("empty"), "{}", empty.reason);
}

#[test]
fn expecting_malformed_bytes_is_itself_a_defect() {
    let drift = assert_fixture_shape("testdata/corrupt.json", JsonShape::Malformed, "{oops")
        .expect_err("a test that expects unparseable bytes passes on any corruption");
    assert!(
        drift.reason.contains("cannot be malformed"),
        "{}",
        drift.reason
    );
    assert_eq!(drift.found, JsonShape::Malformed);
}

// ---------------------------------------------------------------------------
// Invariant 4: the fix must be proven in both directions, on real bytes.
// ---------------------------------------------------------------------------

#[test]
fn captured_bytes_that_break_the_old_code_and_pass_the_new_one_are_evidence() {
    let verdict = fix_evidence(&proof(captured(), old_code_failed(), new_code_parsed()));
    assert_eq!(verdict, FixEvidence::NeededAndFixed);
    assert!(verdict.is_evidence());
    assert!(verdict.reason().contains("pre-fix code fails"));
}

#[test]
fn a_proof_over_authored_bytes_is_refused_before_its_outcomes_are_read() {
    // "Look, the old code failed!" on bytes the author wrote proves only that
    // the invention differs from the parser — the cheapest possible nothing.
    let verdict = fix_evidence(&proof(authored(), old_code_failed(), new_code_parsed()));
    assert_eq!(verdict, FixEvidence::RunOnAuthoredBytes);
    assert!(!verdict.is_evidence());
    assert!(verdict.reason().contains("not bytes the dependency sent"));
}

#[test]
fn old_code_that_already_handled_the_bytes_means_the_bug_was_not_reproduced() {
    let verdict = fix_evidence(&proof(
        captured(),
        ProbeOutcome::parsed(3),
        new_code_parsed(),
    ));
    assert_eq!(verdict, FixEvidence::BugNotReproduced);
    assert!(!verdict.is_evidence());
    assert!(verdict.reason().contains("do not reproduce the bug"));
}

#[test]
fn new_code_still_failing_on_captured_bytes_is_an_incomplete_fix() {
    let verdict = fix_evidence(&proof(
        captured(),
        old_code_failed(),
        ProbeOutcome::failed(0, "invalid type: sequence"),
    ));
    assert_eq!(verdict, FixEvidence::FixIncomplete);
    assert!(!verdict.is_evidence());
}

#[test]
fn silently_parsing_zero_records_is_a_failure_not_a_pass() {
    // A parser that yields nothing on a real fleet response makes the fleet
    // look idle rather than broken: the dangerous version of the bug.
    let silent = ProbeOutcome::parsed(0);
    assert!(silent.failed_to_parse());
    assert_eq!(
        fix_evidence(&proof(captured(), old_code_failed(), silent)),
        FixEvidence::FixIncomplete
    );
    assert_eq!(ProbeOutcome::parsed(1).describe(), "parsed 1");
    assert_eq!(
        ProbeOutcome::failed(0, "boom").describe(),
        "failed to parse: boom"
    );
    assert_eq!(
        ProbeOutcome::failed(2, "boom").describe(),
        "parsed 2 then failed: boom"
    );
}

#[test]
fn the_divergence_report_shows_the_verdict_and_both_outcomes() {
    let good = divergence_report(&proof(captured(), old_code_failed(), new_code_parsed()));
    assert!(
        good.starts_with("GET /slots (captured from http://llama-01:8080/slots): "),
        "{good}"
    );
    assert!(good.contains("verified both directions"), "{good}");
    assert!(
        good.contains("old: failed to parse: invalid type: sequence"),
        "{good}"
    );
    assert!(good.contains("new: parsed 3"), "{good}");

    let bad = divergence_report(&proof(authored(), old_code_failed(), new_code_parsed()));
    assert!(bad.contains("NOT EVIDENCE"), "{bad}");
    assert!(bad.contains("authored by the test"), "{bad}");
}

// ---------------------------------------------------------------------------
// Invariant 5: the spec must name the sample source or be refused.
// ---------------------------------------------------------------------------

#[test]
fn a_declared_recorded_fixture_is_accepted_only_when_it_is_data() {
    assert_eq!(
        parse_sample_source("fixture: internal/proxy/testdata/slots_llamacpp.json"),
        Ok(SampleSource::RecordedFixture {
            path: "internal/proxy/testdata/slots_llamacpp.json".to_string()
        })
    );
    assert_eq!(
        parse_sample_source("  RECORDED-FIXTURE:   fixtures/slots.yaml  "),
        Ok(SampleSource::RecordedFixture {
            path: "fixtures/slots.yaml".to_string()
        })
    );
    let recorded_line = SampleSource::RecordedFixture {
        path: "testdata/slots.json".to_string(),
    }
    .line();
    assert!(
        recorded_line.contains("recorded fixture testdata/slots.json"),
        "{recorded_line}"
    );

    let drift = parse_sample_source("fixture: internal/proxy/slots_fixture.go")
        .expect_err("a .go file is not a fixture");
    assert_eq!(
        drift,
        SampleSourceError::FixtureNotDataFile {
            path: "internal/proxy/slots_fixture.go".to_string()
        }
    );
    let message = drift.message();
    assert!(message.contains("testdata"), "{message}");
    assert!(message.contains("json"), "{message}");
    assert_eq!(message, drift.to_string());
}

#[test]
fn a_live_sample_source_must_name_an_http_endpoint() {
    assert_eq!(
        parse_sample_source("contract: http://10.0.0.5:8080"),
        Ok(SampleSource::ContractTest {
            base_url: "http://10.0.0.5:8080".to_string()
        })
    );
    assert_eq!(
        parse_sample_source("schema: https://llama.cpp/slots.openapi.json"),
        Ok(SampleSource::PublishedSchema {
            url: "https://llama.cpp/slots.openapi.json".to_string()
        })
    );
    let live_line = SampleSource::ContractTest {
        base_url: "http://10.0.0.5:8080".to_string(),
    }
    .line();
    assert!(
        live_line.contains("contract test against http://10.0.0.5:8080"),
        "{live_line}"
    );

    let vague = parse_sample_source("contract: the staging server")
        .expect_err("a live source must name the endpoint");
    assert_eq!(
        vague,
        SampleSourceError::UrlMissing {
            value: "the staging server".to_string()
        }
    );
    assert!(
        vague.message().contains("http(s) URL"),
        "{}",
        vague.message()
    );
}

#[test]
fn an_unspecified_sample_source_is_refused_naming_the_three_accepted_forms() {
    assert_eq!(parse_sample_source(""), Err(SampleSourceError::Empty));
    assert_eq!(parse_sample_source("   "), Err(SampleSourceError::Empty));

    let err = parse_sample_source("we will hand-write a realistic response")
        .expect_err("\"realistic\" is exactly the belief that produced the incident");
    assert_eq!(
        err,
        SampleSourceError::Unspecified {
            decl: "we will hand-write a realistic response".to_string()
        }
    );
    let message = err.message();
    assert!(message.contains("fixture:"), "{message}");
    assert!(message.contains("contract:"), "{message}");
    assert!(message.contains("schema:"), "{message}");

    let keywordless = parse_sample_source("testdata/slots.json")
        .expect_err("a path without its keyword names nothing");
    assert!(matches!(keywordless, SampleSourceError::Unspecified { .. }));
}

#[test]
fn wire_format_labels_are_stable_for_report_matching() {
    assert_eq!(WireFormat::Json.label(), "JSON");
    assert_eq!(WireFormat::Xml.label(), "XML");
    assert_eq!(WireFormat::ProtobufText.label(), "protobuf text");
    assert_eq!(
        serde_json::to_string(&WireFormat::ProtobufText).expect("serialisable"),
        "\"protobuf_text\""
    );
    assert_eq!(
        serde_json::to_string(&JsonShape::BareArray).expect("serialisable"),
        "\"bare_array\""
    );
}

#[test]
fn an_excerpt_is_flattened_and_truncated_so_one_finding_is_one_line() {
    let long = format!("`{{\"a\": 1, \"filler\": \"{}\"}}`", "x".repeat(200));
    let payloads = scan_wire_literals("a/x_test.go", &long);
    assert_eq!(payloads.len(), 1);
    let excerpt = &payloads[0].excerpt;
    assert!(!excerpt.contains('\n'), "{excerpt}");
    assert!(
        excerpt.chars().count() <= 61,
        "{} chars",
        excerpt.chars().count()
    );
    assert!(excerpt.ends_with('…'), "{excerpt}");
}

#[test]
fn the_scanner_is_total_over_truncated_and_pathological_input() {
    // Unterminated literals, lone delimiters, raw-string prefixes that are
    // identifiers: none may panic or hang.
    for source in [
        "x := \"{\n",
        "x := r#\"{\"",
        "x := r##\"{\"#\n",
        "x := `{\"",
        "\"",
        "r",
        "r#",
        "/* unterminated {\"a\": 1}",
        "//",
        "",
        "\n\n\n",
        CAPTURED_SLOTS,
    ] {
        let payloads = scan_wire_literals("a/x_test.go", source);
        for payload in payloads {
            let _: &InlinePayload = &payload;
            assert!(payload.line >= 1);
        }
    }
}

#[test]
fn shape_drift_and_sample_source_errors_carry_their_own_text() {
    // The report line is the artefact a reviewer reads; it must name the file.
    let drift = ShapeDrift {
        fixture: "testdata/slots.json".to_string(),
        expected: JsonShape::BareArray,
        found: JsonShape::Object,
        reason: "fixture is no longer the shape the regression needs; the regression it guards \
                 has changed shape",
    };
    assert!(
        drift.line().starts_with("testdata/slots.json: "),
        "{}",
        drift.line()
    );

    let err = SampleSourceError::FixtureNotDataFile {
        path: "testdata/slots.go".to_string(),
    };
    assert!(
        err.message().contains("testdata/slots.go"),
        "{}",
        err.message()
    );
}
