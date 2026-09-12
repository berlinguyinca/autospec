//! Silent false negatives: an empty result is a distinct outcome, not a
//! value that flows onward (issue #4449).
//!
//! Three recorded pitfalls fired again in one session, each *after* its
//! rule had already been written down:
//!
//! | tool | what it returned | what was true |
//! |---|---|---|
//! | `comm -12` on numerically-sorted files | 0 candidates | 68 candidates |
//! | `grep 'worktree add'` | no matches | 4 call sites |
//! | `pkill -f 'cargo.*test'` | exit 144, no output | killed the invoking shell |
//!
//! In every case the emptiness was manufactured by how the tool was called
//! rather than observed in the world, and the type carried the emptiness
//! onward as if it were a measurement. The rules against all three were
//! prohibitions to recall at authoring time — "don't `comm`
//! numerically-sorted input", "never `pkill -f` your own argv" — and none
//! survived contact with a routine-looking command. The fix is the shape
//! [`crate::gate_verdict::GateVerdict::NotMeasured`] proved in #4434: the
//! empty case is a distinct variant the caller must handle, not a bool or
//! an empty collection.
//!
//! **The invariant:** an empty result from a search, set operation, or
//! process query is not usable as evidence until the call has been shown
//! capable of returning a non-empty one. The discharge is a positive
//! control, and it is cheap in all three cases:
//!
//! - **set difference** — assert both inputs are non-empty and the
//!   intersection non-empty before trusting a difference of zero, or do the
//!   operation on real sets, where sort order cannot silently change the
//!   answer ([`set_difference`], [`set_intersection`])
//! - **search** — run the broadest single token first; if that is
//!   non-empty and the narrowed pattern is empty, the narrowing removed the
//!   hits ([`broad_then_narrowed`])
//! - **process query** — resolve to pids and count them before acting
//!   ([`process_targets`])
//!
//! Everything here is pure: the caller supplies the raw results and the
//! protected pids; the verdict says which reading is usable.

use std::collections::BTreeSet;

/// The outcome of a set operation that can return a silent false negative.
///
/// An empty set and an unanswered question are different facts. `Measured`
/// keeps them apart: an empty answer arrives as [`Measured::ProvenEmpty`]
/// (the positive control held: the call was shown capable of a non-empty
/// result) or [`Measured::Unmeasured`] (it was not), never as an empty
/// collection the caller can flow onward as "nothing exists". A non-empty
/// answer is its own positive control.
///
/// There is deliberately no `From<Measured<T>> for BTreeSet<T>`: the whole
/// point is that there is no silent coercion from "I did not measure" to a
/// value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Measured<T: Ord + Clone> {
    /// The call returned a non-empty result.
    NonEmpty(BTreeSet<T>),
    /// The call returned empty, and the positive control held: the call was
    /// shown capable of returning a non-empty one, so the zero is evidence.
    ProvenEmpty {
        /// What the control showed, for the report.
        control: String,
    },
    /// The call returned empty and the positive control failed: the
    /// emptiness was manufactured by how the call was made, not observed.
    /// Not "nothing exists"; "not measured".
    Unmeasured {
        /// Why the call was not shown capable of a non-empty result.
        why: String,
    },
}

impl<T: Ord + Clone> Measured<T> {
    /// A zero whose positive control held.
    pub fn proven_empty(control: impl Into<String>) -> Self {
        Self::ProvenEmpty {
            control: control.into(),
        }
    }

    /// A zero whose positive control failed: not usable as evidence.
    pub fn unmeasured(why: impl Into<String>) -> Self {
        Self::Unmeasured { why: why.into() }
    }

    /// The set the operation produced, `None` for the zero variants. `None`
    /// is the distinct outcome: callers pattern-match on it rather than
    /// falling through to an empty set.
    pub fn as_set(&self) -> Option<&BTreeSet<T>> {
        match self {
            Self::NonEmpty(set) => Some(set),
            Self::ProvenEmpty { .. } | Self::Unmeasured { .. } => None,
        }
    }

