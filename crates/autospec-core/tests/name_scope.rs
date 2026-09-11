//! Regression tests for name scope and collision detection (issue #4251).
//!
//! The tests reconstruct the session's four collisions:
//!
//! 1. `qwen3.8-27b-*` also matching `qwen3.8-27b-vision-*` (the fleet was
//!    reported 6/24 when it was 5/20);
//! 2. `$LLM/*/out/issue-*` spanning four projects with colliding issue
//!    numbers;
//! 3. two gateway instances on one request path, with evidence about one
//!    carried to a claim about the other without naming the producer;
//! 4. two gateway components in two repos sharing a bare name across
//!    directory, crate, service, and repository kinds.

use autospec_core::name_scope::*;
use std::collections::BTreeMap;

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

// --- Rule 1: anchor prefix matches at the boundary you mean --------------

#[test]
fn glob_matches_basics() {
    // `*` matches any run, including empty: the incident glob selected
    // the vision worker name, but not the bare worker name (the literal
    // part ends in `-`).
    assert!(!glob_matches("qwen3.8-27b-*", "qwen3.8-27b"));
    assert!(glob_matches("qwen3.8-27b-*", "qwen3.8-27b-vision"));
    assert!(glob_matches("qwen3.8-27b*", "qwen3.8-27b"));
    assert!(glob_matches("issue-*", "issue-14"));
    assert!(!glob_matches("issue-*", "issues"));
    assert!(glob_matches("issue-1?", "issue-14"));
    assert!(!glob_matches("issue-1?", "issue-1"));
    // No other metacharacters: `[` is a literal, so a pattern is never
    // looser than the reader expects.
    assert!(glob_matches("a[b]*", "a[b]c"));
    assert!(!glob_matches("a[b]*", "ac"));
    assert!(glob_matches("*", ""));
    assert!(!glob_matches("a*", ""));
}

#[test]
fn glob_literal_prefix_strips_at_the_first_metacharacter() {
    assert_eq!(glob_literal_prefix("qwen3.8-27b-*"), "qwen3.8-27b-");
    assert_eq!(glob_literal_prefix("$LLM/*/out/issue-*"), "$LLM/");
    assert_eq!(glob_literal_prefix("issue-1?"), "issue-1");
    assert_eq!(glob_literal_prefix("*"), "");
    assert_eq!(glob_literal_prefix("literal"), "literal");
}

#[test]
fn prefix_collision_detects_the_vision_worker_case() {
    // The incident: the glob meant to select the `qwen3.8-27b` workers
    // also selects the `qwen3.8-27b-vision` workers, and the fleet was
    // reported as 6 workers/24 slots instead of 5/20.
    let globs = s(&["qwen3.8-27b-*"]);
    let known = s(&["qwen3.8-27b", "qwen3.8-27b-vision"]);
    let hits = prefix_collisions(&globs, &known, NAME_SEPARATORS);
    assert_eq!(
        hits,
        vec![PrefixCollision {
            glob: "qwen3.8-27b-*".into(),
            shorter: "qwen3.8-27b".into(),
            longer: "qwen3.8-27b-vision".into(),
        }]
    );
}

#[test]
fn sibling_numbers_are_not_a_collision() {
    // `issue-1` is a strict prefix of `issue-14`, but the continuation is
    // a digit, not a component-name boundary: sibling issues of the same
    // family, not two things wearing one name.
    let globs = s(&["issue-*"]);
    let known = s(&["issue-1", "issue-14", "issue-2"]);
    assert!(prefix_collisions(&globs, &known, NAME_SEPARATORS).is_empty());
}

#[test]
fn prefix_collision_flags_the_generic_case_and_dedups() {
    // `foo-*` matches `foo-bar-*`: the literal part crosses a boundary.
    let globs = s(&["foo-*"]);
    let known = s(&["foo", "foo-bar", "foo-bar"]); // duplicate known entry
    let hits = prefix_collisions(&globs, &known, NAME_SEPARATORS);
    assert_eq!(
        hits,
        vec![PrefixCollision {
            glob: "foo-*".into(),
            shorter: "foo".into(),
            longer: "foo-bar".into(),
        }]
    );
}

#[test]
fn a_glob_selecting_one_family_is_clean() {
    // The vision-only glob selects exactly one known identifier: nothing
    // to collide with.
    let globs = s(&["qwen3.8-27b-vision-*"]);
    let known = s(&["qwen3.8-27b", "qwen3.8-27b-vision"]);
    assert!(prefix_collisions(&globs, &known, NAME_SEPARATORS).is_empty());
}

#[test]
fn the_boundary_is_the_separator_set() {
    // No separators means no boundary anywhere: the same pair is a
    // sibling family, not a collision.
    let globs = s(&["qwen3.8-27b-*"]);
    let known = s(&["qwen3.8-27b", "qwen3.8-27b-vision"]);
    assert!(prefix_collisions(&globs, &known, &[]).is_empty());
    // A narrower set than the default still catches the `-` boundary.
    let hits = prefix_collisions(&globs, &known, &['-']);
    assert_eq!(hits.len(), 1);
}

