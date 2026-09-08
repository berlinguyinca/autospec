//! Repair-loop observability (#3614).
//!
//! A repair loop that always finds work is a defect report nobody is reading:
//! the sweep re-registers the same identity every five minutes, its output is
//! indistinguishable from the healthy run, and the operator sees nothing. This
//! module gives every self-healing loop (the reconciler that replaces
//! preempted workers, the sweep that removes dead endpoints, the autoscaler,
//! the retry path) a small durable ledger that:
//!
//! 1. Records the repair *rate* — repaired sweeps within a rolling window —
//!    not just individual repair events. A sustained non-zero rate against the
//!    same target is an alert, not a status line.
//! 2. Escalates the same identity repaired on N consecutive sweeps, naming the
//!    count ("re-registered on 6 consecutive sweeps").
//! 3. Prints healthy and unhealthy runs differently: `0 missing (expected 0)`
//!    and `4 missing` never render to the same line.
//! 4. Tracks which defect ticket a permanently-running repair stands in for;
//!    a persistent repair with no attached ticket is reported as an untracked
//!    defect.
//!
//! [`RepairTracker`] is JSON-serializable so a cron-style sweep can persist the
//! ledger between runs and keep counting consecutive streaks across processes.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// Default number of most recent sweeps the repair rate is computed over.
pub const DEFAULT_RATE_WINDOW: usize = 5;

/// Default number of consecutive sweeps repairing the same identity before
/// the repair escalates from a status line to an alert.
pub const DEFAULT_ESCALATION_THRESHOLD: u64 = 3;

/// One pass of a repair loop over the identities it reconciles.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepairSweep {
    /// Identities the loop expected to be present during this sweep (the
    /// population it reconciles). The report states this count explicitly so
    /// `0 missing` can be read against what was actually expected.
    pub expected: BTreeSet<String>,
    /// Identities found missing and re-established during this sweep.
    pub repaired: BTreeSet<String>,
}

/// Whether a recorded sweep was healthy, did one-off work, or is masking a
/// defect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepairVerdict {
    /// Nothing was missing; the loop is idle.
    Idle,
    /// One or more identities were repaired, but no identity is sustained yet.
    Repaired,
    /// The same identity has been repaired on enough consecutive sweeps that
    /// the repair is standing in for an unhealed defect.
    Persistent,
}

/// One identity that keeps getting repaired, with the evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepairEscalation {
    /// The identity (worker id, endpoint, pod, ...) that keeps coming back.
    pub identity: String,
    /// Consecutive sweeps this identity has been repaired.
    pub consecutive_sweeps: u64,
    /// Lifetime repair count for this identity.
    pub total_repairs: u64,
    /// The defect ticket this repair stands in for, if one has been attached.
    pub defect_ticket: Option<String>,
}

/// The outcome of one recorded sweep, ready to print or persist.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepairReport {
    /// Classification of this sweep.
    pub verdict: RepairVerdict,
    /// Sorted identities that were missing (and repaired) this sweep.
    pub missing: Vec<String>,
    /// Size of the expected population this sweep reconciled.
    pub expected_count: usize,
    /// Fraction of the most recent sweeps in the window that contained a
    /// repair (1.0 = every sweep in the window repaired something).
    pub repair_rate: f64,
    /// Identities repaired on the escalation threshold or more consecutive
    /// sweeps.
    pub persistent: Vec<RepairEscalation>,
    /// Persistent identities with no attached defect ticket: untracked
    /// defects the repair is silently standing in for.
    pub untracked: Vec<String>,
    /// The operator-facing line. Healthy and unhealthy runs never print the
    /// same way.
    pub line: String,
}