    /// True when the operation returned zero (either zero).
    pub fn is_empty(&self) -> bool {
        !matches!(self, Self::NonEmpty(..))
    }

    /// True when the zero is not usable as evidence. Callers must not
    /// proceed as if nothing existed.
    pub fn is_unmeasured(&self) -> bool {
        matches!(self, Self::Unmeasured { .. })
    }

    /// The report line. An unmeasured zero never renders as an ordinary
    /// count.
    pub fn line(&self) -> String {
        match self {
            Self::NonEmpty(set) => format!("measured: {} element(s)", set.len()),
            Self::ProvenEmpty { control } => format!("empty (proven): {control}"),
            Self::Unmeasured { why } => {
                format!("UNMEASURED: {why} — an empty result is not evidence of absence")
            }
        }
    }
}

/// Set difference with a positive control (issue #4449).
///
/// `a − b`, computed on a real set: the answer cannot depend on input
/// order, unlike `comm`, which silently misreads numerically-sorted input
/// as unsorted, warns on stderr, and then writes a manufactured zero to
/// stdout anyway. A non-empty difference is its own proof. A zero
/// difference is usable as evidence only when the call was shown capable
/// of returning a non-empty one: both inputs non-empty and the intersection
/// non-empty — which for a zero difference holds exactly when `a` was
/// non-empty (`a ⊆ b` then forces `b` and the intersection to be non-empty
/// as well). A zero difference built on an empty input is
/// [`Measured::Unmeasured`]: the call could not have returned a non-empty
/// result, so its zero was never a measurement.
pub fn set_difference<T: Ord + Clone>(
    a_name: &str,
    a: impl IntoIterator<Item = T>,
    b_name: &str,
    b: impl IntoIterator<Item = T>,
) -> Measured<T> {
    let a: BTreeSet<T> = a.into_iter().collect();
    let b: BTreeSet<T> = b.into_iter().collect();
    let diff: BTreeSet<T> = a.difference(&b).cloned().collect();
    if !diff.is_empty() {
        return Measured::NonEmpty(diff);
    }
    if a.is_empty() {
        return Measured::unmeasured(format!(
            "'{a_name}' was empty, so the difference could not have returned a non-empty \
             result — confirm the upstream enumeration actually produced data"
        ));
    }
    Measured::proven_empty(format!(
        "'{a_name}' had {} element(s) and every one was present in '{b_name}': \
         non-empty inputs were compared and the zero is the answer",
        a.len()
    ))
}

/// Set intersection with a positive control (issue #4449).
///
/// The `comm -12` case: two lists expected to overlap, intersected through
/// a tool whose sort contract the input broke, and the zero read as
/// "nothing overlaps" against a true 68. On a real set the answer cannot
/// depend on order; a non-empty intersection is its own proof, and a zero
/// intersection is usable as evidence only when both inputs were non-empty
/// — otherwise the call was never shown capable of returning a hit, and
/// the zero is [`Measured::Unmeasured`].
pub fn set_intersection<T: Ord + Clone>(
    a_name: &str,
    a: impl IntoIterator<Item = T>,
    b_name: &str,
    b: impl IntoIterator<Item = T>,
) -> Measured<T> {
    let a: BTreeSet<T> = a.into_iter().collect();
    let b: BTreeSet<T> = b.into_iter().collect();
    let inter: BTreeSet<T> = a.intersection(&b).cloned().collect();
    if !inter.is_empty() {
        return Measured::NonEmpty(inter);
    }
    match (a.is_empty(), b.is_empty()) {
        (true, true) => Measured::unmeasured(format!(
            "both '{a_name}' and '{b_name}' were empty: the intersection could not have \
             returned a non-empty result — confirm the upstream enumerations ran"
        )),
        (true, false) => Measured::unmeasured(format!(
            "'{a_name}' was empty: the intersection could not have returned a non-empty \
             result — confirm the upstream enumeration actually produced data"
        )),
        (false, true) => Measured::unmeasured(format!(
            "'{b_name}' was empty: the intersection could not have returned a non-empty \
             result — confirm the upstream enumeration actually produced data"
        )),
        (false, false) => Measured::proven_empty(format!(
            "'{a_name}' had {} element(s) and '{b_name}' had {} element(s): \
             non-empty inputs were compared and the zero is the answer",
            a.len(),
            b.len()
        )),
    }
}

