//! The shared golden fixtures (`tests/fixtures/protocol/`) asserted by the real
//! Rust validator. The TypeScript side reads the same files.

use std::path::{Path, PathBuf};

use autospec_core::protocol::negotiate::check_envelope;
use autospec_core::protocol::schema_ir::{load_dir, SchemaSet};
use autospec_core::protocol::sequence::{Accept, MissionSequencer};
use autospec_core::protocol::ProtocolError;
use serde_json::Value;

const ENTITIES: [&str; 9] = [
    "mission",
    "task",
    "execution",
    "worker",
    "event",
    "resource-record",
    "checkpoint",
    "lease",
    "budget",
];

/// Extensions this test reader implements; any other required one is refused.
const UNDERSTOOD: &[&str] = &["x-known"];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixture(name: &str) -> Value {
    let path = repo_root().join("tests/fixtures/protocol").join(name);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{name} is not JSON: {e}"))
}

fn schemas() -> SchemaSet {
    load_dir(&repo_root().join("schemas")).expect("protocol schemas load")
}

/// The class a refusal belongs to. Fixtures assert the class, so the two
/// structural codes the Rust reader emits both count as `SCHEMA_VIOLATION`.
fn class_of(error: &ProtocolError) -> String {
    match error.code() {
        "MISSING_FIELD" | "INVALID_FIELD" => "SCHEMA_VIOLATION".to_string(),
        other => other.to_string(),
    }
}

/// What the validator pipeline says about one record, given the records a
/// consumer has already accepted for the same mission.
///
/// Stages run in order and the first refusal wins: envelope check, schema
/// validation (reported as `violation`), then the mission sequencer.
fn outcome(
    set: &SchemaSet,
    sequencer: &mut MissionSequencer,
    entity: &str,
    record: &Value,
) -> String {
    let envelope = match check_envelope(record, UNDERSTOOD) {
        Ok(envelope) => envelope,
        Err(error) => return class_of(&error),
    };
    if !set.validate(entity, record).is_empty() {
        return "violation".to_string();
    }
    let digest = serde_json::to_string(record).expect("record serializes");
    match sequencer.accept(&envelope, &digest) {
        Ok(Accept::Applied) => "applied".to_string(),
        Ok(Accept::Duplicate) => "duplicate".to_string(),
        Err(error) => class_of(&error),
    }
}

#[test]
fn valid_fixture_has_one_key_per_entity_and_every_key_has_a_schema() {
    let valid = fixture("valid.json");
    let mut keys: Vec<&str> = valid
        .as_object()
        .expect("object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    let mut expected = ENTITIES.to_vec();
    expected.sort_unstable();
    assert_eq!(keys, expected);
    let set = schemas();
    for entity in ENTITIES {
        assert!(set.entities().contains(&entity), "{entity} has no schema");
    }
}

#[test]
fn every_valid_record_has_no_violations_and_passes_the_envelope_check() {
    let set = schemas();
    let valid = fixture("valid.json");
    for entity in ENTITIES {
        let records = valid[entity]
            .as_array()
            .unwrap_or_else(|| panic!("{entity}: array"));
        assert!(!records.is_empty(), "{entity}: at least one record");
        for record in records {
            assert_eq!(
                record["schema"],
                format!("autospec.protocol.{entity}.v1"),
                "{entity}: schema id"
            );
            let violations = set.validate(entity, record);
            assert!(violations.is_empty(), "{entity}: {violations:?}");
            check_envelope(record, UNDERSTOOD).unwrap_or_else(|e| panic!("{entity}: {e}"));
        }
    }
}

#[test]
fn valid_events_are_one_mission_with_sequences_one_to_four_and_all_apply() {
    let valid = fixture("valid.json");
    let events = valid["event"].as_array().expect("events");
    let sequences: Vec<u64> = events
        .iter()
        .map(|e| e["sequence"].as_u64().unwrap())
        .collect();
    assert_eq!(sequences, vec![1, 2, 3, 4]);
    let mission = events[0]["mission_id"].clone();
    assert!(events.iter().all(|e| e["mission_id"] == mission));
    let set = schemas();
    let mut sequencer = MissionSequencer::new();
    for event in events {
        assert_eq!(outcome(&set, &mut sequencer, "event", event), "applied");
    }
}

#[test]
fn every_invalid_case_yields_exactly_its_expected_outcome() {
    let set = schemas();
    let invalid = fixture("invalid.json");
    let cases = invalid.as_array().expect("array");
    assert!(cases.len() >= 6, "only {} cases", cases.len());
    let mut names = std::collections::BTreeSet::new();
    for case in cases {
        let name = case["name"].as_str().expect("name");
        assert!(names.insert(name.to_string()), "duplicate case name {name}");
        let entity = case["entity"].as_str().expect("entity");
        assert!(
            ENTITIES.contains(&entity),
            "{name}: unknown entity {entity}"
        );
        let expect = case["expect"].as_str().expect("expect");

        // Records the consumer accepted before this one (for sequence cases).
        let mut sequencer = MissionSequencer::new();
        for prior in case
            .get("after")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            assert_eq!(
                outcome(&set, &mut sequencer, entity, prior),
                "applied",
                "{name}: prior record"
            );
        }
        assert_eq!(
            outcome(&set, &mut sequencer, entity, &case["record"]),
            expect,
            "{name}"
        );
    }
}

#[test]
fn invalid_fixture_covers_the_contract_failure_classes() {
    let invalid = fixture("invalid.json");
    let expects: std::collections::BTreeSet<&str> = invalid
        .as_array()
        .expect("array")
        .iter()
        .map(|c| c["expect"].as_str().expect("expect"))
        .collect();
    for needed in [
        "violation",
        "SCHEMA_VIOLATION",
        "UNKNOWN_REQUIRED_FIELD",
        "PROTOCOL_VERSION_UNSUPPORTED",
        "SEQUENCE_GAP",
        "SEQUENCE_CONFLICT",
        "IDEMPOTENCY_CONFLICT",
        "IDEMPOTENCY_KEY_REUSED",
    ] {
        assert!(expects.contains(needed), "no case expects {needed}");
    }
}
