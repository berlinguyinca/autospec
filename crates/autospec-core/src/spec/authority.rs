//! Spec authority currency (#3947).
//!
//! The pipeline reads a spec as an instruction and never checks whether the
//! document that names itself authoritative is still the one in force. Two
//! programs can claim permanent ownership of the same problem space with no
//! cross-reference in either direction, and a fleet can merge dozens of
//! patches in a day toward a superseded program while every gate stays
//! green. This module makes the currency of a spec set an explicit, checkable
//! claim:
//!
//! 1. A document that claims authority declares a currency marker — a
//!    `## Version` line, a `## Supersedes` list, or a `## Superseded by`
//!    pointer — and the pipeline reads it ([`currency_verdict`]).
//! 2. Dispatch refuses a spec set marked superseded, or one that claims
//!    authority with no currency marker ([`gate_dispatch`]).
//! 3. Two documents claiming authority over the same component surface as a
//!    reported conflict for a human to decide, not a discovery made by
//!    accident while tracing an unrelated dependency
//!    ([`find_authority_conflicts`]).
//! 4. Task records carry the spec authority they derive from, so throughput
//!    is reportable by authority — a large volume number can no longer be
//!    read as direction ([`TaskRecord`], [`throughput_by_authority`]).
//!
//! Everything here is pure: the caller hands in the documents it read and
//! gets a verdict back.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::parser::{first_nonblank, section_lines};

/// One document that may claim authority over a set of components: a program
/// README, an authority table, a spec-set charter.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SpecAuthorityDoc {
    /// Identity of the document in reports, e.g. `inferweave-v2/README.md`.
    pub id: String,
    /// The `## Version` line, when present.
    pub version: Option<String>,
    /// Spec sets this document supersedes (`## Supersedes` bullets).
    pub supersedes: Vec<String>,
    /// The document this one points at as its replacement (`## Superseded by`).
    pub superseded_by: Option<String>,
    /// Components this document claims authority over (`## Authority` bullets).
    pub authority: Vec<String>,
}

impl SpecAuthorityDoc {
    /// True when the document claims authority at all. Only such documents
    /// are bound by the currency rules: a plain issue body carrying none of
    /// the authority sections is not a spec-set document and dispatches as
    /// before.
    pub fn claims_authority(&self) -> bool {
        !self.authority.is_empty() || !self.supersedes.is_empty() || self.superseded_by.is_some()
    }
}

/// Parse the optional currency and authority sections out of a document.
/// Sections the document does not carry simply stay empty.
pub fn parse_authority_doc(id: &str, source: &str) -> SpecAuthorityDoc {
    SpecAuthorityDoc {
        id: id.to_string(),
        version: section_lines(source, "Version").and_then(first_nonblank),
        supersedes: bullet_items(source, "Supersedes"),
        superseded_by: section_lines(source, "Superseded by").and_then(first_nonblank),
        authority: bullet_items(source, "Authority"),
    }
}

fn bullet_items(source: &str, heading: &str) -> Vec<String> {
    let Some(lines) = section_lines(source, heading) else {
        return Vec::new();
    };
    lines
        .into_iter()
        .filter_map(|(_, line)| {
            line.trim()
                .strip_prefix("- ")
                .map(str::trim)
                .map(|item| item.trim_matches('`').to_string())
                .filter(|item| !item.is_empty())
        })
        .collect()
}

/// Whether a document's currency marker says it is still the one in force.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CurrencyVerdict {
    /// Carries a version or a supersedes list and names no replacement.
    Current {
        version: Option<String>,
        supersedes: Vec<String>,
    },
    /// Names its own replacement; dispatching against it is dispatching
    /// against a superseded program.
    Superseded { by: String },
    /// Claims authority but carries no version and no supersedes pointer:
    /// its currency cannot be verified, so it is reported rather than assumed.
    NoCurrencyMarker,
}

