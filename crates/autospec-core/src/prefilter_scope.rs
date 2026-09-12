//! A pre-filter narrower than the gate admits what the gate rejects (issue
//! #4489).
//!
//! The conversion pass pre-filters patches before the expensive gate: apply
//! the patch, run clippy, and only batch the survivors. The pre-filter ran
//! `cargo clippy -p autospec-core --all-targets` while the gate ran
//! `cargo clippy --workspace --all-targets`. A patch touching
//! `autospec-cli` therefore passed the pre-filter with `clippy=0` and carried
//! two workspace clippy errors into the batch — and the same patch also
//! broke a test the pre-filter never ran. The batch failed, and isolating
//! the culprit cost three bisect runs plus a re-gate of the survivors,
//! precisely the expense the pre-filter exists to avoid.
//!
//! A pre-filter is a prediction of the gate's verdict. Its value is
//! entirely in that prediction being *conservative*: it may reject
//! something the gate would accept (wasting one patch) but must never
//! accept something the gate rejects, because that is what turns a cheap
//! check into an expensive one. Narrowing scope for speed inverts the error
//! direction: `-p autospec-core` is faster than `--workspace` precisely
//! because it examines less, and what it does not examine is where the
//! false accept comes from.
//!
//! The contract this module makes checkable:
//!
//! 1. **The pre-filter's scope is derived from the patch's touched crates,
//!    or is the full workspace — never a fixed single crate.**
//!    [`crates_touched`] reads the crate set out of the patch's paths and
//!    [`derive_prefilter_scope`] turns it into a [`CheckScope`]: one crate
//!    gets `-p <crate>`, several get all of them, and no resolvable crate
//!    falls back to `--workspace` (the conservative scope, fail-closed).
//!    A fixed single crate cannot be produced by the derivation at all.
//! 2. **A batch failure reports the scope gap.** [`scope_gaps`] compares
//!    each member's recorded pre-filter scope against the gate's scope, and
//!    [`batch_failure_line`] renders the verdict: every member admitted at
//!    a narrower scope than the gate is named, so a later batch failure is
//!    attributed to a scope gap instead of re-diagnosed by bisect.
//! 3. **The pre-filter and the gate share one definition of "the
//!    checks".** [`GATE_CHECKS`] is the single definition;
//!    [`PREFILTER_CHECK_NAMES`] is a subset of it, and [`gate_commands`] /
//!    [`prefilter_commands`] render both from the same [`CheckDef`] at the
//!    same [`CheckScope`]. The two differ only in which checks run, never
//!    in what a check covers.

use std::collections::BTreeSet;
use std::fmt;

/// What a check evaluates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckScope {
    /// The full workspace: `--workspace`.
    Workspace,
    /// A named crate set: `-p <a> -p <b>` (sorted).
    Crates(BTreeSet<String>),
}

impl CheckScope {
    /// The scope tokens for the command line: `--workspace` or
    /// `-p a -p b`.
    pub fn tokens(&self) -> Vec<String> {
        match self {
            Self::Workspace => vec!["--workspace".to_string()],
            Self::Crates(crates) => {
                let mut tokens = Vec::with_capacity(crates.len() * 2);
                for crate_name in crates {
                    tokens.push("-p".to_string());
                    tokens.push(crate_name.clone());
                }
                tokens
            }
        }
    }

    /// Whether `self` evaluates everything `other` evaluates.
    ///
    /// `--workspace` covers every scope; a crate set covers another only
    /// when it is a superset of it.
    pub fn covers(&self, other: &CheckScope) -> bool {
        match (self, other) {
            (Self::Workspace, _) => true,
            (Self::Crates(_), Self::Workspace) => false,
            (Self::Crates(outer), Self::Crates(inner)) => inner.is_subset(outer),
        }
    }

    /// The false-accept direction of issue #4489: whether `self` examines
    /// less than `gate`. A pre-filter scope narrower than the gate's is
    /// where a false accept comes from.
    pub fn is_narrower_than(&self, gate: &CheckScope) -> bool {
        !self.covers(gate)
    }
}

impl fmt::Display for CheckScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.tokens().join(" "))
    }
}

/// The crates a patch touches: every path under `crates/<name>/` (at any
/// depth) claims `<name>`. Paths outside a crate claim nothing.
pub fn crates_touched(paths: &[impl AsRef<str>]) -> BTreeSet<String> {
    let mut crates = BTreeSet::new();
    for path in paths {
        let normalized = path.as_ref().replace('\\', "/");
        let mut parts = normalized.split('/');
        if parts.next() != Some("crates") {
            continue;
        }
        if let Some(name) = parts.next() {
            if !name.is_empty() && name != "." && name != ".." {
                crates.insert(name.to_string());
            }
        }
    }
    crates
}

/// The pre-filter's scope for a patch, derived from the patch's touched
/// crates: the crates themselves, or the full workspace when the patch
/// touches no resolvable crate (fail-closed: an unresolvable patch narrows
/// nothing).
///
/// A fixed single crate is not a possible output: the scope is a function
/// of the patch, and the incident's `-p autospec-core` on an
/// `autospec-cli` patch is exactly what this refuses to derive.
pub fn derive_prefilter_scope(paths: &[impl AsRef<str>]) -> CheckScope {
    let crates = crates_touched(paths);
    if crates.is_empty() {
        CheckScope::Workspace
    } else {
        CheckScope::Crates(crates)
    }
}

