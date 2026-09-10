//! One canonical model name across two registries, a scale ratchet that
//! cannot ratchet against dying workers, and a startup line that says which
//! registry refused (issue #3664).
//!
//! The incident: the fleet registry (`models.tsv`) named the model
//! `qwen3.8-27b`; the serving registry (pick-config) knew only
//! `qwen3.8-27b-q8`. The mapping between them was a `contains` check, which
//! is untestable and wrong for anything that does not share a prefix. When
//! pick-config was regenerated without the suffixed name, every startup died
//! on a bare "unknown model": no line said which registry refused, and the
//! autoscaler read the dead workers as a capacity deficit and kept raising
//! the desired count (5 → 6 → 7). Each raised worker died the same way. The
//! fleet shrank to 1 while the autoscaler demanded 7.
//!
//! Four invariants close that loop:
//!
//! 1. **One name, enforced.** The canonical name is the fleet registry's
//!    base name; the serving name is *derived* from it with the
//!    quantization column of the same row
//!    ([`FleetModelEntry::serving_name`]). There is no substring fallback:
//!    a name the bridge does not derive is unknown, full stop.
//!    [`Registries::audit`] reports every model the two registries name
//!    differently, so a pick-config regeneration that drops a suffixed name
//!    is a loud audit finding, not a silent startup death.
//! 2. **A lookup miss on a known-good canonical name is its own error.**
//!    [`ResolveError::RegistryMismatch`] — the model is in the fleet
//!    registry but the serving registry does not know its derived name —
//!    is a distinct variant from [`ResolveError::Unknown`], so an operator
//!    can tell a typo'd request apart from a broken registry.
//! 3. **The autoscaler may not ratchet against dying workers.**
//!    [`ScaleRatchet::tick`] raises the desired count only when the deficit
//!    is strictly shrinking from the previous reading. A flat or growing
//!    deficit holds the count and raises an alarm instead — the exact
//!    5 → 6 → 7 pattern from the incident is now a hold with an alarm.
//! 4. **Startup says which registry refused.**
//!    [`StartupRejection::log_line`] renders a single line naming the
//!    refusing registry, the model, and the reason.
//!
//! Everything here is pure in-memory state — no I/O, no clock, no
//! subprocess — so the node-repo shell entry points (worker.sh,
//! pick-config.py) can adopt these types as their source of truth without
//! changing what they observe.

use std::collections::{BTreeMap, BTreeSet};

/// One row of the fleet registry (`models.tsv`).
///
/// The `canonical` base name is the single name the fleet uses; `quant` is
/// the row's quantization column and the only thing that may differ between
/// the two registries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FleetModelEntry {
    pub canonical: String,
    pub quant: String,
    pub vram_mib: u64,
}

impl FleetModelEntry {
    /// The serving name pick-config must know for this row: the canonical
    /// name plus the row's quantization, joined by `-`. Derivation, not
    /// matching: there is no `contains` fallback anywhere in this module.
    pub fn serving_name(&self) -> String {
        format!("{}-{}", self.canonical, self.quant)
    }
}

fn reject_component(component: &str, what: &str) -> Result<(), String> {
    if component.is_empty() || component.chars().any(|c| c.is_whitespace()) {
        return Err(format!(
            "fleet registry: {what} must be non-empty without whitespace"
        ));
    }
    Ok(())
}

/// The fleet registry: the models the fleet is allowed to run, keyed by
/// canonical name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FleetRegistry {
    entries: BTreeMap<String, FleetModelEntry>,
}

impl FleetRegistry {
    /// Build from explicit rows. Fails closed on an invalid or duplicate
    /// canonical name — a duplicate means two rows would derive competing
    /// serving names, which is exactly the ambiguity this module removes.
    pub fn new(entries: Vec<FleetModelEntry>) -> Result<Self, String> {
        let mut map = BTreeMap::new();
        for entry in entries {
            reject_component(&entry.canonical, "canonical name")?;
            reject_component(&entry.quant, "quantization")?;
            if entry.vram_mib == 0 {
                return Err(format!(
                    "fleet registry: model {} must declare a non-zero vram_mib",
                    entry.canonical
                ));
            }
            if map.contains_key(&entry.canonical) {
                return Err(format!(
                    "fleet registry: duplicate canonical model name {}",
                    entry.canonical
                ));
            }
            map.insert(entry.canonical.clone(), entry);
        }
        Ok(Self { entries: map })
    }

