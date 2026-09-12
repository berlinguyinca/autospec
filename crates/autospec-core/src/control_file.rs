//! A write to a control file is not a control action until verified
//! against the reader's predicate (issue #4453).
//!
//! The incident: to stop eight issues being re-dispatched forever (#4451),
//! they were appended to the dispatcher's hold list with a reason and a
//! timestamp:
//!
//! ```text
//! 3550 repeatedly-dispatched-no-patch 2026-09-11T23:03:45-07:00
//! ```
//!
//! The dispatcher reads that file like this:
//!
//! ```sh
//! grep -qx "$n" "$L/autospec/queue-hold.txt" 2>/dev/null && continue
//! ```
//!
//! `-x` is a *whole-line* match. `3550` does not equal
//! `3550 repeatedly-dispatched-no-patch 2026-09-11T23:03:45-07:00`, so not
//! one of the holds would have taken effect. The file would have listed
//! them, the log would have said they were held, and the dispatcher would
//! have kept sending them to GPUs.
//!
//! The format was *visible* — the existing content was two bare numbers —
//! and a richer line was appended anyway, because adding information to a
//! control record is a natural instinct and it is exactly the mistake:
//! the reader's predicate, not the writer's intent, defines the format.
//!
//! Writing to a shared control file is an inter-process API call written
//! in a file format. Everything that makes an ordinary API call safe — a
//! signature, a type, a compile error — is absent: an appended line is
//! always syntactically valid; the only thing that can be wrong is the
//! semantics, and nothing checks them. This module makes the semantics
//! checkable, with the writer's intent and the reader's predicate as the
//! same object:
//!
//! 1. **The reader's predicate defines the record's format.**
//!    [`ControlRecord`] renders to exactly the line the consumer's
//!    whole-line predicate matches — for the hold list that is the bare
//!    issue number, one per line — and [`ControlFile::parse`] rejects any
//!    line the record's format does not cover. A free-text line in a
//!    typed control file is a load-time error naming the line, never
//!    invisible dead weight.
//! 2. **A write verifies its own post-condition against the consumer's
//!    predicate.** [`ControlFile::write_verified`] publishes atomically,
//!    then re-reads the file from disk and runs the consumer's predicate
//!    ([`visible`], the same whole-line match as `grep -qx`) over the
//!    re-read content. A record the reader cannot see makes the write
//!    fail loudly at the point of writing ([`ControlFileError::InvisibleAfterWrite`]).
//! 3. **Annotation lives in a sidecar keyed by issue.** The reason and
//!    the timestamp go to [`HoldSidecar`], not into the matched record —
//!    so annotation never has to corrupt the matched record to be
//!    recorded. The sidecar is verified the same way: re-read, parse,
//!    and every just-written key must be present.
//! 4. **No free-text append path remains.** The only mutations of a
//!    [`ControlFile`] are typed [`ControlFile::add`] /
//!    [`ControlFile::remove`]; there is no API that appends an arbitrary
//!    line, so the writer's intent and the reader's predicate are the
//!    same object rather than two beliefs about the same bytes.
//!
//! The convenience operations [`hold`] and [`release`] are the procedure
//! from the incident: load the hold list typed (a free-text file fails
//! here, loudly), mutate, publish verified, and record the annotation in
//! the sidecar. A dispatcher that keeps the plain-text format exercises
//! the same post-condition with the consumer's own command:
//!
//! ```sh
//! grep -qx "$n" "$hold" && echo ok    # exactly what the dispatcher runs
//! ```

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

// ── Invariant 1: the reader's predicate defines the record's format ────

/// A line-based record a control file carries.
///
/// The record's rendering *is* the format: it is the exact line the
/// consumer's whole-line predicate matches. Rendering and matching being
/// the same object is what an API signature gives a typed call — a
/// writer cannot emit a line the reader cannot see.
pub trait ControlRecord: Clone + Eq + Ord + fmt::Display {
    /// Parse one whole line. `None` when the line is not this record's
    /// format — which for the hold list means: not exactly the bare issue
    /// number, no annotation, no padding, no leading zeros.
    fn parse_line(line: &str) -> Option<Self>;
}

