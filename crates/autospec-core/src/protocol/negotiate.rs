//! Version negotiation and the fail-closed envelope check.

use serde_json::{Map, Value};

use super::{ProtocolError, PROTOCOL_VERSIONS};

/// The envelope fields every message carries (the read subset).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Envelope {
    pub schema: String,
    pub protocol_version: u32,
    pub idempotency_key: String,
    pub mission_id: String,
    pub sequence: u64,
}

/// Highest protocol version both sides list.
///
/// A version of 0 is not a protocol major version, so a list holding one is
/// malformed and the exchange fails; there is no fallback to a lower or "best
/// effort" version.
// linter:allow-UNWIRED_PUB_ITEM deliberate staging: the Rust protocol library lands first; its callers arrive in later T08 issues
pub fn negotiate(local: &[u32], remote: &[u32]) -> Result<u32, ProtocolError> {
    let unsupported = |detail: String| ProtocolError::VersionUnsupported { detail };
    if local.is_empty() || remote.is_empty() {
        return Err(unsupported(
            "a supported_versions list is empty".to_string(),
        ));
    }
    if local.contains(&0) || remote.contains(&0) {
        return Err(unsupported("a supported_versions list holds 0".to_string()));
    }
    local
        .iter()
        .filter(|v| remote.contains(v))
        .max()
        .copied()
        .ok_or_else(|| unsupported(format!("no common version in {local:?} and {remote:?}")))
}

fn invalid(field: &'static str, reason: &str) -> ProtocolError {
    ProtocolError::InvalidField {
        field,
        reason: reason.to_string(),
    }
}

fn get<'a>(obj: &'a Map<String, Value>, field: &'static str) -> Result<&'a Value, ProtocolError> {
    obj.get(field).ok_or(ProtocolError::MissingField { field })
}

fn non_empty_string(
    obj: &Map<String, Value>,
    field: &'static str,
) -> Result<String, ProtocolError> {
    match get(obj, field)? {
        Value::String(s) if !s.is_empty() => Ok(s.clone()),
        Value::String(_) => Err(invalid(field, "must not be empty")),
        _ => Err(invalid(field, "must be a string")),
    }
}

/// `autospec.protocol.<entity>.v<N>` with N >= 1. The schema subset has no
/// string-shape keyword, so the form is checked here.
fn schema_form_ok(schema: &str) -> bool {
    let Some(rest) = schema.strip_prefix("autospec.protocol.") else {
        return false;
    };
    let Some((entity, version)) = rest.rsplit_once(".v") else {
        return false;
    };
    !entity.is_empty()
        && entity
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        && !version.is_empty()
        && version.chars().all(|c| c.is_ascii_digit())
        && version.parse::<u64>().is_ok_and(|n| n >= 1)
}

