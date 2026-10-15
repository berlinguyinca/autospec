//! The protocol schema loader and validator against the real `schemas/` files.

use std::path::{Path, PathBuf};

use autospec_core::protocol::schema_ir::{load_dir, SchemaSet, Violation};
use autospec_core::protocol::ProtocolError;
use serde_json::{json, Value};

fn schemas_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas")
}

fn real() -> SchemaSet {
    load_dir(&schemas_dir()).expect("the real protocol schemas load")
}

/// A fresh directory holding copies of the real protocol schemas.
fn copy_of_schemas(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("protocol-schemas-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for entry in std::fs::read_dir(schemas_dir()).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if name.starts_with("autospec-protocol-") && name.ends_with(".v1.schema.json") {
            std::fs::copy(&path, dir.join(&name)).unwrap();
        }
    }
    dir
}

fn edit(dir: &Path, file: &str, f: impl FnOnce(&mut Value)) {
    let path = dir.join(file);
    let mut v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    f(&mut v);
    std::fs::write(path, serde_json::to_string_pretty(&v).unwrap()).unwrap();
}

fn execution() -> Value {
    json!({
        "schema": "autospec.protocol.execution.v1",
        "protocol_version": 1,
        "idempotency_key": "idem-1",
        "mission_id": "m-1",
        "sequence": 1,
        "required_extensions": [],
        "execution_id": "e-1",
        "attempt_id": "a-1",
        "task_id": "t-1",
        "worker_id": "w-1",
        "executor": "pi-local",
        "model_policy": {"model": "x", "provider": "y"},
        "start_evidence": {"agent_rc": 0, "session_recorded": true, "assistant_turns": 2},
        "status": "OK"
    })
}

fn has(vs: &[Violation], pointer: &str, rule: &str) -> bool {
    vs.iter().any(|v| v.pointer == pointer && v.rule == rule)
}

#[test]
fn load_dir_loads_exactly_the_ten_protocol_schemas() {
    let set = real();
    assert_eq!(
        set.entities(),
        vec![
            "budget",
            "checkpoint",
            "envelope",
            "event",
            "execution",
            "lease",
            "mission",
            "resource-record",
            "task",
            "worker"
        ]
    );
}

#[test]
fn a_valid_execution_has_no_violations() {
    assert_eq!(real().validate("execution", &execution()), vec![]);
}

#[test]
fn a_missing_required_field_is_named_by_pointer() {
    let mut rec = execution();
    rec.as_object_mut().unwrap().remove("execution_id");
    let vs = real().validate("execution", &rec);
    assert!(has(&vs, "/execution_id", "required"), "{vs:?}");
}

#[test]
fn an_unknown_top_level_field_breaks_additional_properties() {
    let mut rec = execution();
    rec["surprise"] = json!(1);
    let vs = real().validate("execution", &rec);
    assert!(has(&vs, "/surprise", "additionalProperties"), "{vs:?}");
}

#[test]
fn every_violation_is_returned_not_the_first() {
    let mut rec = execution();
    let obj = rec.as_object_mut().unwrap();
    obj.remove("task_id");
    obj.insert("status".into(), json!("BOGUS"));
    obj.insert("sequence".into(), json!(0));
    obj.insert("protocol_version".into(), json!("1"));
    obj.insert("schema".into(), json!("autospec.protocol.task.v1"));
    let vs = real().validate("execution", &rec);
    assert!(has(&vs, "/task_id", "required"), "{vs:?}");
    assert!(has(&vs, "/status", "enum"), "{vs:?}");
    assert!(has(&vs, "/sequence", "minimum"), "{vs:?}");
    assert!(has(&vs, "/protocol_version", "type"), "{vs:?}");
    assert!(has(&vs, "/schema", "const"), "{vs:?}");
}

#[test]
fn nested_objects_report_nested_pointers() {
    let mut rec = execution();
    rec["model_policy"] = json!({"model": 3, "extra": true});
    rec["start_evidence"]["assistant_turns"] = json!(-1);
    let vs = real().validate("execution", &rec);
    assert!(has(&vs, "/model_policy/model", "type"), "{vs:?}");
    assert!(has(&vs, "/model_policy/provider", "required"), "{vs:?}");
    assert!(
        has(&vs, "/model_policy/extra", "additionalProperties"),
        "{vs:?}"
    );
    assert!(
        has(&vs, "/start_evidence/assistant_turns", "minimum"),
        "{vs:?}"
    );
}

#[test]
fn a_non_object_message_and_an_unknown_entity_are_refused() {
    let set = real();
    let vs = set.validate("execution", &json!([1]));
    assert!(has(&vs, "", "type"), "{vs:?}");
    let vs = set.validate("no-such-entity", &json!({}));
    assert!(has(&vs, "", "unknown-entity"), "{vs:?}");
}

#[test]
fn an_empty_object_fails_every_entity_on_its_required_fields() {
    let set = real();
    for entity in set.entities() {
        let vs = set.validate(entity, &json!({}));
        assert!(has(&vs, "/schema", "required"), "{entity}: {vs:?}");
        assert!(
            has(&vs, "/required_extensions", "required"),
            "{entity}: {vs:?}"
        );
    }
}

