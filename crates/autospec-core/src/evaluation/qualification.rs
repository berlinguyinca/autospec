//! Verdicts — the seed of the paired-qualification module.
//!
//! Paired qualification (incumbent vs challenger over a protected anchor
//! suite), `ChallengerTrial`, the `VerdictSource` trait and `RecordedVerdicts`
//! land with their own task and need the anchor and evaluator types. This seed
//! carries only [`Verdict`], the value that those types,
//! [`crate::evaluation::record::EvaluationRecord::outcome`] and the CLI all
//! speak.

use serde::{Deserialize, Serialize};

/// What one evaluator concluded about one subject.
///
/// `Unavailable` is a first-class outcome, not an error: a judge that timed
/// out, refused, or returned something unparseable is recorded so that it is
/// tallied separately instead of silently counting as a match or a mismatch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Accept,
    Reject,
    Unavailable,
}

impl Verdict {
    /// The serialized form, which is also the label vocabulary used by anchor
    /// fixtures (`"accept"`, `"reject"`, `"unavailable"`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Accept => "accept",
            Self::Reject => "reject",
            Self::Unavailable => "unavailable",
        }
    }

    /// Did this verdict agree with `label`, where `label` is `"accept"` or
    /// `"reject"`?
    ///
    /// `None` means *no judgment was made* — either the verdict is
    /// [`Verdict::Unavailable`] or `label` is not a label word at all. `None`
    /// is deliberately not `false`: an absent judgment is counted in
    /// `EvaluatorTally::unavailable`, while a mismatch counts as an error, and
    /// conflating the two would let a broken judge look like a wrong one.
    ///
    /// The typed home of the expected side is `ProtectedLabel` in
    /// `evaluation/anchor.rs`; `matches(label.as_str())` is the call site once
    /// that module lands.
    pub fn matches(self, label: &str) -> Option<bool> {
        match (self, label) {
            (Self::Unavailable, _) => None,
            (_, "accept") => Some(self == Self::Accept),
            (_, "reject") => Some(self == Self::Reject),
            _ => None,
        }
    }
}

impl std::fmt::Display for Verdict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for Verdict {
    type Err = crate::evaluation::error::EvaluationError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "accept" => Ok(Self::Accept),
            "reject" => Ok(Self::Reject),
            "unavailable" => Ok(Self::Unavailable),
            other => Err(crate::evaluation::error::EvaluationError::parse(format!(
                "unknown verdict {other:?} (expected accept|reject|unavailable)"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verdict_agrees_with_its_own_label() {
        assert_eq!(Verdict::Accept.matches("accept"), Some(true));
        assert_eq!(Verdict::Accept.matches("reject"), Some(false));
        assert_eq!(Verdict::Reject.matches("reject"), Some(true));
        assert_eq!(Verdict::Reject.matches("accept"), Some(false));
    }

    #[test]
    fn unavailable_judgment_never_matches() {
        assert_eq!(Verdict::Unavailable.matches("accept"), None);
        assert_eq!(Verdict::Unavailable.matches("reject"), None);
        assert_eq!(Verdict::Accept.matches("maybe"), None, "not a label word");
    }

    #[test]
    fn verdict_round_trips_through_text_and_json() {
        for verdict in [Verdict::Accept, Verdict::Reject, Verdict::Unavailable] {
            assert_eq!(verdict.to_string(), verdict.as_str());
            assert_eq!(verdict.as_str().parse::<Verdict>().unwrap(), verdict);
            let json = serde_json::to_string(&verdict).unwrap();
            assert_eq!(
                serde_json::from_str::<Verdict>(&json).unwrap(),
                verdict,
                "{json} should round-trip"
            );
        }
        assert_eq!(
            serde_json::to_string(&Verdict::Accept).unwrap(),
            "\"accept\""
        );
        assert!(Verdict::Accept.matches("accept").unwrap());
        assert!("unknown".parse::<Verdict>().is_err());
    }
}
