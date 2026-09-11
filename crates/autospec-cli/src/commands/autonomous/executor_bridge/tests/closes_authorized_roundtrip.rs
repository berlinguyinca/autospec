//! `closes_authorized` must survive persistence, and legacy records must not
//! silently acquire permission to close.
//!
//! The field decides whether a finished attempt may write `Closes #N` or must
//! write `Refs #N`. It is computed once, while the acceptance criteria and the
//! closeout body are both in hand, and is then read back from disk by a later
//! stage that has neither. So a round trip that dropped it would restore the
//! permissive default and close an issue whose criteria were never met -- the
//! exact failure this field exists to prevent, reappearing through the
//! serialization layer rather than the decision.

use super::super::{invocation_from_value, invocation_to_value};
use super::support_invocation::persisted_invocation;

#[test]
fn a_withheld_close_authorization_survives_a_round_trip() {
    let mut invocation = persisted_invocation();
    invocation.closes_authorized = false;

    let restored = invocation_from_value(invocation_to_value(&invocation))
        .expect("a persisted invocation must reload");

    assert!(
        !restored.closes_authorized,
        "withheld close authorization must not be restored as permission to close"
    );
}

#[test]
fn a_granted_close_authorization_survives_a_round_trip() {
    let mut invocation = persisted_invocation();
    invocation.closes_authorized = true;

    let restored = invocation_from_value(invocation_to_value(&invocation))
        .expect("a persisted invocation must reload");

    assert!(restored.closes_authorized, "granted authorization must persist");
}

#[test]
fn a_legacy_record_without_the_field_reloads_as_authorized() {
    // Records written before the field existed carry no opinion. They backfill
    // to `true`, which preserves the behaviour those attempts ran under; the
    // guard applies to attempts evaluated after it landed. Pinned here so the
    // migration default is a decision on the record rather than an accident.
    let mut value = invocation_to_value(&persisted_invocation());
    value
        .as_object_mut()
        .expect("invocation serializes to an object")
        .remove("closes_authorized")
        .expect("the field is present before removal");

    let restored = invocation_from_value(value).expect("a legacy record must reload");

    assert!(
        restored.closes_authorized,
        "a legacy record must reload under the behaviour it ran under"
    );
}