/// One definition of a check, shared by the pre-filter and the gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckDef {
    /// The check's name (the join key between gate and pre-filter).
    pub name: &'static str,
    /// The scope-independent command. The first argument is the tool
    /// (`cargo`); the scope tokens are inserted after it by
    /// [`CheckDef::command`], never typed per call site.
    pub program: &'static [&'static str],
    /// Whether the check takes scope tokens (`--workspace` / `-p …`).
    pub scoped: bool,
}

impl CheckDef {
    /// Render the check's command line at the given scope:
    /// `cargo clippy --workspace --all-targets` or
    /// `cargo clippy -p autospec-cli --all-targets`.
    pub fn command(&self, scope: &CheckScope) -> Vec<String> {
        let program = self.program;
        // The scope tokens go after the tool and its subcommand
        // (`cargo clippy --workspace …`, never `cargo --workspace clippy`).
        let mut command: Vec<String> = program[..2].iter().map(|arg| arg.to_string()).collect();
        if self.scoped {
            command.extend(scope.tokens());
        }
        command.extend(program[2..].iter().map(|arg| arg.to_string()));
        command
    }
}

/// The single definition of "the checks" the gate runs. The pre-filter is a
/// subset of this list — so the two cannot disagree about what a check
/// covers.
pub const GATE_CHECKS: &[CheckDef] = &[
    CheckDef {
        name: "fmt",
        program: &["cargo", "fmt", "--all", "--check"],
        scoped: false,
    },
    CheckDef {
        name: "clippy",
        program: &["cargo", "clippy", "--all-targets"],
        scoped: true,
    },
    CheckDef {
        name: "test",
        program: &["cargo", "test", "--no-fail-fast"],
        scoped: true,
    },
];

/// The checks the pre-filter runs: a subset of [`GATE_CHECKS`] by name.
/// Speed comes from running fewer *checks*, never from narrowing the
/// *scope* of a check.
pub const PREFILTER_CHECK_NAMES: &[&str] = &["clippy"];

/// The gate's full check list at the given scope.
pub fn gate_commands(scope: &CheckScope) -> Vec<Vec<String>> {
    GATE_CHECKS
        .iter()
        .map(|check| check.command(scope))
        .collect()
}

/// The pre-filter's check list at the given scope: the subset of
/// [`GATE_CHECKS`] named by [`PREFILTER_CHECK_NAMES`], rendered from the
/// same [`CheckDef`] the gate uses.
pub fn prefilter_commands(scope: &CheckScope) -> Vec<Vec<String>> {
    PREFILTER_CHECK_NAMES
        .iter()
        .map(|name| {
            GATE_CHECKS
                .iter()
                .find(|check| check.name == *name)
                .unwrap_or_else(|| {
                    panic!("PREFILTER_CHECK_NAMES names {name}, which is not in GATE_CHECKS")
                })
                .command(scope)
        })
        .collect()
}

/// One member of a conversion batch, with the scope its pre-filter
/// recorded. The record is what makes a later batch failure attributable
/// instead of re-diagnosed from scratch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchMember {
    /// The member's patch (or issue) identifier.
    pub patch: String,
    /// The scope the pre-filter actually ran at, as recorded.
    pub recorded_scope: CheckScope,
}

/// A pre-filter scope gap: a member admitted at a scope narrower than the
/// gate's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeGap {
    /// The member's patch (or issue) identifier.
    pub patch: String,
    /// The scope the member passed the pre-filter at.
    pub recorded_scope: CheckScope,
}

/// The members of a failed batch that passed the pre-filter at a scope
/// narrower than the gate's, in batch order.
pub fn scope_gaps(batch: &[BatchMember], gate: &CheckScope) -> Vec<ScopeGap> {
    batch
        .iter()
        .filter(|member| member.recorded_scope.is_narrower_than(gate))
        .map(|member| ScopeGap {
            patch: member.patch.clone(),
            recorded_scope: member.recorded_scope.clone(),
        })
        .collect()
}

/// Whether any member of the batch passed the pre-filter at a narrower
/// scope than the gate.
pub fn has_scope_gap(batch: &[BatchMember], gate: &CheckScope) -> bool {
    !scope_gaps(batch, gate).is_empty()
}

/// The batch-failure report line. Names the gate's scope always, and every
/// member admitted at a narrower scope than the gate — so the failure is
/// attributed to a scope gap (or the gap is explicitly ruled out) instead
/// of re-diagnosed by bisect.
pub fn batch_failure_line(batch: &[BatchMember], gate: &CheckScope) -> String {
    let gaps = scope_gaps(batch, gate);
    if gaps.is_empty() {
        return format!(
            "batch failure: no member passed the pre-filter at a narrower scope than the gate ({gate})"
        );
    }
    let named = gaps
        .iter()
        .map(|gap| format!("{}: {} < {gate}", gap.patch, gap.recorded_scope))
        .collect::<Vec<_>>()
        .join("; ");
    format!(
        "batch failure: {} of {} members passed the pre-filter at a narrower scope than the gate ({gate}): {named} — attributed to a scope gap",
        gaps.len(),
        batch.len()
    )
}
