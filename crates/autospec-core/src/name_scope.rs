//! Name scope and collision detection (issue #4251).
//!
//! In one session, four name collisions each cost real time, and none of
//! them would have been caught by a test:
//!
//! 1. The glob `qwen3.8-27b-*` also matched `qwen3.8-27b-vision-*`: the
//!    vision workers were counted as `qwen3.8-27b` workers, and the fleet
//!    was reported as 6 workers/24 slots when it was 5/20.
//! 2. `$LLM/*/out/issue-*` spanned four projects (`autospec`, `iw`, `disp`,
//!    `orch`) and the issue numbers collide across them.
//! 3. Two gateway *instances* sat on one request path (edge + hive): an
//!    hour went into proving the healthy one healthy while the other was
//!    emitting the user's 503s.
//! 4. Two gateway *components* lived in two repos: `InferWeave/inferweave`'s
//!    unimplemented Rust gateway and `metabolomics-us/inferweave-gateway`'s
//!    production Go service. A terminal full of evidence about a different
//!    program of the same name sat one command away from wrongly closing
//!    the correct issue.
//!
//! The rule: **an identifier is only meaningful inside the scope that
//! issued it, and a bare noun is not a scope.**
//!
//! The primitives make each detection a checkable invariant:
//!
//! 1. **Anchor prefix matches at the boundary you mean.**
//!    [`glob_matches`] / [`prefix_collisions`]: a glob whose literal part
//!    is a component-boundary prefix of a *different* known identifier
//!    selects two things when it meant one.
//! 2. **Qualify the identifier when it crosses a boundary.**
//!    [`ScopedId`] / [`parse_scoped_id`] / [`qualify`] / [`key_findings`]:
//!    a patch is `InferWeave#14`, never "issue 14"; a pipeline moving work
//!    between scopes must attach the scope at read time.
//! 3. **A bare name in two kinds is probably two things.**
//!    [`cross_kind_collisions`]: a component name that appears as a
//!    directory, a crate, a service, *and* a repository name — one of them
//!    is probably a different thing.
//! 4. **State which instance produced the evidence before carrying it.**
//!    [`CarriedClaim`]: "the gateway reports 10 workers" is not a fact
//!    until it says *which* gateway.
//! 5. **A spec that names a component states where else that name is
//!    used.** [`unstated_name_uses`]: a spec entry for a name with known
//!    other uses and an empty other-uses list is a finding. And a spec
//!    that moves records between systems names the scope-qualified
//!    identifier: [`SpecMove`].

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

// --- Rule 1: anchor prefix matches at the boundary you mean --------------

/// The separator characters that mark a component-name boundary. A name
/// that continues past another name at one of these characters is a
/// *different* component (`qwen3.8-27b` vs `qwen3.8-27b-vision`); a
/// continuation at any other character is a sibling of the same family
/// (`issue-1` vs `issue-14` — different issues, not a collision).
pub const NAME_SEPARATORS: &[char] = &['-', '_', '/', '.'];

/// Glob matching over `*` (any run, including empty) and `?` (exactly one
/// character). No other metacharacters are interpreted; `[` and friends
/// are literals, so a pattern is never *looser* than the reader expects.
pub fn glob_matches(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    let (mut pi, mut ni) = (0usize, 0usize);
    let (mut star, mut mark) = (usize::MAX, 0usize);
    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = pi;
            mark = ni;
            pi += 1;
        } else if star != usize::MAX {
            pi = star + 1;
            mark += 1;
            ni = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// The leading run of a glob free of metacharacters: the part the pattern
/// asserts literally. `qwen3.8-27b-*` asserts `qwen3.8-27b-`; `$LLM/*/out/issue-*`
/// asserts `$LLM/`.
pub fn glob_literal_prefix(pattern: &str) -> &str {
    pattern.split(['*', '?']).next().unwrap_or("")
}

/// A glob that selected two *different* things when it meant one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrefixCollision {
    /// The glob as written.
    pub glob: String,
    /// The identifier the glob was probably meant to select (the shorter
    /// name).
    pub shorter: String,
    /// A different known identifier the glob also selects: it continues
    /// the shorter name at a component-name boundary.
    pub longer: String,
}