// --- Rule 2: qualify the identifier when it crosses a boundary -----------

#[test]
fn scoped_id_round_trips() {
    let id = parse_scoped_id("InferWeave#14").unwrap();
    assert_eq!(
        id,
        ScopedId {
            scope: "InferWeave".into(),
            number: 14
        }
    );
    assert_eq!(id.render(), "InferWeave#14");
    assert_eq!(qualify("InferWeave", 14), "InferWeave#14");
}

#[test]
fn a_bare_identifier_does_not_parse_as_scoped() {
    // "issue 14" is the shape that sent the patch to the wrong project.
    for bad in [
        "issue 14",
        "14",
        "#14",
        "InferWeave#",
        "InferWeave#14#x",
        "InferWeave#-1",
        "InferWeave#1.5",
        "InferWeave# 14",
    ] {
        assert!(parse_scoped_id(bad).is_err(), "{bad:?} should not parse");
    }
    assert_eq!(
        parse_scoped_id("14").unwrap_err(),
        "\"14\": no '#' — a bare identifier is not scope-qualified"
    );
    assert!(parse_scoped_id("#14").unwrap_err().contains("empty scope"));
}

#[test]
fn bare_number_detection() {
    assert!(is_bare_number("14"));
    assert!(is_bare_number("014"));
    assert!(!is_bare_number("issue-14"));
    assert!(!is_bare_number(""));
    assert!(!is_bare_number("1a"));
}

#[test]
fn key_findings_reconstructs_the_four_project_pipeline() {
    // `$LLM/*/out/issue-*` spanned autospec, iw, disp, orch; issue-14 is
    // InferWeave's and issue-1 is the dispatcher's.
    let records = vec![
        Record {
            scope: Some("autospec".into()),
            number: 1,
        },
        Record {
            scope: Some("iw".into()),
            number: 14,
        },
        Record {
            scope: Some("disp".into()),
            number: 1,
        },
        Record {
            scope: Some("orch".into()),
            number: 14,
        },
    ];
    let findings = key_findings(&records);
    assert_eq!(
        findings,
        vec![
            KeyFinding::CollidingNumber {
                number: 1,
                scopes: s(&["autospec", "disp"]),
            },
            KeyFinding::CollidingNumber {
                number: 14,
                scopes: s(&["iw", "orch"]),
            },
        ]
    );
}

#[test]
fn an_unscoped_record_in_a_multi_scope_pipeline_is_a_finding() {
    // The patch was keyed on "issue 14" and had to be re-attached to
    // InferWeave later, by convention, because the scope was not attached
    // at read time.
    let records = vec![
        Record {
            scope: Some("autospec".into()),
            number: 1,
        },
        Record {
            scope: None,
            number: 14,
        },
        Record {
            scope: Some("iw".into()),
            number: 7,
        },
    ];
    assert_eq!(
        key_findings(&records),
        vec![KeyFinding::Unscoped { number: 14 }]
    );
}

#[test]
fn a_single_scope_pipeline_keyed_on_bare_numbers_is_clean() {
    // One conversation, one scope: "the gateway" is the gateway.
    let records = vec![
        Record {
            scope: Some("autospec".into()),
            number: 1,
        },
        Record {
            scope: Some("autospec".into()),
            number: 14,
        },
    ];
    assert!(key_findings(&records).is_empty());

    // Nothing observed from another scope: still single-scope.
    let records = vec![
        Record {
            scope: None,
            number: 1,
        },
        Record {
            scope: None,
            number: 14,
        },
    ];
    assert!(key_findings(&records).is_empty());
}

// --- Rule 3: a bare name in two kinds is probably two things -------------

#[test]
fn cross_kind_collision_detects_the_two_gateway_components() {
    // `InferWeave/inferweave`'s unimplemented Rust gateway and
    // `metabolomics-us/inferweave-gateway`'s production Go service: a
    // bare name across directory, crate, service, and repository.
    let things = vec![
        NamedThing {
            name: "gateway".into(),
            kind: NameKind::Directory,
        },
        NamedThing {
            name: "gateway".into(),
            kind: NameKind::Crate,
        },
        NamedThing {
            name: "gateway".into(),
            kind: NameKind::Service,
        },
        NamedThing {
            name: "gateway".into(),
            kind: NameKind::Repository,
        },
        NamedThing {
            name: "edge-gateway".into(),
            kind: NameKind::Service,
        },
    ];
    let hits = cross_kind_collisions(&things);
    assert_eq!(
        hits,
        vec![CrossKindCollision {
            name: "gateway".into(),
            kinds: vec![
                NameKind::Directory,
                NameKind::Crate,
                NameKind::Service,
                NameKind::Repository,
            ],
        }]
    );
}

