use std::io;
use std::path::{Path, PathBuf};

mod cache;
mod lexicon;
mod model;
mod roster_json;
mod scan;
mod strict;

pub use model::{DetectedDomain, FileLineEvidence, SpecialistRoster, SuggestedSpecialist};
pub use strict::{
    collect_strict_domains, StrictCollectorError, StrictCollectorErrorCode,
    StrictCollectorEvidence, StrictCollectorOptions,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanOptions {
    pub repo_dir: PathBuf,
    pub num_specialists: usize,
    pub force: bool,
}

impl ScanOptions {
    pub fn new(repo_dir: impl AsRef<Path>) -> Self {
        Self {
            repo_dir: repo_dir.as_ref().to_path_buf(),
            num_specialists: 3,
            force: false,
        }
    }

    pub fn with_num_specialists(mut self, num_specialists: usize) -> Self {
        self.num_specialists = num_specialists.min(6);
        self
    }

    pub fn force(mut self, force: bool) -> Self {
        self.force = force;
        self
    }
}

pub fn scan_specialists(options: &ScanOptions) -> io::Result<SpecialistRoster> {
    Ok(load_or_derive(options)?.capped(options.num_specialists))
}

pub fn scan_specialists_json(options: &ScanOptions) -> io::Result<String> {
    Ok(scan_specialists(options)?.to_json_pretty())
}

fn load_or_derive(options: &ScanOptions) -> io::Result<SpecialistRoster> {
    if !options.force {
        if let Some(roster) = cache::load(&options.repo_dir)? {
            return Ok(roster);
        }
    }

    let proposal_input = std::env::var("AUTOSPEC_SPECIALIST_LLM_STUB_OUTPUT").ok();
    let roster = scan::derive_roster(
        &options.repo_dir,
        proposal_input.as_deref(),
        options.num_specialists,
    );
    cache::store(&options.repo_dir, &roster)?;
    Ok(roster)
}
