//! Platform gates: a red platform CI job cannot be ignored (issue #4312).
//!
//! A platform-specific CI job (`macos-test`, `windows-test`, `freebsd-test`
//! in this repository's `rust-suites` workflow) is the *sole* verification of
//! the code behind that platform's `#[cfg]` gate. When it goes red, all code
//! behind that cfg loses verification entirely: every change to that surface
//! merges unverified, and a single-run view cannot tell a job that broke
//! moments ago from one that has been red for days. The #4306 macOS breakage
//! hid for a day exactly that way, and #4311 repeated the same shape on
//! Windows; each instance was fixed at the code level, and this module is
//! the class — tooling that makes a red platform job impossible to ignore.
//!
//! Three invariants, each a primitive here:
//!
//! 1. **A platform job's failure is a coverage loss, not a flaky check.**
//!    `classify_failure` maps a failing job to `GateFailureKind::CoverageLoss`
//!    when the job is the sole CI verification of a platform surface, and to
//!    `Ordinary` otherwise; the two render differently and escalate
//!    differently.
//! 2. **The alarm fires on the gate's pass rate for the default branch, not
//!    on individual runs.** `rate_alarm` fires when the rate is
//!    all-failing (0/N with N > 0) — conspicuous on its own, no state change
//!    required — and names the lost coverage when the platform is known.
//!    It reuses `merge_gate::PassRate` rather than duplicating it.
//! 3. **The local gate is not authority for surfaces it cannot compile.**
//!    `local_authority` scans a patch for `#[cfg(...)]` attributes that gate
//!    platform surfaces and returns `Hold` when the gate's host does not
//!    compile a surface the patch touches: the merge must defer to the named
//!    platform job. A green local gate for a `#[cfg(target_os = "macos")]`
//!    block on a Linux host verified nothing about that block.
//!
//! The predicate parser claims only what it can classify — `target_os`,
//! `target_family`, the bare `unix`/`windows` families, and those under a
//! single `not(...)`. Feature flags, `target_arch`, `any(...)`/`all(...)`
//! and unknown values return `None`: absence is the honest encoding for
//! what the parser does not understand (issue invariant 5), and the
//! fail-safe direction of over-reporting is an unnecessary hold, never an
//! unverified merge. `cfg!(...)` is a runtime branch — its code compiles on
//! every platform — and is not a surface.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// A platform surface a `#[cfg]` gate can express. Declaration order is the
/// `Ord` order; nothing else depends on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Platform {
    Linux,
    Macos,
    Windows,
    FreeBSD,
}

impl Platform {
    /// The `target_os` value for this platform (the cfg spelling).
    pub fn label(self) -> &'static str {
        match self {
            Platform::Linux => "linux",
            Platform::Macos => "macos",
            Platform::Windows => "windows",
            Platform::FreeBSD => "freebsd",
        }
    }

    /// The name of the job that solely verifies this platform in this
    /// repository's `rust-suites` workflow — the default used when the
    /// caller does not supply its own `job_of` map. Linux's suite-running
    /// job is `build-test` (`main-builds` additionally builds on Linux but
    /// is not the suite).
    pub fn ci_job(self) -> &'static str {
        match self {
            Platform::Linux => "build-test",
            Platform::Macos => "macos-test",
            Platform::Windows => "windows-test",
            Platform::FreeBSD => "freebsd-test",
        }
    }
}

/// One `#[cfg(...)]` platform surface a patch touches: the predicate as
/// written and the platforms that compile under it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CfgSurface {
    /// The predicate text between the parentheses, as written.
    pub predicate: String,
    /// The platforms that compile under this predicate, sorted in `Ord`
    /// order; never empty (a surface is classified only if at least one
    /// known platform compiles it).
    pub compiles_on: Vec<Platform>,
}

