//! Roll by observed state, not by age (issue #4356).
//!
//! The incident: a configuration change was rolled across a Slurm worker
//! fleet by selecting the two oldest workers like this:
//!
//! ```sh
//! squeue -u gw -h -o "%i %j %M" | grep iw-worker-qwen3.8-27b | sort -k3 -r | head -2
//! ```
//!
//! Slurm renders elapsed time as `13:20` for thirteen minutes and
//! `1-04:47:07` for over a day. Sorted as text, `13:20` sorts *above*
//! `1-04:47:07`, so the command selected a worker started thirteen
//! minutes earlier — one that already carried the new configuration — and
//! cancelled it, leaving the genuinely old one untouched. The selection
//! logic was wrong in a way that is invisible until the durations straddle
//! a day boundary, which is exactly when a rolling restart is running.
//!
//! The invariants this module makes checkable:
//!
//! 1. **Roll by observed state, not by age or launch order.** "Old" is a
//!    proxy; the configuration itself is the fact, and it is usually as
//!    easy to read. [`select_stale`] selects on the observed value of the
//!    property the roll is changing, never on elapsed time.
//! 2. **A selection predicate for a destructive action must be
//!    idempotent.** Re-running it mid-roll must select only the remainder.
//!    An age-ranked list is not: it reshuffles as replacements start.
//!    [`select_stale`] returns exactly the workers still carrying the old
//!    state, whatever replacements have already come up.
//! 3. **Never sort mixed-format duration strings as text.**
//!    `1-04:47:07` and `13:20` are not comparable lexically; neither are
//!    `9m`/`10m` or `2h`/`30m`. [`parse_elapsed`] turns them into whole
//!    seconds and [`compare_elapsed`] orders them numerically; an
//!    unparsable duration has no order and says so, instead of being
//!    compared as text.
//! 4. **After a partial roll, report the remaining count from the same
//!    predicate that drives the action.** [`roll_status`] counts with the
//!    same old/target comparison [`select_stale`] selects with, so
//!    `stale=0` from the same check that selects work is a real
//!    completion signal; a count derived some other way can disagree with
//!    what the action will do next.
//!
//! Everything here is pure: no I/O, no clock, no subprocess. The caller
//! runs `squeue` and reads each worker's configuration; this module
//! decides what the observations mean for the roll.

use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

/// One worker as a roll observes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RollWorker {
    /// The Slurm job id. It increases monotonically with start order, so
    /// it is the identifier the selection orders by.
    pub job_id: u64,
    /// The elapsed time exactly as the scheduler rendered it (`13:20`,
    /// `1-04:47:07`). Kept for the report; never compared as text.
    pub elapsed: String,
    /// The observed value of the property the roll is changing, read from
    /// the running worker (e.g. the `n_ctx_slot` it serves). The fact the
    /// selection reads.
    pub state: String,
}

/// Parse a scheduler elapsed-time string into whole seconds.
///
/// Accepts the `squeue` elapsed forms — `MM:SS`, `HH:MM:SS`,
/// `DD-HH:MM:SS` — and the unit-suffixed forms `45s`, `30m`, `2h`, `1d`.
/// Returns `None` for anything else: an unparsable duration has no
/// numeric order, and the caller falls back to a monotonic identifier
/// rather than comparing the strings as text.
pub fn parse_elapsed(text: &str) -> Option<u64> {
    fn digits(part: &str) -> Option<u64> {
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        part.parse::<u64>().ok()
    }

    let (day_part, rest) = match text.split_once('-') {
        Some((days, remainder)) => (Some(days), remainder),
        None => (None, text),
    };

    // Unit-suffixed form: no dash, no colons, a single trailing unit.
    if day_part.is_none() && !rest.contains(':') {
        let len = rest.len();
        if len == 0 {
            return None;
        }
        let (num, unit) = rest.split_at(len - 1);
        let multiplier = match unit.as_bytes()[0] {
            b's' => 1u64,
            b'm' => 60,
            b'h' => 3600,
            b'd' => 86_400,
            _ => return None,
        };
        return digits(num).map(|n| n.saturating_mul(multiplier));
    }

    // A dash commits the form to `DD-HH:MM:SS`: the day field is present
    // and must be numeric.
    let days = match day_part {
        Some(day) => Some(digits(day)?),
        None => None,
    };
    let fields: Vec<&str> = rest.split(':').collect();
    match fields.len() {
        // `MM:SS`: less than an hour.
        2 if days.is_none() => {
            let minutes = digits(fields[0])?;
            let seconds = digits(fields[1])?;
            Some(minutes.saturating_mul(60).saturating_add(seconds))
        }
        // `HH:MM:SS` or `DD-HH:MM:SS`.
        3 => {
            let hours = digits(fields[0])?;
            let minutes = digits(fields[1])?;
            let seconds = digits(fields[2])?;
            let secs = hours
                .saturating_mul(3600)
                .saturating_add(minutes.saturating_mul(60))
                .saturating_add(seconds);
            Some(days.map_or(secs, |d| d.saturating_mul(86_400).saturating_add(secs)))
        }
        _ => None,
    }
}