/// One hold: a bare issue number.
///
/// The dispatcher matches it with `grep -qx "$n"` — a whole-line match
/// against the number and nothing else. That predicate, not the writer's
/// wish to record more, is the format this record owns.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct HoldRecord {
    /// The issue number the hold applies to.
    pub issue: u64,
}

impl fmt::Display for HoldRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Exactly the line the consumer's whole-line predicate matches.
        // Nothing before it, nothing after it.
        f.write_str(&self.issue.to_string())
    }
}

impl ControlRecord for HoldRecord {
    fn parse_line(line: &str) -> Option<Self> {
        let issue: u64 = line.parse().ok()?;
        // Canonical form only: the round-trip must be byte-exact, or the
        // consumer's predicate matches a different string than the one on
        // disk. `03550` parses as 3550 but would render back as `3550`,
        // changing the record's identity under the reader's feet.
        if issue.to_string() != line {
            return None;
        }
        Some(HoldRecord { issue })
    }
}

// ── The typed control file ──────────────────────────────────────────────

/// A control file, typed: the records it holds and the only way to render
/// it.
///
/// Mutation is [`add`](ControlFile::add) and
/// [`remove`](ControlFile::remove) against typed records; there is no API
/// that appends an arbitrary line. [`render`](ControlFile::render) emits
/// one record line per line, and [`parse`](ControlFile::parse) is strict:
/// a line the record's format does not cover is an error naming the line.
/// That is what removes the free-text append path — a file containing
/// `3550 repeatedly-dispatched-no-patch 2026-09-11T23:03:45-07:00` cannot
/// be loaded as a hold list at all, so it cannot masquerade as one.
///
/// Records are kept sorted and deduplicated, so the file always renders
/// identically no matter in what order the holds were taken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlFile<T: ControlRecord> {
    records: Vec<T>,
}

/// Why a typed control-file operation failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlFileError {
    /// A non-empty line the record's format does not cover: free text in a
    /// typed control file. The consumer's predicate matches no record in
    /// it — the exact defect of issue #4451, now a load-time error.
    OffFormatLine {
        /// The offending line, verbatim.
        line: String,
    },
    /// The file on disk after the write is not what the writer intended:
    /// one of the just-written records is invisible to the consumer's
    /// whole-line predicate. The write is not a control action.
    InvisibleAfterWrite {
        /// The path that was written.
        path: PathBuf,
        /// The record lines the consumer's predicate cannot see.
        invisible: Vec<String>,
    },
    /// The sidecar content is not the JSON the sidecar reader parses.
    /// The path is empty when the content was parsed in memory rather
    /// than read from a file.
    InvalidSidecar {
        /// The path that carried the content.
        path: PathBuf,
        /// What the parser refused to say.
        message: String,
    },
    /// A hold was requested without a reason. A hold that cannot say why
    /// is the silence the sidecar exists to remove.
    BlankReason {
        /// The issue the hold was requested for.
        issue: u64,
    },
    /// A filesystem operation failed.
    Io {
        path: PathBuf,
        operation: String,
        message: String,
    },
}

impl fmt::Display for ControlFileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ControlFileError::OffFormatLine { line } => write!(
                f,
                "off-format line in a typed control file (the consumer's predicate matches no record in it): {line:?}"
            ),
            ControlFileError::InvisibleAfterWrite { path, invisible } => write!(
                f,
                "write to {} not visible to the consumer's whole-line predicate: {}",
                path.display(),
                invisible.join(", ")
            ),
            ControlFileError::InvalidSidecar { path, message } => {
                if path.as_os_str().is_empty() {
                    write!(f, "sidecar content is not parseable: {message}")
                } else {
                    write!(f, "sidecar at {} is not parseable: {message}", path.display())
                }
            }
            ControlFileError::BlankReason { issue } => {
                write!(f, "hold of #{issue} was requested without a reason")
            }
            ControlFileError::Io {
                path,
                operation,
                message,
            } => write!(f, "{operation} on {} failed: {message}", path.display()),
        }
    }
}

impl std::error::Error for ControlFileError {}

