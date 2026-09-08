//! Integer fixed-point helpers for the qualification statistic.
//!
//! This module holds [`Ppm`] today; the epsilon-best-belief estimator that
//! consumes it lands with the statistics task. Everything here is integer
//! arithmetic — the persisted documents in this subsystem carry no binary
//! floating point, so a stored probability round-trips through JSON exactly.

use serde::{Deserialize, Serialize};

/// Parts per million, `0..=1_000_000`.
///
/// A probability without rounding error: the comparison path never touches a
/// float, and the serialized form is an integer a reviewer can read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Ppm(pub u32);

impl Ppm {
    pub const ONE: Ppm = Ppm(1_000_000);

    /// `numerator / denominator` in ppm, rounded to nearest, saturating at
    /// [`Ppm::ONE`]. `None` for a zero denominator rather than a panic.
    pub fn from_ratio(numerator: u32, denominator: u32) -> Option<Ppm> {
        if denominator == 0 {
            return None;
        }
        let scaled = numerator as u64 * 1_000_000 + denominator as u64 / 2;
        Some(Ppm((scaled / denominator as u64).min(1_000_000) as u32))
    }

    pub fn as_u32(self) -> u32 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ppm_from_ratio_rounds_to_nearest_and_saturates() {
        assert_eq!(Ppm::from_ratio(1, 3), Some(Ppm(333_333)));
        assert_eq!(Ppm::from_ratio(2, 3), Some(Ppm(666_667)));
        assert_eq!(Ppm::from_ratio(5, 5), Some(Ppm::ONE));
        assert_eq!(Ppm::from_ratio(7, 5), Some(Ppm::ONE));
        assert_eq!(Ppm::from_ratio(1, 0), None);
    }

    #[test]
    fn ppm_serializes_as_a_bare_integer() {
        assert_eq!(serde_json::to_string(&Ppm(50_000)).unwrap(), "50000");
        assert_eq!(serde_json::from_str::<Ppm>("50000").unwrap(), Ppm(50_000));
    }
}
