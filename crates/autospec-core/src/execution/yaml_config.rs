//! YAML config gate (#3888, from InferWeave#246): a duplicate key in a
//! workflow or config YAML file is a blocking finding, and the record names
//! every occurrence, not the first one the scanner happened to hit.
//!
//! YAML parsers in this repository (`yaml-edit`) do not merge duplicate keys
//! when they parse — the duplicates survive in the tree, and the value a
//! consumer actually reads depends on the *consumer's* lookup semantics
//! (first occurrence here; last occurrence in PyYAML and most other
//! libraries). The file on disk says one thing, the diff shows another, and
//! the running config does a third. Editing such a file "adds" a key in the
//! diff while silently removing (or failing to change) the value the
//! consumer reads — adding a key can delete one.
//!
//! The policy is encoded here as pure, testable primitives; callers run the
//! check over their workflow/config files and act on the verdict:
//!
//! 1. **Duplicate keys are blocking, and every occurrence is named.**
//!    [`find_duplicate_keys`] walks every document, every mapping (including
//!    mappings nested inside sequences), and reports the first line of each
//!    duplicated key plus the line of each later occurrence.
//! 2. **The record reports all findings, not the first.**
//!    [`check_yaml_sources`] scans every file and names every duplicate and
//!    every parse failure, in a deterministic order, so a record acted on
//!    later still points at the real defects ([`YamlConfigVerdict::line`]).
//! 3. **A file that does not parse is a finding, never a silent pass.**
//!    Parse failures are collected as blocking [`ParseFailure`] entries
//!    rather than dropping the file from the check.
//! 4. **When editing config, assert on the parsed value the consumer reads,
//!    not on the diff.** The diff shows the text that changed; the config
//!    behaves according to the value the consumer's parser resolves.
//!    [`consumer_scalar`] returns exactly that value (first-occurrence
//!    semantics, matching `yaml_edit::Mapping::get`), so an edit to a config
//!    file should be verified by asserting on what the consumer reads —
//!    and by keeping the duplicate-key check green, since with duplicates
//!    present no diff can be trusted to predict the parsed value.

use std::collections::BTreeMap;

use yaml_edit::{byte_offset_to_line_column, AsYaml, MappingEntry, Parse, YamlFile, YamlNode};

/// One occurrence of a duplicated key in one mapping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateKeyFinding {
    /// The file the finding belongs to, as supplied by the caller.
    pub path: String,
    /// Zero-based index of the document within the file (YAML files may
    /// hold several `---`-separated documents).
    pub document: usize,
    /// The key as a consumer sees it: unquoted for scalar keys.
    pub key: String,
    /// One-based line of the first occurrence of the key in that mapping.
    pub first_line: usize,
    /// One-based line of this duplicate occurrence.
    pub duplicate_line: usize,
}

impl DuplicateKeyFinding {
    /// The record line for this finding: file, duplicate line, key, and the
    /// line of the first occurrence — everything needed to fix it without
    /// re-running the scan.
    pub fn line(&self) -> String {
        format!(
            "{path}:{duplicate_line} duplicate key `{key}` (first at {first_line})",
            path = self.path,
            duplicate_line = self.duplicate_line,
            key = self.key,
            first_line = self.first_line,
        )
    }
}

/// A workflow/config file the check could not even parse.
///
/// Recorded, not dropped: a file that fails to parse is a blocking finding
/// in its own right, and the check never reports it as clean.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseFailure {
    /// The file the failure belongs to, as supplied by the caller.
    pub path: String,
    /// One-based line where the parser first reported trouble.
    pub line: usize,
    /// The parser's own message, carried verbatim into the record.
    pub message: String,
}

impl ParseFailure {
    /// The record line for this failure.
    pub fn line(&self) -> String {
        format!("{}:{}: {}", self.path, self.line, self.message)
    }
}