#[test]
fn a_unique_name_and_a_same_kind_repeat_are_clean() {
    let things = vec![
        NamedThing {
            name: "edge-gateway".into(),
            kind: NameKind::Service,
        },
        NamedThing {
            name: "edge-gateway".into(),
            kind: NameKind::Service,
        },
    ];
    assert!(cross_kind_collisions(&things).is_empty());
}

#[test]
fn two_kinds_is_the_sensitivity() {
    // Two different kinds sharing a bare name is already suspicious: the
    // detector flags, it does not judge.
    let things = vec![
        NamedThing {
            name: "gateway".into(),
            kind: NameKind::Crate,
        },
        NamedThing {
            name: "gateway".into(),
            kind: NameKind::Repository,
        },
    ];
    let hits = cross_kind_collisions(&things);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].kinds, vec![NameKind::Crate, NameKind::Repository]);
}

// --- Rule 4: state which instance produced the evidence -------------------

#[test]
fn evidence_carried_across_instances_must_name_the_producer() {
    // Two gateway instances (edge + hive) on one request path. The
    // evidence was about hive; the claim was about "the gateway".
    // Naming the producer is what makes the hop visible.
    let attributed = CarriedClaim {
        produced_by: Some("hive-gateway".into()),
        applied_to: "edge-gateway".into(),
    };
    assert_eq!(attributed.verdict(), ClaimVerdict::Attributed);

    // Same instance on both sides: a local fact.
    let local = CarriedClaim {
        produced_by: Some("edge-gateway".into()),
        applied_to: "edge-gateway".into(),
    };
    assert_eq!(local.verdict(), ClaimVerdict::Local);

    // Nothing names the producer: "the gateway reports 10 workers" is not
    // a fact until it says which gateway.
    let unattributed = CarriedClaim {
        produced_by: None,
        applied_to: "gateway".into(),
    };
    assert_eq!(unattributed.verdict(), ClaimVerdict::Unattributed);
}

// --- Rule 5: specs state where names are used and how records move --------

#[test]
fn a_spec_naming_a_component_states_where_else_the_name_is_used() {
    let mut known_uses = BTreeMap::new();
    known_uses.insert(
        "gateway".to_string(),
        s(&[
            "InferWeave/inferweave",
            "metabolomics-us/inferweave-gateway",
        ]),
    );

    // Unstated: the spec says "gateway" and the reader cannot know it is
    // not about the other one.
    let hits = unstated_name_uses(
        &[SpecName {
            name: "gateway".into(),
            other_uses: vec![],
        }],
        &known_uses,
    );
    assert_eq!(
        hits,
        vec![UnstatedUses {
            name: "gateway".into(),
            missing: s(&[
                "InferWeave/inferweave",
                "metabolomics-us/inferweave-gateway"
            ]),
        }]
    );

    // Partially stated: the remaining use is still the finding.
    let hits = unstated_name_uses(
        &[SpecName {
            name: "gateway".into(),
            other_uses: s(&["InferWeave/inferweave"]),
        }],
        &known_uses,
    );
    assert_eq!(
        hits,
        vec![UnstatedUses {
            name: "gateway".into(),
            missing: s(&["metabolomics-us/inferweave-gateway"]),
        }]
    );

    // Fully stated: clean.
    let hits = unstated_name_uses(
        &[SpecName {
            name: "gateway".into(),
            other_uses: s(&[
                "InferWeave/inferweave",
                "metabolomics-us/inferweave-gateway",
            ]),
        }],
        &known_uses,
    );
    assert!(hits.is_empty());

    // A name with no known other uses needs no other-uses list.
    let hits = unstated_name_uses(
        &[SpecName {
            name: "edge-gateway".into(),
            other_uses: vec![],
        }],
        &known_uses,
    );
    assert!(hits.is_empty());
}

#[test]
fn a_spec_moving_records_between_systems_names_the_scoped_identifier() {
    // The incident fix: the patch is `InferWeave#14`, never "issue 14".
    let qualified = SpecMove {
        identifier: "InferWeave#14".into(),
        systems: s(&["metabolomics-us/inferweave", "autospec"]),
    };
    assert_eq!(qualified.verdict(), SpecMoveVerdict::Qualified);

    // A cross-system move keyed on the bare number: the implementation
    // can — and will — key on the bare number without noticing.
    let bare = SpecMove {
        identifier: "14".into(),
        systems: s(&["metabolomics-us/inferweave", "autospec"]),
    };
    assert_eq!(bare.verdict(), SpecMoveVerdict::BareKey);

    // "issue 14" is equally bare.
    let prose = SpecMove {
        identifier: "issue 14".into(),
        systems: s(&["metabolomics-us/inferweave", "autospec"]),
    };
    assert_eq!(prose.verdict(), SpecMoveVerdict::BareKey);

    // One system: a bare identifier is unambiguous there.
    let single = SpecMove {
        identifier: "14".into(),
        systems: s(&["metabolomics-us/inferweave"]),
    };
    assert_eq!(single.verdict(), SpecMoveVerdict::SingleSystem);
}