impl<T: ControlRecord> Default for ControlFile<T> {
    fn default() -> Self {
        Self {
            records: Vec::new(),
        }
    }
}

impl<T: ControlRecord> ControlFile<T> {
    /// An empty control file.
    pub fn new() -> Self {
        Self::default()
    }

    /// The records, ascending.
    pub fn records(&self) -> &[T] {
        &self.records
    }

    /// Whether `record` is held.
    pub fn contains(&self, record: &T) -> bool {
        self.records.contains(record)
    }

    /// Add one typed record. `false` when it was already present — a
    /// duplicate hold is a no-op, never a second line.
    pub fn add(&mut self, record: T) -> bool {
        self.insert_sorted(record)
    }

    /// Remove one typed record. `false` when it was not present.
    pub fn remove(&mut self, record: &T) -> bool {
        match self.records.binary_search(record) {
            Ok(pos) => {
                self.records.remove(pos);
                true
            }
            Err(_) => false,
        }
    }

    /// The consumer's predicate over arbitrary content: does `record`
    /// appear as a *whole line*, exactly as rendered — the same match as
    /// `grep -qx`? A prefix, a suffix, an annotation after the number:
    /// none of it counts.
    pub fn visible(content: &str, record: &T) -> bool {
        content.lines().any(|line| line == record.to_string())
    }

    /// Insert `record` keeping the list sorted and deduplicated.
    fn insert_sorted(&mut self, record: T) -> bool {
        match self.records.binary_search(&record) {
            Ok(_) => false,
            Err(pos) => {
                self.records.insert(pos, record);
                true
            }
        }
    }

    /// The post-condition, as a checkable unit: every record the writer
    /// just wrote must be visible to the consumer's predicate in `content`.
    /// Returns the record lines that are not — empty means the write is a
    /// control action.
    pub fn verify_visible(content: &str, records: &[T]) -> Vec<String> {
        records
            .iter()
            .filter(|record| !Self::visible(content, record))
            .map(|record| record.to_string())
            .collect()
    }

    /// Parse `content` strictly: every non-empty line must be exactly one
    /// record line. Blank lines are tolerated (trailing newlines); anything
    /// else is [`ControlFileError::OffFormatLine`], which is the #4451
    /// incident caught at load time instead of at dispatch time.
    pub fn parse(content: &str) -> Result<Self, ControlFileError> {
        let mut file = Self::new();
        for line in content.lines() {
            if line.is_empty() {
                continue;
            }
            let record = T::parse_line(line).ok_or_else(|| ControlFileError::OffFormatLine {
                line: line.to_string(),
            })?;
            file.insert_sorted(record);
        }
        Ok(file)
    }

    /// The exact bytes this file carries on disk: one record line per
    /// line, ascending, each line exactly what the consumer matches.
    pub fn render(&self) -> String {
        let mut out = String::new();
        for record in &self.records {
            out.push_str(&record.to_string());
            out.push('\n');
        }
        out
    }

    /// Publish `self` to `path`, then verify the write against the
    /// consumer's predicate.
    ///
    /// The write is atomic (temporary file in the same directory, renamed
    /// over the target) so a reader never sees a partial file. The
    /// verification is the point: the file is re-read from disk and every
    /// record just written is checked with the same whole-line predicate
    /// the consumer runs. A record the reader cannot see makes the whole
    /// operation fail with [`ControlFileError::InvisibleAfterWrite`],
    /// loudly, at the point of writing — not a later dispatch pass that
    /// re-sends the "held" issue to GPUs.
    pub fn write_verified(&self, path: &Path) -> Result<(), ControlFileError> {
        let rendered = self.render();
        atomic_write(path, &rendered)?;
        let on_disk = fs::read_to_string(path).map_err(|e| ControlFileError::Io {
            path: path.to_path_buf(),
            operation: "re-read after write".to_string(),
            message: e.to_string(),
        })?;
        let invisible = Self::verify_visible(&on_disk, &self.records);
        if !invisible.is_empty() {
            return Err(ControlFileError::InvisibleAfterWrite {
                path: path.to_path_buf(),
                invisible,
            });
        }
        Ok(())
    }
}