    /// Parse `models.tsv`: tab-separated `canonical  quant  vram_mib` rows.
    /// Blank lines and `#` comments are skipped; anything else that is not
    /// exactly three valid columns is an error.
    pub fn parse_tsv(text: &str) -> Result<Self, String> {
        let mut entries = Vec::new();
        for (line_no, line) in text.lines().enumerate() {
            let line = line.trim_end();
            if line.is_empty() || line.trim_start().starts_with('#') {
                continue;
            }
            let columns = line.split('\t').map(str::trim).collect::<Vec<_>>();
            if columns.len() != 3 {
                return Err(format!(
                    "models.tsv line {}: expected 3 tab-separated columns, found {}",
                    line_no + 1,
                    columns.len()
                ));
            }
            let vram = columns[2].parse::<u64>().map_err(|_| {
                format!(
                    "models.tsv line {}: vram_mib is not a number: {:?}",
                    line_no + 1,
                    columns[2]
                )
            })?;
            entries.push(FleetModelEntry {
                canonical: columns[0].to_string(),
                quant: columns[1].to_string(),
                vram_mib: vram,
            });
        }
        Self::new(entries)
    }

    pub fn get(&self, canonical: &str) -> Option<&FleetModelEntry> {
        self.entries.get(canonical)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn entries(&self) -> impl Iterator<Item = &FleetModelEntry> {
        self.entries.values()
    }
}

/// The serving registry: the names pick-config accepts at startup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServingRegistry {
    names: BTreeSet<String>,
}

impl ServingRegistry {
    pub fn new(names: Vec<String>) -> Result<Self, String> {
        let mut set = BTreeSet::new();
        for name in names {
            reject_component(&name, "serving name")?;
            set.insert(name);
        }
        Ok(Self { names: set })
    }

    pub fn knows(&self, serving: &str) -> bool {
        self.names.contains(serving)
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    pub fn names(&self) -> impl Iterator<Item = &String> {
        self.names.iter()
    }
}

/// Both registries together — the unit a startup decision and an audit run
/// against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registries {
    pub fleet: FleetRegistry,
    pub serving: ServingRegistry,
}

/// Which registry refused a lookup or a startup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefusingRegistry {
    Fleet,
    Serving,
}

impl RefusingRegistry {
    pub fn as_str(self) -> &'static str {
        match self {
            RefusingRegistry::Fleet => "fleet registry (models.tsv)",
            RefusingRegistry::Serving => "serving registry (pick-config)",
        }
    }
}

/// A lookup miss, distinguished by *which* registry refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    /// The name is not in the fleet registry: a request the fleet does not
    /// run at all (a typo, or a retired model).
    Unknown { name: String },
    /// The fleet registry knows the model and derives a serving name for
    /// it, but the serving registry does not know that name. A known-good
    /// model refused at startup is a registry defect, not a bad request,
    /// and it must fail as one.
    RegistryMismatch { canonical: String, serving: String },
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResolveError::Unknown { name } => {
                write!(f, "model {name} is not in the fleet registry (models.tsv)")
            }
            ResolveError::RegistryMismatch { canonical, serving } => write!(
                f,
                "model {canonical} is in the fleet registry with derived serving name \
                 {serving}, but the serving registry (pick-config) does not know it"
            ),
        }
    }
}

/// A model resolved against both registries: safe to launch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedModel {
    pub entry: FleetModelEntry,
    pub serving: String,
}

impl Registries {
    /// Resolve a canonical name to the serving name the launch must use.
    ///
    /// Exact-key lookup in both registries only. A name that is a prefix,
    /// a case variant, or a substring of a known name does not resolve.
    pub fn resolve(&self, canonical: &str) -> Result<ResolvedModel, ResolveError> {
        let entry = self
            .fleet
            .get(canonical)
            .ok_or_else(|| ResolveError::Unknown {
                name: canonical.to_string(),
            })?;
        let serving = entry.serving_name();
        if !self.serving.knows(&serving) {
            return Err(ResolveError::RegistryMismatch {
                canonical: entry.canonical.clone(),
                serving,
            });
        }
        Ok(ResolvedModel {
            entry: entry.clone(),
            serving,
        })
    }

