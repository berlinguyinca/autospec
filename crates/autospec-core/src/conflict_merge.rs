//! The keep-both text merge (issue #4560): applying a certified resolution
//! to a conflicted file.
//!
//! The classifier (`conflict_resolution`) decides that a file's shape admits
//! keep-both; this module is what keep-both means as a transformation of the
//! conflicted text. Every conflict hunk is replaced by both sides — in order
//! for append-only lists, or as their union without duplicates for additive
//! declarations. Everything the parser cannot prove safe — a stray marker, an
//! unbalanced hunk, a diff3-style region, a file with no markers at all — is
//! an error, and the caller holds the patch. A resolution the parser cannot
//! certify is not a resolution: the parser's own failure modes are part of
//! the check, and each one fails closed.

/// Why the keep-both merge could not be computed. The caller treats every
/// variant as "this file is not resolvable by the pass" and holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeError {
    /// A conflict marker that does not form a well-formed hunk: a separator
    /// or close with no open hunk, a close with no separator, a second
    /// separator, a nested open, or a hunk still open at end of file.
    UnbalancedMarkers { detail: String },
    /// A marker style the pass does not handle (diff3's `|||||||` base
    /// region): resolving it would mean reading three sides, and a
    /// two-sided merge of a three-sided conflict is a guess.
    UnsupportedConflictStyle { detail: String },
    /// The file has no conflict markers at all. A file git left unmerged but
    /// the pass cannot see the conflict in: nothing to merge, something wrong.
    NoHunks,
}

/// Git's classic conflict markers. The open and close carry exactly seven
/// characters, a space, and a label; the separator is exactly seven `=`.
const OPEN: &str = "<<<<<<< ";
const SEP: &str = "=======";
const CLOSE: &str = ">>>>>>> ";
const DIFF3: &str = "|||||||";

/// Replace every conflict hunk in `content` with both sides.
///
/// `deduplicate` selects the flavour: `false` keeps both sides in their
/// original order (append-only lists — an entry on each side stays, in the
/// order each side wrote it); `true` keeps the union without duplicates
/// (additive declarations — a declaration both sides added is one
/// declaration). Lines outside the hunks pass through byte-for-byte.
///
/// The result is exactly the union of the hunk sides, order preserved, no
/// duplicates — the property the caller asserts, not infers.
pub fn merge_keep_both(content: &str, deduplicate: bool) -> Result<String, MergeError> {
    let mut out = String::new();
    let mut base = String::new();
    let mut branch = String::new();
    let mut hunks = 0;
    let mut state = 0usize; // 0 outside, 1 base side, 2 branch side
    for raw in content.split_inclusive('\n') {
        let line = raw.trim_end_matches(['\n', '\r']);
        if state == 0 {
            if line.starts_with(OPEN) {
                base.clear();
                branch.clear();
                state = 1;
            } else if line == SEP {
                return Err(MergeError::UnbalancedMarkers {
                    detail: "a separator with no open hunk".to_string(),
                });
            } else if line.starts_with(CLOSE) {
                return Err(MergeError::UnbalancedMarkers {
                    detail: "a close with no open hunk".to_string(),
                });
            } else if line.starts_with(DIFF3) {
                return Err(MergeError::UnsupportedConflictStyle {
                    detail: "diff3-style base region: a two-sided merge of a three-sided \
                              conflict is a guess"
                        .to_string(),
                });
            } else {
                out.push_str(raw);
            }
        } else if state == 1 {
            if line == SEP {
                state = 2;
            } else if line.starts_with(OPEN) {
                return Err(MergeError::UnbalancedMarkers {
                    detail: "a nested open marker".to_string(),
                });
            } else if line.starts_with(CLOSE) {
                return Err(MergeError::UnbalancedMarkers {
                    detail: "a close with no separator".to_string(),
                });
            } else if line.starts_with(DIFF3) {
                return Err(MergeError::UnsupportedConflictStyle {
                    detail: "diff3-style base region inside a hunk".to_string(),
                });
            } else {
                base.push_str(raw);
            }
        } else if line.starts_with(CLOSE) {
            hunks += 1;
            merge_hunk(&mut out, &base, &branch, deduplicate);
            state = 0;
        } else if line == SEP {
            return Err(MergeError::UnbalancedMarkers {
                detail: "a second separator in one hunk".to_string(),
            });
        } else if line.starts_with(OPEN) {
            return Err(MergeError::UnbalancedMarkers {
                detail: "a nested open marker".to_string(),
            });
        } else if line.starts_with(DIFF3) {
            return Err(MergeError::UnsupportedConflictStyle {
                detail: "diff3-style base region inside a hunk".to_string(),
            });
        } else {
            branch.push_str(raw);
        }
    }
    if state != 0 {
        return Err(MergeError::UnbalancedMarkers {
            detail: if state == 1 {
                "a hunk open at end of file with no separator".to_string()
            } else {
                "a hunk open at end of file with no close".to_string()
            },
        });
    }
    if hunks == 0 {
        return Err(MergeError::NoHunks);
    }
    Ok(out)
}