/// Write `content` to `path` atomically: a temporary file in the same
/// directory, then a rename over the target. The rename is atomic with
/// respect to opening, so a concurrent reader sees either the old or the
/// new bytes, never a partial file.
fn atomic_write(path: &Path, content: &str) -> Result<(), ControlFileError> {
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty());
    if let Some(parent) = parent {
        fs::create_dir_all(parent).map_err(|e| ControlFileError::Io {
            path: parent.to_path_buf(),
            operation: "create parent directory".to_string(),
            message: e.to_string(),
        })?;
    }
    let stem = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "control-file".to_string());
    let tmp = path.with_file_name(format!("{stem}.tmp-{}", std::process::id()));
    fs::write(&tmp, content).map_err(|e| ControlFileError::Io {
        path: tmp.clone(),
        operation: "write temporary file".to_string(),
        message: e.to_string(),
    })?;
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        ControlFileError::Io {
            path: path.to_path_buf(),
            operation: "rename temporary file into place".to_string(),
            message: e.to_string(),
        }
    })
}

// ── Invariant 3: annotation lives in a sidecar keyed by issue ──────────

/// The annotation for one hold: the reason it exists and when it was
/// recorded. This is the richer line from #4451 — it belongs here, keyed
/// by issue, out of the record the reader's predicate matches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HoldAnnotation {
    /// Why the issue is held.
    pub reason: String,
    /// When the hold was recorded (rendered by the caller, e.g. RFC 3339).
    pub recorded_at: String,
}

/// The hold-list sidecar: reasons and timestamps keyed by issue.
///
/// The hold list itself stays exactly what the dispatcher's whole-line
/// predicate matches — bare numbers, one per line. Everything a human
/// wants beside the number (the reason, the timestamp) is recorded here,
/// so annotation does not have to corrupt the matched record to be
/// recorded. The reader of the sidecar parses JSON and looks up the issue
/// key; that is its predicate, and [`HoldSidecar::write_verified`] runs
/// it as the write's post-condition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct HoldSidecar {
    #[serde(default)]
    entries: BTreeMap<u64, HoldAnnotation>,
}

impl HoldSidecar {
    /// An empty sidecar.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record (or update) the annotation for `issue`.
    pub fn record(
        &mut self,
        issue: u64,
        reason: impl Into<String>,
        recorded_at: impl Into<String>,
    ) -> &HoldAnnotation {
        self.entries.insert(
            issue,
            HoldAnnotation {
                reason: reason.into(),
                recorded_at: recorded_at.into(),
            },
        );
        self.entries.get(&issue).expect("record just inserted")
    }

    /// The recorded annotation for `issue`, if any.
    pub fn annotation(&self, issue: u64) -> Option<&HoldAnnotation> {
        self.entries.get(&issue)
    }

    /// Every issue with a recorded annotation, ascending.
    pub fn issues(&self) -> Vec<u64> {
        self.entries.keys().copied().collect()
    }

    /// Parse sidecar content. A missing sidecar is not a parse: the
    /// caller treats `None` as empty.
    pub fn parse_json(content: &str) -> Result<Self, ControlFileError> {
        serde_json::from_str(content).map_err(|e| ControlFileError::InvalidSidecar {
            path: PathBuf::new(),
            message: e.to_string(),
        })
    }

    /// The exact bytes this sidecar carries: pretty JSON, stable key
    /// order (the `BTreeMap`), so two runs over the same annotations
    /// produce identical bytes.
    pub fn render_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("a BTreeMap of serializable fields cannot fail")
    }

    /// Publish the sidecar to `path`, then verify against its reader's
    /// predicate: re-read, parse as JSON, and every annotation just
    /// written must be retrievable by issue key. A write the reader
    /// cannot see fails loudly at the point of writing.
    pub fn write_verified(&self, path: &Path) -> Result<(), ControlFileError> {
        let rendered = self.render_json();
        atomic_write(path, &rendered)?;
        let on_disk = fs::read_to_string(path).map_err(|e| ControlFileError::Io {
            path: path.to_path_buf(),
            operation: "re-read after write".to_string(),
            message: e.to_string(),
        })?;
        let re_read = serde_json::from_str::<HoldSidecar>(&on_disk).map_err(|e| {
            ControlFileError::InvalidSidecar {
                path: path.to_path_buf(),
                message: e.to_string(),
            }
        })?;
        for (issue, annotation) in &self.entries {
            match re_read.entries.get(issue) {
                Some(found) if found == annotation => {}
                _ => {
                    return Err(ControlFileError::InvisibleAfterWrite {
                        path: path.to_path_buf(),
                        invisible: vec![issue.to_string()],
                    })
                }
            }
        }
        Ok(())
    }
}