/// Compare two elapsed-time strings as durations.
///
/// `None` when either side is unparsable: "not comparable" is the answer a
/// destructive action can act on, unlike a text ordering that happens to
/// be wrong.
pub fn compare_elapsed(a: &str, b: &str) -> Option<Ordering> {
    match (parse_elapsed(a), parse_elapsed(b)) {
        (Some(x), Some(y)) => Some(x.cmp(&y)),
        _ => None,
    }
}

/// The jobs the roll should touch next: every worker still carrying the
/// old state, in job-id order (a monotonic identifier).
///
/// The selection reads the property being changed, not a proxy for it
/// (invariant 1), and it is idempotent (invariant 2): a worker already at
/// `target` is never selected, so re-running the roll after a partial
/// completion selects exactly the remainder, never something already
/// done. A no-op roll (`old == target`) selects nothing: there is no
/// change to make, and selecting everything would be the worst possible
/// misreading of that.
pub fn select_stale(workers: &[RollWorker], old: &str, target: &str) -> Vec<u64> {
    if old == target {
        return Vec::new();
    }
    let mut ids: Vec<u64> = workers
        .iter()
        .filter(|worker| worker.state == old)
        .map(|worker| worker.job_id)
        .collect();
    ids.sort_unstable();
    ids
}

/// The roll's progress. The counts come from the same old/target
/// comparison [`select_stale`] selects with (invariant 4), so the line is
/// the selection rendered as counts: `stale=0` is a real completion
/// signal, and a count derived some other way cannot disagree with what
/// the action will do next.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RollStatus {
    /// Workers already carrying `target`.
    pub ok: usize,
    /// Workers still carrying `old` — exactly what [`select_stale`]
    /// returns.
    pub stale: usize,
    /// Workers carrying neither value: neither done nor selected. They are
    /// reported, not silently counted as done.
    pub other: usize,
}

impl RollStatus {
    /// `ok=7 stale=1 other=0`.
    pub fn line(&self) -> String {
        format!("ok={} stale={} other={}", self.ok, self.stale, self.other)
    }

    /// The roll is complete when nothing remains for the selection.
    pub fn complete(&self) -> bool {
        self.stale == 0
    }
}

/// Count the roll's progress with the same predicate [`select_stale`]
/// uses.
pub fn roll_status(workers: &[RollWorker], old: &str, target: &str) -> RollStatus {
    let mut status = RollStatus {
        ok: 0,
        stale: 0,
        other: 0,
    };
    for worker in workers {
        if worker.state == target {
            status.ok += 1;
        } else if worker.state == old {
            status.stale += 1;
        } else {
            status.other += 1;
        }
    }
    status
}

#[cfg(test)]
mod tests {
    use super::*;

    fn worker(job_id: u64, elapsed: &str, state: &str) -> RollWorker {
        RollWorker {
            job_id,
            elapsed: elapsed.to_string(),
            state: state.to_string(),
        }
    }

    // ---- invariant 3: durations are compared numerically, never as text ----

    #[test]
    fn parse_slurm_elapsed_forms() {
        assert_eq!(parse_elapsed("00:00"), Some(0));
        assert_eq!(parse_elapsed("13:20"), Some(800));
        assert_eq!(parse_elapsed("04:47:07"), Some(4 * 3600 + 47 * 60 + 7));
        assert_eq!(
            parse_elapsed("1-04:47:07"),
            Some(86_400 + 4 * 3600 + 47 * 60 + 7)
        );
        assert_eq!(parse_elapsed("0-00:00:00"), Some(0));
    }

    #[test]
    fn parse_unit_suffixed_forms() {
        assert_eq!(parse_elapsed("45s"), Some(45));
        assert_eq!(parse_elapsed("9m"), Some(540));
        assert_eq!(parse_elapsed("10m"), Some(600));
        assert_eq!(parse_elapsed("30m"), Some(1800));
        assert_eq!(parse_elapsed("2h"), Some(7200));
        assert_eq!(parse_elapsed("1d"), Some(86_400));
    }

    #[test]
    fn unparsable_elapsed_has_no_order() {
        for text in [
            "",
            "13:",
            ":20",
            "1-2-04:47:07",
            "13:20:00:00",
            "1h30m",
            "abc",
            "1-",
            "-04:47:07",
            " 13:20",
            "13:20 ",
            "1.5h",
            "90",
        ] {
            assert_eq!(parse_elapsed(text), None, "{text:?} must not parse");
        }
    }