/// Detect globs whose literal part can reach a known identifier that
/// continues a *different* known identifier at a component-name boundary.
///
/// The incident shape: the known set holds two component names, one a
/// boundary-extension of the other (`qwen3.8-27b` and
/// `qwen3.8-27b-vision`), and the glob was written for the shorter one:
/// `qwen3.8-27b-*`. The glob's literal part (`qwen3.8-27b-`) sits in the
/// shorter name's territory and reaches into the longer name's, so the
/// glob selects both families when it meant one.
///
/// Concretely, for each pair of known identifiers `(A, B)` where `B`
/// extends `A` at a [`NAME_SEPARATORS`] boundary, a glob collides when
/// its literal part `L` satisfies:
///
/// - `B` starts with `L` — the glob can reach `B`'s name; and
/// - `A` and `L` are in a prefix relation (either one is a prefix of the
///   other) — the glob's literal part sits in `A`'s name territory, i.e.
///   it was written against `A`, not `B`.
///
/// `issue-1` vs `issue-14` is never reported — the continuation is a
/// digit, not a boundary — and a glob written for the longer name
/// (`qwen3.8-27b-vision-*`) is clean: its literal part is longer than
/// `B`, so it cannot reach `B`'s name and selects only the longer
/// family. A glob whose literal part is empty (`*`) asserts nothing and
/// is not anchored to a name, so it is not reported.
///
/// Prefer reading fields from files over inferring structure from names
/// whenever the file has a field to read; this detector is for the globs
/// that must exist.
pub fn prefix_collisions(
    globs: &[String],
    known: &[String],
    separators: &[char],
) -> Vec<PrefixCollision> {
    // Boundary-extension pairs in the known set: (a, b) where b continues
    // a at a component-name boundary.
    let mut known_sorted = known.to_vec();
    known_sorted.sort();
    known_sorted.dedup();
    let mut pairs: Vec<(String, String)> = Vec::new();
    for a in &known_sorted {
        for b in &known_sorted {
            if a.is_empty() || a == b {
                continue;
            }
            if let Some(rest) = b.strip_prefix(a.as_str()) {
                if !rest.is_empty() && separators.iter().any(|s| rest.starts_with(*s)) {
                    pairs.push((a.clone(), b.clone()));
                }
            }
        }
    }

    let mut out = Vec::new();
    for glob in globs {
        let lit = glob_literal_prefix(glob);
        if lit.is_empty() {
            continue;
        }
        for (a, b) in &pairs {
            if b.starts_with(lit) && (a.starts_with(lit) || lit.starts_with(a.as_str())) {
                out.push(PrefixCollision {
                    glob: glob.clone(),
                    shorter: a.clone(),
                    longer: b.clone(),
                });
            }
        }
    }
    out
}

// --- Rule 2: qualify the identifier when it crosses a boundary -----------

/// An identifier qualified by the scope that issued it: `InferWeave#14`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopedId {
    /// The scope that issued the identifier (repository, project,
    /// instance). A bare noun is not a scope.
    pub scope: String,
    /// The number within that scope.
    pub number: u64,
}

impl ScopedId {
    pub fn new(scope: impl Into<String>, number: u64) -> Self {
        Self {
            scope: scope.into(),
            number,
        }
    }

    /// Render as `Scope#14`.
    pub fn render(&self) -> String {
        format!("{}#{}", self.scope, self.number)
    }
}

/// Parse a scope-qualified identifier (`InferWeave#14`).
///
/// Exactly one `#`, a non-empty scope, and an all-digit number. Anything
/// else — `issue 14`, `14`, `#14`, `InferWeave#` — is rejected: a parse
/// failure here is the pipeline noticing it is about to key on something
/// it cannot qualify.
pub fn parse_scoped_id(s: &str) -> Result<ScopedId, String> {
    let (scope, number) = s
        .split_once('#')
        .ok_or_else(|| format!("{s:?}: no '#' — a bare identifier is not scope-qualified"))?;
    if scope.is_empty() {
        return Err(format!("{s:?}: empty scope — a bare noun is not a scope"));
    }
    if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!("{s:?}: number after '#' must be all digits"));
    }
    let number = number
        .parse::<u64>()
        .map_err(|e| format!("{s:?}: number does not fit: {e}"))?;
    Ok(ScopedId {
        scope: scope.to_string(),
        number,
    })
}

