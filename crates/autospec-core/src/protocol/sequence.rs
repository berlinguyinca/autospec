//! Per-mission monotonic sequence and idempotent redelivery.

use std::collections::BTreeMap;

use super::negotiate::Envelope;
use super::ProtocolError;

/// What an accepted message means for the consumer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Accept {
    /// First delivery: apply the effect once.
    Applied,
    /// Redelivery of an accepted message: acknowledge, apply nothing.
    Duplicate,
}

/// Tracks, per mission, the high watermark and the accepted keys.
#[derive(Debug, Default)]
pub struct MissionSequencer {
    missions: BTreeMap<String, MissionState>,
}

#[derive(Debug, Default)]
struct MissionState {
    watermark: u64,
    accepted: BTreeMap<String, (u64, String)>,
}

impl MissionSequencer {
    // linter:allow-UNWIRED_PUB_ITEM deliberate staging: the Rust protocol library lands first; its callers arrive in later T08 issues
    pub fn new() -> Self {
        Self::default()
    }

    /// Judge one incoming message against the contract's sequence table
    /// (`docs/contracts/autospec-protocol-v1.md`, Sequence and idempotency).
    /// The first matching rule wins, and a refusal changes no state.
    ///
    /// `payload_digest` identifies the message body: the same key redelivered
    /// with a different digest is refused, never silently treated as a duplicate.
    // linter:allow-UNWIRED_PUB_ITEM deliberate staging: the Rust protocol library lands first; its callers arrive in later T08 issues
    pub fn accept(
        &mut self,
        env: &Envelope,
        payload_digest: &str,
    ) -> Result<Accept, ProtocolError> {
        let state = self.missions.entry(env.mission_id.clone()).or_default();
        if let Some((seen_at, digest)) = state.accepted.get(&env.idempotency_key) {
            if *seen_at != env.sequence {
                return Err(ProtocolError::IdempotencyKeyReused {
                    key: env.idempotency_key.clone(),
                });
            }
            if digest != payload_digest {
                return Err(ProtocolError::IdempotencyConflict {
                    key: env.idempotency_key.clone(),
                });
            }
            return Ok(Accept::Duplicate);
        }
        let expected = state.watermark + 1;
        match env.sequence.cmp(&expected) {
            std::cmp::Ordering::Equal => {
                state.accepted.insert(
                    env.idempotency_key.clone(),
                    (env.sequence, payload_digest.to_string()),
                );
                state.watermark = env.sequence;
                Ok(Accept::Applied)
            }
            std::cmp::Ordering::Greater => Err(ProtocolError::SequenceGap {
                expected,
                got: env.sequence,
            }),
            std::cmp::Ordering::Less => Err(ProtocolError::SequenceConflict {
                expected,
                got: env.sequence,
            }),
        }
    }

    /// Restart a mission after a checkpoint: the watermark becomes `sequence`,
    /// so `sequence + 1` is next. Keys recorded past the checkpoint are
    /// forgotten because replay will deliver those positions again.
    // linter:allow-UNWIRED_PUB_ITEM deliberate staging: the Rust protocol library lands first; its callers arrive in later T08 issues
    pub fn resume_from(&mut self, mission_id: &str, sequence: u64) {
        let state = self.missions.entry(mission_id.to_string()).or_default();
        state.watermark = sequence;
        state.accepted.retain(|_, (at, _)| *at <= sequence);
    }

    /// Highest accepted sequence of a mission; 0 when none was accepted.
    // linter:allow-UNWIRED_PUB_ITEM deliberate staging: the Rust protocol library lands first; its callers arrive in later T08 issues
    pub fn high_watermark(&self, mission_id: &str) -> u64 {
        self.missions.get(mission_id).map_or(0, |m| m.watermark)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(mission: &str, sequence: u64, key: &str) -> Envelope {
        Envelope {
            schema: "autospec.protocol.event.v1".to_string(),
            protocol_version: 1,
            idempotency_key: key.to_string(),
            mission_id: mission.to_string(),
            sequence,
        }
    }

    fn code(r: Result<Accept, ProtocolError>) -> &'static str {
        r.expect_err("expected a refusal").code()
    }

    #[test]
    fn consecutive_sequences_are_applied() {
        let mut s = MissionSequencer::new();
        for n in 1..=3 {
            assert_eq!(
                s.accept(&env("m1", n, &format!("k{n}")), "d"),
                Ok(Accept::Applied)
            );
        }
        assert_eq!(s.high_watermark("m1"), 3);
    }

