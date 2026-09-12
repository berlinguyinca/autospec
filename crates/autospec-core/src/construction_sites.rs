//! A rule that only a build enforces is one platform's view (issue #4348).
//!
//! Two cycles after filing #4312 ("a red platform job is the sole verification
//! of the code behind its `#[cfg]`"), the same author converted #4312: added a
//! field to `PersistedInvocation`, updated every initializer a Linux
//! `cargo build --workspace --all-targets` could see, and merged. One
//! initializer lives behind
//! `#[cfg(any(target_os = "macos", target_os = "freebsd", windows))]`. The
//! next run, `macos-test` and `freebsd-test` went red while `main-builds`
//! stayed green — precisely the shape the #4312 rule describes. The rule was
//! correct, recently written, and being actively worked on. **It still did not
//! fire**, because nothing in the workflow asks the question; the rule lives
//! in a document, and a document does not fire.
//!
//! The two cheap things that would have caught it, both reached for only
//! afterwards:
//!
//! - **Search by text, not by compiling.** `grep 'PersistedInvocation {'`
//!   finds every literal regardless of which platform a module is gated to. A
//!   compiler sees only this host's configuration; a text search has no
//!   configuration.
//! - **Temporarily widen the `cfg` and build.** Adding the host to the gate,
//!   building, then restoring compiles the gated module in seconds.
//!
//! Four invariants, each a primitive here:
//!
//! 1. **A rule that matters must become a step, a check, or a prompt — not a
//!    document entry.** "Documented" and "enforced" are different states, and
//!    the gap is invisible until something slips through it. This module is
//!    the check: it does not restate the rule, it decides from the
//!    construction sites.
//! 2. **When a change adds a field to a shared struct, enumerate its
//!    construction sites by text before trusting a build.** The build's view
//!    is one platform's; the text's view is all of them. [`invisible_to_host`]
//!    is the difference between those two views: the sites the local build on
//!    `host` cannot compile — exactly where a missed field-initializer hides.
//! 3. **Where a `cfg`-gated module exists, there must be a supported way to
//!    check it locally.** The hold line names both remedies — the text search
//!    and the temporary-widen-and-build — so the check is also the procedure.
//! 4. **When a rule you wrote is violated, the rule is not the problem — its
//!    absence from the workflow is.** The response is the check, not a
//!    resolution to remember harder. This module is that response.
//!
//! The sites are supplied by the caller's text search (every `Struct {`
//! literal in the tree, each tagged with the `#[cfg(...)]` that compiles it),
//! not read from the patch: the missed site's gate is not *in* the patch
//! (the site is unchanged and far from any edited line), which is the whole
//! reason the build — which sees the patch and one platform at a time —
//! cannot find it.
//!
//! The evaluator claims only what it can classify, the way
//! [`platform_gate`](crate::platform_gate) does: a predicate classifies to a
//! platform set only from platform terms and their `any`/`all`/`not`
//! compositions. A term that is not a platform predicate (a feature flag,
//! `target_arch`, `test`, an unknown value) is platform-neutral — it does not
//! restrict or extend the platform set — so `any`/`all` skip it, and a
//! composition with **no** platform term determines no platform set and
//! returns [`None`]. The safe direction is over-flagging: a needless hold,
//! never a missed site.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::platform_gate::{parse_platform_predicate, Platform};

/// Every platform in the known set, in `Ord` order.
const ALL_PLATFORMS: [Platform; 4] = [
    Platform::Linux,
    Platform::Macos,
    Platform::Windows,
    Platform::FreeBSD,
];

/// One construction site of a struct literal, as found by a text search (the
/// grep a build cannot do). The build sees only the sites for the host
/// platform; the text sees all of them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConstructionSite {
    /// Where the site is, for the hold line (e.g. `src/foo.rs:51`).
    pub location: String,
    /// The predicate of the `#[cfg(...)]` gate that compiles the site, or
    /// `None` when the site is ungated (it compiles on every platform).
    pub cfg: Option<String>,
}