/// Classify one `#[cfg(...)]` predicate as a platform surface.
///
/// Returns `None` for every predicate that does not gate a platform
/// unambiguously: feature flags, `target_arch`, `any(...)`/`all(...)`
/// compositions, unknown `target_os` values, and anything else. That
/// absence is the honest encoding (issue invariant 5) — the parser does
/// not invent semantics it cannot check.
pub fn parse_platform_predicate(inner: &str) -> Option<CfgSurface> {
    let inner = inner.trim();
    if let Some(inside) = strip_not(inner) {
        let simple = parse_simple(inside)?;
        return Some(CfgSurface {
            predicate: inner.to_owned(),
            compiles_on: complement(simple.compiles_on),
        });
    }
    let simple = parse_simple(inner)?;
    Some(CfgSurface {
        predicate: inner.to_owned(),
        compiles_on: simple.compiles_on,
    })
}

/// A surface the local gate cannot verify, with the job that solely
/// verifies each unverified platform.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeferredSurface {
    /// The predicate text as written in the patch.
    pub predicate: String,
    /// The platforms that compile this surface but are not the gate's host,
    /// in `Ord` order.
    pub unverified_on: Vec<Platform>,
    /// The CI job that solely verifies each platform (parallel to
    /// `unverified_on`).
    pub jobs: Vec<String>,
}

/// Whether the local gate is authority for every platform surface the
/// patch touches (invariant 3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlatformDeferral {
    /// The gate's host compiles every platform surface the patch touches: a
    /// green local gate covers the patch.
    None,
    /// The patch touches at least one surface the gate's host cannot
    /// compile: the merge must hold until the named platform jobs pass.
    Hold {
        host: Platform,
        surfaces: Vec<DeferredSurface>,
    },
}

impl PlatformDeferral {
    /// The merge may proceed on local-gate authority only when this is
    /// false.
    pub fn must_hold(&self) -> bool {
        matches!(self, PlatformDeferral::Hold { .. })
    }

    /// The one line for the merge record and the monitor log.
    pub fn line(&self) -> String {
        match self {
            PlatformDeferral::None => "no platform deferral: the local gate compiles every \
                                       platform surface the patch touches"
                .to_owned(),
            PlatformDeferral::Hold { host, surfaces } => format!(
                "hold: the local gate on {} cannot compile the patch's platform surfaces: {} — the merge must defer to the named jobs",
                host.label(),
                surfaces
                    .iter()
                    .map(surface_line)
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
        }
    }
}

/// The one line for a single deferred surface; the job grammar (singular /
/// plural) lives here so the deferral line stays flat.
fn surface_line(s: &DeferredSurface) -> String {
    let (jobs, verb) = if s.jobs.len() == 1 {
        (s.jobs[0].clone(), "is")
    } else {
        (join_and(&s.jobs), "are")
    };
    format!(
        "#[cfg({})] — {jobs} {verb} the sole verification of that surface",
        s.predicate
    )
}

/// What a failing CI job means (invariant 1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GateFailureKind {
    /// The job is not the sole CI verification of a platform surface: one
    /// verification layer is lost; a re-run may clear it.
    Ordinary { job: String },
    /// The job is the sole CI verification of a platform surface: all code
    /// behind that cfg loses verification.
    CoverageLoss { job: String, platform: Platform },
}

impl GateFailureKind {
    /// The one line for the monitor log; the two kinds must read apart.
    pub fn line(&self) -> String {
        match self {
            GateFailureKind::Ordinary { job } => format!(
                "{job} failed: one verification layer lost — a re-run may clear it"
            ),
            GateFailureKind::CoverageLoss { job, platform } => format!(
                "{job} failed: total loss of coverage for the {} cfg surface — no other check compiles that code, so every change to it merges unverified until the job is fixed",
                platform.label()
            ),
        }
    }
}

/// The state of the gate's pass rate for the default branch (invariant 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RateAlarm {
    /// The workflow has runs and has failed every one: the red state that
    /// hid the #4306 breakage for a day. Conspicuous on its own.
    AllFailing,
    /// Not all-failing — including the empty window, which is unknown, not
    /// red.
    Quiet,
}