/// Durable per-loop ledger: consecutive per-identity streaks, a rolling
/// repair rate, and the defect tickets permanently-running repairs stand in
/// for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepairTracker {
    /// Stable name of the repair loop this ledger belongs to.
    pub loop_name: String,
    /// Number of most recent sweeps the repair rate is computed over.
    pub rate_window: usize,
    /// Consecutive sweeps repairing the same identity before escalation.
    pub escalation_threshold: u64,
    /// Total sweeps recorded since the ledger was created.
    pub sweep_count: u64,
    /// One entry per recorded sweep, oldest first, `true` when that sweep
    /// contained a repair; bounded to `rate_window`.
    pub repaired_history: Vec<bool>,
    /// Consecutive-sweep repair streak per identity (absent when idle).
    pub streaks: BTreeMap<String, u64>,
    /// Lifetime repair count per identity.
    pub repairs_total: BTreeMap<String, u64>,
    /// Defect tickets attached to identities the repair is standing in for.
    pub defect_tickets: BTreeMap<String, String>,
}

impl RepairTracker {
    /// A ledger with the default policy: rate over the last 5 sweeps,
    /// escalation after 3 consecutive sweeps repairing the same identity.
    pub fn new(loop_name: impl Into<String>) -> Self {
        Self::with_policy(loop_name, DEFAULT_RATE_WINDOW, DEFAULT_ESCALATION_THRESHOLD)
    }

    /// A ledger with an explicit policy; a zero window or threshold is
    /// clamped to 1 so a corrupt ledger cannot divide by zero.
    pub fn with_policy(
        loop_name: impl Into<String>,
        rate_window: usize,
        escalation_threshold: u64,
    ) -> Self {
        Self {
            loop_name: loop_name.into(),
            rate_window: rate_window.max(1),
            escalation_threshold: escalation_threshold.max(1),
            sweep_count: 0,
            repaired_history: Vec::new(),
            streaks: BTreeMap::new(),
            repairs_total: BTreeMap::new(),
            defect_tickets: BTreeMap::new(),
        }
    }

    /// Record one sweep and classify it.
    pub fn record(&mut self, sweep: &RepairSweep) -> RepairReport {
        self.sweep_count += 1;
        let missing: Vec<String> = sweep.repaired.iter().cloned().collect();
        let any_repaired = !missing.is_empty();

        self.advance_streaks(sweep);

        self.repaired_history.push(any_repaired);
        while self.repaired_history.len() > self.rate_window {
            self.repaired_history.remove(0);
        }

        let persistent = self.persistent_escalations();
        let verdict = if persistent.is_empty() {
            if any_repaired {
                RepairVerdict::Repaired
            } else {
                RepairVerdict::Idle
            }
        } else {
            RepairVerdict::Persistent
        };
        let untracked = persistent
            .iter()
            .filter(|escalation| escalation.defect_ticket.is_none())
            .map(|escalation| escalation.identity.clone())
            .collect();
        let line = self.render(sweep, &missing, &persistent, verdict);

        RepairReport {
            verdict,
            missing,
            expected_count: sweep.expected.len(),
            repair_rate: self.repair_rate(),
            persistent,
            untracked,
            line,
        }
    }

    /// Advance per-identity streaks: an identity repaired this sweep keeps
    /// counting, every other previously-repaired identity resets.
    fn advance_streaks(&mut self, sweep: &RepairSweep) {
        // Owned keys, so the loop may mutate the map it was drawn from.
        let mut touched: BTreeSet<String> = self.streaks.keys().cloned().collect();
        touched.extend(sweep.repaired.iter().cloned());
        for identity in touched {
            if sweep.repaired.contains(&identity) {
                let streak = self.streaks.get(&identity).copied().unwrap_or(0) + 1;
                self.streaks.insert(identity.clone(), streak);
                *self.repairs_total.entry(identity.clone()).or_insert(0) += 1;
            } else {
                self.streaks.remove(&identity);
            }
        }
    }