/// The gate's verdict over a set of workflow/config YAML files.
///
/// Built by [`check_yaml_sources`]; every file is either clean, contributes
/// duplicate-key findings, or is recorded as a parse failure. A verdict is
/// blocking unless there are no findings of either kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YamlConfigVerdict {
    files_checked: usize,
    duplicates: Vec<DuplicateKeyFinding>,
    parse_failures: Vec<ParseFailure>,
}

impl YamlConfigVerdict {
    /// How many files the check ran over.
    pub fn files_checked(&self) -> usize {
        self.files_checked
    }

    /// Every duplicate-key finding, deterministic order.
    pub fn duplicates(&self) -> &[DuplicateKeyFinding] {
        &self.duplicates
    }

    /// Every parse failure, deterministic order.
    pub fn parse_failures(&self) -> &[ParseFailure] {
        &self.parse_failures
    }

    /// Whether the config set is held: any duplicate key or any parse
    /// failure blocks.
    pub fn is_blocking(&self) -> bool {
        !self.duplicates.is_empty() || !self.parse_failures.is_empty()
    }

    /// The durable record: every duplicate and every parse failure, in
    /// deterministic order. An actor reading this line later finds every
    /// defect named, not just the first one the scanner hit.
    pub fn line(&self) -> String {
        if !self.is_blocking() {
            return format!(
                "yaml config clean: {} file(s) checked, no duplicate keys",
                self.files_checked
            );
        }
        let parts = self
            .duplicates
            .iter()
            .map(DuplicateKeyFinding::line)
            .chain(self.parse_failures.iter().map(ParseFailure::line))
            .collect::<Vec<_>>()
            .join("; ");
        format!("HELD: {parts}")
    }
}

/// The error a duplicate-key-rejecting loader returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum YamlConfigError {
    /// The file does not parse at all; the failure is recorded, not hidden.
    Parse(ParseFailure),
    /// The file parses but contains duplicate keys — every occurrence.
    Duplicates {
        /// All duplicate-key findings in the file.
        findings: Vec<DuplicateKeyFinding>,
    },
}

impl std::fmt::Display for YamlConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(failure) => write!(f, "{}", failure.line()),
            Self::Duplicates { findings } => write!(
                f,
                "{}",
                findings
                    .iter()
                    .map(DuplicateKeyFinding::line)
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
        }
    }
}

impl std::error::Error for YamlConfigError {}

/// The duplicate-key-rejecting loader (#3888): parse a workflow or config
/// YAML file and refuse it if any mapping in any document carries a key
/// more than once.
///
/// Returns the parsed [`YamlFile`] for consumers on success. On failure the
/// error names *every* blocking finding — all occurrences of every
/// duplicated key, or the parse failure — never just the first.
pub fn load_yaml_rejecting_duplicates(
    path: &str,
    source: &str,
) -> Result<YamlFile, YamlConfigError> {
    let parse = YamlFile::parse(source);
    if let Some(failure) = first_parse_failure(path, source, &parse) {
        return Err(YamlConfigError::Parse(failure));
    }
    let file = parse.tree();
    let findings = scan_documents(path, source, &file);
    if findings.is_empty() {
        Ok(file)
    } else {
        Err(YamlConfigError::Duplicates { findings })
    }
}

/// Find every duplicate key in one YAML source: all documents, all
/// mappings, including mappings nested inside sequences.
///
/// Returns the findings in document order, then mapping order, then
/// occurrence order — empty for a clean file. Parse failure is an `Err`
/// (recorded, never a silent clean).
pub fn find_duplicate_keys(
    path: &str,
    source: &str,
) -> Result<Vec<DuplicateKeyFinding>, ParseFailure> {
    let parse = YamlFile::parse(source);
    if let Some(failure) = first_parse_failure(path, source, &parse) {
        return Err(failure);
    }
    let file = parse.tree();
    Ok(scan_documents(path, source, &file))
}

