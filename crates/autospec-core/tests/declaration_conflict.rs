//! The declaration-only conflict union (issue #4463): a module list that every
//! change must touch but no change is about, resolved by canonical union rather
//! than judgement.

use autospec_core::declaration_conflict::{
    classify_line, declared_modules, detect, orphaned, sorted_union, Conflict, Line, UnionError,
};
fn conflict(ours: &str, theirs: &str) -> String {
    format!(
        "pub mod base;\n<<<<<<< HEAD\n{ours}=======\n{theirs}>>>>>>> origin/main\npub mod tail;\n"
    )
}

#[test]
fn the_incident_shape_is_declaration_only() {
    // Two agents, two modules, one `lib.rs`. Neither patch overlaps the
    // other, and six of ten conflicts in the batch looked exactly like
    // this.
    let text = conflict(
        "pub mod alpha;\npub mod shared;\n",
        "pub mod shared;\npub mod beta;\n",
    );
    assert_eq!(detect(&text), Conflict::DeclarationOnly);
    assert!(Conflict::DeclarationOnly.is_declaration_only());
}

#[test]
fn the_union_is_sorted_and_deduplicated() {
    let text = conflict(
        "pub mod alpha;\npub mod shared;\n",
        "pub mod shared;\npub mod beta;\n",
    );
    let merged = sorted_union(&text).unwrap();
    assert_eq!(
        merged,
        "pub mod base;\npub mod alpha;\npub mod beta;\npub mod shared;\npub mod tail;\n"
    );
    // Both sides' work survives, and no module is declared twice.
    assert_eq!(declared_modules(&merged).len(), 5);
}

#[test]
fn the_union_is_idempotent_so_position_stops_depending_on_merge_order() {
    // The half of #4463 that stops the contention rather than resolving it:
    // a canonical position means the next addition lands elsewhere.
    let text = conflict("pub mod zeta;\n", "pub mod aleph;\n");
    let once = sorted_union(&text).unwrap();
    assert_eq!(detect(&once), Conflict::NoConflict);
    assert!(once.find("pub mod aleph").unwrap() < once.find("pub mod zeta").unwrap());
    // Re-running over a file with no markers refuses rather than silently
    // rewriting it.
    assert_eq!(sorted_union(&once), Err(UnionError::NoConflict));
}

#[test]
fn attributes_travel_with_their_declaration() {
    let text = conflict(
        "#[cfg(test)]\nmod support;\n",
        "#[cfg(feature = \"deep\")]\npub mod deep;\n",
    );
    assert_eq!(detect(&text), Conflict::DeclarationOnly);
    let merged = sorted_union(&text).unwrap();
    let support = merged.find("#[cfg(test)]\nmod support;").unwrap();
    let deep = merged
        .find("#[cfg(feature = \"deep\")]\npub mod deep;")
        .unwrap();
    assert!(deep < support, "alpha order puts `deep` first:\n{merged}");
    // No attribute is orphaned above a different declaration.
    assert_eq!(merged.matches("#[").count(), 2);
}

#[test]
fn a_hunk_containing_code_is_never_unioned() {
    // The invariant forbids automating judgement, not automating
    // bookkeeping: one line of real code makes this a conflict about the
    // patch.
    let text = conflict("pub mod alpha;\n", "pub fn helper() {}\n");
    assert_eq!(detect(&text), Conflict::Mixed);
    assert_eq!(sorted_union(&text), Err(UnionError::NotDeclarationOnly));
    assert!(Conflict::Mixed.note().is_empty());
}

#[test]
fn an_inline_module_is_not_a_declaration_line() {
    // `mod support {` and its closing brace cannot be unioned line-wise.
    assert_eq!(classify_line("mod support {"), Line::Other("mod support {"));
    assert_eq!(
        classify_line("pub mod support;"),
        Line::Declaration("pub mod support;")
    );
    assert_eq!(
        classify_line("pub(crate) mod inner;"),
        Line::Declaration("pub(crate) mod inner;")
    );
    assert_eq!(classify_line(""), Line::Other(""));
    assert_eq!(
        classify_line("#![allow(dead_code)]"),
        Line::Other("#![allow(dead_code)]")
    );
    assert_eq!(classify_line("use std::fs;"), Line::Other("use std::fs;"));
    assert_eq!(classify_line("mod;"), Line::Other("mod;"));
}

#[test]
fn malformed_markers_are_mixed_never_safe() {
    let open = "pub mod a;\n<<<<<<< HEAD\npub mod b;\n";
    assert_eq!(detect(open), Conflict::Mixed);
    let stray_close = "pub mod a;\n=======\npub mod b;\n>>>>>>> x\n";
    assert_eq!(detect(stray_close), Conflict::Mixed);
    let diff3 =
        "pub mod a;\n<<<<<<< HEAD\npub mod b;\n||||||| base\n=======\npub mod c;\n>>>>>>> x\n";
    assert_eq!(detect(diff3), Conflict::Mixed);
}

#[test]
fn a_file_without_markers_is_not_this_module_s_business() {
    assert_eq!(detect("pub mod a;\npub mod b;\n"), Conflict::NoConflict);
    assert_eq!(sorted_union("pub mod a;\n"), Err(UnionError::NoConflict));
}

#[test]
fn a_union_naming_a_module_no_file_provides_is_refused() {
    // The one thing a two-sided union cannot see: the other side deleted
    // it. The filesystem settles it.
    let merged = "pub mod alpha;\npub mod gone;\n";
    assert_eq!(
        orphaned(merged, |name| name != "gone"),
        vec!["gone".to_string()]
    );
    assert!(orphaned(merged, |_| true).is_empty());
}

#[test]
fn every_qualified_declaration_is_checked_against_the_tree() {
    let text = "pub mod plain;\npub(crate) mod crate_only;\npub(super) mod super_only;\n\
                    mod bare;\npub mod spaced ;\nmod inline {\n";
    assert_eq!(
        declared_modules(text),
        vec![
            "plain".to_string(),
            "crate_only".to_string(),
            "super_only".to_string(),
            "bare".to_string(),
            "spaced".to_string()
        ]
    );
}

#[test]
fn the_note_is_what_makes_the_class_measurable() {
    // Acceptance criterion 3: a held patch records whether its conflict was
    // declaration-only, so the size of the class is counted and not sampled.
    assert_eq!(Conflict::DeclarationOnly.note(), ", declaration-only");
    assert_eq!(Conflict::Mixed.note(), "");
    assert_eq!(Conflict::NoConflict.note(), "");
}
