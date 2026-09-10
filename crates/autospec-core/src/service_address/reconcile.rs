//! Launched-vs-registered reconciliation (issue #3745).
//!
//! "Launched" and "registered" are two different facts. A worker can be
//! `RUNNING` in the scheduler, serving, answering `/v1/models` with 200 —
//! and still be absent from the dispatch pool, because the endpoint entry
//! dispatchers read for it does not exist, or exists in a form no dispatcher
//! can select. That happened four times: three GPUs, roughly five GPU-hours,
//! serving nothing, while the fleet reported `endpoints=8 reachable=8` —
//! the counts only ever looked at entries that were already there.
//!
//! The reconciler is the check every previous fix skipped: it does not make
//! registration more reliable, it *notices* when registration simply did not
//! happen, whatever the cause. A launcher bug that writes the entry in the
//! wrong format is invisible to the registration path by construction — the
//! write succeeds, the file exists — and only visible to a comparison of
//! what is running against what is registered.

use std::collections::HashSet;
use std::fmt;

/// One `iw-worker-*` job as the scheduler reports it, plus what a probe just
/// observed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningWorker {
    /// Slurm job id.
    pub job_id: u64,
    /// Job name, e.g. `iw-worker-qwen3.8-27b`.
    pub name: String,
    /// Scheduler state (`RUNNING`, `PENDING`, ...). A belief, not a health:
    /// the scheduler's word about the job carries no information about the
    /// service behind it.
    pub state: String,
    /// The worker's own log. The `serving <model> at <url>` line is the fact
    /// a registration is supposed to record — and it is present even when
    /// every launch path skipped writing the endpoint file.
    pub log: String,
    /// Whether the worker answered a health probe (`/v1/models`) just now.
    /// This — not [`RunningWorker::state`] — is the fact the reconciliation
    /// counts.
    pub serving: bool,
}

/// The `serving <model> at <url>` line a worker writes when its server starts
/// listening.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServingLine {
    pub model: String,
    pub url: String,
}

/// Find the worker's serving line, if its log has one.
///
/// The last such line wins: a worker that restarts its server logs the line
/// again, and the earlier address is exactly the kind of stale fact this
/// module exists to catch.
pub fn serving_line(log: &str) -> Option<ServingLine> {
    log.lines()
        .filter_map(|line| {
            let rest = line.trim().strip_prefix("serving")?;
            let (model, url) = rest.trim_start().split_once(" at ")?;
            let model = model.trim();
            let url = url.trim();
            if model.is_empty() || !url.starts_with("http") {
                return None;
            }
            Some(ServingLine {
                model: model.to_string(),
                url: url.to_string(),
            })
        })
        .last()
}

/// One file in `state/endpoints/` as it sits on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointEntry {
    /// File name. Dispatchers glob by `<model>-*`, so the name is part of
    /// the contract: it must be `<model>-<jobid>`.
    pub filename: String,
    /// Raw file content. The contract is one line of six tab-separated
    /// fields.
    pub content: String,
}

/// An endpoint entry exactly as a dispatcher reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchableEndpoint {
    pub url: String,
    pub model: String,
    pub job_id: u64,
    pub card: String,
    /// Slot count, as the server's `/slots` reports it.
    pub slots: u32,
    /// Per-slot context, as the server's `/props` reports it.
    pub ctx_per_slot: u32,
}

/// Why an endpoint file exists on disk but a dispatcher cannot select it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EndpointParseError {
    /// The line does not carry the six tab-separated fields the dispatcher
    /// cuts. Space-separated content is one field to `cut -fN`.
    FieldCount { found: usize },
    /// A field the dispatcher needs as a number is not one (blank included).
    NotANumber { field: &'static str },
    /// A field the dispatcher needs is blank.
    Blank { field: &'static str },
    /// The file name is not `<model>-<jobid>` for the fields it carries, so
    /// a dispatcher globbing `<model>-*` never finds it no matter how correct
    /// the content is.
    InvisibleName { filename: String },
}

impl fmt::Display for EndpointParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FieldCount { found } => write!(
                f,
                "expected 6 tab-separated fields, found {found} (space-separated content is one field)"
            ),
            Self::NotANumber { field } => write!(f, "field {field:?} is not a number"),
            Self::Blank { field } => write!(f, "field {field:?} is blank"),
            Self::InvisibleName { filename } => write!(
                f,
                "filename {filename:?} is not <model>-<jobid>; a <model>-* glob never finds it"
            ),
        }
    }
}

