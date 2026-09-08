//! Reference-value tests for the integer-only ε-best-belief statistic.
//!
//! Reference values were computed by two independent methods (binomial tail
//! in log space, and Simpson integration of the Beta density) and agree to
//! the ppm; the assertion tolerance is 2 ppm to absorb the fixed-point
//! recurrence's rounding.
use autospec_core::evaluation::statistics::{best_belief_ppm, Ppm, MAX_TRIALS};

const EPS_05: Ppm = Ppm(50_000);
const EPS_10: Ppm = Ppm(100_000);

// (successes, failures, expected ppm at eps=0.05, expected ppm at eps=0.10)
const REFERENCE: &[(u32, u32, u32, u32)] = &[
    (0, 0, 50_000, 100_000),
    (1, 0, 223_607, 316_228),
    (0, 1, 25_321, 51_317),
    (9, 1, 635_641, 689_757),
    (19, 1, 793_275, 827_065),
    (3, 7, 135_076, 169_233),
    (30, 10, 621_494, 649_346),
    (31, 9, 648_201, 675_704),
    (32, 8, 675_387, 702_441),
    (36, 4, 790_495, 814_420),
    (38, 2, 854_295, 875_358),
    (40, 0, 929_539, 945_387),
    (0, 40, 1_250, 2_567),
    (20, 20, 374_396, 401_514),
    (50, 50, 418_909, 436_655),
    (90, 10, 837_845, 851_560),
    (180, 20, 858_701, 867_833),
];

fn within(actual: Ppm, expected: u32, tolerance: u32) -> bool {
    actual.0.abs_diff(expected) <= tolerance
}

#[test]
fn best_belief_matches_independently_computed_beta_quantiles() {
    for &(s, f, e05, e10) in REFERENCE {
        let got05 = best_belief_ppm(s, f, EPS_05).unwrap();
        let got10 = best_belief_ppm(s, f, EPS_10).unwrap();
        assert!(
            within(got05, e05, 2),
            "S={s} F={f} eps=0.05: got {} want {e05}",
            got05.0
        );
        assert!(
            within(got10, e10, 2),
            "S={s} F={f} eps=0.10: got {} want {e10}",
            got10.0
        );
    }
}

#[test]
fn best_belief_is_monotone_in_successes_and_below_the_mean() {
    let mut previous = 0;
    for s in 0..=40 {
        let bb = best_belief_ppm(s, 40 - s, EPS_05).unwrap().0;
        assert!(bb >= previous, "S={s}: {bb} < {previous}");
        previous = bb;
        let mean_ppm = ((s as u64 + 1) * 1_000_000 / 42) as u32;
        assert!(
            bb <= mean_ppm,
            "S={s}: lower bound {bb} exceeds posterior mean {mean_ppm}"
        );
    }
}

#[test]
fn best_belief_rejects_out_of_range_inputs() {
    assert!(best_belief_ppm(0, 0, Ppm(0)).is_err());
    assert!(best_belief_ppm(0, 0, Ppm(1_000_000)).is_err());
    assert!(best_belief_ppm(MAX_TRIALS, 1, EPS_05).is_err());
}

#[test]
fn ppm_from_ratio_rounds_to_nearest() {
    assert_eq!(Ppm::from_ratio(1, 3), Some(Ppm(333_333)));
    assert_eq!(Ppm::from_ratio(2, 3), Some(Ppm(666_667)));
    assert_eq!(Ppm::from_ratio(5, 5), Some(Ppm::ONE));
    assert_eq!(Ppm::from_ratio(1, 0), None);
}