impl RateAlarm {
    /// The one line for the monitor log. `platform`, when known, names the
    /// surface whose verification is gone.
    pub fn line(&self, rate: &crate::merge_gate::PassRate, platform: Option<Platform>) -> String {
        match self {
            RateAlarm::Quiet => format!("no alarm: {}", rate.line()),
            RateAlarm::AllFailing => match platform {
                Some(p) => format!(
                    "ALARM: {} — total loss of coverage for the {} cfg surface: no other check compiles that code",
                    rate.line(),
                    p.label()
                ),
                None => format!("ALARM: {} — the gate has failed every run it has run", rate.line()),
            },
        }
    }
}

/// Invariant 2: alarm on the rate, not on individual runs.
///
/// Fires when the rate is all-failing — `0/N` with `N > 0`. A single-run
/// view cannot distinguish a freshly broken job from a long-red one, but
/// the rate always can, so the alarm needs no state change to fire.
pub fn rate_alarm(rate: &crate::merge_gate::PassRate) -> RateAlarm {
    if rate.all_failing() {
        RateAlarm::AllFailing
    } else {
        RateAlarm::Quiet
    }
}

/// Invariant 1: classify a failing job.
///
/// `job_of` is the same mapping `local_authority` takes — the platforms
/// whose sole CI verification the job names. A job that matches no platform
/// in the map is an `Ordinary` failure.
pub fn classify_failure(job: &str, job_of: &BTreeMap<Platform, String>) -> GateFailureKind {
    if let Some((platform, _)) = job_of.iter().find(|(_, name)| name.as_str() == job) {
        GateFailureKind::CoverageLoss {
            job: job.to_owned(),
            platform: *platform,
        }
    } else {
        GateFailureKind::Ordinary {
            job: job.to_owned(),
        }
    }
}

/// Invariant 3: is the local gate (running on `host`) authority for every
/// platform surface the patch touches?
///
/// `job_of` maps each platform to the CI job that solely verifies it; the
/// caller reads it from the workflow definition, which is the source of
/// truth for job names (per the CI name-drift contract, the steps are the
/// contract and the names come from there). A surface is unverified on
/// every platform that compiles it other than `host`; if any surface is
/// unverified on at least one platform, the deferral is `Hold` and the
/// merge must defer to the named jobs.
pub fn local_authority(
    host: Platform,
    job_of: &BTreeMap<Platform, String>,
    patch: &str,
) -> PlatformDeferral {
    let mut surfaces = Vec::new();
    for surface in patch_surfaces(patch) {
        let unverified_on: Vec<Platform> = surface
            .compiles_on
            .iter()
            .copied()
            .filter(|p| *p != host)
            .collect();
        if unverified_on.is_empty() {
            continue;
        }
        let jobs = unverified_on
            .iter()
            .map(|p| {
                job_of
                    .get(p)
                    .cloned()
                    .unwrap_or_else(|| p.ci_job().to_owned())
            })
            .collect();
        surfaces.push(DeferredSurface {
            predicate: surface.predicate,
            unverified_on,
            jobs,
        });
    }
    if surfaces.is_empty() {
        PlatformDeferral::None
    } else {
        PlatformDeferral::Hold { host, surfaces }
    }
}

/// Every platform surface the patch touches.
///
/// Scans every line of the patch — added (`+`), deleted (`-`) and context —
/// for `#[cfg(...)]` attributes, classifies each predicate, and returns the
/// surfaces in first-seen order, deduplicated by predicate text. The only
/// excluded lines are the `+++`/`---` file headers. The scan is deliberately
/// conservative: a hunk that shows the attribute touches the surface, and
/// the scanner does not try to prove which lines *inside* the block
/// changed. A cfg inside a comment or a string over-reports — an
/// unnecessary hold, which is the safe direction, never an unverified
/// merge. A deleted cfg line still touches the surface (fail-safe).
/// `cfg!(...)` is a runtime branch, not an attribute, and is not scanned.
pub fn patch_surfaces(patch: &str) -> Vec<CfgSurface> {
    let mut out: Vec<CfgSurface> = Vec::new();
    for line in patch.lines() {
        if line.starts_with("+++") || line.starts_with("---") {
            continue;
        }
        let mut from = 0;
        while let Some(rel) = line[from..].find("#[cfg(") {
            let start = from + rel;
            let open = start + "#[cfg(".len();
            if let Some(inner) = balanced_inner(line, open) {
                if let Some(surface) = parse_platform_predicate(inner) {
                    if !out.iter().any(|s| s.predicate == surface.predicate) {
                        out.push(surface);
                    }
                }
            }
            from = open;
        }
    }
    out
}

