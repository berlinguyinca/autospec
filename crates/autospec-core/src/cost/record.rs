//! One fleet run's cost record, read from `out/issue-*/status.txt`.
//!
//! The status file is a plain `key: value` document written by the fleet
//! harness when a run reaches a terminal status. This reader is deliberately
//! tolerant in one direction and strict in the other:
//!
//! - **Tolerant of growth**: unknown keys are ignored, so the harness can add
//!   lines without breaking the aggregator. Blank lines and `#` comments are
//!   ignored too.
//! - **Strict about what it does read**: a known key with a value this reader
//!   cannot interpret (a non-integer `agent_secs`, a misspelled
//!   `disposition`, a non-ISO timestamp) fails the whole file. A silently
//!   dropped number would understate GPU-hours, and understated cost is the
//!   exact failure this accounting exists to surface.
//!
//! A record is *costed* only when it has both a terminal `status` and
//! `agent_secs`. A run that died before recording either is not dropped: the
//! report lists it under `incomplete` so the operator sees the evidence gap
//! instead of a quietly smaller denominator.

use serde::Serialize;

use super::time::parse_iso8601;

/// How a finished run is counted in the cost report.
///
/// `productive` is the default: hours that shipped (or were still in flight
/// at the cut) count as the work's own cost. The other three mark hours that
/// must be paid for again — the rework the report separates from productive
/// time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum Disposition {
    /// Hours that count as the work's own cost.
    Productive,
    /// Run discarded: its change was thrown away.
    Discarded,
    /// Run held: blocked, awaiting a decision.
    Held,
    /// Run superseded: a later run replaced it.
    Superseded,
}

impl Disposition {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Productive => "productive",
            Self::Discarded => "discarded",
            Self::Held => "held",
            Self::Superseded => "superseded",
        }
    }

    pub fn is_rework(self) -> bool {
        !matches!(self, Self::Productive)
    }

    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "productive" => Ok(Self::Productive),
            "discarded" => Ok(Self::Discarded),
            "held" => Ok(Self::Held),
            "superseded" => Ok(Self::Superseded),
            other => Err(format!(
                "disposition expects productive|discarded|held|superseded, got {other}"
            )),
        }
    }
}

/// One agent run as read from its `status.txt`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RunRecord {
    /// Run directory name (e.g. `issue-3793`).
    pub issue: String,
    /// Terminal status recorded by the run, when present.
    pub status: Option<String>,
    /// Wall-clock agent time in seconds, when recorded.
    pub agent_secs: Option<u64>,
    /// GPUs the run held for its agent time.
    pub gpus: u64,
    /// Files the run changed.
    pub changed_files: u64,
    /// Worker that ran the job, when recorded.
    pub worker: Option<String>,
    /// Run start, epoch seconds, when recorded.
    pub started_at: Option<i64>,
    /// Run finish, epoch seconds, when recorded.
    pub finished_at: Option<i64>,
    /// How the run's hours are counted (default [`Disposition::Productive`]).
    pub disposition: Disposition,
}

impl RunRecord {
    /// A record with no fields yet: parsing fills it in from the file.
    pub fn new(issue: impl Into<String>) -> Self {
        Self {
            issue: issue.into(),
            status: None,
            agent_secs: None,
            gpus: 1,
            changed_files: 0,
            worker: None,
            started_at: None,
            finished_at: None,
            disposition: Disposition::Productive,
        }
    }

    /// Parse a `status.txt` body. Returns an error naming the offending line
    /// when a known key carries an unreadable value.
    pub fn parse(issue: &str, content: &str) -> Result<Self, String> {
        let mut record = Self::new(issue);
        for (index, line) in content.lines().enumerate() {
            let line_number = index + 1;
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (key, value) = line
                .split_once(':')
                .ok_or_else(|| format!("line {line_number}: not 'key: value': {line}"))?;
            let key = key.trim();
            let value = value.trim();
            if value.is_empty() {
                return Err(format!("line {line_number}: value for `{key}` is empty"));
            }
            record
                .set(key, value)
                .map_err(|error| format!("line {line_number}: {error}"))?;
        }
        Ok(record)
    }

    /// True when the record carries enough to cost: a terminal status and the
    /// agent time it spent.
    pub fn is_costed(&self) -> bool {
        self.status.is_some() && self.agent_secs.is_some()
    }

    /// GPU-hours this run consumed: `agent_secs * gpus / 3600`, or `None`
    /// when the record is not costed — an uncosted run contributes no hours.
    pub fn gpu_hours(&self) -> Option<f64> {
        self.status.as_ref()?;
        let secs = self.agent_secs?;
        Some(secs as f64 * self.gpus as f64 / 3600.0)
    }