/// Validate the envelope of one message, failing closed.
///
/// `understood` lists the `extensions` keys this reader implements. A name in
/// `required_extensions` outside that list refuses the whole message; `extensions`
/// content is otherwise ignored, so an optional unknown key never fails.
// linter:allow-UNWIRED_PUB_ITEM deliberate staging: the Rust protocol library lands first; its callers arrive in later T08 issues
pub fn check_envelope(v: &Value, understood: &[&str]) -> Result<Envelope, ProtocolError> {
    let obj = v
        .as_object()
        .ok_or_else(|| invalid("$", "message must be a JSON object"))?;

    let schema = non_empty_string(obj, "schema")?;
    if !schema_form_ok(&schema) {
        return Err(invalid("schema", "must be autospec.protocol.<entity>.v<N>"));
    }

    let protocol_version = match get(obj, "protocol_version")?.as_u64() {
        Some(n) if n >= 1 && n <= u64::from(u32::MAX) => n as u32,
        _ => return Err(invalid("protocol_version", "must be an integer >= 1")),
    };
    let idempotency_key = non_empty_string(obj, "idempotency_key")?;
    let mission_id = non_empty_string(obj, "mission_id")?;
    let sequence = match get(obj, "sequence")?.as_u64() {
        Some(n) if n >= 1 => n,
        _ => return Err(invalid("sequence", "must be an integer >= 1")),
    };

    let required = get(obj, "required_extensions")?
        .as_array()
        .ok_or_else(|| invalid("required_extensions", "must be an array"))?;
    let extensions = match obj.get("extensions") {
        None => None,
        Some(Value::Object(m)) => Some(m),
        Some(_) => return Err(invalid("extensions", "must be an object")),
    };
    for entry in required {
        let name = entry
            .as_str()
            .ok_or_else(|| invalid("required_extensions", "entries must be strings"))?;
        if !understood.contains(&name) {
            return Err(ProtocolError::UnknownRequiredField {
                name: name.to_string(),
            });
        }
        if !extensions.is_some_and(|m| m.contains_key(name)) {
            return Err(invalid(
                "required_extensions",
                "names a key that is absent from extensions",
            ));
        }
    }

    // The sender's version must be one this build reads. The envelope check has
    // no per-connection negotiated value, so the build's own list is the floor.
    if !PROTOCOL_VERSIONS.contains(&protocol_version) {
        return Err(ProtocolError::VersionUnsupported {
            detail: format!("protocol_version {protocol_version} is not in {PROTOCOL_VERSIONS:?}"),
        });
    }

    Ok(Envelope {
        schema,
        protocol_version,
        idempotency_key,
        mission_id,
        sequence,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn base() -> Value {
        json!({
            "schema": "autospec.protocol.event.v1",
            "protocol_version": 1,
            "idempotency_key": "idem-1",
            "mission_id": "m-1",
            "sequence": 1,
            "required_extensions": []
        })
    }

    fn code(r: Result<Envelope, ProtocolError>) -> &'static str {
        r.expect_err("expected a refusal").code()
    }

    #[test]
    fn negotiate_picks_the_highest_common_version() {
        assert_eq!(negotiate(&[1, 2], &[2, 3]), Ok(2));
        assert_eq!(negotiate(&[3, 1, 2], &[1, 2, 3]), Ok(3));
    }

    #[test]
    fn negotiate_with_no_common_version_is_unsupported() {
        let e = negotiate(&[1], &[2]).unwrap_err();
        assert_eq!(e.code(), "PROTOCOL_VERSION_UNSUPPORTED");
    }

    #[test]
    fn negotiate_never_falls_back_when_a_side_is_empty_or_malformed() {
        assert_eq!(
            negotiate(&[], &[1]).unwrap_err().code(),
            "PROTOCOL_VERSION_UNSUPPORTED"
        );
        assert_eq!(
            negotiate(&[1], &[]).unwrap_err().code(),
            "PROTOCOL_VERSION_UNSUPPORTED"
        );
        // 0 is not a valid protocol major version; it must not satisfy a match.
        assert_eq!(
            negotiate(&[0, 1], &[0]).unwrap_err().code(),
            "PROTOCOL_VERSION_UNSUPPORTED"
        );
    }

    #[test]
    fn a_complete_envelope_is_read() {
        let env = check_envelope(&base(), &[]).unwrap();
        assert_eq!(env.schema, "autospec.protocol.event.v1");
        assert_eq!(env.protocol_version, 1);
        assert_eq!(env.idempotency_key, "idem-1");
        assert_eq!(env.mission_id, "m-1");
        assert_eq!(env.sequence, 1);
    }

    #[test]
    fn an_unknown_required_extension_fails_closed() {
        let mut v = base();
        v["required_extensions"] = json!(["x_new"]);
        v["extensions"] = json!({"x_new": {"a": 1}});
        let e = check_envelope(&v, &[]).unwrap_err();
        assert_eq!(e.code(), "UNKNOWN_REQUIRED_FIELD");
        assert!(e.to_string().contains("x_new"));
    }

    #[test]
    fn an_understood_required_extension_is_accepted() {
        let mut v = base();
        v["required_extensions"] = json!(["x_new"]);
        v["extensions"] = json!({"x_new": true});
        assert!(check_envelope(&v, &["x_new"]).is_ok());
    }

    #[test]
    fn a_required_extension_without_its_extensions_key_is_malformed() {
        let mut v = base();
        v["required_extensions"] = json!(["x_new"]);
        assert_eq!(code(check_envelope(&v, &["x_new"])), "INVALID_FIELD");
    }

    #[test]
    fn an_optional_unknown_extension_is_ignored() {
        let mut v = base();
        v["extensions"] = json!({"x_new": {"anything": [1, 2, 3]}});
        assert!(check_envelope(&v, &[]).is_ok());
    }

    #[test]
    fn sequence_below_one_is_invalid() {
        for bad in [json!(0), json!(-1), json!(1.5), json!("1"), json!(null)] {
            let mut v = base();
            v["sequence"] = bad.clone();
            assert_eq!(
                code(check_envelope(&v, &[])),
                "INVALID_FIELD",
                "sequence {bad}"
            );
        }
    }

    #[test]
    fn every_envelope_field_is_required_including_the_extension_list() {
        for field in [
            "schema",
            "protocol_version",
            "idempotency_key",
            "mission_id",
            "sequence",
            "required_extensions",
        ] {
            let mut v = base();
            v.as_object_mut().unwrap().remove(field);
            let e = check_envelope(&v, &[]).unwrap_err();
            assert_eq!(e.code(), "MISSING_FIELD", "removing {field}");
            assert!(e.to_string().contains(field));
        }
    }

    #[test]
    fn wrongly_typed_fields_are_invalid() {
        for (field, bad) in [
            ("schema", json!(7)),
            ("schema", json!("event")),
            ("schema", json!("autospec.protocol.event.v0")),
            ("protocol_version", json!("1")),
            ("protocol_version", json!(0)),
            ("idempotency_key", json!(1)),
            ("idempotency_key", json!("")),
            ("mission_id", json!(null)),
            ("mission_id", json!("")),
            ("required_extensions", json!("x")),
            ("required_extensions", json!([1])),
        ] {
            let mut v = base();
            v[field] = bad.clone();
            assert_eq!(
                code(check_envelope(&v, &[])),
                "INVALID_FIELD",
                "{field}={bad}"
            );
        }
    }

    #[test]
    fn a_non_object_message_is_invalid() {
        assert_eq!(code(check_envelope(&json!([1]), &[])), "INVALID_FIELD");
    }

    #[test]
    fn a_protocol_version_this_build_lacks_is_unsupported() {
        let mut v = base();
        v["protocol_version"] = json!(2);
        assert_eq!(
            code(check_envelope(&v, &[])),
            "PROTOCOL_VERSION_UNSUPPORTED"
        );
    }
}