    #[test]
    fn a_gap_is_refused_with_the_expected_sequence_and_changes_nothing() {
        let mut s = MissionSequencer::new();
        s.accept(&env("m1", 1, "k1"), "d").unwrap();
        assert_eq!(
            s.accept(&env("m1", 3, "k3"), "d"),
            Err(ProtocolError::SequenceGap {
                expected: 2,
                got: 3
            })
        );
        assert_eq!(s.high_watermark("m1"), 1);
        assert_eq!(s.accept(&env("m1", 2, "k2"), "d"), Ok(Accept::Applied));
    }

    #[test]
    fn redelivery_with_same_key_and_digest_is_a_duplicate() {
        let mut s = MissionSequencer::new();
        s.accept(&env("m1", 1, "k1"), "d1").unwrap();
        s.accept(&env("m1", 2, "k2"), "d2").unwrap();
        assert_eq!(s.accept(&env("m1", 2, "k2"), "d2"), Ok(Accept::Duplicate));
        assert_eq!(s.high_watermark("m1"), 2);
    }

    #[test]
    fn same_key_with_a_different_digest_is_a_conflict() {
        let mut s = MissionSequencer::new();
        s.accept(&env("m1", 1, "k1"), "d1").unwrap();
        assert_eq!(
            code(s.accept(&env("m1", 1, "k1"), "other")),
            "IDEMPOTENCY_CONFLICT"
        );
    }

    #[test]
    fn a_key_reused_at_another_sequence_is_refused_even_when_it_is_next() {
        let mut s = MissionSequencer::new();
        s.accept(&env("m1", 1, "k1"), "d").unwrap();
        assert_eq!(
            code(s.accept(&env("m1", 2, "k1"), "d")),
            "IDEMPOTENCY_KEY_REUSED"
        );
        assert_eq!(
            code(s.accept(&env("m1", 7, "k1"), "d")),
            "IDEMPOTENCY_KEY_REUSED"
        );
        assert_eq!(s.high_watermark("m1"), 1);
    }

    #[test]
    fn a_taken_position_under_a_new_key_is_a_sequence_conflict() {
        let mut s = MissionSequencer::new();
        s.accept(&env("m1", 1, "k1"), "d").unwrap();
        s.accept(&env("m1", 2, "k2"), "d").unwrap();
        assert_eq!(
            s.accept(&env("m1", 2, "other"), "d"),
            Err(ProtocolError::SequenceConflict {
                expected: 3,
                got: 2
            })
        );
    }

    #[test]
    fn missions_do_not_share_sequences_or_keys() {
        let mut s = MissionSequencer::new();
        assert_eq!(s.accept(&env("m1", 1, "k"), "d"), Ok(Accept::Applied));
        assert_eq!(s.accept(&env("m2", 1, "k"), "d"), Ok(Accept::Applied));
        assert_eq!(s.high_watermark("m2"), 1);
        assert_eq!(s.high_watermark("unseen"), 0);
    }

    #[test]
    fn sequence_zero_never_applies() {
        let mut s = MissionSequencer::new();
        assert_eq!(
            code(s.accept(&env("m1", 0, "k0"), "d")),
            "SEQUENCE_CONFLICT"
        );
        assert_eq!(s.high_watermark("m1"), 0);
    }

    #[test]
    fn resume_from_restarts_replay_after_the_checkpoint() {
        let mut s = MissionSequencer::new();
        s.resume_from("m1", 5);
        assert_eq!(s.high_watermark("m1"), 5);
        assert_eq!(s.accept(&env("m1", 6, "k6"), "d"), Ok(Accept::Applied));
        assert_eq!(
            code(s.accept(&env("m1", 5, "k5"), "d")),
            "SEQUENCE_CONFLICT"
        );
        assert_eq!(code(s.accept(&env("m1", 8, "k8"), "d")), "SEQUENCE_GAP");
    }

    #[test]
    fn resuming_backwards_forgets_messages_past_the_checkpoint() {
        let mut s = MissionSequencer::new();
        for n in 1..=4 {
            s.accept(&env("m1", n, &format!("k{n}")), "d").unwrap();
        }
        s.resume_from("m1", 2);
        assert_eq!(s.high_watermark("m1"), 2);
        // replay after the checkpoint applies again, under the recorded keys
        assert_eq!(s.accept(&env("m1", 3, "k3"), "d"), Ok(Accept::Applied));
        // a key recorded at or before the checkpoint stays known
        assert_eq!(s.accept(&env("m1", 1, "k1"), "d"), Ok(Accept::Duplicate));
    }
}