// ── The procedure: hold and release, verified ──────────────────────────

/// The report line for a completed, verified hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoldReceipt {
    /// The issue that is now held and verified.
    pub issue: u64,
    /// The reason recorded in the sidecar.
    pub reason: String,
    /// The timestamp recorded in the sidecar.
    pub recorded_at: String,
}

impl HoldReceipt {
    /// One line for the log: the issue, that the hold is verified against
    /// the consumer's predicate, and where the annotation lives.
    pub fn line(&self) -> String {
        format!(
            "held #{} [{}]: {} (verified against the consumer's whole-line predicate; annotation in sidecar)",
            self.issue, self.recorded_at, self.reason
        )
    }
}

/// Hold `issue`: the procedure from #4451, done in the order that makes
/// it a control action.
///
/// 1. Load the hold list **typed** — a file containing a free-text line
///    fails here, loudly, instead of being appended to and listed as
///    held while matching nothing.
/// 2. Add the record (the bare issue number, nothing else).
/// 3. Publish it, then re-read and verify every record is visible to the
///    consumer's whole-line predicate.
/// 4. Record the reason and timestamp in the sidecar, and verify those
///    too.
///
/// Any failure leaves the earlier files as they were for the failed
/// step; the caller sees the error, not a log line claiming a hold that
/// the dispatcher cannot see.
pub fn hold(
    holds_path: &Path,
    sidecar_path: &Path,
    issue: u64,
    reason: &str,
    recorded_at: &str,
) -> Result<HoldReceipt, ControlFileError> {
    if reason.trim().is_empty() {
        return Err(ControlFileError::BlankReason { issue });
    }
    let mut file = load_control_file(holds_path)?;
    file.add(HoldRecord { issue });
    file.write_verified(holds_path)?;

    let mut sidecar = load_sidecar(sidecar_path)?;
    sidecar.record(issue, reason, recorded_at);
    sidecar.write_verified(sidecar_path)?;

    Ok(HoldReceipt {
        issue,
        reason: reason.to_string(),
        recorded_at: recorded_at.to_string(),
    })
}

/// Release `issue`: remove the hold record and verify the write.
///
/// The sidecar keeps the annotation — it is the record of *why* the issue
/// was held, not control state, and deleting history to make room for the
/// next decision is how the written record outlives the fact.
pub fn release(
    holds_path: &Path,
    _sidecar_path: &Path,
    issue: u64,
) -> Result<(), ControlFileError> {
    let mut file = load_control_file(holds_path)?;
    file.remove(&HoldRecord { issue });
    file.write_verified(holds_path)
}

/// Load a control file, treating a missing file as empty and a
/// free-text file as the error it is.
fn load_control_file(path: &Path) -> Result<ControlFile<HoldRecord>, ControlFileError> {
    match fs::read_to_string(path) {
        Ok(content) => ControlFile::<HoldRecord>::parse(&content),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(ControlFile::new()),
        Err(e) => Err(ControlFileError::Io {
            path: path.to_path_buf(),
            operation: "read hold list".to_string(),
            message: e.to_string(),
        }),
    }
}

/// Load the sidecar, treating a missing file as empty.
fn load_sidecar(path: &Path) -> Result<HoldSidecar, ControlFileError> {
    match fs::read_to_string(path) {
        Ok(content) => HoldSidecar::parse_json(&content),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(HoldSidecar::new()),
        Err(e) => Err(ControlFileError::Io {
            path: path.to_path_buf(),
            operation: "read sidecar".to_string(),
            message: e.to_string(),
        }),
    }
}