/// The outcome of a search run broad-first (issue #4449).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchVerdict {
    /// The narrowed pattern matched. A non-empty answer is its own proof.
    Found {
        /// The pattern that matched.
        pattern: String,
        /// The hits, for the report.
        hits: Vec<String>,
    },
    /// The broadest token matched but the narrowed pattern did not: the
    /// empty was produced by the narrowing, not observed in the world. The
    /// `grep 'worktree add'` case — four call sites existed, and the
    /// pattern that removed them is the finding.
    NarrowingRemovedHits {
        /// The broad token that proved the search works.
        broad_token: String,
        /// What the narrowing removed: carried so the caller can see it.
        broad_hits: Vec<String>,
        /// The narrowed pattern that matched nothing.
        pattern: String,
    },
    /// The broadest token matched nothing: the search was never shown
    /// capable of returning a hit, so no pattern's zero over this source is
    /// evidence of absence.
    Unmeasured {
        /// The broad token that was supposed to prove the search works.
        broad_token: String,
        /// The narrowed pattern that matched nothing.
        pattern: String,
        /// Why the search was not shown capable of a hit.
        why: String,
    },
}

impl SearchVerdict {
    /// True when the zero is not usable as evidence of absence.
    pub fn is_unmeasured(&self) -> bool {
        matches!(self, Self::Unmeasured { .. })
    }

    /// The report line. An unmeasured zero never renders as "no matches".
    pub fn line(&self) -> String {
        match self {
            Self::Found { pattern, hits } => {
                format!("found: {} hit(s) for '{pattern}'", hits.len())
            }
            Self::NarrowingRemovedHits {
                broad_token,
                broad_hits,
                pattern,
            } => format!(
                "NARROWING REMOVED THE HITS: the broad token '{broad_token}' matched {} \
                 time(s) but '{pattern}' matched 0 — the empty was produced by the \
                 narrowing, not observed; inspect the broad hits",
                broad_hits.len()
            ),
            Self::Unmeasured { why, .. } => {
                format!("UNMEASURED: {why} — an empty result is not evidence of absence")
            }
        }
    }
}

/// Run the discharge for a narrowed search: the broadest single token
/// first, then the narrowed pattern (issue #4449).
///
/// If the broad token is non-empty and the narrowed pattern is empty, the
/// narrowing removed the hits — that is [`SearchVerdict::
/// NarrowingRemovedHits`], not "no matches". If the broad token is empty,
/// the search was never shown capable of a hit and the verdict is
/// [`SearchVerdict::Unmeasured`].
// linter:allow-UNWIRED_PUB_ITEM deliberate staging (issue #4449): the search discharge of the silent-false-negative surface; its motivating call sites are shell tools (grep) that cannot call Rust, and the set-operation discharge is wired into execution::backlog
pub fn broad_then_narrowed(
    broad_token: &str,
    broad_hits: impl IntoIterator<Item = impl AsRef<str>>,
    pattern: &str,
    narrowed_hits: impl IntoIterator<Item = impl AsRef<str>>,
) -> SearchVerdict {
    let broad: Vec<String> = broad_hits
        .into_iter()
        .map(|h| h.as_ref().to_string())
        .collect();
    let narrowed: Vec<String> = narrowed_hits
        .into_iter()
        .map(|h| h.as_ref().to_string())
        .collect();
    if !narrowed.is_empty() {
        return SearchVerdict::Found {
            pattern: pattern.to_string(),
            hits: narrowed,
        };
    }
    if !broad.is_empty() {
        return SearchVerdict::NarrowingRemovedHits {
            broad_token: broad_token.to_string(),
            broad_hits: broad,
            pattern: pattern.to_string(),
        };
    }
    SearchVerdict::Unmeasured {
        broad_token: broad_token.to_string(),
        pattern: pattern.to_string(),
        why: format!(
            "the broadest token '{broad_token}' matched nothing: the search was never shown \
             capable of returning a hit over this source"
        ),
    }
}