    /// Fraction of the most recent sweeps in the window that contained at
    /// least one repair (0.0 before the first sweep).
    pub fn repair_rate(&self) -> f64 {
        if self.repaired_history.is_empty() {
            return 0.0;
        }
        let hit = self.repaired_history.iter().filter(|entry| **entry).count();
        hit as f64 / self.repaired_history.len() as f64
    }

    /// Consecutive sweeps repairing `identity` (0 when the streak is broken
    /// or the identity was never repaired).
    pub fn streak(&self, identity: &str) -> u64 {
        self.streaks.get(identity).copied().unwrap_or(0)
    }

    /// Total sweeps recorded on this ledger.
    pub fn sweep_count(&self) -> u64 {
        self.sweep_count
    }

    /// The name of the loop this ledger tracks.
    pub fn loop_name(&self) -> &str {
        &self.loop_name
    }

    /// The rolling window the repair rate is computed over.
    pub fn window_size(&self) -> usize {
        self.rate_window
    }

    /// Active per-identity streaks, identity order.
    pub fn active_streaks(&self) -> Vec<(&String, u64)> {
        self.streaks
            .iter()
            .map(|(identity, streak)| (identity, *streak))
            .collect()
    }

    /// Total sweeps in which `identity` has ever been repaired.
    pub fn total_repairs(&self, identity: &str) -> u64 {
        self.repairs_total.get(identity).copied().unwrap_or(0)
    }

    /// Defect tickets attached to identities, identity order.
    pub fn defect_tickets(&self) -> Vec<(&String, &String)> {
        self.defect_tickets.iter().collect()
    }

    /// Attach the defect ticket a permanently-running repair of `identity`
    /// stands in for, so the alert can be traced to the unhealed defect.
    pub fn attach_defect_ticket(&mut self, identity: &str, ticket: impl Into<String>) {
        self.defect_tickets
            .insert(identity.to_string(), ticket.into());
    }

    /// Detach a defect ticket (the defect was healed; the repair goes back to
    /// being plain work).
    pub fn clear_defect_ticket(&mut self, identity: &str) {
        self.defect_tickets.remove(identity);
    }

    /// Every identity repaired on the escalation threshold or more
    /// consecutive sweeps, oldest identity first.
    pub fn persistent_escalations(&self) -> Vec<RepairEscalation> {
        self.streaks
            .iter()
            .filter(|(_, streak)| **streak >= self.escalation_threshold)
            .map(|(identity, streak)| RepairEscalation {
                identity: identity.clone(),
                consecutive_sweeps: *streak,
                total_repairs: self.repairs_total.get(identity).copied().unwrap_or(0),
                defect_ticket: self.defect_tickets.get(identity).cloned(),
            })
            .collect()
    }

    /// The operator-facing line for this sweep. `Idle` prints
    /// `0 missing (expected N) — idle`; `Repaired` names the missing
    /// identities; `Persistent` is an `ALERT` naming each identity's
    /// consecutive count and the window repair rate.
    fn render(
        &self,
        sweep: &RepairSweep,
        missing: &[String],
        persistent: &[RepairEscalation],
        verdict: RepairVerdict,
    ) -> String {
        let missing_count = missing.len();
        let expected_count = sweep.expected.len();
        match verdict {
            RepairVerdict::Idle => format!(
                "repair {}: {} missing (expected {}) — idle, nothing to repair",
                self.loop_name, missing_count, expected_count
            ),
            RepairVerdict::Repaired => format!(
                "repair {}: {} missing (expected {}) — repaired: {}",
                self.loop_name,
                missing_count,
                expected_count,
                missing.join(", ")
            ),
            RepairVerdict::Persistent => {
                self.render_alert(missing_count, expected_count, persistent)
            }
        }
    }

