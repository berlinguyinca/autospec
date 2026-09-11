//! Dispatch outcome attribution (#4025).
//!
//! Before this module, the outcome of a dispatch (which model ran it, what the
//! spec looked like, how much of the budget it consumed) and what the
//! conversion pass did with its patch (converted / held / retired) lived in
//! two separate logs and a directory tree, joined by nothing. This module is
//! the join: one record per dispatch in one durable place, with the
//! conversion result written back to the record that produced it, and a
//! report that attributes outcomes to the model and the spec conditions.
//!
//! The ledger is append-only JSONL: a new line per state change, and the last
//! line per `dispatch_id` wins on load. Nothing is rewritten in place, so a
//! crash mid-append cannot corrupt earlier records.
//!
//! Measurement first (#4025): the report is advisory evidence for the
//! dispatcher's model selection. Nothing in this module routes a dispatch.

use crate::error::AutospecError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

/// How a dispatch ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalOutcome {
    /// The worker produced a patch; what the conversion pass did with it is
    /// recorded separately on the same record.
    PatchProduced,
    /// The run ended without producing a patch.
    NoOutput,
    /// The run was killed by its supervisor at the wall-clock budget.
    Timeout,
}

impl TerminalOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PatchProduced => "patch_produced",
            Self::NoOutput => "no_output",
            Self::Timeout => "timeout",
        }
    }
}

/// What the conversion pass did with the dispatch's patch, written back to
/// the dispatch record that produced it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversionOutcome {
    /// The patch became a pull request.
    Converted,
    /// The patch is held; the reason is the join key back to the hold record.
    Held { reason: String },
    /// The patch is retired as non-applying.
    RetiredNonApplying,
}

impl ConversionOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Converted => "converted",
            Self::Held { .. } => "held",
            Self::RetiredNonApplying => "retired_non_applying",
        }
    }
}

/// Spec size bands for the model × band cross-tabulation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpecSizeBand {
    Small,
    Medium,
    Large,
}

impl SpecSizeBand {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Small => "small",
            Self::Medium => "medium",
            Self::Large => "large",
        }
    }
}

/// Specs below this many bytes are the `small` band.
pub const SPEC_BAND_SMALL_MAX_BYTES: u64 = 16 * 1024;
/// Specs below this many bytes (and at the small bound) are the `medium` band.
pub const SPEC_BAND_MEDIUM_MAX_BYTES: u64 = 64 * 1024;

pub fn spec_band_for(spec_bytes: u64) -> SpecSizeBand {
    if spec_bytes < SPEC_BAND_SMALL_MAX_BYTES {
        SpecSizeBand::Small
    } else if spec_bytes < SPEC_BAND_MEDIUM_MAX_BYTES {
        SpecSizeBand::Medium
    } else {
        SpecSizeBand::Large
    }
}

/// One dispatch: the model that ran it, the spec conditions, the budget, and
/// what happened. Written once at dispatch and extended in place (append-only
/// history) when the terminal outcome and the conversion result land.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DispatchRecord {
    /// Stable id for this dispatch (one per dispatch, never per issue).
    pub dispatch_id: String,
    /// Issue number or id the dispatch was answering.
    pub issue: String,
    /// Model that ran the dispatch, verbatim as dispatched.
    pub model: String,
    /// Size of the spec in bytes at dispatch time.
    pub spec_bytes: u64,
    /// Wall-clock budget for the run, in seconds.
    pub budget_secs: u64,
    /// Dispatch start, epoch seconds.
    pub started_at: i64,
    /// Dispatch end, epoch seconds, once terminal.
    pub ended_at: Option<i64>,
    /// Terminal outcome, once known.
    pub terminal: Option<TerminalOutcome>,
    /// Conversion result written back by the conversion pass, once known.
    pub conversion: Option<ConversionOutcome>,
}

impl DispatchRecord {
    pub fn new(
        dispatch_id: impl Into<String>,
        issue: impl Into<String>,
        model: impl Into<String>,
        spec_bytes: u64,
        budget_secs: u64,
        started_at: i64,
    ) -> Self {
        Self {
            dispatch_id: dispatch_id.into(),
            issue: issue.into(),
            model: model.into(),
            spec_bytes,
            budget_secs,
            started_at,
            ended_at: None,
            terminal: None,
            conversion: None,
        }
    }

    pub fn band(&self) -> SpecSizeBand {
        spec_band_for(self.spec_bytes)
    }

    /// Wall-clock seconds consumed, once the dispatch has an end time.
    pub fn wall_clock_secs(&self) -> Option<u64> {
        let ended = self.ended_at?;
        (ended >= self.started_at).then(|| (ended - self.started_at) as u64)
    }

