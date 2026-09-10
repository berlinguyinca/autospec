//! The authoritative address record and the cache that failure invalidates.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use super::registration::RegistrationOutcome;

/// Where the value a caller holds came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressOrigin {
    /// Read from the authoritative record during this resolution.
    AuthoritativeRecord,
    /// Captured at launch and used because the record could not be read.
    /// Every use of such a value is a use of a snapshot with an unknown expiry.
    LaunchArgument,
}

impl fmt::Display for AddressOrigin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AuthoritativeRecord => f.write_str("record"),
            Self::LaunchArgument => f.write_str("launch-arg"),
        }
    }
}

/// Why the authoritative record could not supply an address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordError {
    /// The record file does not exist (nothing has written it yet).
    Missing,
    /// The record exists but is blank after trimming.
    Empty,
    /// The record exists but is not a usable service address.
    Malformed(String),
}

impl fmt::Display for RecordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing => f.write_str("authoritative address record is missing"),
            Self::Empty => f.write_str("authoritative address record is empty"),
            Self::Malformed(raw) => write!(f, "authoritative address record is not a URL: {raw:?}"),
        }
    }
}

/// Normalize a raw address: trimmed, non-empty, `scheme://authority`.
///
/// Trailing slashes and whitespace are dropped. A bare host without a scheme,
/// or an empty string, is rejected — an empty default is how the incident
/// reached a worker as "no address" without ever tripping an unset check.
pub fn parse_address(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }
    let rest = trimmed.split_once("://")?.1;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    (!authority.is_empty()).then(|| trimmed.to_string())
}

/// Read the authoritative address record.
///
/// Only the first non-blank line is used, so a record carrying a comment or a
/// trailing blank line still resolves. Read failures are [`RecordError`], never
/// an empty string: the caller must distinguish "no record" from "record says
/// nothing".
pub fn read_record(path: &Path) -> Result<String, RecordError> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Err(RecordError::Missing),
        Err(err) => return Err(RecordError::Malformed(err.to_string())),
    };
    let line = raw
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .ok_or(RecordError::Empty)?;
    parse_address(line).ok_or_else(|| RecordError::Malformed(line.to_string()))
}

/// The outcome of one resolution: the address plus where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    /// The address to use for this one use.
    pub address: String,
    /// Whether it came from the record or from a launch-time capture.
    pub origin: AddressOrigin,
    /// Monotonic counter bumped on every fresh read of the record. Two
    /// resolutions with different generations are different reads.
    pub generation: u64,
}

/// No address could be produced, from the record or from the capture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoAddress {
    /// Why the authoritative record failed.
    pub record: RecordError,
}

impl fmt::Display for NoAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "no service address available: {}", self.record)
    }
}

/// An address source with a cache that failure invalidates.
///
/// The cache exists so a hot loop does not re-read a file per request; it is
/// *not* a snapshot of the service. Any failure calls [`Self::record_failure`],
/// so the next use re-reads the record and follows the service to wherever its
/// record now points.
#[derive(Debug, Clone)]
pub struct AddressResolver {
    record_path: PathBuf,
    launch_arg: Option<String>,
    cached: Option<(String, u64)>,
    generation: u64,
    rereads: u64,
}

impl AddressResolver {
    /// Build a resolver over the record at `record_path`. `launch_arg` is the
    /// value handed to the process at launch (often empty); it is a fallback,
    /// never the primary source.
    pub fn new<P: Into<PathBuf>>(record_path: P, launch_arg: Option<&str>) -> Self {
        Self {
            record_path: record_path.into(),
            launch_arg: launch_arg.and_then(parse_address),
            cached: None,
            generation: 0,
            rereads: 0,
        }
    }

    /// The path of the authoritative record this resolver reads.
    pub fn record_path(&self) -> &Path {
        &self.record_path
    }

    /// Number of fresh reads of the record so far. A long-lived process whose
    /// re-read count never exceeds 1 has cached the address forever.
    pub fn rereads(&self) -> u64 {
        self.rereads
    }

    /// The currently cached address, if any.
    pub fn cached(&self) -> Option<&str> {
        self.cached.as_ref().map(|(url, _)| url.as_str())
    }

    /// Invalidate the cached address: the next [`Self::resolve`] re-reads the
    /// record. Call this on *any* failure to reach the service.
    pub fn record_failure(&mut self) {
        self.cached = None;
    }

    /// Resolve the address for use.
    ///
    /// A valid cached value is returned as-is; otherwise the record is read and
    /// cached. Only when the record cannot supply an address does the
    /// launch-time capture get used, and the resolution names it.
    pub fn resolve(&mut self) -> Result<Resolution, NoAddress> {
        if let Some((address, generation)) = self.cached.clone() {
            return Ok(Resolution {
                address,
                origin: AddressOrigin::AuthoritativeRecord,
                generation,
            });
        }
        match read_record(&self.record_path) {
            Ok(address) => Ok(self.recorded(address)),
            Err(record) => self
                .launch_arg
                .clone()
                .map(|address| self.launch_resolution(address))
                .ok_or(NoAddress { record }),
        }
    }

    /// Wrap the launch-time capture as a resolution, naming its origin.
    fn launch_resolution(&self, address: String) -> Resolution {
        Resolution {
            address,
            origin: AddressOrigin::LaunchArgument,
            generation: self.generation,
        }
    }

    /// Cache a freshly read address as a new generation and return it.
    fn recorded(&mut self, address: String) -> Resolution {
        self.generation += 1;
        self.rereads += 1;
        self.cached = Some((address.clone(), self.generation));
        Resolution {
            address,
            origin: AddressOrigin::AuthoritativeRecord,
            generation: self.generation,
        }
    }

    /// One registration attempt against the resolved address.
    ///
    /// `attempt` performs the request (the caller owns the transport) and
    /// reports what came back. Any failure invalidates the cache first, so an
    /// address that has stopped working is never used twice from a cache — the
    /// next call re-reads the record and follows the service.
    pub fn register<F>(&mut self, mut attempt: F) -> RegistrationOutcome
    where
        F: FnMut(&str) -> RegistrationOutcome,
    {
        let address = match self.resolve() {
            Ok(resolution) => resolution.address,
            Err(_) => {
                self.record_failure();
                return RegistrationOutcome::Unreachable;
            }
        };
        let outcome = attempt(&address);
        if !outcome.succeeded() {
            self.record_failure();
        }
        outcome
    }
}