    /// The `ALERT` line for a sustained repair: per-identity consecutive
    /// counts, the window repair rate, and — for each identity — the defect
    /// ticket the repair stands in for, or an `UNTRACKED DEFECT` marker when
    /// no ticket is attached.
    fn render_alert(
        &self,
        missing_count: usize,
        expected_count: usize,
        persistent: &[RepairEscalation],
    ) -> String {
        let escalations = persistent
            .iter()
            .map(|escalation| {
                format!(
                    "{} re-registered on {} consecutive sweeps",
                    escalation.identity, escalation.consecutive_sweeps
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        let mut line = format!(
            "ALERT repair {}: {} missing (expected {}) — {}; repair rate {:.1} over last {} sweeps",
            self.loop_name,
            missing_count,
            expected_count,
            escalations,
            self.repair_rate(),
            self.repaired_history.len()
        );
        for escalation in persistent {
            match &escalation.defect_ticket {
                Some(ticket) => line.push_str(&format!(
                    "; defect ticket for {}: {ticket}",
                    escalation.identity
                )),
                None => line.push_str(&format!(
                    "; UNTRACKED DEFECT {}: no ticket attached — the repair is standing in for an untracked defect",
                    escalation.identity
                )),
            }
        }
        line
    }

    /// Serialize the ledger so a cron-style sweep keeps its streaks across
    /// runs.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("RepairTracker serializes")
    }

    /// Load a ledger from JSON, clamping corrupt policy fields so a bad file
    /// degrades to the default policy instead of panicking.
    pub fn from_json(text: &str) -> Result<Self, serde_json::Error> {
        let mut tracker: Self = serde_json::from_str(text)?;
        if tracker.rate_window == 0 {
            tracker.rate_window = DEFAULT_RATE_WINDOW;
        }
        if tracker.escalation_threshold == 0 {
            tracker.escalation_threshold = DEFAULT_ESCALATION_THRESHOLD;
        }
        tracker.repaired_history.truncate(tracker.rate_window);
        Ok(tracker)
    }

    /// JSON document for `autospec repair-loop status --json`.
    pub fn status_json(&self) -> String {
        let name = serde_json::to_string(&self.loop_name).expect("loop name serializes");
        format!(
            "{{\"command\":\"repair-loop\",\"subcommand\":\"status\",\"loop\":{name},\"ledger\":{}}}",
            self.to_json()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sweep_of(expected: &[&str], repaired: &[&str]) -> RepairSweep {
        RepairSweep {
            expected: expected.iter().map(|id| (*id).to_string()).collect(),
            repaired: repaired.iter().map(|id| (*id).to_string()).collect(),
        }
    }

    #[test]
    fn idle_sweep_prints_expected_population_and_zero_missing() {
        let mut tracker = RepairTracker::new("gw-workers");
        let report = tracker.record(&sweep_of(&[], &[]));

        assert_eq!(report.verdict, RepairVerdict::Idle);
        assert_eq!(report.expected_count, 0);
        assert!(
            report.line.contains("0 missing (expected 0)"),
            "idle line must state the missing count against the expected population: {}",
            report.line
        );
        assert!(!report.line.contains("ALERT"));
        assert!(report.persistent.is_empty());
    }

    #[test]
    fn healthy_and_unhealthy_lines_never_print_the_same() {
        let mut healthy = RepairTracker::new("gw-workers");
        let healthy_line = healthy
            .record(&sweep_of(&["w1", "w2", "w3", "w4"], &[]))
            .line;

        let mut unhealthy = RepairTracker::new("gw-workers");
        let unhealthy_line = unhealthy
            .record(&sweep_of(
                &["w1", "w2", "w3", "w4"],
                &["w1", "w2", "w3", "w4"],
            ))
            .line;

        assert_ne!(healthy_line, unhealthy_line);
        assert!(healthy_line.contains("0 missing (expected 4)"));
        assert!(unhealthy_line.contains("4 missing (expected 4)"));
        assert!(!unhealthy_line.contains("idle, nothing to repair"));
    }

    #[test]
    fn one_off_repair_is_a_status_line_not_an_alert() {
        let mut tracker = RepairTracker::new("gw-workers");
        let report = tracker.record(&sweep_of(&["w1", "w2"], &["w1"]));

        assert_eq!(report.verdict, RepairVerdict::Repaired);
        assert!(!report.line.contains("ALERT"));
        assert!(report.line.contains("repaired: w1"));
        assert!(report.persistent.is_empty());
    }

    #[test]
    fn same_identity_repaired_n_consecutive_sweeps_escalates_naming_the_count() {
        let mut tracker = RepairTracker::new("gw-workers");
        let population = ["w1", "w2", "w3", "w4"];

        for sweep in 1..=2 {
            let report = tracker.record(&sweep_of(&population, &["w1"]));
            assert_eq!(
                report.verdict,
                RepairVerdict::Repaired,
                "sweep {sweep} is below the threshold"
            );
        }

        let report = tracker.record(&sweep_of(&population, &["w1"]));
        assert_eq!(report.verdict, RepairVerdict::Persistent);
        assert_eq!(report.persistent.len(), 1);
        assert_eq!(report.persistent[0].identity, "w1");
        assert_eq!(report.persistent[0].consecutive_sweeps, 3);
        assert!(
            report
                .line
                .contains("w1 re-registered on 3 consecutive sweeps"),
            "alert must name the consecutive count: {}",
            report.line
        );
    }

    #[test]
    fn streak_resets_when_the_identity_is_not_repaired() {
        let mut tracker = RepairTracker::new("gw-workers");
        tracker.record(&sweep_of(&["w1"], &["w1"]));
        tracker.record(&sweep_of(&["w1"], &["w1"]));
        let idle = tracker.record(&sweep_of(&["w1"], &[]));
        assert_eq!(idle.verdict, RepairVerdict::Idle);
        assert_eq!(tracker.streak("w1"), 0);

        let report = tracker.record(&sweep_of(&["w1"], &["w1"]));
        assert_eq!(report.verdict, RepairVerdict::Repaired);
        assert_eq!(tracker.streak("w1"), 1);
    }

    #[test]
    fn repair_rate_tracks_the_rolling_window() {
        let mut tracker = RepairTracker::with_policy("gw-workers", 4, 10);
        tracker.record(&sweep_of(&["w1"], &["w1"]));
        tracker.record(&sweep_of(&["w1"], &["w2"]));
        tracker.record(&sweep_of(&["w1", "w2"], &[]));
        tracker.record(&sweep_of(&["w1", "w2"], &[]));
        assert!((tracker.repair_rate() - 0.5).abs() < f64::EPSILON);

        // The window slides: the two idle sweeps push a repair out.
        tracker.record(&sweep_of(&["w1"], &[]));
        assert!((tracker.repair_rate() - 0.25).abs() < f64::EPSILON);
    }

    #[test]
    fn sustained_rate_against_the_same_target_is_an_alert() {
        let mut tracker = RepairTracker::new("gw-workers");
        let population = ["w1", "w2", "w3", "w4"];

        // Eager loop: a lazy iterator adapter would evaluate only the last
        // sweep and never build the streak the escalation is measured on.
        let mut report = None;
        for _ in 0..3 {
            report = Some(tracker.record(&sweep_of(&population, &["w2"])));
        }
        let report = report.expect("sweep recorded");

        assert_eq!(report.verdict, RepairVerdict::Persistent);
        assert!((report.repair_rate - 1.0).abs() < f64::EPSILON);
        assert!(report.line.contains("repair rate 1.0"));
        assert!(report
            .line
            .contains("w2 re-registered on 3 consecutive sweeps"));
    }

    #[test]
    fn persistent_repair_without_a_ticket_is_an_untracked_defect() {
        let mut tracker = RepairTracker::new("gw-workers");
        for _ in 0..3 {
            tracker.record(&sweep_of(&["w1"], &["w1"]));
        }
        let report = tracker.record(&sweep_of(&["w1"], &["w1"]));

        assert_eq!(report.verdict, RepairVerdict::Persistent);
        assert_eq!(report.untracked, vec!["w1".to_string()]);
        assert!(
            report.line.contains("UNTRACKED DEFECT w1"),
            "a permanent repair with no ticket must be flagged: {}",
            report.line
        );

        tracker.attach_defect_ticket("w1", "inferweave-gateway#60");
        let traced = tracker.record(&sweep_of(&["w1"], &["w1"]));
        assert!(traced.untracked.is_empty());
        assert!(
            traced
                .line
                .contains("defect ticket for w1: inferweave-gateway#60"),
            "the alert must trace to the ticket the repair stands in for: {}",
            traced.line
        );
    }

    #[test]
    fn clearing_a_ticket_reopens_the_untracked_defect_flag() {
        let mut tracker = RepairTracker::new("gw-workers");
        tracker.attach_defect_ticket("w1", "#60");
        for _ in 0..3 {
            tracker.record(&sweep_of(&["w1"], &["w1"]));
        }
        tracker.clear_defect_ticket("w1");
        let report = tracker.record(&sweep_of(&["w1"], &["w1"]));
        assert_eq!(report.untracked, vec!["w1".to_string()]);
    }

    #[test]
    fn each_persistent_identity_escalates_with_its_own_count() {
        let mut tracker = RepairTracker::new("gw-workers");
        tracker.record(&sweep_of(&["w1", "w2"], &["w1", "w2"]));
        tracker.record(&sweep_of(&["w1", "w2"], &["w2"]));
        tracker.record(&sweep_of(&["w1", "w2"], &["w2"]));
        tracker.record(&sweep_of(&["w1", "w2"], &["w2"]));

        let escalations = tracker.persistent_escalations();
        assert_eq!(escalations.len(), 1);
        assert_eq!(escalations[0].identity, "w2");
        // w2 was repaired on all four sweeps; w1 only on the first, so its
        // streak reset and it does not escalate at all.
        assert_eq!(escalations[0].consecutive_sweeps, 4);
        assert_eq!(escalations[0].total_repairs, 4);
    }

    #[test]
    fn json_roundtrip_preserves_streaks_rate_and_tickets() {
        let mut tracker = RepairTracker::new("gw-workers");
        tracker.attach_defect_ticket("w1", "#60");
        tracker.record(&sweep_of(&["w1"], &["w1"]));
        tracker.record(&sweep_of(&["w1"], &["w1"]));
        tracker.record(&sweep_of(&["w1"], &[]));
        let rate = tracker.repair_rate();

        let restored = RepairTracker::from_json(&tracker.to_json()).expect("ledger parses");
        assert_eq!(restored, tracker);
        // The third sweep broke w1's streak; the reset and the cumulative
        // total must both survive the round-trip.
        assert_eq!(restored.streak("w1"), 0);
        assert_eq!(restored.repairs_total.get("w1"), Some(&2));
        assert!((restored.repair_rate() - rate).abs() < f64::EPSILON);
        assert_eq!(restored.defect_tickets.get("w1"), Some(&"#60".to_string()));
    }

    #[test]
    fn corrupt_policy_fields_degrade_to_the_defaults() {
        let text = r#"{"loop_name":"gw-workers","rate_window":0,"escalation_threshold":0,"sweep_count":9,"repaired_history":[true,true,false,true,true,true,true],"streaks":{},"repairs_total":{},"defect_tickets":{}}"#;
        let tracker = RepairTracker::from_json(text).expect("ledger parses");
        assert_eq!(tracker.rate_window, DEFAULT_RATE_WINDOW);
        assert_eq!(tracker.escalation_threshold, DEFAULT_ESCALATION_THRESHOLD);
        assert_eq!(tracker.repaired_history.len(), DEFAULT_RATE_WINDOW);
    }
}