/// The outcome of resolving a process query to pids before acting
/// (issue #4449).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessVerdict {
    /// Pids resolved and at least one is outside the protected set. Act on
    /// exactly these — counted before any signal goes out.
    Targets {
        /// The query, for the report.
        query: String,
        /// The pids to signal, counted before acting.
        pids: BTreeSet<u32>,
        /// Resolved pids excluded because they are the invoker or its
        /// lineage: carried so the exclusion is visible, not silent.
        excluded_protected: BTreeSet<u32>,
    },
    /// Every resolved pid is protected: the query matched the invoker's own
    /// lineage. The `pkill -f 'cargo.*test'` case — the pattern matched the
    /// shell running it, and the signal killed the caller (exit 144, no
    /// output). Acting here is not "no targets"; it is "the targets are
    /// me".
    AllProtected {
        /// The query, for the report.
        query: String,
        /// The pids the query resolved to, all protected.
        pids: BTreeSet<u32>,
    },
    /// The query resolved to no pids at all: "no process matched" and "the
    /// query could not resolve" are different facts, and only the latter is
    /// on record.
    Unresolved {
        /// The query, for the report.
        query: String,
        /// Why the query produced no usable answer.
        why: String,
    },
}

impl ProcessVerdict {
    /// True when the query produced no usable answer.
    pub fn is_unmeasured(&self) -> bool {
        matches!(self, Self::Unresolved { .. })
    }

    /// The report line. An unresolved query never renders as "nothing to
    /// kill".
    pub fn line(&self) -> String {
        match self {
            Self::Targets {
                query,
                pids,
                excluded_protected,
            } => {
                let mut line = format!(
                    "targets: {} pid(s) for '{query}' [{}], counted before acting",
                    pids.len(),
                    join_pids(pids)
                );
                if !excluded_protected.is_empty() {
                    line.push_str(&format!(
                        " ({} protected pid(s) excluded: {})",
                        excluded_protected.len(),
                        join_pids(excluded_protected)
                    ));
                }
                line
            }
            Self::AllProtected { query, pids } => format!(
                "ALL PROTECTED: '{query}' resolved to {} pid(s) [{}], every one in the \
                 protected set — acting would kill the invoker; do not signal",
                pids.len(),
                join_pids(pids)
            ),
            Self::Unresolved { query, why } => format!(
                "UNRESOLVED: '{query}' — {why}; an empty pid list is not evidence the \
                 process does not exist"
            ),
        }
    }
}