/// One hunk's replacement: both sides, in order, with duplicates removed
/// when `deduplicate` names the flavour. A duplicate is the same line on
/// both sides (compared without the line terminator, so a final line missing
/// its newline still deduplicates); the first occurrence keeps its place.
fn merge_hunk(out: &mut String, base: &str, branch: &str, deduplicate: bool) {
    if !deduplicate {
        out.push_str(base);
        out.push_str(branch);
        return;
    }
    let mut seen: Vec<&str> = Vec::new();
    for raw in base
        .split_inclusive('\n')
        .chain(branch.split_inclusive('\n'))
    {
        let line = raw.trim_end_matches(['\n', '\r']);
        if seen.iter().any(|seen| *seen == line) {
            continue;
        }
        seen.push(line);
        out.push_str(raw);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape the issue names: two agents each appending a line to the
    /// same append-only index, git showing the three-way result.
    fn one_hunk(base: &str, branch: &str) -> String {
        format!(
            "context before\n{OPEN}HEAD\n{base}{SEP}\n{branch}{CLOSE}convert-77\ncontext after\n"
        )
    }

    #[test]
    fn keep_both_in_order_is_both_sides_in_the_order_each_wrote_them() {
        let merged = merge_keep_both(&one_hunk("b1\nb2\n", "t1\nt2\n"), false).unwrap();
        assert_eq!(merged, "context before\nb1\nb2\nt1\nt2\ncontext after\n");
    }

    #[test]
    fn keep_both_deduplicated_is_the_union_order_preserved_no_duplicates() {
        // Both sides added `pub mod b;` and one added what the other did not:
        // the union keeps each declaration once, first occurrence in place.
        let merged = merge_keep_both(
            &one_hunk("pub mod a;\npub mod b;\n", "pub mod b;\npub mod c;\n"),
            true,
        )
        .unwrap();
        assert_eq!(
            merged,
            "context before\npub mod a;\npub mod b;\npub mod c;\ncontext after\n"
        );
    }

    #[test]
    fn the_deduplicated_result_is_the_union_of_both_sides_asserted_directly() {
        // The acceptance property from #4560, computed independently: the
        // hunk region of the result must equal the union of the sides.
        let base = "pub mod a;\npub mod b;\n";
        let branch = "pub mod b;\npub mod c;\npub mod b;\n";
        let content = one_hunk(base, branch);
        let merged = merge_keep_both(&content, true).unwrap();
        let mut expected_union = Vec::new();
        for line in base.lines().chain(branch.lines()) {
            if !expected_union.iter().any(|seen| *seen == line) {
                expected_union.push(line);
            }
        }
        // The whole file, byte for byte: the context, the union, the context.
        assert_eq!(
            merged,
            format!(
                "context before\n{}\ncontext after\n",
                expected_union.join("\n")
            )
        );
    }

    #[test]
    fn every_hunk_is_merged_not_just_the_first() {
        let content = format!(
            "first\n{OPEN}HEAD\nx\n{SEP}\ny\n{CLOSE}b\nmid\n{OPEN}HEAD\nx\n{SEP}\nz\n{CLOSE}b\n"
        );
        let merged = merge_keep_both(&content, false).unwrap();
        assert_eq!(merged, "first\nx\ny\nmid\nx\nz\n");
        let merged = merge_keep_both(&content, true).unwrap();
        assert_eq!(merged, "first\nx\ny\nmid\nx\nz\n");
    }

    #[test]
    fn an_empty_side_keeps_the_other_side() {
        let merged = merge_keep_both(&one_hunk("only-base\n", ""), true).unwrap();
        assert_eq!(merged, "context before\nonly-base\ncontext after\n");
    }

    #[test]
    fn a_final_line_without_a_terminator_still_deduplicates() {
        // The branch side's final line lacks its newline (the close marker is
        // the file's last line without one): the deduplication still matches
        // it against the base side's identical line, and the kept line is the
        // first occurrence — the base side's, with its terminator.
        let content = format!("a\n{OPEN}HEAD\nx\n{SEP}\nx\n{CLOSE}b");
        let merged = merge_keep_both(&content, true).unwrap();
        assert_eq!(merged, "a\nx\n");
    }

    #[test]
    fn each_unbalanced_marker_shape_is_a_named_error() {
        let cases = [
            // separator with no open hunk
            (format!("a\n{SEP}\nb\n"), "a separator with no open hunk"),
            // close with no open hunk
            (format!("a\n{CLOSE}b\n"), "a close with no open hunk"),
            // close with no separator
            (
                format!("{OPEN}HEAD\nx\n{CLOSE}b\n"),
                "a close with no separator",
            ),
            // second separator
            (
                format!("{OPEN}HEAD\nx\n{SEP}\ny\n{SEP}\n{CLOSE}b\n"),
                "a second separator in one hunk",
            ),
            // nested open
            (
                format!("{OPEN}HEAD\n{OPEN}HEAD\nx\n{SEP}\ny\n{CLOSE}b\n"),
                "a nested open marker",
            ),
            // open at end of file (base side)
            (
                format!("a\n{OPEN}HEAD\nx\n"),
                "a hunk open at end of file with no separator",
            ),
            // open at end of file (branch side)
            (
                format!("a\n{OPEN}HEAD\nx\n{SEP}\ny\n"),
                "a hunk open at end of file with no close",
            ),
        ];
        for (content, detail) in cases {
            match merge_keep_both(&content, false) {
                Err(MergeError::UnbalancedMarkers { detail: got }) => {
                    assert_eq!(got, detail, "content: {content:?}")
                }
                other => panic!("expected unbalanced, got {other:?} for {content:?}"),
            }
        }
    }

    #[test]
    fn a_file_with_no_markers_has_nothing_to_merge() {
        match merge_keep_both("plain\nfile\n", false) {
            Err(MergeError::NoHunks) => {}
            other => panic!("expected NoHunks, got {other:?}"),
        }
    }

    #[test]
    fn a_diff3_region_is_not_a_two_sided_conflict() {
        let content = format!("a\n{OPEN}HEAD\nx\n||||||| parent\np\n{SEP}\ny\n{CLOSE}b\n");
        match merge_keep_both(&content, true) {
            Err(MergeError::UnsupportedConflictStyle { .. }) => {}
            other => panic!("expected unsupported style, got {other:?}"),
        }
    }

    #[test]
    fn context_lines_pass_through_byte_for_byte() {
        let content = format!(
            "keep\tthis exactly\nspaced  double\n{OPEN}HEAD\nx\n{SEP}\ny\n{CLOSE}b\nkeep the end\n"
        );
        let merged = merge_keep_both(&content, false).unwrap();
        assert!(
            merged.starts_with("keep\tthis exactly\nspaced  double\n"),
            "{merged:?}"
        );
        assert!(merged.ends_with("\nkeep the end\n"), "{merged:?}");
    }

    // --- #4560 acceptance: the classifier's certification is the gate -----

    use crate::conflict_resolution::{classify_file, FileShape};

    #[test]
    fn a_function_body_conflict_is_not_an_additive_declaration() {
        // The incident #4560 pins: a conflict that cuts through a function
        // body looks additive (lines on both sides) and keeps compiling in
        // some placements, but it is not a declaration index. In a file the
        // classifier does not certify by path, the shape is unknown — and
        // unknown is refused, never defaulted.
        let content = format!(
            "fn engine() -> u32 {{\n    let v = 1;\n{OPEN}HEAD\n    let v = v + 1;\n{SEP}\n    \
             let v = v + 2;\n{CLOSE}convert-77\n    v\n}}\n"
        );
        assert_eq!(classify_file("src/engine.rs", &content), FileShape::Unknown);
    }

    #[test]
    fn the_certified_paths_are_still_certified_for_a_declaration_conflict() {
        let content =
            format!("pub mod a;\n{OPEN}HEAD\npub mod b;\n{SEP}\npub mod c;\n{CLOSE}convert-77\n");
        assert_eq!(
            classify_file("src/lib.rs", &content),
            FileShape::AdditiveDeclarations
        );
    }
}