    pub fn validate(&self) -> Result<(), AutospecError> {
        let empty = |field: &str| {
            AutospecError::validation(format!("dispatch record field '{field}' must not be empty"))
        };
        if self.dispatch_id.trim().is_empty() {
            return Err(empty("dispatch_id"));
        }
        if self.issue.trim().is_empty() {
            return Err(empty("issue"));
        }
        if self.model.trim().is_empty() {
            return Err(empty("model"));
        }
        if matches!(&self.conversion, Some(ConversionOutcome::Held { reason }) if reason.trim().is_empty())
        {
            return Err(AutospecError::validation(
                "a held conversion outcome needs a non-empty reason",
            ));
        }
        if self.conversion.is_some() && self.terminal.is_none() {
            return Err(AutospecError::validation(
                "a conversion outcome can only be written back after the dispatch is terminal",
            ));
        }
        Ok(())
    }
}

/// Append-only JSONL ledger of [`DispatchRecord`]s, one durable file.
///
/// `append` writes one line; `load` folds the file with last-line-per-
/// `dispatch_id` wins; `record_terminal` and `record_conversion` are the
/// write-backs — they load, update, and append. A second terminal write for
/// a record whose conversion was already written back fails validation, and
/// the append history keeps the full sequence.
#[derive(Debug, Clone)]
pub struct DispatchLedger {
    path: PathBuf,
}

impl DispatchLedger {
    pub fn open(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Append one record line, creating parent directories on first write.
    pub fn append(&self, record: &DispatchRecord) -> Result<(), AutospecError> {
        record.validate()?;
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                AutospecError::io(
                    "create parent of dispatch-outcomes ledger",
                    self.path.to_string_lossy(),
                    e,
                )
            })?;
        }
        let mut line = serde_json::to_string(record)
            .map_err(|e| AutospecError::other(format!("serialize dispatch record: {e}")))?;
        line.push('\n');
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|e| {
                AutospecError::io(
                    "append to dispatch-outcomes ledger",
                    self.path.to_string_lossy(),
                    e,
                )
            })?;
        file.write_all(line.as_bytes()).map_err(|e| {
            AutospecError::io(
                "append to dispatch-outcomes ledger",
                self.path.to_string_lossy(),
                e,
            )
        })
    }

    /// Load the ledger. A missing file is an empty ledger; a corrupt line is
    /// an error — a silently dropped dispatch would undercount a model's
    /// failures, which is the exact failure this measurement exists to show.
    pub fn load(&self) -> Result<Vec<DispatchRecord>, AutospecError> {
        let Ok(text) = fs::read_to_string(&self.path) else {
            return Ok(Vec::new());
        };
        let mut index: HashMap<String, usize> = HashMap::new();
        let mut records: Vec<DispatchRecord> = Vec::new();
        for (offset, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let record: DispatchRecord = serde_json::from_str(line).map_err(|e| {
                AutospecError::parse(
                    "dispatch-outcomes ledger",
                    format!("line {}: {e}", offset + 1),
                )
            })?;
            record.validate()?;
            match index.get(&record.dispatch_id).copied() {
                Some(i) => records[i] = record,
                None => {
                    index.insert(record.dispatch_id.clone(), records.len());
                    records.push(record);
                }
            }
        }
        Ok(records)
    }

    /// Write back the terminal outcome for one dispatch.
    pub fn record_terminal(
        &self,
        dispatch_id: &str,
        ended_at: i64,
        terminal: TerminalOutcome,
    ) -> Result<(), AutospecError> {
        let mut records = self.load()?;
        let record = records
            .iter_mut()
            .find(|r| r.dispatch_id == dispatch_id)
            .ok_or_else(|| {
                AutospecError::state(
                    "dispatch-outcomes ledger",
                    format!("no dispatch record for '{dispatch_id}'"),
                )
            })?;
        record.ended_at = Some(ended_at);
        record.terminal = Some(terminal);
        self.append(record)
    }

    /// Write back the conversion result for one dispatch.
    pub fn record_conversion(
        &self,
        dispatch_id: &str,
        outcome: ConversionOutcome,
    ) -> Result<(), AutospecError> {
        let mut records = self.load()?;
        let record = records
            .iter_mut()
            .find(|r| r.dispatch_id == dispatch_id)
            .ok_or_else(|| {
                AutospecError::state(
                    "dispatch-outcomes ledger",
                    format!("no dispatch record for '{dispatch_id}'"),
                )
            })?;
        record.conversion = Some(outcome);
        self.append(record)
    }
}

/// Minimum decided outcomes per row before a rate is shown; below it the row
/// is reported as insufficient data, never as a percentage.
pub const DEFAULT_MIN_SAMPLES: u64 = 10;

/// Verdict for one report row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RateStatus {
    /// `converted / sample`, as a fraction.
    Rate(f64),
    /// Fewer than the minimum samples; no rate is presented.
    InsufficientData,
}

