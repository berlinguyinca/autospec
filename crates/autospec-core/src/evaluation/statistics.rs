//! Conservative binary qualification statistic (RQGM ε-best-belief) in integer
//! arithmetic. `BB_ε(S, F)` is the ε-quantile of `Beta(1+S, 1+F)`. For integer
//! parameters the Beta CDF is a binomial upper tail:
//! `I_x(a, b) = P[Bin(a+b-1, x) >= a]`, so the quantile is the smallest `x`
//! (in parts per million) whose tail probability reaches ε. No binary
//! floating-point arithmetic anywhere in this module: the `financial_no_f64`
//! architecture gate scans this crate.
use serde::{Deserialize, Serialize};

use super::error::EvaluationError;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Ppm(pub u32);

impl Ppm {
    pub const ONE: Ppm = Ppm(1_000_000);
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

/// Largest `S + F + 1` supported; keeps every intermediate inside `u128`.
pub const MAX_TRIALS: u32 = 65_536;
/// Fixed-point scale for relative term weights (2^40).
const WEIGHT_ONE: u128 = 1u128 << 40;
/// Fixed-point scale for probabilities (2^64).
const PROB_ONE: u128 = 1u128 << 64;

/// `P[Bin(n, p) >= k]` scaled to `PROB_ONE`. Terms are computed relative to the
/// modal term so nothing under- or overflows for `n <= MAX_TRIALS`.
fn binomial_upper_tail(n: u32, k: u32, p: Ppm) -> u128 {
    if k == 0 {
        return PROB_ONE;
    }
    if k > n || p.0 == 0 {
        return 0;
    }
    if p.0 >= 1_000_000 {
        return PROB_ONE;
    }
    let p_num = p.0 as u128;
    let q_num = (1_000_000 - p.0) as u128;
    let mode = (((n as u128 + 1) * p_num) / 1_000_000).min(n as u128) as usize;
    let mut weights = vec![0u128; n as usize + 1];
    weights[mode] = WEIGHT_ONE;
    let mut j = mode;
    while j < n as usize {
        // t_{j+1} / t_j = (n - j) / (j + 1) * p / q
        let next = weights[j] * (n as u128 - j as u128) * p_num / ((j as u128 + 1) * q_num);
        weights[j + 1] = next;
        if next == 0 {
            break;
        }
        j += 1;
    }
    let mut j = mode;
    while j > 0 {
        // t_{j-1} / t_j = j / (n - j + 1) * q / p
        let prev = weights[j] * j as u128 * q_num / ((n as u128 - j as u128 + 1) * p_num);
        weights[j - 1] = prev;
        if prev == 0 {
            break;
        }
        j -= 1;
    }
    let total: u128 = weights.iter().sum();
    let tail: u128 = weights[k as usize..].iter().sum();
    (tail << 64) / total
}

/// ε-quantile of `Beta(1 + successes, 1 + failures)`, in parts per million.
pub fn best_belief_ppm(
    successes: u32,
    failures: u32,
    epsilon: Ppm,
) -> Result<Ppm, EvaluationError> {
    if epsilon.0 == 0 || epsilon.0 >= 1_000_000 {
        return Err(EvaluationError::invariant(format!(
            "epsilon must be in (0, 1) ppm-exclusive, got {}",
            epsilon.0
        )));
    }
    let n = successes
        .checked_add(failures)
        .and_then(|t| t.checked_add(1))
        .filter(|&t| t <= MAX_TRIALS)
        .ok_or_else(|| {
            EvaluationError::invariant(format!("successes + failures + 1 must be <= {MAX_TRIALS}"))
        })?;
    let k = successes + 1;
    // Parenthesised on purpose: `<<` binds looser than `/` in Rust.
    let threshold = ((epsilon.0 as u128) << 64) / 1_000_000;
    let (mut lo, mut hi) = (0u32, 1_000_000u32);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if binomial_upper_tail(n, k, Ppm(mid)) >= threshold {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    Ok(Ppm(lo))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_endpoints_are_exact() {
        assert_eq!(binomial_upper_tail(1, 1, Ppm(500_000)), PROB_ONE / 2);
        assert_eq!(binomial_upper_tail(5, 0, Ppm(1)), PROB_ONE);
        assert_eq!(binomial_upper_tail(5, 6, Ppm(999_999)), 0);
    }

    #[test]
    fn uniform_prior_quantile_is_epsilon_within_fixed_point_truncation() {
        assert!(
            best_belief_ppm(0, 0, Ppm(50_000))
                .unwrap()
                .0
                .abs_diff(50_000)
                <= 1
        );
        assert!(
            best_belief_ppm(0, 0, Ppm(250_000))
                .unwrap()
                .0
                .abs_diff(250_000)
                <= 1
        );
    }
}