/// Qualify a number under a scope: `("InferWeave", 14)` → `InferWeave#14`.
pub fn qualify(scope: &str, number: u64) -> String {
    ScopedId::new(scope, number).render()
}

/// True if the string is a bare integer — the key shape that collides
/// across scopes.
pub fn is_bare_number(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// A record a pipeline moves between systems.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    /// The scope the identifier was read from (repository, project,
    /// instance). `None` = the record was read with a bare key and the
    /// scope was not attached at read time.
    pub scope: Option<String>,
    /// The identifier within that scope.
    pub number: u64,
}

/// What is wrong with how a pipeline keys its records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeyFinding {
    /// A record moved by a multi-scope pipeline with no scope attached at
    /// read time: it is keyed on the bare number, and the pipeline will
    /// have to re-attach the scope later by convention.
    Unscoped { number: u64 },
    /// The same number exists under two or more distinct scopes: keyed on
    /// the bare number, these records are indistinguishable.
    CollidingNumber {
        number: u64,
        /// The scopes the number exists under, sorted.
        scopes: Vec<String>,
    },
}

/// Check a pipeline's records against the scoping rule.
///
/// Findings apply once the pipeline is multi-scope (two or more distinct
/// scopes appear in the records). A single-scope pipeline keyed on bare
/// numbers is unambiguous — "the gateway" is the gateway in one
/// conversation — and is not flagged.
pub fn key_findings(records: &[Record]) -> Vec<KeyFinding> {
    let scopes: BTreeSet<String> = records.iter().filter_map(|r| r.scope.clone()).collect();
    if scopes.len() < 2 {
        return Vec::new();
    }

    let mut findings: Vec<KeyFinding> = records
        .iter()
        .filter(|r| r.scope.is_none())
        .map(|r| KeyFinding::Unscoped { number: r.number })
        .collect();

    let by_number: std::collections::BTreeMap<u64, BTreeSet<String>> = records
        .iter()
        .filter_map(|r| r.scope.clone().map(|s| (r.number, s)))
        .fold(std::collections::BTreeMap::new(), |mut m, (n, s)| {
            m.entry(n).or_default().insert(s);
            m
        });
    for (number, scopes) in by_number {
        if scopes.len() >= 2 {
            findings.push(KeyFinding::CollidingNumber {
                number,
                scopes: scopes.into_iter().collect(),
            });
        }
    }
    findings.sort_by_key(|f| match f {
        KeyFinding::Unscoped { number } => (0u8, *number),
        KeyFinding::CollidingNumber { number, .. } => (1, *number),
    });
    findings
}

// --- Rule 3: a bare name in two kinds is probably two things -------------

/// The kind of system object a name is used for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum NameKind {
    Directory,
    Crate,
    Service,
    Repository,
}

/// A name as it is used for one kind of object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamedThing {
    pub name: String,
    pub kind: NameKind,
}

/// A bare name in use for two or more kinds of object: one of them is
/// probably a different thing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrossKindCollision {
    /// The bare name shared across kinds.
    pub name: String,
    /// The kinds it is used for, sorted; two or more.
    pub kinds: Vec<NameKind>,
}

/// Detect names used for two or more kinds of system object.
///
/// The incident form: a component name that appears as a directory, a
/// crate, a service, *and* a repository name — one of them is probably a
/// different thing. Two kinds is the sensitivity: the detector flags, it
/// does not judge, and renaming (`iw-gateway` / `edge-gateway`) is cheaper
/// than the investigations the shared name keeps causing.
pub fn cross_kind_collisions(things: &[NamedThing]) -> Vec<CrossKindCollision> {
    let mut by_name: std::collections::BTreeMap<String, BTreeSet<NameKind>> =
        std::collections::BTreeMap::new();
    for t in things {
        by_name.entry(t.name.clone()).or_default().insert(t.kind);
    }
    by_name
        .into_iter()
        .filter_map(|(name, kinds)| {
            (kinds.len() >= 2).then(|| CrossKindCollision {
                name,
                kinds: kinds.into_iter().collect(),
            })
        })
        .collect()
}