    /// The incident, at the string level: Slurm's two renderings sorted the
    /// way the incident's command sorted them — as text. `13:20` sorts
    /// *above* `1-04:47:07`, which is what selected the freshly-fixed
    /// thirteen-minute-old worker over the genuinely old one.
    #[test]
    fn the_incident_text_sort_orders_the_young_worker_first() {
        let young = "13:20";
        let old = "1-04:47:07";
        // Document the trap: this is why sorting durations as text is
        // wrong.
        assert!(young > old, "precondition: the text order is inverted");
        // The numeric order is the other way around.
        assert_eq!(compare_elapsed(young, old), Some(Ordering::Less));
        assert_eq!(compare_elapsed(old, young), Some(Ordering::Greater));
        assert_eq!(compare_elapsed(old, old), Some(Ordering::Equal));
    }

    #[test]
    fn unit_forms_are_not_lexically_comparable_either() {
        assert!("9m" > "10m", "precondition: text order is wrong here too");
        assert_eq!(compare_elapsed("9m", "10m"), Some(Ordering::Less));
        assert!("2h" < "30m", "precondition: text order is wrong here too");
        assert_eq!(compare_elapsed("2h", "30m"), Some(Ordering::Greater));
    }

    #[test]
    fn compare_elapsed_is_none_when_either_side_unparsable() {
        assert_eq!(compare_elapsed("13:20", "bogus"), None);
        assert_eq!(compare_elapsed("bogus", "13:20"), None);
        assert_eq!(compare_elapsed("", ""), None);
    }

    // ---- invariants 1 and 2: select by state, idempotently ----

    #[test]
    fn selection_reads_the_state_not_the_age() {
        // The incident's shape: the young worker already carries the new
        // configuration; the old one still carries the old. Text-sorted by
        // elapsed time the young one comes first; the state predicate
        // selects only the old one.
        let workers = vec![
            worker(200, "13:20", "65536"),
            worker(150, "1-04:47:07", "32768"),
        ];
        assert_eq!(select_stale(&workers, "32768", "65536"), vec![150]);
    }

    #[test]
    fn selection_is_idempotent_after_a_partial_roll() {
        let mut workers = vec![
            worker(101, "1-04:47:07", "32768"),
            worker(102, "1-04:46:00", "32768"),
            worker(103, "1-04:45:00", "32768"),
            worker(104, "1-04:44:00", "32768"),
        ];
        assert_eq!(
            select_stale(&workers, "32768", "65536"),
            vec![101, 102, 103, 104]
        );

        // A partial roll replaces two of them. The replacements are new
        // jobs with new ids, so any age ranking reshuffles; the state
        // predicate selects exactly the remainder.
        workers[0] = worker(105, "00:12", "65536");
        workers[1] = worker(106, "00:11", "65536");
        assert_eq!(select_stale(&workers, "32768", "65536"), vec![103, 104]);
    }

    #[test]
    fn a_noop_roll_selects_nothing() {
        let workers = vec![
            worker(1, "13:20", "65536"),
            worker(2, "1-04:47:07", "65536"),
        ];
        assert!(select_stale(&workers, "65536", "65536").is_empty());
    }

    #[test]
    fn selection_never_touches_done_or_unknown_workers() {
        let workers = vec![worker(1, "13:20", "65536"), worker(2, "05:00", "131072")];
        assert!(select_stale(&workers, "32768", "65536").is_empty());
    }

    // ---- invariant 4: the count comes from the same predicate ----

    #[test]
    fn status_counts_come_from_the_same_predicate() {
        let workers = vec![
            worker(1, "1-04:47:07", "65536"),
            worker(2, "13:20", "65536"),
            worker(3, "0-12:00:00", "32768"),
        ];
        let status = roll_status(&workers, "32768", "65536");
        assert_eq!((status.ok, status.stale, status.other), (2, 1, 0));
        assert_eq!(status.line(), "ok=2 stale=1 other=0");
        assert!(!status.complete());
        // The stale count is exactly the number of jobs the selection
        // returns: same predicate, same answer.
        assert_eq!(status.stale, select_stale(&workers, "32768", "65536").len());
    }

    #[test]
    fn stale_zero_is_the_completion_signal() {
        let workers = vec![
            worker(1, "00:12", "65536"),
            worker(2, "00:11", "65536"),
            worker(3, "00:10", "65536"),
        ];
        let status = roll_status(&workers, "32768", "65536");
        assert!(status.complete());
        assert_eq!(status.line(), "ok=3 stale=0 other=0");
        assert!(select_stale(&workers, "32768", "65536").is_empty());
    }

    #[test]
    fn unknown_state_is_reported_not_counted_done() {
        let workers = vec![worker(1, "13:20", "131072")];
        let status = roll_status(&workers, "32768", "65536");
        assert_eq!((status.ok, status.stale, status.other), (0, 0, 1));
        assert_eq!(status.line(), "ok=0 stale=0 other=1");
    }
}
