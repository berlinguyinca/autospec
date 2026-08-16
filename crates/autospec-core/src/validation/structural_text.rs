//! Small text helpers for the structural validator, split out of `structural.rs`
//! to keep that module from growing past the file-size ratchet limit.

/// Strip a single leading blank line (LF or CRLF) from a document.
pub(crate) fn strip_first_blank_line(document: &str) -> String {
    document
        .strip_prefix("\n")
        .or_else(|| document.strip_prefix("\r\n"))
        .unwrap_or(document)
        .to_string()
}