impl ConstructionSite {
    /// The platforms that compile this site. `None` cfg means every platform.
    /// A `Some(pred)` is classified by [`predicate_platforms`], which returns
    /// `None` when the predicate does not determine a platform set (a pure
    /// feature or mode gate) — such a site is not a platform-gate concern and
    /// is not flagged by this check.
    pub fn platform_scope(&self) -> Option<Vec<Platform>> {
        match &self.cfg {
            None => Some(ALL_PLATFORMS.to_vec()),
            Some(pred) => predicate_platforms(pred),
        }
    }

    /// True when a local build on `host` compiles this site. A site whose cfg
    /// does not determine a platform set is not platform-restricted, so a
    /// build on any host is assumed to reach it: this check is about platform
    /// gates, not feature or mode gates.
    pub fn visible_to(&self, host: Platform) -> bool {
        match self.platform_scope() {
            Some(scope) => scope.contains(&host),
            None => true,
        }
    }
}

/// The sites the local build on `host` cannot compile (invariant 2): the
/// difference between the text's view (all sites) and the build's view (the
/// host's sites). These are the sites where the build's green is not
/// evidence — exactly where a missed field-initializer hides.
pub fn invisible_to_host(host: Platform, sites: &[ConstructionSite]) -> Vec<&ConstructionSite> {
    sites.iter().filter(|s| !s.visible_to(host)).collect()
}

/// The verdict (invariants 1 and 2): does the local build on `host` cover
/// every construction site of the struct the change adds a field to?
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SiteVisibility {
    /// The host compiles every site: the build's green is evidence for the
    /// whole struct.
    Clear { struct_name: String, total: usize },
    /// The host cannot compile at least one site: the build is not authority
    /// for it, and the change must verify it by text or by widening the cfg
    /// and building.
    Hold {
        struct_name: String,
        host: Platform,
        total: usize,
        /// The sites the host cannot compile, in input order.
        invisible: Vec<ConstructionSite>,
    },
}

impl SiteVisibility {
    /// The merge may proceed on local-build authority only when this is
    /// false.
    pub fn must_hold(&self) -> bool {
        matches!(self, SiteVisibility::Hold { .. })
    }

