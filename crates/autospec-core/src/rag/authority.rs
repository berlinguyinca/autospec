//! Source authority classification and precedence (spec section 9).
//!
//! Precedence is a *total order over classes*, not a similarity score. Two
//! pieces of evidence that disagree are ranked by where their source sits in
//! the project's authority ladder; when neither outranks the other the
//! disagreement is surfaced as a contradiction rather than resolved by picking
//! the higher embedding score.

/// Where a piece of evidence sits in the project's authority ladder.
///
/// Ordered lowest to highest so the derived [`Ord`] matches precedence: a
/// larger variant outranks a smaller one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SourceAuthority {
    /// Blogs, forums, answers outside the project.
    ExternalCommunity,
    /// Issue threads and design discussion.
    Discussion,
    /// Superseded or historical implementation.
    HistoricalImplementation,
    /// AutoSpec observational memory.
    ProjectMemory,
    /// Official documentation for a dependency or this project.
    OfficialDocumentation,
    /// Tests currently in the tree.
    CurrentTests,
    /// An architectural decision record still in force.
    CurrentAdr,
    /// Source code as it exists at the retrieved revision.
    Implementation,
    /// The accepted specification for the work in hand.
    AcceptedSpecification,
    /// An explicit instruction from the user.
    ExplicitUserRequirement,
}

/// Default precedence, highest first, as listed in specification section 9.
pub const DEFAULT_PRECEDENCE: [SourceAuthority; 10] = [
    SourceAuthority::ExplicitUserRequirement,
    SourceAuthority::AcceptedSpecification,
    SourceAuthority::Implementation,
    SourceAuthority::CurrentAdr,
    SourceAuthority::CurrentTests,
    SourceAuthority::OfficialDocumentation,
    SourceAuthority::ProjectMemory,
    SourceAuthority::HistoricalImplementation,
    SourceAuthority::Discussion,
    SourceAuthority::ExternalCommunity,
];

impl SourceAuthority {
    /// Stable wire identifier.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ExplicitUserRequirement => "explicit_user_requirement",
            Self::AcceptedSpecification => "accepted_specification",
            Self::Implementation => "implementation",
            Self::CurrentAdr => "current_adr",
            Self::CurrentTests => "current_tests",
            Self::OfficialDocumentation => "official_documentation",
            Self::ProjectMemory => "project_memory",
            Self::HistoricalImplementation => "historical_implementation",
            Self::Discussion => "discussion",
            Self::ExternalCommunity => "external_community",
        }
    }

    /// Parse a wire identifier.
    pub fn parse(text: &str) -> Result<Self, String> {
        DEFAULT_PRECEDENCE
            .iter()
            .copied()
            .find(|authority| authority.as_str() == text)
            .ok_or_else(|| format!("unknown source authority: {text}"))
    }

    /// Return every authority class, highest precedence first.
    pub const fn precedence() -> [Self; 10] {
        DEFAULT_PRECEDENCE
    }

    /// Return `true` when this class outranks `other` under the default ladder.
    pub fn outranks(self, other: Self) -> bool {
        self > other
    }

    /// Return `true` when retrieved content from this class may carry
    /// instructions that bind the agent.
    ///
    /// Only an explicit user requirement does. Everything else — including the
    /// accepted specification, which reaches the agent through AutoSpec's own
    /// instruction channel rather than through retrieval — is data (spec
    /// section 29). A repository file that outranks a blog post on *facts* has
    /// no more right to issue orders than the blog post does.
    pub const fn may_carry_instructions(self) -> bool {
        matches!(self, Self::ExplicitUserRequirement)
    }
}

/// A project-level override of the default authority ladder (spec section 9:
/// "This ordering MAY be overridden per project").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorityLadder {
    ordered: Vec<SourceAuthority>,
}

impl Default for AuthorityLadder {
    fn default() -> Self {
        Self {
            ordered: DEFAULT_PRECEDENCE.to_vec(),
        }
    }
}

impl AuthorityLadder {
    /// Build a ladder from a highest-first ordering.
    ///
    /// Every class must appear exactly once: a partial ladder would leave some
    /// pairs unordered, and an unordered pair silently becomes "no
    /// contradiction" at comparison time.
    pub fn new(ordered: Vec<SourceAuthority>) -> Result<Self, String> {
        if ordered.len() != DEFAULT_PRECEDENCE.len() {
            return Err(format!(
                "authority ladder must list all {} classes, found {}",
                DEFAULT_PRECEDENCE.len(),
                ordered.len()
            ));
        }
        for authority in DEFAULT_PRECEDENCE {
            let seen = ordered
                .iter()
                .filter(|candidate| **candidate == authority)
                .count();
            if seen != 1 {
                return Err(format!(
                    "authority ladder lists {} {} time(s), expected exactly one",
                    authority.as_str(),
                    seen
                ));
            }
        }
        Ok(Self { ordered })
    }

    /// Rank of an authority class; `0` is the highest.
    pub fn rank(&self, authority: SourceAuthority) -> usize {
        self.ordered
            .iter()
            .position(|candidate| *candidate == authority)
            .unwrap_or(self.ordered.len())
    }

    /// Return `true` when `left` outranks `right` under this ladder.
    pub fn outranks(&self, left: SourceAuthority, right: SourceAuthority) -> bool {
        self.rank(left) < self.rank(right)
    }

    /// Return the ordering, highest precedence first.
    pub fn ordered(&self) -> &[SourceAuthority] {
        &self.ordered
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_requirement_outranks_specification_which_outranks_code() {
        assert!(SourceAuthority::ExplicitUserRequirement
            .outranks(SourceAuthority::AcceptedSpecification));
        assert!(SourceAuthority::AcceptedSpecification.outranks(SourceAuthority::Implementation));
        assert!(SourceAuthority::Implementation.outranks(SourceAuthority::CurrentAdr));
    }

    #[test]
    fn default_precedence_is_strictly_descending() {
        for window in DEFAULT_PRECEDENCE.windows(2) {
            assert!(window[0].outranks(window[1]), "{window:?} not descending");
        }
    }

    #[test]
    fn only_explicit_user_requirements_may_carry_instructions() {
        for authority in DEFAULT_PRECEDENCE {
            assert_eq!(
                authority.may_carry_instructions(),
                authority == SourceAuthority::ExplicitUserRequirement,
                "{authority:?}"
            );
        }
    }

    #[test]
    fn parse_round_trips_every_authority() {
        for authority in DEFAULT_PRECEDENCE {
            assert_eq!(SourceAuthority::parse(authority.as_str()).unwrap(), authority);
        }
    }

    #[test]
    fn ladder_override_reorders_precedence() {
        let mut ordered = DEFAULT_PRECEDENCE.to_vec();
        ordered.swap(1, 2);
        let ladder = AuthorityLadder::new(ordered).unwrap();

        assert!(ladder.outranks(
            SourceAuthority::Implementation,
            SourceAuthority::AcceptedSpecification
        ));
    }

    #[test]
    fn ladder_rejects_incomplete_ordering() {
        let ordered = DEFAULT_PRECEDENCE[..4].to_vec();
        assert!(AuthorityLadder::new(ordered).is_err());
    }

    #[test]
    fn ladder_rejects_duplicate_class() {
        let mut ordered = DEFAULT_PRECEDENCE.to_vec();
        ordered[3] = SourceAuthority::Implementation;
        assert!(AuthorityLadder::new(ordered).is_err());
    }
}