/// One report row: a model, or a model within a spec size band.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OutcomeRateRow {
    pub model: String,
    /// `None` for by-model rows; the band for by-model × band rows.
    pub band: Option<SpecSizeBand>,
    pub converted: u64,
    pub held: u64,
    pub retired: u64,
    /// Dispatches with no conversion result written back yet.
    pub pending: u64,
    /// Decided outcomes (`converted + held + retired`); the rate denominator.
    pub sample: u64,
    pub status: RateStatus,
}

/// Outcome rates by model and by model × spec size band.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OutcomeReport {
    pub min_samples: u64,
    pub dispatches: u64,
    pub by_model: Vec<OutcomeRateRow>,
    pub by_model_band: Vec<OutcomeRateRow>,
}

#[derive(Default)]
struct Cell {
    converted: u64,
    held: u64,
    retired: u64,
    pending: u64,
}

impl Cell {
    fn observe(&mut self, record: &DispatchRecord) {
        match &record.conversion {
            Some(ConversionOutcome::Converted) => self.converted += 1,
            Some(ConversionOutcome::Held { .. }) => self.held += 1,
            Some(ConversionOutcome::RetiredNonApplying) => self.retired += 1,
            None => self.pending += 1,
        }
    }

    fn row(self, model: &str, band: Option<SpecSizeBand>, min_samples: u64) -> OutcomeRateRow {
        let sample = self.converted + self.held + self.retired;
        let status = if sample > 0 && sample >= min_samples {
            RateStatus::Rate(self.converted as f64 / sample as f64)
        } else {
            RateStatus::InsufficientData
        };
        OutcomeRateRow {
            model: model.to_string(),
            band,
            converted: self.converted,
            held: self.held,
            retired: self.retired,
            pending: self.pending,
            sample,
            status,
        }
    }
}

/// Build the outcome report: rate of conversion by model and by
/// (model × spec size band), with the sample count on every row. Rows below
/// `min_samples` decided outcomes are reported as
/// [`RateStatus::InsufficientData`], never as a rate.
pub fn outcome_report(records: &[DispatchRecord], min_samples: u64) -> OutcomeReport {
    let mut per_model: HashMap<String, Cell> = HashMap::new();
    let mut per_model_band: HashMap<(String, SpecSizeBand), Cell> = HashMap::new();
    for record in records {
        per_model
            .entry(record.model.clone())
            .or_default()
            .observe(record);
        per_model_band
            .entry((record.model.clone(), record.band()))
            .or_default()
            .observe(record);
    }
    let mut by_model: Vec<OutcomeRateRow> = per_model
        .into_iter()
        .map(|(model, cell)| cell.row(&model, None, min_samples))
        .collect();
    by_model.sort_by(|a, b| a.model.cmp(&b.model));
    let mut by_model_band: Vec<OutcomeRateRow> = per_model_band
        .into_iter()
        .map(|((model, band), cell)| cell.row(&model, Some(band), min_samples))
        .collect();
    by_model_band.sort_by(|a, b| a.model.cmp(&b.model).then_with(|| a.band.cmp(&b.band)));
    OutcomeReport {
        min_samples,
        dispatches: records.len() as u64,
        by_model,
        by_model_band,
    }
}

fn rate_cell(status: &RateStatus, sample: u64) -> String {
    match status {
        RateStatus::Rate(r) => format!("{:.1}%", r * 100.0),
        RateStatus::InsufficientData => format!("insufficient data (n={sample})"),
    }
}

impl OutcomeReport {
    pub fn to_markdown(&self) -> String {
        let mut out = String::new();
        out.push_str("# Dispatch outcome report\n\n");
        out.push_str(&format!(
            "Dispatches recorded: {}; minimum samples for a rate: {}.\n\n",
            self.dispatches, self.min_samples
        ));
        out.push_str("## By model\n\n");
        out.push_str(&self.render_table(&self.by_model));
        out.push_str("## By model x spec size band\n\n");
        out.push_str(&self.render_table(&self.by_model_band));
        out
    }

    fn render_table(&self, rows: &[OutcomeRateRow]) -> String {
        let mut out = String::new();
        if rows.is_empty() {
            out.push_str("_no dispatch records_\n\n");
            return out;
        }
        out.push_str(
            "| model | band | converted | held | retired | pending | sample | conversion rate |\n",
        );
        out.push_str("| --- | --- | --- | --- | --- | --- | --- | --- |\n");
        for row in rows {
            let band = row.band.map(SpecSizeBand::as_str).unwrap_or("-");
            out.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} | {} | {} |\n",
                row.model,
                band,
                row.converted,
                row.held,
                row.retired,
                row.pending,
                row.sample,
                rate_cell(&row.status, row.sample)
            ));
        }
        out.push('\n');
        out
    }
}