// --- parsing helpers -------------------------------------------------------

/// The contents of the parenthesised group opening at `open`, or `None`
/// when the line runs out before the group balances. A truncated line is
/// an honest absence, not a surface.
fn balanced_inner(line: &str, open: usize) -> Option<&str> {
    let bytes = line.as_bytes();
    let mut depth = 0usize;
    for i in open..bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => {
                if depth == 0 {
                    return Some(&line[open..i]);
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    None
}

/// `not(<balanced predicate>)` -> the inner predicate, or `None` when the
/// input is not exactly one `not(...)` wrapping a balanced group. A
/// composition like `not(unix) and windows` is not a single negation and
/// is not classified.
fn strip_not(inner: &str) -> Option<&str> {
    let rest = inner.strip_prefix("not(")?;
    let end = rest.strip_suffix(")")?;
    let mut depth = 1usize;
    for ch in end.chars() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return None;
                }
            }
            _ => {}
        }
    }
    Some(end)
}

/// The predicate forms that unambiguously gate a platform. Anything else
/// returns `None` — the parser does not claim what it cannot classify.
fn parse_simple(inner: &str) -> Option<CfgSurface> {
    let compiles_on: Vec<Platform> = match inner {
        "unix" => [Platform::Linux, Platform::Macos, Platform::FreeBSD].to_vec(),
        "windows" => vec![Platform::Windows],
        _ => {
            let (key, value) = split_kv(inner)?;
            match (key, value) {
                ("target_os", "linux") => vec![Platform::Linux],
                ("target_os", "macos") => vec![Platform::Macos],
                ("target_os", "windows") => vec![Platform::Windows],
                ("target_os", "freebsd") => vec![Platform::FreeBSD],
                ("target_family", "unix") => {
                    [Platform::Linux, Platform::Macos, Platform::FreeBSD].to_vec()
                }
                ("target_family", "windows") => vec![Platform::Windows],
                _ => return None,
            }
        }
    };
    Some(CfgSurface {
        predicate: inner.to_owned(),
        compiles_on,
    })
}

/// `target_os = "macos"` -> `("target_os", "macos")`. Requires exactly one
/// `=`, an identifier on the left, a double-quoted string on the right.
fn split_kv(inner: &str) -> Option<(&str, &str)> {
    let eq = inner.find('=')?;
    if inner[eq + 1..].contains('=') {
        return None;
    }
    let key = inner[..eq].trim();
    let value = inner[eq + 1..].trim();
    let value = value.strip_prefix('"')?.strip_suffix('"')?;
    if key.is_empty() || value.is_empty() {
        return None;
    }
    Some((key, value))
}

const ALL_PLATFORMS: [Platform; 4] = [
    Platform::Linux,
    Platform::Macos,
    Platform::Windows,
    Platform::FreeBSD,
];

/// The platforms in the known set that do not compile the given set.
fn complement(set: Vec<Platform>) -> Vec<Platform> {
    ALL_PLATFORMS
        .iter()
        .copied()
        .filter(|p| !set.contains(p))
        .collect()
}

fn join_and(items: &[String]) -> String {
    match items.len() {
        0 => String::new(),
        1 => items[0].clone(),
        2 => format!("{} and {}", items[0], items[1]),
        _ => {
            let head = items[..items.len() - 1].join(", ");
            format!("{head} and {}", items[items.len() - 1])
        }
    }
}