/// The gate over a set of workflow/config files: every file is scanned,
/// every finding is reported (not the first), and the verdict is in
/// deterministic order so the record is stable across runs.
pub fn check_yaml_sources(entries: &[(String, String)]) -> YamlConfigVerdict {
    let mut duplicates: Vec<DuplicateKeyFinding> = Vec::new();
    let mut parse_failures: Vec<ParseFailure> = Vec::new();
    for (path, source) in entries {
        match find_duplicate_keys(path, source) {
            Ok(found) => duplicates.extend(found),
            Err(failure) => parse_failures.push(failure),
        }
    }
    duplicates.sort_by(|a, b| {
        (
            a.path.as_str(),
            a.document,
            a.duplicate_line,
            a.key.as_str(),
        )
            .cmp(&(
                b.path.as_str(),
                b.document,
                b.duplicate_line,
                b.key.as_str(),
            ))
    });
    parse_failures.sort_by(|a, b| (a.path.as_str(), a.line).cmp(&(b.path.as_str(), b.line)));
    YamlConfigVerdict {
        files_checked: entries.len(),
        duplicates,
        parse_failures,
    }
}

/// The value a consumer in this repository reads for `dotted_key` from the
/// first document of `source` — the value a config edit actually changes,
/// not what the diff text says.
///
/// Resolution follows `yaml_edit::Mapping::get`: the **first** occurrence
/// of a key wins. Other YAML libraries (PyYAML, most others) let the
/// **last** occurrence win. Which occurrence wins is a property of the
/// consumer, never of the file — which is exactly why the guidance in this
/// module is to assert on the parsed value the consumer reads rather than
/// on the diff, and to keep the duplicate-key check green so that no file
/// can rely on either convention.
///
/// Returns `Ok(None)` when the key path does not resolve to a scalar (or
/// the file holds no document); `Err` when the file does not parse.
pub fn consumer_scalar(source: &str, dotted_key: &str) -> Result<Option<String>, ParseFailure> {
    let parse = YamlFile::parse(source);
    if let Some(failure) = first_parse_failure("<source>", source, &parse) {
        return Err(failure);
    }
    let file = parse.tree();
    let Some(document) = file.documents().next() else {
        return Ok(None);
    };
    let mut node = match document.as_mapping() {
        Some(mapping) => YamlNode::Mapping(mapping),
        None => return Ok(None),
    };
    for segment in dotted_key.split('.') {
        let mapping = match node {
            YamlNode::Mapping(mapping) => mapping,
            _ => return Ok(None),
        };
        node = match mapping.get(segment) {
            Some(next) => next,
            None => return Ok(None),
        };
    }
    match node {
        YamlNode::Scalar(scalar) => Ok(Some(scalar.as_string())),
        _ => Ok(None),
    }
}

/// The parse failure for `parse`, if the parser reported any.
fn first_parse_failure(path: &str, source: &str, parse: &Parse<YamlFile>) -> Option<ParseFailure> {
    let first = parse.positioned_errors().first()?;
    let position = byte_offset_to_line_column(source, first.range.start as usize);
    Some(ParseFailure {
        path: path.to_string(),
        line: position.line,
        message: first.message.clone(),
    })
}

/// Walk every document of `file`, collecting duplicate-key findings.
///
/// `source` is the exact text `file` was parsed from: line numbers in the
/// findings are recovered from it.
fn scan_documents(path: &str, source: &str, file: &YamlFile) -> Vec<DuplicateKeyFinding> {
    let mut findings: Vec<DuplicateKeyFinding> = Vec::new();
    for (document, doc) in file.documents().enumerate() {
        let root = match (doc.as_mapping(), doc.as_sequence()) {
            (Some(mapping), _) => YamlNode::Mapping(mapping),
            (None, Some(sequence)) => YamlNode::Sequence(sequence),
            // A scalar (or empty) document root carries no keys.
            (None, None) => continue,
        };
        walk_node(&root, path, source, document, &mut findings);
    }
    findings
}