    /// The one line for the merge record and the monitor log. It names the
    /// host, the sites, and both remedies (invariant 3): the text search and
    /// the temporary-widen-and-build.
    pub fn line(&self) -> String {
        match self {
            SiteVisibility::Clear {
                struct_name,
                total,
            } => format!(
                "no platform-invisible construction site: the local build compiles all {} {} of {struct_name}",
                total,
                sites_word(*total)
            ),
            SiteVisibility::Hold {
                struct_name,
                host,
                total,
                invisible,
            } => format!(
                "hold: the local build on {} cannot compile {} of {} {} of {} — the build's green is not evidence there; verify each by text (grep '{} {{') or by widening the cfg to include the host and building: {}",
                host.label(),
                invisible.len(),
                total,
                sites_word(*total),
                struct_name,
                struct_name,
                invisible
                    .iter()
                    .map(|s| s.location.clone())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
}

/// Decide the verdict from the host and the text-enumerated sites of the
/// struct. `sites` is what the caller's text search found (every `Struct {`
/// literal in the tree, each tagged with the cfg that compiles it); the
/// decision is which of those the build on `host` cannot reach.
pub fn visibility(host: Platform, struct_name: &str, sites: &[ConstructionSite]) -> SiteVisibility {
    let total = sites.len();
    let invisible: Vec<ConstructionSite> = sites
        .iter()
        .filter(|s| !s.visible_to(host))
        .cloned()
        .collect();
    if invisible.is_empty() {
        SiteVisibility::Clear {
            struct_name: struct_name.to_owned(),
            total,
        }
    } else {
        SiteVisibility::Hold {
            struct_name: struct_name.to_owned(),
            host,
            total,
            invisible,
        }
    }
}

/// Classify a `cfg` predicate as the set of platforms that compile under it.
///
/// A simple platform term delegates to [`parse_platform_predicate`]. The
/// compositions `any(...)`, `all(...)` and `not(...)` combine such terms:
/// `any` is the union, `all` the intersection, `not` the complement. A term
/// that is not a platform predicate (a feature flag, `target_arch`, `test`,
/// an unknown value) is platform-neutral — it does not restrict or extend the
/// platform set — so `any`/`all` skip it. A composition with **no** platform
/// term determines no platform set and returns `None` — a pure feature or
/// mode gate is not a platform restriction, and [`ConstructionSite::visible_to`]
/// treats such a site as not platform-restricted.
pub fn predicate_platforms(inner: &str) -> Option<Vec<Platform>> {
    let inner = inner.trim();
    if let Some(body) = strip_delimited(inner, "any(") {
        let mut set: BTreeSet<Platform> = BTreeSet::new();
        let mut saw_platform = false;
        for term in split_top_level(body) {
            if let Some(scope) = predicate_platforms(&term) {
                saw_platform = true;
                set.extend(scope);
            }
        }
        return if saw_platform {
            Some(set.into_iter().collect())
        } else {
            None
        };
    }
    if let Some(body) = strip_delimited(inner, "all(") {
        let mut acc: Option<BTreeSet<Platform>> = None;
        for term in split_top_level(body) {
            if let Some(scope) = predicate_platforms(&term) {
                let s: BTreeSet<Platform> = scope.into_iter().collect();
                acc = Some(match acc {
                    None => s,
                    Some(a) => a.intersection(&s).copied().collect(),
                });
            }
        }
        return acc.map(|s| s.into_iter().collect());
    }
    if let Some(body) = strip_delimited(inner, "not(") {
        let scope = predicate_platforms(body)?;
        let set: BTreeSet<Platform> = scope.into_iter().collect();
        return Some(complement(&set));
    }
    parse_platform_predicate(inner).map(|s| s.compiles_on)
}

// --- predicate helpers -----------------------------------------------------

/// The contents of the parenthesised group opening at `open`, or `None` when
/// the input is not exactly one `<open>...<balanced group>)`. A group that
/// closes before the end of the string is not a single composition and is not
/// classified (mirrors `platform_gate`'s `not(...)` handling).
fn strip_delimited<'a>(inner: &'a str, open: &str) -> Option<&'a str> {
    let rest = inner.strip_prefix(open)?;
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
    if depth != 1 {
        return None;
    }
    Some(end)
}

/// Split a composition body on top-level commas (not those inside nested
/// parentheses). Empty parts are dropped and the remainder of each part is
/// trimmed, so the parts recurse cleanly.
fn split_top_level(body: &str) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut depth = 0usize;
    let mut cur = String::new();
    for ch in body.chars() {
        match ch {
            '(' => {
                depth += 1;
                cur.push(ch);
            }
            ')' => {
                depth -= 1;
                cur.push(ch);
            }
            ',' if depth == 0 => {
                let t = cur.trim().to_owned();
                if !t.is_empty() {
                    parts.push(t);
                }
                cur.clear();
            }
            _ => cur.push(ch),
        }
    }
    let t = cur.trim().to_owned();
    if !t.is_empty() {
        parts.push(t);
    }
    parts
}

/// The known platforms that are not in `set`.
fn complement(set: &BTreeSet<Platform>) -> Vec<Platform> {
    ALL_PLATFORMS
        .iter()
        .copied()
        .filter(|p| !set.contains(p))
        .collect()
}

/// `1 -> "site"`, otherwise `"sites"` — keeps the hold line grammatical
/// without a pluralisation helper.
fn sites_word(n: usize) -> &'static str {
    if n == 1 {
        "site"
    } else {
        "sites"
    }
}