impl EndpointEntry {
    /// Parse the way a dispatcher does: six tab-separated fields
    /// (url, model, jobid, card, slots, ctx), no trimming — a dispatcher does
    /// not trim — and a file name a `<model>-*` glob would find.
    ///
    /// A file that fails here is not "registered with a typo": it is not in
    /// the pool at all.
    pub fn dispatchable(&self) -> Result<DispatchableEndpoint, EndpointParseError> {
        let line = self
            .content
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("");
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() != 6 {
            return Err(EndpointParseError::FieldCount {
                found: fields.len(),
            });
        }
        let (url, model, jobid, card, slots, ctx) = (
            fields[0],
            fields[1],
            fields[2],
            fields[3],
            fields[4],
            fields[5],
        );
        if url.is_empty() {
            return Err(EndpointParseError::Blank { field: "url" });
        }
        if model.is_empty() {
            return Err(EndpointParseError::Blank { field: "model" });
        }
        let job_id = parse_number(jobid, "jobid")?;
        let slots = parse_number(slots, "slots")?;
        let ctx_per_slot = parse_number(ctx, "ctx")?;
        if self.filename != format!("{model}-{job_id}") {
            return Err(EndpointParseError::InvisibleName {
                filename: self.filename.clone(),
            });
        }
        Ok(DispatchableEndpoint {
            url: url.to_string(),
            model: model.to_string(),
            job_id,
            card: card.to_string(),
            slots,
            ctx_per_slot,
        })
    }

    /// The job id the file *names*, when its name carries one: a bare job id
    /// or anything ending in `-<jobid>`.
    pub fn named_job(&self) -> Option<u64> {
        self.filename.rsplit('-').next()?.parse().ok()
    }
}

fn parse_number(field: &str, name: &'static str) -> Result<u32, EndpointParseError> {
    field
        .parse::<u32>()
        .map_err(|_| EndpointParseError::NotANumber { field: name })
}

/// A registration derived from the running worker instead of from whichever
/// launch path started it.
///
/// The worker's log carries `serving <model> at <url>`; the server itself
/// answers `/slots` and `/props`. All the inputs are on the worker, so the
/// derivation cannot be missed by a launch path — there is no launch path to
/// get it wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedRegistration {
    pub job_id: u64,
    pub model: String,
    pub url: String,
    pub card: String,
    pub slots: u32,
    pub ctx_per_slot: u32,
}

impl DerivedRegistration {
    /// Derive it from a listening worker: the serving line from its log, the
    /// slot count and per-slot context from the server's own `/slots` and
    /// `/props` answers.
    pub fn from_worker(
        worker: &RunningWorker,
        card: &str,
        slots: u32,
        ctx_per_slot: u32,
    ) -> Option<Self> {
        let ServingLine { model, url } = serving_line(&worker.log)?;
        Some(Self {
            job_id: worker.job_id,
            model,
            url,
            card: card.to_string(),
            slots,
            ctx_per_slot,
        })
    }

    /// The file name a `<model>-*` glob finds.
    pub fn filename(&self) -> String {
        format!("{}-{}", self.model, self.job_id)
    }

    /// The six tab-separated fields, in the order every dispatcher cuts.
    pub fn content(&self) -> String {
        format!(
            "{}\t{}\t{}\t{}\t{}\t{}\n",
            self.url, self.model, self.job_id, self.card, self.slots, self.ctx_per_slot
        )
    }

    /// The entry as it will sit on disk. Parsing it back must succeed — that
    /// is the definition of "in the dispatch pool".
    pub fn entry(&self) -> EndpointEntry {
        EndpointEntry {
            filename: self.filename(),
            content: self.content(),
        }
    }
}

/// A worker the probe says is serving but the dispatch pool does not know.
/// Money being burned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnregisteredWorker {
    pub job_id: u64,
    pub name: String,
    /// Why the pool does not know it.
    pub reason: UnregisteredReason,
}

/// Why a serving worker is not in the dispatch pool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnregisteredReason {
    /// No endpoint file names this job at all.
    Missing,
    /// A file exists, but in a form no dispatcher can select. This is the
    /// two-writers defect: the write succeeded, the file exists, and it is
    /// invisible to the pool — wrong name, wrong separator, missing fields.
    Invisible { filename: String, detail: String },
}

impl fmt::Display for UnregisteredReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing => f.write_str("no endpoint entry"),
            Self::Invisible { filename, detail } => {
                write!(f, "entry {filename} is not dispatchable: {detail}")
            }
        }
    }
}

/// An endpoint that must be retired: dispatchers would select it and die.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadEndpoint {
    pub filename: String,
    /// The job the entry names, when its name carries one.
    pub job_id: Option<u64>,
    /// Why it must go.
    pub reason: DeadReason,
}

/// Why an endpoint must be retired.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeadReason {
    /// The job the entry names is not in the scheduler's list.
    JobGone,
    /// The file is not in a form a dispatcher reads, so it is not in the pool
    /// no matter what, and its presence only hides the real state.
    Unreadable { detail: String },
}

impl fmt::Display for DeadReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::JobGone => f.write_str("job gone"),
            Self::Unreadable { detail } => write!(f, "not dispatchable: {detail}"),
        }
    }
}