/// Recursively check one node and everything it contains.
fn walk_node(
    node: &YamlNode,
    path: &str,
    source: &str,
    document: usize,
    findings: &mut Vec<DuplicateKeyFinding>,
) {
    match node {
        YamlNode::Mapping(mapping) => {
            walk_mapping_entries(mapping.entries(), path, source, document, findings)
        }
        YamlNode::Sequence(sequence) => {
            for index in 0..sequence.len() {
                if let Some(item) = sequence.get(index) {
                    walk_node(&item, path, source, document, findings);
                }
            }
        }
        // A `!!omap` tag wraps a sequence of single-key mappings whose
        // entries are walkable. Other tagged nodes: yaml-edit's public
        // API does not expose a tagged block mapping's inner value, and
        // no workflow or config file in this repository uses one — a
        // tagged scalar carries no keys either way.
        YamlNode::TaggedNode(tagged) => {
            if let Some(entries) = tagged.as_ordered_mapping() {
                walk_mapping_entries(entries, path, source, document, findings);
            }
        }
        // Scalars and aliases carry no keys of their own.
        YamlNode::Scalar(_) | YamlNode::Alias(_) => {}
    }
}

/// Check one mapping's entries for duplicate keys and recurse into values.
fn walk_mapping_entries(
    entries: impl IntoIterator<Item = MappingEntry>,
    path: &str,
    source: &str,
    document: usize,
    findings: &mut Vec<DuplicateKeyFinding>,
) {
    let mut first_seen: BTreeMap<String, usize> = BTreeMap::new();
    for entry in entries {
        let Some(key_node) = entry.key_node() else {
            continue;
        };
        let key = canonical_key(&key_node);
        let line = line_of_node(&key_node, source);
        match first_seen.get(&key).copied() {
            Some(first_line) => findings.push(DuplicateKeyFinding {
                path: path.to_string(),
                document,
                key,
                first_line,
                duplicate_line: line,
            }),
            None => {
                first_seen.insert(key, line);
            }
        }
        if let Some(value_node) = entry.value_node() {
            walk_node(&value_node, path, source, document, findings);
        }
    }
}

/// The key as a consumer sees it: unquoted for scalar keys, the raw text
/// for anything exotic (nested keys are legal YAML and must still be
/// comparable for equality).
fn canonical_key(key: &YamlNode) -> String {
    match key.as_scalar() {
        Some(scalar) => scalar.as_string(),
        None => key.to_string().trim().to_string(),
    }
}