// --- Rule 4: state which instance produced the evidence -------------------

/// Evidence carried from the instance that produced it to a claim about a
/// component.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CarriedClaim {
    /// The instance that produced the evidence. `None` = nothing names
    /// the producer.
    pub produced_by: Option<String>,
    /// The component the claim is about.
    pub applied_to: String,
}

/// Whether a claim is a fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClaimVerdict {
    /// The claim is about the instance that produced the evidence.
    Local,
    /// Evidence carried from another instance, with the producing
    /// instance named: still a fact, and the reader can see the hop.
    Attributed,
    /// Nothing names which instance produced the evidence behind a claim
    /// about a component: not a fact. "The gateway reports 10 workers" is
    /// not a fact until it says *which* gateway.
    Unattributed,
}

impl CarriedClaim {
    pub fn verdict(&self) -> ClaimVerdict {
        match &self.produced_by {
            Some(p) if p == &self.applied_to => ClaimVerdict::Local,
            Some(_) => ClaimVerdict::Attributed,
            None => ClaimVerdict::Unattributed,
        }
    }
}

// --- Rule 5: specs state where names are used and how records move --------

/// A component a spec names, together with where else that name is used,
/// as the spec states it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpecName {
    /// The component name the spec uses.
    pub name: String,
    /// Where else that name is used: other repositories, other
    /// deployments, other instances of the same service.
    pub other_uses: Vec<String>,
}

/// A spec entry that failed to state a known other use of the name it
/// names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnstatedUses {
    /// The component name the spec uses.
    pub name: String,
    /// Known other uses the spec did not state, sorted.
    pub missing: Vec<String>,
}

/// Check a spec's component names against the system-wide registry of
/// where each name is used.
///
/// A spec that names a component must state where else that name is used —
/// other repositories, other deployments, other instances of the same
/// service. A spec entry for a name that has known other uses, with those
/// uses unstated, is the exact shape of the "wrong program of the same
/// name" incident: the reader cannot know the spec is about *this*
/// gateway and not the other one.
pub fn unstated_name_uses(
    spec: &[SpecName],
    known_uses: &std::collections::BTreeMap<String, Vec<String>>,
) -> Vec<UnstatedUses> {
    spec.iter()
        .filter_map(|entry| {
            let known = known_uses.get(&entry.name)?;
            let missing: Vec<String> = known
                .iter()
                .filter(|use_| !entry.other_uses.contains(use_))
                .cloned()
                .collect();
            (!missing.is_empty()).then(|| UnstatedUses {
                name: entry.name.clone(),
                missing,
            })
        })
        .collect()
}

/// A record-move step declared in a spec.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpecMove {
    /// The identifier the spec names for the records it moves.
    pub identifier: String,
    /// The systems (scopes) the move crosses.
    pub systems: Vec<String>,
}

/// Whether a spec's record-move naming is safe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpecMoveVerdict {
    /// The move stays inside one system: a bare identifier is
    /// unambiguous there.
    SingleSystem,
    /// The move crosses systems and names the scope-qualified
    /// identifier: the implementation cannot key on the bare number
    /// without noticing.
    Qualified,
    /// The move crosses systems but names a bare identifier: the
    /// implementation can — and will — key on the bare number, and the
    /// records of different systems become indistinguishable.
    BareKey,
}

impl SpecMove {
    pub fn verdict(&self) -> SpecMoveVerdict {
        if self.systems.len() < 2 {
            return SpecMoveVerdict::SingleSystem;
        }
        match parse_scoped_id(&self.identifier) {
            Ok(_) => SpecMoveVerdict::Qualified,
            Err(_) => SpecMoveVerdict::BareKey,
        }
    }
}