    /// The instant the window filter keys on: finish time, falling back to
    /// start time for a run that started but never recorded a finish.
    pub fn window_stamp(&self) -> Option<i64> {
        self.finished_at.or(self.started_at)
    }

    fn set(&mut self, key: &str, value: &str) -> Result<(), String> {
        match key {
            "status" => self.status = Some(value.to_string()),
            "agent_secs" => self.agent_secs = Some(parse_nonnegative_int(key, value)?),
            "gpus" => self.gpus = parse_nonnegative_int(key, value)?,
            "changed_files" => self.changed_files = parse_nonnegative_int(key, value)?,
            "worker" => self.worker = Some(value.to_string()),
            "started_at" => {
                self.started_at = Some(parse_iso8601(value).map_err(|_| value.to_string())?);
            }
            "finished_at" => {
                self.finished_at = Some(parse_iso8601(value).map_err(|_| value.to_string())?);
            }
            "disposition" => self.disposition = Disposition::parse(value)?,
            // Unknown keys are ignored: the harness may add lines, and the
            // aggregator must not break on a key it does not consume.
            _ => {}
        }
        Ok(())
    }
}

fn parse_nonnegative_int(key: &str, value: &str) -> Result<u64, String> {
    value
        .parse::<u64>()
        .map_err(|_| format!("{key} expects a non-negative integer, got {value}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = "\
issue: issue-3793
status: VERIFIED
agent_secs: 7200
gpus: 2
changed_files: 14
worker: g1
started_at: 2026-09-01T00:00:00Z
finished_at: 2026-09-01T02:00:00Z
disposition: productive
";

    #[test]
    fn parses_full_record() {
        let record = RunRecord::parse("issue-3793", FULL).expect("full record parses");
        assert_eq!(record.status.as_deref(), Some("VERIFIED"));
        assert_eq!(record.agent_secs, Some(7200));
        assert_eq!(record.gpus, 2);
        assert_eq!(record.changed_files, 14);
        assert_eq!(record.worker.as_deref(), Some("g1"));
        assert_eq!(record.disposition, Disposition::Productive);
        assert!(record.is_costed());
        assert_eq!(record.gpu_hours(), Some(4.0));
        // 2026-09-01T00:00:00Z is 1_788_220_800; the record finished 2h in.
        assert_eq!(record.window_stamp(), Some(1_788_220_800 + 7200));
    }

    #[test]
    fn defaults_apply_for_absent_keys() {
        let record = RunRecord::parse("issue-1", "status: VERIFIED\nagent_secs: 3600\n")
            .expect("minimal record parses");
        assert_eq!(record.gpus, 1);
        assert_eq!(record.changed_files, 0);
        assert_eq!(record.worker, None);
        assert_eq!(record.started_at, None);
        assert_eq!(record.disposition, Disposition::Productive);
        assert!(record.is_costed());
        assert_eq!(record.gpu_hours(), Some(1.0));
        assert_eq!(record.window_stamp(), None);
    }

    #[test]
    fn record_without_status_or_secs_is_not_costed() {
        for body in ["", "worker: g1\n", "status: VERIFIED\n", "agent_secs: 90\n"] {
            let record = RunRecord::parse("issue-2", body).expect("parses");
            assert!(!record.is_costed(), "body {body:?} must not be costed");
            assert_eq!(record.gpu_hours(), None);
        }
    }

    #[test]
    fn unknown_keys_and_comments_are_ignored() {
        let record = RunRecord::parse(
            "issue-3",
            "# fleet harness v2\nmodel: opus\n\nstatus: VERIFIED\nagent_secs: 60\n",
        )
        .expect("unknown keys tolerated");
        assert!(record.is_costed());
    }

    #[test]
    fn duplicate_keys_take_the_last_value() {
        let record = RunRecord::parse(
            "issue-4",
            "status: TIMEOUT\nstatus: VERIFIED\nagent_secs: 60\n",
        )
        .expect("parses");
        assert_eq!(record.status.as_deref(), Some("VERIFIED"));
    }

    #[test]
    fn known_keys_with_bad_values_fail_the_file() {
        let cases = [
            "agent_secs: abc",
            "agent_secs: -5",
            "gpus: 1.5",
            "changed_files: many",
            "disposition: lost",
            "started_at: yesterday",
            "finished_at: 2026-02-30T00:00:00Z",
            "no colon in this line",
            "status:",
        ];
        for body in &cases {
            let error = match RunRecord::parse("issue-5", body) {
                Ok(record) => panic!("{body:?} must be rejected, parsed: {record:?}"),
                Err(error) => error,
            };
            assert!(
                error.contains("line 1"),
                "{body:?}: error should name the offending line, got {error:?}"
            );
        }
    }
}