    /// Every model the two registries name differently. This is the check a
    /// pick-config regeneration must pass before it ships: any
    /// [`AuditFinding::ServingNameMissing`] is a model whose startup would
    /// die the way the incident's did.
    pub fn audit(&self) -> Vec<AuditFinding> {
        let mut findings = Vec::new();
        for entry in self.fleet.entries() {
            let serving = entry.serving_name();
            if !self.serving.knows(&serving) {
                findings.push(AuditFinding::ServingNameMissing {
                    canonical: entry.canonical.clone(),
                    serving,
                });
            }
        }
        for name in self.serving.names() {
            if !self
                .fleet
                .entries()
                .any(|entry| entry.serving_name() == *name)
            {
                findings.push(AuditFinding::OrphanServingName {
                    serving: name.clone(),
                });
            }
        }
        findings
    }
}

/// One registry disagreement found by [`Registries::audit`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuditFinding {
    /// The fleet registry lists the model; the serving registry does not
    /// know its derived serving name. Startup for this model will die.
    ServingNameMissing { canonical: String, serving: String },
    /// The serving registry knows a name that no fleet row derives.
    OrphanServingName { serving: String },
}

impl AuditFinding {
    /// Only a missing serving name blocks: an orphan is stale inventory,
    /// a missing name is a fleet that cannot start.
    pub fn is_blocking(&self) -> bool {
        matches!(self, AuditFinding::ServingNameMissing { .. })
    }
}

/// The startup path's single line for a refused launch.
///
/// The incident's silence came from the refusal not being attributed to a
/// registry at all; this type makes attribution part of the data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupRejection {
    pub registry: RefusingRegistry,
    pub model: String,
    pub reason: String,
}

impl StartupRejection {
    /// Build the rejection from a resolve error: the error already knows
    /// which registry it came from.
    pub fn from_resolve(requested: &str, error: &ResolveError) -> Self {
        match error {
            ResolveError::Unknown { .. } => Self {
                registry: RefusingRegistry::Fleet,
                model: requested.to_string(),
                reason: format!("not in the fleet registry (models.tsv): {error}"),
            },
            ResolveError::RegistryMismatch { .. } => Self {
                registry: RefusingRegistry::Serving,
                model: requested.to_string(),
                reason: error.to_string(),
            },
        }
    }

    /// One line, no embedded newline: the refusal, attributed to the
    /// registry that made it.
    pub fn log_line(&self) -> String {
        format!(
            "worker start: model `{}` refused by {}: {}",
            self.model,
            self.registry.as_str(),
            self.reason
        )
    }
}

/// The autoscaler's desired-count ratchet.
///
/// The ratchet owns the desired count and one previous deficit reading. It
/// never lowers the count (pruning is a separate loop, see
/// [`crate::repair_loop`] for the staging-grace half of that fight).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScaleRatchet {
    desired: u32,
    last_deficit: Option<u32>,
    alarm_count: u32,
}

/// What a scaler tick decided to do with the desired count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScaleAction {
    /// The deficit is strictly shrinking: the fleet is absorbing capacity,
    /// so one more requested worker is grounded.
    Raise { next_desired: u32 },
    /// No raise this tick. `alarm` is set when a raise was wanted but
    /// blocked by a flat or growing deficit — the incident's pattern.
    Hold { deficit: u32, alarm: bool },
}

impl ScaleRatchet {
    pub fn new(desired: u32) -> Self {
        Self {
            desired,
            last_deficit: None,
            alarm_count: 0,
        }
    }

    pub fn desired(&self) -> u32 {
        self.desired
    }

    /// How many times a raise has been blocked and alarmed.
    pub fn alarm_count(&self) -> u32 {
        self.alarm_count
    }

    /// One scaler tick. `running` is the fleet's live count.
    ///
    /// The deficit is `desired - running`. A raise is permitted only when
    /// the deficit is strictly smaller than the previous reading — the
    /// first reading has no previous, so it may raise once. A flat or
    /// growing deficit means new workers are not coming up (the incident:
    /// every raised worker died at startup), so the ratchet holds and
    /// alarms instead of ratcheting.
    pub fn tick(&mut self, running: u32) -> ScaleAction {
        let deficit = self.desired.saturating_sub(running);
        let action = if deficit == 0 {
            ScaleAction::Hold {
                deficit,
                alarm: false,
            }
        } else if self.last_deficit.is_none_or(|prev| deficit < prev) {
            self.desired = self.desired.saturating_add(1);
            ScaleAction::Raise {
                next_desired: self.desired,
            }
        } else {
            self.alarm_count = self.alarm_count.saturating_add(1);
            ScaleAction::Hold {
                deficit,
                alarm: true,
            }
        };
        self.last_deficit = Some(deficit);
        action
    }
}