pub fn currency_verdict(doc: &SpecAuthorityDoc) -> CurrencyVerdict {
    if let Some(by) = &doc.superseded_by {
        return CurrencyVerdict::Superseded { by: by.clone() };
    }
    if doc.version.is_some() || !doc.supersedes.is_empty() {
        return CurrencyVerdict::Current {
            version: doc.version.clone(),
            supersedes: doc.supersedes.clone(),
        };
    }
    CurrencyVerdict::NoCurrencyMarker
}

/// A component that two or more documents both claim authority over.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorityConflict {
    pub component: String,
    /// The claiming documents, in sorted order (two or more).
    pub documents: Vec<String>,
}

/// Every component claimed by more than one document, sorted by component.
/// An empty result is the only state in which "which program is current" has
/// a determinate answer.
pub fn find_authority_conflicts(docs: &[SpecAuthorityDoc]) -> Vec<AuthorityConflict> {
    let mut claims: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for doc in docs {
        for component in &doc.authority {
            claims
                .entry(component.clone())
                .or_default()
                .insert(doc.id.clone());
        }
    }
    claims
        .into_iter()
        .filter(|(_, documents)| documents.len() > 1)
        .map(|(component, documents)| AuthorityConflict {
            component,
            documents: documents.into_iter().collect(),
        })
        .collect()
}

/// Why a spec set may not be dispatched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthorityRefusal {
    /// A document names its own replacement.
    Superseded { document: String, by: String },
    /// A document claims authority with no currency marker, so its currency
    /// cannot be verified.
    NoCurrencyMarker { document: String },
    /// Two or more documents claim authority over the same component; which
    /// program is current is a decision no pipeline may make for a human.
    Conflicts { conflicts: Vec<AuthorityConflict> },
}

impl AuthorityRefusal {
    /// The one-line message a refused dispatch prints: it names the document
    /// and the reason, because the operator's next question is exactly which
    /// program this was pointed at.
    pub fn message(&self) -> String {
        match self {
            Self::Superseded { document, by } => format!(
                "spec authority {document} is marked superseded (by {by}); dispatch against a superseded spec set is refused"
            ),
            Self::NoCurrencyMarker { document } => format!(
                "spec authority {document} claims authority but declares no currency marker (a ## Version line or a ## Supersedes list); its currency cannot be verified, so dispatch is refused"
            ),
            Self::Conflicts { conflicts } => {
                let parts: Vec<String> = conflicts
                    .iter()
                    .map(|conflict| {
                        format!(
                            "component '{}' is claimed by {}",
                            conflict.component,
                            conflict.documents.join(" and ")
                        )
                    })
                    .collect();
                format!(
                    "spec authority conflict: {}; which program is current is a human decision, so dispatch is refused",
                    parts.join("; ")
                )
            }
        }
    }
}

/// The dispatch decision for a spec set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchGate {
    Admitted,
    Refused { refusal: AuthorityRefusal },
}

/// Decide whether the spec set may be handed to a worker.
///
/// Only documents that claim authority are examined. The refusals are checked
/// in severity order: a document marked superseded blocks outright, then
/// cross-document conflicts, then a document whose currency cannot
/// be verified.
pub fn gate_dispatch(docs: &[SpecAuthorityDoc]) -> DispatchGate {
    let claiming: Vec<&SpecAuthorityDoc> =
        docs.iter().filter(|doc| doc.claims_authority()).collect();
    if claiming.is_empty() {
        return DispatchGate::Admitted;
    }

    for doc in &claiming {
        if let CurrencyVerdict::Superseded { by } = currency_verdict(doc) {
            return DispatchGate::Refused {
                refusal: AuthorityRefusal::Superseded {
                    document: doc.id.clone(),
                    by,
                },
            };
        }
    }

    let conflicts = find_authority_conflicts(docs);
    if !conflicts.is_empty() {
        return DispatchGate::Refused {
            refusal: AuthorityRefusal::Conflicts { conflicts },
        };
    }

    for doc in &claiming {
        if matches!(currency_verdict(doc), CurrencyVerdict::NoCurrencyMarker) {
            return DispatchGate::Refused {
                refusal: AuthorityRefusal::NoCurrencyMarker {
                    document: doc.id.clone(),
                },
            };
        }
    }

    DispatchGate::Admitted
}

