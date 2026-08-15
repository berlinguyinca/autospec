use super::*;

#[test]
fn recovery_event_kinds_round_trip_with_strict_generation_fields() {
    let cases = [
        (
            EventKind::HeartbeatPublicationDeferred {
                issue: 42,
                claim_id: "claim-generation-1".to_owned(),
            },
            json!({
                "type": "heartbeat_publication_deferred",
                "issue": 42,
                "claim_id": "claim-generation-1",
            }),
        ),
        (
            EventKind::StartupClaimRecovered {
                issue: 42,
                previous_claim_id: "claim-generation-1".to_owned(),
                next_claim_id: "claim-generation-2".to_owned(),
            },
            json!({
                "type": "startup_claim_recovered",
                "issue": 42,
                "previous_claim_id": "claim-generation-1",
                "next_claim_id": "claim-generation-2",
            }),
        ),
    ];

    for (kind, expected) in cases {
        assert_eq!(kind.to_value(), expected);
        assert_eq!(EventKind::from_value(&expected).unwrap(), kind);
    }
}