fn join_pids(pids: &BTreeSet<u32>) -> String {
    pids.iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Resolve a process query to pids and count them before acting
/// (issue #4449).
///
/// `resolved` is what the query (a pgrep, a listing) returned; `protected`
/// is the set that must never be signalled — self, the invoking shell, the
/// lineage. The resolution is the discharge: a query that resolved to pids
/// has been shown capable, and the act is a counted signal to named pids,
/// never a pattern broadcast that can match the invoker.
// linter:allow-UNWIRED_PUB_ITEM deliberate staging (issue #4449): the process-query discharge of the silent-false-negative surface; its motivating call sites are shell tools (pkill) that cannot call Rust, and the set-operation discharge is wired into execution::backlog
pub fn process_targets(
    query: &str,
    resolved: impl IntoIterator<Item = u32>,
    protected: impl IntoIterator<Item = u32>,
) -> ProcessVerdict {
    let resolved: BTreeSet<u32> = resolved.into_iter().collect();
    if resolved.is_empty() {
        return ProcessVerdict::Unresolved {
            query: query.to_string(),
            why: "it resolved to no pids — 'no process matched' and 'the query could not \
                  resolve' are different facts, and only a broader match or a positive \
                  control tells them apart"
                .to_string(),
        };
    }
    let protected: BTreeSet<u32> = protected.into_iter().collect();
    let pids: BTreeSet<u32> = resolved.difference(&protected).copied().collect();
    let excluded_protected: BTreeSet<u32> = resolved.intersection(&protected).copied().collect();
    if pids.is_empty() {
        return ProcessVerdict::AllProtected {
            query: query.to_string(),
            pids: resolved,
        };
    }
    ProcessVerdict::Targets {
        query: query.to_string(),
        pids,
        excluded_protected,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u64_set(values: &[u64]) -> BTreeSet<u64> {
        values.iter().copied().collect()
    }

    fn u32_set(values: &[u32]) -> BTreeSet<u32> {
        values.iter().copied().collect()
    }

    // ── set operations: the zero is proven or unmeasured ─────────────────

    #[test]
    fn a_zero_difference_built_on_an_empty_input_is_unmeasured() {
        // The comm incident: the pipeline consumed the 0 and would have
        // reported "nothing to convert" on a backlog of 68.
        let verdict = set_difference(
            "conversion candidates",
            BTreeSet::<u64>::new().iter().copied(),
            "converted",
            u64_set(&[1, 2, 3]).iter().copied(),
        );
        assert!(verdict.is_unmeasured(), "{}", verdict.line());
        assert!(verdict.as_set().is_none());
        assert!(
            verdict.line().starts_with("UNMEASURED:"),
            "{}",
            verdict.line()
        );
    }

    #[test]
    fn a_zero_difference_with_a_non_empty_input_is_proven() {
        // a ⊆ b with a non-empty: the call compared non-empty data (both
        // inputs non-empty, intersection non-empty); the zero is the answer.
        let verdict = set_difference(
            "conversion candidates",
            u64_set(&[1, 2]).iter().copied(),
            "converted",
            u64_set(&[1, 2, 3]).iter().copied(),
        );
        assert!(!verdict.is_unmeasured(), "{}", verdict.line());
        assert!(verdict.is_empty());
        assert!(
            verdict.line().starts_with("empty (proven):"),
            "{}",
            verdict.line()
        );
    }

    #[test]
    fn a_non_empty_difference_is_its_own_proof() {
        let verdict = set_difference(
            "conversion candidates",
            u64_set(&[1, 2, 3]).iter().copied(),
            "converted",
            u64_set(&[3]).iter().copied(),
        );
        assert_eq!(verdict.as_set(), Some(&u64_set(&[1, 2])));
        assert!(!verdict.is_unmeasured());
    }

    #[test]
    fn the_answer_does_not_depend_on_input_order() {
        // comm read numerically-sorted files as unsorted and manufactured a
        // zero out of 68. A real set has no order to get wrong.
        let diff_forward = set_difference("a", [10u64, 200, 3000], "b", [3000u64, 200, 9]);
        let diff_reversed = set_difference("a", [3000u64, 200, 10], "b", [9u64, 200, 3000]);
        assert_eq!(diff_forward, diff_reversed);

        let inter_forward = set_intersection("a", [10u64, 200, 3000], "b", [3000u64, 200, 9]);
        let inter_reversed = set_intersection("a", [3000u64, 200, 10], "b", [9u64, 200, 3000]);
        assert_eq!(inter_forward, inter_reversed);
        assert_eq!(inter_forward.as_set(), Some(&u64_set(&[200, 3000])));
    }

    #[test]
    fn a_zero_intersection_with_an_empty_input_is_unmeasured() {
        let verdict = set_intersection(
            "conversion candidates",
            BTreeSet::<u64>::new().iter().copied(),
            "on main",
            u64_set(&[1, 2, 3]).iter().copied(),
        );
        assert!(verdict.is_unmeasured(), "{}", verdict.line());
        assert!(
            verdict.line().contains("conversion candidates"),
            "{}",
            verdict.line()
        );
    }

    #[test]
    fn a_zero_intersection_of_two_non_empty_sets_is_proven() {
        let verdict = set_intersection(
            "a",
            u64_set(&[1]).iter().copied(),
            "b",
            u64_set(&[2]).iter().copied(),
        );
        assert!(!verdict.is_unmeasured(), "{}", verdict.line());
        assert!(verdict.is_empty());
    }

    // ── search: broad first, then narrow ─────────────────────────────────

    #[test]
    fn a_narrowed_zero_with_broad_hits_is_narrowing_not_absence() {
        // The grep incident: 'worktree add' matched nothing; the broadest
        // token found the four call sites the pattern had removed.
        let verdict = broad_then_narrowed(
            "worktree",
            ["a.rs:1", "b.rs:2", "c.rs:3", "d.rs:4"],
            "worktree add",
            Vec::<&str>::new(),
        );
        match verdict {
            SearchVerdict::NarrowingRemovedHits {
                ref broad_token,
                ref broad_hits,
                ref pattern,
            } => {
                assert_eq!(*broad_token, "worktree");
                assert_eq!(broad_hits.len(), 4);
                assert_eq!(*pattern, "worktree add");
            }
            other => panic!("expected NarrowingRemovedHits, got {other:?}"),
        }
        assert!(!verdict.is_unmeasured());
        assert!(
            verdict.line().starts_with("NARROWING REMOVED THE HITS:"),
            "{}",
            verdict.line()
        );
    }

    #[test]
    fn a_narrowed_zero_with_no_broad_hits_is_unmeasured() {
        let verdict = broad_then_narrowed(
            "worktree",
            Vec::<&str>::new(),
            "worktree add",
            Vec::<&str>::new(),
        );
        assert!(verdict.is_unmeasured(), "{}", verdict.line());
        assert!(
            verdict.line().starts_with("UNMEASURED:"),
            "{}",
            verdict.line()
        );
    }

    #[test]
    fn a_narrowed_hit_is_found() {
        let verdict = broad_then_narrowed("worktree", ["a.rs:1"], "worktree add", ["a.rs:1"]);
        assert!(
            matches!(verdict, SearchVerdict::Found { ref hits, .. } if hits.len() == 1),
            "{verdict:?}"
        );
    }

    // ── process query: resolve, count, then act ──────────────────────────

    #[test]
    fn a_query_that_matches_only_the_invoker_is_all_protected() {
        // The pkill incident: 'cargo.*test' matched the shell running it.
        let verdict = process_targets("cargo.*test", [4242], [4242]);
        match verdict {
            ProcessVerdict::AllProtected {
                ref query,
                ref pids,
            } => {
                assert_eq!(*query, "cargo.*test");
                assert_eq!(*pids, u32_set(&[4242]));
            }
            other => panic!("expected AllProtected, got {other:?}"),
        }
        assert!(
            verdict.line().contains("kill the invoker"),
            "{}",
            verdict.line()
        );
    }

    #[test]
    fn resolved_pids_are_counted_and_protected_ones_excluded() {
        let verdict = process_targets("cargo test", [4242, 4243, 4244], [4242]);
        match verdict {
            ProcessVerdict::Targets {
                ref pids,
                ref excluded_protected,
                ..
            } => {
                assert_eq!(*pids, u32_set(&[4243, 4244]));
                assert_eq!(*excluded_protected, u32_set(&[4242]));
            }
            other => panic!("expected Targets, got {other:?}"),
        }
        assert!(!verdict.is_unmeasured());
    }

    #[test]
    fn a_query_that_resolves_to_nothing_is_unresolved_not_absence() {
        let verdict = process_targets("cargo test", [], [4242]);
        assert!(verdict.is_unmeasured(), "{}", verdict.line());
        assert!(
            verdict.line().starts_with("UNRESOLVED:"),
            "{}",
            verdict.line()
        );
    }

    #[test]
    fn an_unmeasured_zero_never_renders_as_an_ordinary_count() {
        let verdict = Measured::<u64>::unmeasured("the input was empty");
        assert!(
            verdict.line().starts_with("UNMEASURED:"),
            "{}",
            verdict.line()
        );
        let proven = Measured::<u64>::proven_empty("the control held");
        assert!(
            proven.line().starts_with("empty (proven):"),
            "{}",
            proven.line()
        );
    }
}