/// One-based line of a node's first byte in the source it was parsed from.
///
/// The byte offset is recovered from the node's own text range and turned
/// into a line by counting newlines in `source` before it. A node without
/// a range (unreachable through parsing) reports line 0, which a record
/// reading it can tell apart from a real line.
fn line_of_node(node: &YamlNode, source: &str) -> usize {
    let Some(syntax) = node.as_node() else {
        return 0;
    };
    let offset = yaml_edit::advanced::syntax_node_range(syntax).start();
    // The offset comes from a node in exactly this source, so it is in
    // bounds; clamp defensively anyway — a line number must never panic.
    let offset = usize::from(offset).min(source.len());
    source[..offset].matches('\n').count() + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A clean workflow-shaped file: the positive case.
    const CLEAN_WORKFLOW: &str = r#"name: CI
on:
  push:
    branches: [main]
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Test
        run: cargo test
    timeout-minutes: 30
"#;

    /// The negative case: a deliberate duplicate. "Adding" `timeout-minutes:
    /// 300` below the existing `timeout-minutes: 30` is exactly the
    /// InferWeave#246 hazard — the diff shows an addition, the consumer
    /// keeps reading the first occurrence.
    const DUPLICATE_WORKFLOW: &str = r#"name: CI
on:
  push:
    branches: [main]
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Test
        run: cargo test
    timeout-minutes: 30
    timeout-minutes: 300
"#;

    #[test]
    fn a_clean_workflow_passes_the_check() {
        let verdict = check_yaml_sources(&[("ci.yml".to_string(), CLEAN_WORKFLOW.to_string())]);
        assert!(!verdict.is_blocking());
        assert_eq!(verdict.files_checked(), 1);
        assert!(verdict.duplicates().is_empty());
        assert!(verdict.parse_failures().is_empty());
        assert_eq!(
            verdict.line(),
            "yaml config clean: 1 file(s) checked, no duplicate keys"
        );
    }

    #[test]
    fn a_duplicate_key_blocks_and_is_named_with_both_lines() {
        // `timeout-minutes` first at line 12, duplicated at line 13.
        let verdict = check_yaml_sources(&[
            ("clean.yml".to_string(), CLEAN_WORKFLOW.to_string()),
            ("ci.yml".to_string(), DUPLICATE_WORKFLOW.to_string()),
        ]);
        assert!(verdict.is_blocking(), "a duplicate key must block");
        assert_eq!(verdict.files_checked(), 2);
        assert_eq!(verdict.duplicates().len(), 1);
        assert!(verdict.parse_failures().is_empty());
        let finding = &verdict.duplicates()[0];
        assert_eq!(finding.path, "ci.yml");
        assert_eq!(finding.document, 0);
        assert_eq!(finding.key, "timeout-minutes");
        assert_eq!(finding.first_line, 12);
        assert_eq!(finding.duplicate_line, 13);
        let line = verdict.line();
        assert!(line.starts_with("HELD: "), "{line}");
        assert!(
            line.contains("ci.yml:13 duplicate key `timeout-minutes` (first at 12)"),
            "{line}"
        );
        // The clean file is checked and stays out of the record.
        assert!(!line.contains("clean.yml"), "{line}");
    }

    #[test]
    fn the_loader_rejects_a_file_with_a_duplicate_and_names_it() {
        let error = load_yaml_rejecting_duplicates("ci.yml", DUPLICATE_WORKFLOW).unwrap_err();
        assert_eq!(
            error,
            YamlConfigError::Duplicates {
                findings: vec![DuplicateKeyFinding {
                    path: "ci.yml".to_string(),
                    document: 0,
                    key: "timeout-minutes".to_string(),
                    first_line: 12,
                    duplicate_line: 13,
                }]
            }
        );
        assert!(error
            .to_string()
            .contains("duplicate key `timeout-minutes`"));
        // The positive side: the same loader accepts the clean file and
        // hands the parsed file to the consumer.
        let file = load_yaml_rejecting_duplicates("ci.yml", CLEAN_WORKFLOW).unwrap();
        let doc = file.documents().next().unwrap();
        assert_eq!(doc.get_string("name").as_deref(), Some("CI"));
    }

    #[test]
    fn every_occurrence_is_reported_not_the_first() {
        // Three occurrences: the record names both later ones, each against
        // the first. A gate that stopped at the first would leave the
        // second duplicate invisible to whoever fixes the record.
        let source = "a: 1\na: 2\na: 3\nb: 4\nb: 5\nb: 6\n";
        let findings = find_duplicate_keys("triple.yaml", source).unwrap();
        assert_eq!(
            findings,
            vec![
                DuplicateKeyFinding {
                    path: "triple.yaml".to_string(),
                    document: 0,
                    key: "a".to_string(),
                    first_line: 1,
                    duplicate_line: 2,
                },
                DuplicateKeyFinding {
                    path: "triple.yaml".to_string(),
                    document: 0,
                    key: "a".to_string(),
                    first_line: 1,
                    duplicate_line: 3,
                },
                DuplicateKeyFinding {
                    path: "triple.yaml".to_string(),
                    document: 0,
                    key: "b".to_string(),
                    first_line: 4,
                    duplicate_line: 5,
                },
                DuplicateKeyFinding {
                    path: "triple.yaml".to_string(),
                    document: 0,
                    key: "b".to_string(),
                    first_line: 4,
                    duplicate_line: 6,
                },
            ]
        );
    }

    #[test]
    fn duplicates_in_nested_mappings_and_sequence_items_are_found() {
        // The duplicate hides where a quick eyeball of the top level
        // would not look: inside a sequence item, and one level deeper.
        let source = "\
steps:
  - name: one
    env:
      CI: true
      CI: false
  - name: two
    env:
      CI: true
jobs:
  build:
    runs-on: ubuntu-latest
    runs-on: macos-latest
";
        let findings = find_duplicate_keys("nested.yml", source).unwrap();
        assert_eq!(
            findings,
            vec![
                DuplicateKeyFinding {
                    path: "nested.yml".to_string(),
                    document: 0,
                    key: "CI".to_string(),
                    first_line: 4,
                    duplicate_line: 5,
                },
                DuplicateKeyFinding {
                    path: "nested.yml".to_string(),
                    document: 0,
                    key: "runs-on".to_string(),
                    first_line: 11,
                    duplicate_line: 12,
                },
            ]
        );
    }

    #[test]
    fn quoted_and_unquoted_forms_of_one_key_are_the_same_key() {
        // A consumer matches keys semantically: `"foo"`, `'foo'`, and `foo`
        // are one key, so a re-added quoted copy is a duplicate too.
        let source = "foo: 1\n'foo': 2\n\"bar\": 1\nbar: 2\nbaz: 1\n";
        let findings = find_duplicate_keys("quotes.yml", source).unwrap();
        assert_eq!(
            findings,
            vec![
                DuplicateKeyFinding {
                    path: "quotes.yml".to_string(),
                    document: 0,
                    key: "foo".to_string(),
                    first_line: 1,
                    duplicate_line: 2,
                },
                DuplicateKeyFinding {
                    path: "quotes.yml".to_string(),
                    document: 0,
                    key: "bar".to_string(),
                    first_line: 3,
                    duplicate_line: 4,
                },
            ]
        );
    }

    #[test]
    fn a_duplicate_in_the_second_document_of_a_multidoc_file_is_found() {
        let source = "---\na: 1\n---\nb: 2\nb: 3\n";
        let findings = find_duplicate_keys("multi.yml", source).unwrap();
        assert_eq!(
            findings,
            vec![DuplicateKeyFinding {
                path: "multi.yml".to_string(),
                document: 1,
                key: "b".to_string(),
                first_line: 4,
                duplicate_line: 5,
            }]
        );
    }

    #[test]
    fn flow_style_duplicates_are_found_too() {
        // `{a: 1, a: 2}` is the same hazard in one line.
        let source = "plain:\n  a: 1\n  a: 2\nflow: {b: 1, b: 2}\n";
        let findings = find_duplicate_keys("flow.yml", source).unwrap();
        let keys: Vec<(&str, usize, usize)> = findings
            .iter()
            .map(|f| (f.key.as_str(), f.first_line, f.duplicate_line))
            .collect();
        assert_eq!(keys, vec![("a", 2, 3), ("b", 4, 4)]);
    }

    #[test]
    fn duplicates_inside_an_omap_tag_are_found_too() {
        // A `!!omap` tag wraps a sequence of single-key mappings; its
        // entries are walkable and a repeated key in them is still a
        // duplicate for the check.
        let source = "config: !!omap\n  - retries: 1\n  - retries: 2\n";
        let findings = find_duplicate_keys("tagged.yml", source).unwrap();
        assert_eq!(
            findings.len(),
            1,
            "the omap duplicate must be named: {findings:?}"
        );
        assert_eq!(findings[0].key, "retries");
    }

    #[test]
    fn a_file_that_does_not_parse_is_a_blocking_failure_not_a_silent_pass() {
        let source = "jobs:\n  build:\n   runs-on: [unclosed\n";
        let verdict = check_yaml_sources(&[("broken.yml".to_string(), source.to_string())]);
        assert!(verdict.is_blocking(), "a parse failure must block");
        assert!(verdict.duplicates().is_empty());
        assert_eq!(verdict.parse_failures().len(), 1);
        let failure = &verdict.parse_failures()[0];
        assert_eq!(failure.path, "broken.yml");
        assert!(failure.line >= 1, "the failure names a line: {failure:?}");
        assert!(verdict.line().starts_with("HELD: "), "{}", verdict.line());

        let error = load_yaml_rejecting_duplicates("broken.yml", source).unwrap_err();
        assert!(matches!(error, YamlConfigError::Parse(_)), "{error:?}");
    }

    #[test]
    fn an_empty_file_is_clean_not_an_error() {
        assert!(find_duplicate_keys("empty.yml", "").unwrap().is_empty());
        let verdict = check_yaml_sources(&[("empty.yml".to_string(), String::new())]);
        assert!(!verdict.is_blocking());
        assert_eq!(consumer_scalar("", "anything"), Ok(None));
    }

    #[test]
    fn the_consumer_reads_the_first_occurrence_so_the_added_key_changes_nothing() {
        // The InferWeave#246 hazard, made concrete: the diff added
        // `timeout-minutes: 300`; the consumer still reads `30`. The edit
        // is invisible to the running config — the diff was the wrong
        // thing to assert on.
        assert_eq!(
            consumer_scalar(DUPLICATE_WORKFLOW, "jobs.build.timeout-minutes"),
            Ok(Some("30".to_string()))
        );
        // And the check catches exactly this file, so the hazard cannot
        // hide in a green pipeline.
        let verdict = check_yaml_sources(&[("ci.yml".to_string(), DUPLICATE_WORKFLOW.to_string())]);
        assert!(verdict.is_blocking());
        // The clean file's consumer value is what the diff would suggest.
        assert_eq!(
            consumer_scalar(CLEAN_WORKFLOW, "jobs.build.timeout-minutes"),
            Ok(Some("30".to_string()))
        );
    }

    #[test]
    fn consumer_scalar_reports_parse_failures_and_missing_paths() {
        assert!(consumer_scalar("a:\n - [unclosed\n", "a").is_err());
        assert_eq!(consumer_scalar(CLEAN_WORKFLOW, "jobs.build.nope"), Ok(None));
        // A path that resolves to a mapping, not a scalar.
        assert_eq!(consumer_scalar(CLEAN_WORKFLOW, "jobs"), Ok(None));
        // Unknown top-level key.
        assert_eq!(consumer_scalar(CLEAN_WORKFLOW, "nope"), Ok(None));
    }

    #[test]
    fn the_verdict_record_is_deterministic_across_file_orders() {
        let clean = ("clean.yml".to_string(), CLEAN_WORKFLOW.to_string());
        let dup = ("ci.yml".to_string(), DUPLICATE_WORKFLOW.to_string());
        let a = check_yaml_sources(&[dup.clone(), clean.clone()]);
        let b = check_yaml_sources(&[clean, dup]);
        assert_eq!(
            a.line(),
            b.line(),
            "the record must not depend on scan order"
        );
    }

    #[test]
    fn the_repos_own_workflow_and_config_files_are_duplicate_free() {
        // The check integrated: the repository's own workflow and config
        // YAML must pass the duplicate-key gate, or this test fails.
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .unwrap();
        let mut entries: Vec<(String, String)> = Vec::new();
        for dir in [".github/workflows", ".autospec"] {
            let full = root.join(dir);
            if let Ok(read) = std::fs::read_dir(&full) {
                for entry in read.flatten() {
                    let path = entry.path();
                    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                    if ext != "yml" && ext != "yaml" {
                        continue;
                    }
                    let Ok(source) = std::fs::read_to_string(&path) else {
                        continue;
                    };
                    let rel = path
                        .strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .to_string();
                    entries.push((rel, source));
                }
            }
        }
        if let Ok(source) = std::fs::read_to_string(root.join("negative-path-patterns.yml")) {
            entries.push(("negative-path-patterns.yml".to_string(), source));
        }
        assert!(
            !entries.is_empty(),
            "the scan found no repo YAML files at all"
        );
        let verdict = check_yaml_sources(&entries);
        assert!(
            !verdict.is_blocking(),
            "repo YAML failed the duplicate-key gate:\n{}",
            verdict.line()
        );
    }
}
