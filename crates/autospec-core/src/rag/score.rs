//! Integer scores for Agentic RAG evidence (spec section 10).
//!
//! The specification writes relevance, confidence, and freshness as decimals
//! (`0.94`). Binary floating point is banned in this workspace's Rust crates
//! (`financial_no_f64` architecture fitness function), and a retrieval loop that
//! branches on a threshold needs a *reproducible* comparison anyway: the same
//! evidence must produce the same stop decision on every host. Scores are
//! therefore held in permille — integers `0..=1000` — and rendered back as
//! three-decimal strings at the serialization boundary.

/// A `0.000`..`1.000` score held as an integer count of thousandths.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Score(u16);

/// The largest representable score, `1.000`.
pub const SCORE_MAX: u16 = 1000;

impl Score {
    /// Lowest score (`0.000`).
    pub const ZERO: Self = Self(0);
    /// Highest score (`1.000`).
    pub const ONE: Self = Self(SCORE_MAX);

    /// Build a score from permille, clamping to `0..=1000`.
    ///
    /// Clamping rather than rejecting keeps an out-of-range evaluator answer
    /// from aborting a retrieval that has already spent its budget; the caller
    /// sees a saturated score instead of an error it cannot act on.
    pub const fn from_permille(permille: u16) -> Self {
        if permille > SCORE_MAX {
            Self(SCORE_MAX)
        } else {
            Self(permille)
        }
    }

    /// Build a score from a percentage, clamping to `0..=100`.
    pub const fn from_percent(percent: u8) -> Self {
        let percent = if percent > 100 { 100 } else { percent };
        Self(percent as u16 * 10)
    }

    /// Parse a decimal score such as `0.94`, `1`, or `.5`.
    ///
    /// Accepts at most three fractional digits; more precision than permille is
    /// a caller error rather than something to silently round, because rounding
    /// would make two distinct inputs compare equal against a threshold.
    pub fn parse(text: &str) -> Result<Self, String> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Err("empty score".to_string());
        }
        let (whole, fraction) = match trimmed.split_once('.') {
            Some((whole, fraction)) => (whole, fraction),
            None => (trimmed, ""),
        };
        let whole = if whole.is_empty() { "0" } else { whole };
        if !whole.bytes().all(|byte| byte.is_ascii_digit())
            || !fraction.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(format!("score is not a decimal number: {trimmed}"));
        }
        if fraction.len() > 3 {
            return Err(format!(
                "score has more precision than permille supports: {trimmed}"
            ));
        }
        let whole: u32 = whole
            .parse()
            .map_err(|_| format!("score whole part out of range: {trimmed}"))?;
        let mut padded = fraction.to_string();
        while padded.len() < 3 {
            padded.push('0');
        }
        let fraction: u32 = padded.parse().unwrap_or(0);
        let permille = whole
            .checked_mul(1000)
            .and_then(|scaled| scaled.checked_add(fraction))
            .ok_or_else(|| format!("score out of range: {trimmed}"))?;
        if permille > u32::from(SCORE_MAX) {
            return Err(format!("score above 1.000: {trimmed}"));
        }
        Ok(Self(permille as u16))
    }

    /// Return the underlying permille value.
    pub const fn permille(self) -> u16 {
        self.0
    }

    /// Render as a three-decimal string (`0.940`).
    pub fn to_decimal_string(self) -> String {
        format!("{}.{:03}", self.0 / 1000, self.0 % 1000)
    }

    /// Multiply two scores, rounding half up.
    ///
    /// Used to combine independent signals (relevance x authority weight)
    /// without leaving permille space.
    pub fn multiply(self, other: Self) -> Self {
        let product = u32::from(self.0) * u32::from(other.0);
        Self(((product + 500) / 1000) as u16)
    }

    /// Return the mean of a set of scores, or [`Score::ZERO`] when empty.
    pub fn mean(scores: &[Self]) -> Self {
        if scores.is_empty() {
            return Self::ZERO;
        }
        let total: u32 = scores.iter().map(|score| u32::from(score.0)).sum();
        let count = scores.len() as u32;
        Self(((total + count / 2) / count) as u16)
    }

    /// Return `true` when this score is at or above `threshold`.
    pub const fn at_least(self, threshold: Self) -> bool {
        self.0 >= threshold.0
    }
}

impl Default for Score {
    fn default() -> Self {
        Self::ZERO
    }
}

impl std::fmt::Display for Score {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.to_decimal_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_permille_clamps_above_one() {
        assert_eq!(Score::from_permille(4000), Score::ONE);
    }

    #[test]
    fn parse_accepts_specification_style_decimals() {
        assert_eq!(Score::parse("0.94").unwrap().permille(), 940);
        assert_eq!(Score::parse("1").unwrap(), Score::ONE);
        assert_eq!(Score::parse(".5").unwrap().permille(), 500);
    }

    #[test]
    fn parse_rejects_more_precision_than_permille() {
        assert!(Score::parse("0.9412").is_err());
    }

    #[test]
    fn parse_rejects_scores_above_one() {
        assert!(Score::parse("1.001").is_err());
    }

    #[test]
    fn decimal_string_round_trips_through_parse() {
        let score = Score::from_permille(94);
        assert_eq!(score.to_decimal_string(), "0.094");
        assert_eq!(Score::parse("0.094").unwrap(), score);
    }

    #[test]
    fn multiply_rounds_half_up() {
        assert_eq!(
            Score::from_permille(500)
                .multiply(Score::from_permille(3))
                .permille(),
            2
        );
    }

    #[test]
    fn mean_of_empty_is_zero() {
        assert_eq!(Score::mean(&[]), Score::ZERO);
    }
}