/// One task's record of where its spec came from. The authority field is
/// what makes throughput reportable by authority: a merge count grouped by it
/// shows which program the volume was actually pointed at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskRecord {
    /// The issue the task worked.
    pub issue: String,
    /// The spec authority (document identity) the task derived from.
    pub spec_authority: String,
    /// Whether the task's patch was merged.
    pub merged: bool,
}

/// Merged throughput grouped by spec authority, in sorted order for stable
/// reports. Authorities with no merged tasks are still listed at zero, so a
/// report can show volume pointed somewhere that produced nothing.
pub fn throughput_by_authority(records: &[TaskRecord]) -> BTreeMap<String, u32> {
    let mut counts: BTreeMap<String, u32> = BTreeMap::new();
    for record in records {
        let entry = counts.entry(record.spec_authority.clone()).or_insert(0);
        if record.merged {
            *entry += 1;
        }
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;

    // The populated case (#3947 evidence, exercised the way #3793's is): a
    // v2 multi-repository program with a currency marker, and a
    // clean-slate monorepo README that names a different specs repository as
    // authoritative but declares no currency of its own. The two claim the
    // same component and cross-reference each other zero times.
    const V2_PROGRAM: &str = "\
# v2 Multi-Repository Program

Supersedes its former product charter and assigns permanent ownership.

## Version
V2

## Supersedes
- former-product-charter

## Authority
- edge-and-gateway
- provider-node
- client-edge
- scheduler
";

    const MONOREPO: &str = "\
# Clean-Slate Implementation Monorepo

The specs repository `inferweave/specs` is authoritative over anything
written here.

## Authority
- edge-and-gateway
- provider-node
";

    const SUPERSEDED_CHARTER: &str = "\
# Former Product Charter

## Version
V1

## Superseded by
inferweave-v2/README.md

## Authority
- edge-and-gateway
";

    fn v2() -> SpecAuthorityDoc {
        parse_authority_doc("inferweave-v2/README.md", V2_PROGRAM)
    }

    fn monorepo() -> SpecAuthorityDoc {
        parse_authority_doc("inferweave-mono/README.md", MONOREPO)
    }

    #[test]
    fn parses_currency_and_authority_sections() {
        let doc = v2();
        assert_eq!(doc.id, "inferweave-v2/README.md");
        assert_eq!(doc.version.as_deref(), Some("V2"));
        assert_eq!(doc.supersedes, vec!["former-product-charter".to_string()]);
        assert_eq!(doc.superseded_by, None);
        assert_eq!(
            doc.authority,
            vec![
                "edge-and-gateway".to_string(),
                "provider-node".to_string(),
                "client-edge".to_string(),
                "scheduler".to_string(),
            ]
        );
        assert!(doc.claims_authority());
        assert_eq!(
            currency_verdict(&doc),
            CurrencyVerdict::Current {
                version: Some("V2".to_string()),
                supersedes: vec!["former-product-charter".to_string()],
            }
        );
    }

    #[test]
    fn populated_case_superseded_blocks_dispatch() {
        let doc = parse_authority_doc("former-charter/README.md", SUPERSEDED_CHARTER);
        assert!(matches!(
            currency_verdict(&doc),
            CurrencyVerdict::Superseded { by } if by == "inferweave-v2/README.md"
        ));

        let gate = gate_dispatch(&[doc]);
        let DispatchGate::Refused { refusal } = gate else {
            panic!("a superseded spec set must block dispatch, got: {gate:?}");
        };
        let AuthorityRefusal::Superseded {
            ref document,
            ref by,
        } = refusal
        else {
            panic!("expected a superseded refusal, got: {refusal:?}");
        };
        assert_eq!(document, "former-charter/README.md");
        assert_eq!(by, "inferweave-v2/README.md");
        assert!(refusal.message().contains("superseded"));
    }

    #[test]
    fn populated_case_currencyless_is_reported() {
        let doc = monorepo();
        assert!(doc.claims_authority());
        assert_eq!(currency_verdict(&doc), CurrencyVerdict::NoCurrencyMarker);

        let gate = gate_dispatch(&[doc]);
        let DispatchGate::Refused { refusal } = gate else {
            panic!("a currency-less authority document must be reported, got: {gate:?}");
        };
        let AuthorityRefusal::NoCurrencyMarker { ref document } = refusal else {
            panic!("expected a no-currency refusal, got: {refusal:?}");
        };
        assert_eq!(document, "inferweave-mono/README.md");
        assert!(refusal.message().contains("currency"));
    }

    #[test]
    fn cross_program_conflict_surfaces() {
        let conflicts = find_authority_conflicts(&[v2(), monorepo()]);
        assert_eq!(
            conflicts,
            vec![
                AuthorityConflict {
                    component: "edge-and-gateway".to_string(),
                    documents: vec![
                        "inferweave-mono/README.md".to_string(),
                        "inferweave-v2/README.md".to_string(),
                    ],
                },
                AuthorityConflict {
                    component: "provider-node".to_string(),
                    documents: vec![
                        "inferweave-mono/README.md".to_string(),
                        "inferweave-v2/README.md".to_string(),
                    ],
                },
            ]
        );

        let gate = gate_dispatch(&[v2(), monorepo()]);
        let DispatchGate::Refused { refusal } = gate else {
            panic!("two conflicting authorities must refuse dispatch, got: {gate:?}");
        };
        let AuthorityRefusal::Conflicts { ref conflicts } = refusal else {
            panic!("expected a conflict refusal, got: {refusal:?}");
        };
        assert_eq!(conflicts.len(), 2);
        assert!(refusal.message().contains("edge-and-gateway"));
    }

    #[test]
    fn superseded_beats_conflict_in_severity() {
        let doc = parse_authority_doc("former-charter/README.md", SUPERSEDED_CHARTER);
        let gate = gate_dispatch(&[v2(), doc]);
        let DispatchGate::Refused { refusal } = gate else {
            panic!("expected a refusal, got: {gate:?}");
        };
        assert!(matches!(refusal, AuthorityRefusal::Superseded { .. }));
    }

    #[test]
    fn version_only_is_current() {
        let doc = parse_authority_doc("specs/README.md", "# Specs\n\n## Version\nV3\n");
        assert_eq!(
            currency_verdict(&doc),
            CurrencyVerdict::Current {
                version: Some("V3".to_string()),
                supersedes: Vec::new(),
            }
        );
        assert_eq!(gate_dispatch(&[doc]), DispatchGate::Admitted);
    }

    #[test]
    fn plain_issue_body_makes_no_authority_claim() {
        let body = "# Fix the queue refresher\n\n## Objective\nRefresh queue.txt on an interval.\n";
        let doc = parse_authority_doc("issue-50", body);
        assert!(!doc.claims_authority());
        assert_eq!(gate_dispatch(&[doc]), DispatchGate::Admitted);
    }

    #[test]
    fn throughput_is_reportable_by_authority() {
        let records = vec![
            TaskRecord {
                issue: "50".to_string(),
                spec_authority: "inferweave-mono/README.md".to_string(),
                merged: true,
            },
            TaskRecord {
                issue: "51".to_string(),
                spec_authority: "inferweave-mono/README.md".to_string(),
                merged: true,
            },
            TaskRecord {
                issue: "52".to_string(),
                spec_authority: "inferweave-mono/README.md".to_string(),
                merged: false,
            },
            TaskRecord {
                issue: "53".to_string(),
                spec_authority: "inferweave-v2/README.md".to_string(),
                merged: false,
            },
        ];
        let throughput = throughput_by_authority(&records);
        assert_eq!(
            throughput,
            BTreeMap::from([
                ("inferweave-mono/README.md".to_string(), 2),
                ("inferweave-v2/README.md".to_string(), 0),
            ])
        );
    }
}