#[test]
fn extensions_accepts_any_object_and_rejects_a_non_object() {
    let set = real();
    let mut rec = execution();
    rec["extensions"] = json!({"anything": [1, {"k": null}]});
    assert_eq!(set.validate("execution", &rec), vec![]);
    rec["extensions"] = json!("nope");
    assert!(has(&set.validate("execution", &rec), "/extensions", "type"));
}

#[test]
fn map_values_are_validated_with_the_key_in_the_pointer() {
    let set = real();
    let schema: Value = serde_json::from_str(
        &std::fs::read_to_string(schemas_dir().join("autospec-protocol-worker.v1.schema.json"))
            .unwrap(),
    )
    .unwrap();
    assert!(schema["properties"]["toolchains"]["additionalProperties"].is_object());
    // Build a worker record from the schema's own required fields is brittle;
    // the empty-object test above covers `required`, so check the map directly.
    let rec = json!({"toolchains": {"rustc": 1, "node": "20"}});
    let vs = set.validate("worker", &rec);
    assert!(has(&vs, "/toolchains/rustc", "type"), "{vs:?}");
    assert!(
        !vs.iter().any(|v| v.pointer == "/toolchains/node"),
        "{vs:?}"
    );
}

#[test]
fn a_pattern_keyword_is_unsupported_and_names_file_and_keyword() {
    let dir = copy_of_schemas("pattern");
    edit(&dir, "autospec-protocol-task.v1.schema.json", |v| {
        v["properties"]["schema"]["pattern"] = json!("^a");
    });
    let err = load_dir(&dir).expect_err("pattern must not load");
    assert_eq!(err.code(), "UNSUPPORTED_SCHEMA_KEYWORD");
    let text = err.to_string();
    assert!(
        text.contains("pattern") && text.contains("autospec-protocol-task.v1.schema.json"),
        "{text}"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn unsupported_keywords_are_refused_wherever_they_appear() {
    for (tag, keyword, place) in [
        ("root", "oneOf", vec![]),
        ("def", "format", vec!["$defs", "sequence"]),
        ("prop", "maxLength", vec!["properties", "schema"]),
        (
            "items",
            "allOf",
            vec!["$defs", "required_extensions", "items"],
        ),
    ] {
        let dir = copy_of_schemas(tag);
        edit(&dir, "autospec-protocol-envelope.v1.schema.json", |v| {
            let mut node = &mut *v;
            for step in &place {
                node = &mut node[*step];
            }
            node[keyword] = json!([]);
        });
        let err = load_dir(&dir).expect_err(keyword);
        assert_eq!(err.code(), "UNSUPPORTED_SCHEMA_KEYWORD", "{keyword}");
        assert!(err.to_string().contains(keyword), "{err}");
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[test]
fn an_unresolvable_or_non_local_ref_fails_to_load() {
    for (tag, target) in [
        ("missing", "#/$defs/nope"),
        ("remote", "other.json#/$defs/x"),
        ("deep", "#/properties/schema"),
    ] {
        let dir = copy_of_schemas(tag);
        edit(&dir, "autospec-protocol-envelope.v1.schema.json", |v| {
            v["properties"]["schema"] = json!({"$ref": target});
        });
        assert!(load_dir(&dir).is_err(), "{target}");
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[test]
fn a_ref_cycle_fails_to_load_instead_of_recursing() {
    let dir = copy_of_schemas("cycle");
    edit(&dir, "autospec-protocol-envelope.v1.schema.json", |v| {
        v["$defs"]["schema"] = json!({"$ref": "#/$defs/mission_id"});
        v["$defs"]["mission_id"] = json!({"$ref": "#/$defs/schema"});
    });
    let err = load_dir(&dir).expect_err("cycle");
    assert_eq!(err.code(), "SCHEMA_LOAD_FAILED");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_node_without_a_supported_type_fails_to_load() {
    let dir = copy_of_schemas("notype");
    edit(&dir, "autospec-protocol-envelope.v1.schema.json", |v| {
        v["properties"]["schema"] = json!({"description": "no type"});
    });
    assert_eq!(
        load_dir(&dir).expect_err("no type").code(),
        "SCHEMA_LOAD_FAILED"
    );
    edit(&dir, "autospec-protocol-envelope.v1.schema.json", |v| {
        v["properties"]["schema"] = json!({"type": ["string", "null"]});
    });
    assert!(load_dir(&dir).is_err());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn malformed_json_and_a_missing_directory_are_errors() {
    let dir = copy_of_schemas("badjson");
    std::fs::write(dir.join("autospec-protocol-task.v1.schema.json"), "{").unwrap();
    assert!(matches!(
        load_dir(&dir),
        Err(ProtocolError::SchemaLoad { .. })
    ));
    let _ = std::fs::remove_dir_all(dir);
    assert!(load_dir(Path::new("/nonexistent/protocol-schemas")).is_err());
}