/// The result of one reconciliation sweep: the two facts, side by side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reconciliation {
    /// Serving workers (the probe answered), whatever the scheduler says.
    running: usize,
    /// Entries a dispatcher can actually select, live job or not — a corpse
    /// is still in the pool and still gets dispatched to.
    registered: usize,
    unregistered: Vec<UnregisteredWorker>,
    dead: Vec<DeadEndpoint>,
}

/// Reconcile what is running against what is registered, in both directions.
///
/// For every serving worker: is there an entry a dispatcher can select for
/// it? For every entry: is its job alive, and is the file even in a form a
/// dispatcher reads? The sweep is the check; how the entries get written is
/// deliberately not its concern.
pub fn reconcile(workers: &[RunningWorker], endpoints: &[EndpointEntry]) -> Reconciliation {
    let live_jobs: HashSet<u64> = workers.iter().map(|w| w.job_id).collect();

    let parsed: Vec<(&EndpointEntry, Result<DispatchableEndpoint, EndpointParseError>)> =
        endpoints.iter().map(|e| (e, e.dispatchable())).collect();

    let mut unregistered = Vec::new();
    let mut dead = Vec::new();
    let mut running = 0;
    let mut registered = 0;

    // Direction one: every serving worker must be in the pool.
    for worker in workers.iter().filter(|w| w.serving) {
        running += 1;
        let covered = parsed
            .iter()
            .any(|(_, r)| matches!(r, Ok(d) if d.job_id == worker.job_id));
        if covered {
            continue;
        }
        // Is there a file that names this job but is unreadable? That is the
        // case that no write-side fix can ever show: the write succeeded.
        let reason = parsed
            .iter()
            .find(|(e, r)| e.named_job() == Some(worker.job_id) && r.is_err())
            .map(|(e, r)| UnregisteredReason::Invisible {
                filename: e.filename.clone(),
                detail: r.as_ref().unwrap_err().to_string(),
            })
            .unwrap_or(UnregisteredReason::Missing);
        unregistered.push(UnregisteredWorker {
            job_id: worker.job_id,
            name: worker.name.clone(),
            reason,
        });
    }

    // Direction two: every entry must be readable and name a live job.
    for (entry, parsed) in &parsed {
        match parsed {
            Ok(d) => {
                registered += 1;
                if !live_jobs.contains(&d.job_id) {
                    dead.push(DeadEndpoint {
                        filename: entry.filename.clone(),
                        job_id: Some(d.job_id),
                        reason: DeadReason::JobGone,
                    });
                }
            }
            Err(err) => dead.push(DeadEndpoint {
                filename: entry.filename.clone(),
                job_id: entry.named_job(),
                reason: DeadReason::Unreadable {
                    detail: err.to_string(),
                },
            }),
        }
    }

    Reconciliation {
        running,
        registered,
        unregistered,
        dead,
    }
}

impl Reconciliation {
    /// Serving workers, from the probe, not the scheduler.
    pub fn running(&self) -> usize {
        self.running
    }

    /// Entries a dispatcher can select, dead or not.
    pub fn registered(&self) -> usize {
        self.registered
    }

    /// Serving workers the pool does not know.
    pub fn unregistered(&self) -> &[UnregisteredWorker] {
        &self.unregistered
    }

    /// Entries that must be retired.
    pub fn dead(&self) -> &[DeadEndpoint] {
        &self.dead
    }

    /// True only when every serving worker is dispatchable and no entry
    /// points at a gone job or sits in an unreadable form.
    pub fn in_sync(&self) -> bool {
        self.unregistered.is_empty() && self.dead.is_empty()
    }

    /// The sweep's line: `running=N registered=M` always, and when the two
    /// facts disagree, the disagreement itself — both directions, never
    /// "nothing to do".
    pub fn report(&self) -> String {
        let mut line = format!("running={} registered={}", self.running, self.registered);
        if self.in_sync() {
            return line;
        }
        let mut parts: Vec<String> = Vec::new();
        for u in &self.unregistered {
            parts.push(format!("{} {}: {}", u.job_id, u.name, u.reason));
        }
        for d in &self.dead {
            parts.push(format!("{}: {}", d.filename, d.reason));
        }
        line.push_str(&format!(" — GAP: {}", parts.join("; ")));
        line
    }

    /// The file names to delete before the next dispatch.
    pub fn retire(&self) -> Vec<String> {
        self.dead.iter().map(|d| d.filename.clone()).collect()
    }

    /// The pool after the dead entries are retired: the set a dispatcher can
    /// select from next.
    pub fn pool_after_retire(&self, endpoints: &[EndpointEntry]) -> Vec<EndpointEntry> {
        let gone: HashSet<&str> = self.dead.iter().map(|d| d.filename.as_str()).collect();
        endpoints
            .iter()
            .filter(|e| !gone.contains(e.filename.as_str()))
            .cloned()
            .collect()
    }
}
