//! #4639: a constructor that counts a message's bytes must attach the message.
//!
//! Three sites in `validation/external.rs` built a failing result by adding
//! the message's LENGTH to `stderr_bytes` and folding the text into the
//! output digest, then dropping the text. The result carried the reason's
//! length and its hash while reporting "no reason captured"; a digest
//! assertion proved the message had been *consumed* at the exact moment it
//! was discarded, which is why the shape survived in three places at once.
//!
//! This test rejects both shapes at the source:
//!
//! 1. the mutation form — `x.stderr_bytes += MESSAGE.len()` without an
//!    attach (`.with_failure` / `with_failure_opt` / `captured_check_failure`)
//!    a few lines later;
//! 2. the constructor form — a failing `CheckResult::completed(...)` (exit
//!    code `1`) that passes a message's `.len()` as a byte count without an
//!    attach in the statement.
//!
//! A digest assertion can never catch this: a hash of X proves X was hashed,
//! not that X is readable or survived. The readable value is asserted by its
//! text in the module's own tests; this test guards the shape.

use std::fs;
use std::path::{Path, PathBuf};

/// The calls that attach a readable reason to a result.
const ATTACH_TOKENS: [&str; 3] = [
    ".with_failure(",
    "with_failure_opt(",
    "captured_check_failure(",
];

fn attach_within(lines: &[String], start: usize, window: usize) -> bool {
    let end = lines.len().min(start + window);
    (start..end).any(|index| {
        ATTACH_TOKENS
            .iter()
            .any(|token| lines[index].contains(token))
    })
}

/// Rule 1: the mutation form — a message counted into `stderr_bytes` without
/// an attach nearby.
fn find_count_without_attach(lines: &[String]) -> Vec<(usize, String)> {
    let mut hits = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.contains(".stderr_bytes +=") && trimmed.contains(".len()") {
            if !attach_within(lines, index, 7) {
                hits.push((index + 1, trimmed.to_string()));
            }
        }
    }
    hits
}

/// The `CheckResult::completed(...)` statements, each as (start line, lines).
///
/// A chain call (`.with_failure(...)` after the closing `);`) is part of the
/// statement: the attach may sit on either side of the constructor.
/// Where the statement starting at `index` ends: the call's closing `;` as a
/// statement, or `)` when the call is an expression — a chain that follows
/// the call is still the same statement. `None` when no closing line exists.
fn statement_end(lines: &[String], index: usize) -> Option<usize> {
    let mut end = index;
    while end + 1 < lines.len() {
        end += 1;
        let trimmed = lines[end].trim_end();
        if trimmed.ends_with(';') || trimmed.ends_with(')') {
            while end + 1 < lines.len() && lines[end + 1].trim_start().starts_with('.') {
                end += 1;
            }
            return Some(end);
        }
    }
    None
}

fn completed_statements(lines: &[String]) -> Vec<(usize, Vec<String>)> {
    let mut statements = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let line = &lines[index];
        // A `completed(` whose call closes on its own line is a one-line
        // statement; one that does not runs to `statement_end`; anything else
        // is not a call at all.
        let end = line.find("completed(").map(|position| {
            if line[position + "completed(".len()..].contains(')') {
                Some(index)
            } else {
                statement_end(lines, index)
            }
        });
        if let Some(end) = end.flatten() {
            statements.push((index, lines[index..=end].to_vec()));
            index = end + 1;
        } else {
            index += 1;
        }
    }
    statements
}

/// Rule 2: the constructor form — a failing `completed(...)` that counts a
/// message into a byte argument without an attach in the statement.
fn find_failing_completed_without_attach(lines: &[String]) -> Vec<(usize, String)> {
    let mut hits = Vec::new();
    for (start, statement) in completed_statements(lines) {
        // rustfmt puts each argument of an 8-argument call on its own line,
        // so the exit code is the fourth line: `completed(`, id, required, exit.
        let failing = statement
            .get(3)
            .map(|line| line.trim() == "1,")
            .unwrap_or(false);
        let counts_a_message = statement
            .iter()
            .any(|line| line.contains(".len(),") || line.trim_end().ends_with(".len()"));
        let attached = statement
            .iter()
            .any(|line| line.contains(".with_failure(") || line.contains("with_failure_opt("));
        if failing && counts_a_message && !attached {
            hits.push((start + 1, statement.join("\n")));
        }
    }
    hits
}

fn collect_rs_files(directory: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, out);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

#[test]
fn the_validation_module_counts_no_message_without_attaching_it() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/validation");
    let mut files = Vec::new();
    collect_rs_files(&root, &mut files);
    assert!(
        !files.is_empty(),
        "the validation module must exist to be scanned"
    );
    let mut violations = Vec::new();
    for file in files {
        let lines = fs::read_to_string(&file)
            .expect("validation source is readable text")
            .lines()
            .map(str::to_string)
            .collect::<Vec<_>>();
        for (line_number, text) in find_count_without_attach(&lines) {
            violations.push(format!(
                "{}:{}: counts a message into stderr_bytes without attaching it: {text}",
                file.display(),
                line_number
            ));
        }
        for (line_number, statement) in find_failing_completed_without_attach(&lines) {
            violations.push(format!(
                "{}:{}: a failing completed() counts a message without attaching it:\n{statement}",
                file.display(),
                line_number
            ));
        }
    }
    assert!(
        violations.is_empty(),
        "a byte count is not a reason:\n{}",
        violations.join("\n\n")
    );
}

#[test]
fn the_mutation_form_is_rejected_without_an_attach() {
    // The shape that sat in run_bash_help_usage: count the message, digest
    // it, drop it.
    let bad: Vec<String> = [
        "    if help.is_success() {",
        "        help.exit_code = Some(1);",
        "        help.stderr_bytes += MESSAGE.len();",
        "        help.output_digest = output_digest(&out, MESSAGE.as_bytes());",
        "    }",
        "    aggregate(vec![help])",
    ]
    .iter()
    .map(|line| line.to_string())
    .collect();
    assert_eq!(
        find_count_without_attach(&bad).len(),
        1,
        "the count-without-attach shape must be rejected"
    );

    // The same lines with the attach: the one helper keeps the text.
    let good: Vec<String> = [
        "    if help.is_success() {",
        "        help.exit_code = Some(1);",
        "        help.stderr_bytes += MESSAGE.len();",
        "        help.output_digest = output_digest(&out, MESSAGE.as_bytes());",
        "    }",
        "    captured_check_failure(help, &out, Some(MESSAGE))",
    ]
    .iter()
    .map(|line| line.to_string())
    .collect();
    assert!(
        find_count_without_attach(&good).is_empty(),
        "an attach within the window is the fix"
    );
}

#[test]
fn the_constructor_form_is_rejected_without_an_attach() {
    let base: Vec<String> = [
        "    CheckResult::completed(",
        "        id,",
        "        required,",
        "        1,",
        "        0,",
        "        0,",
        "        0,",
        "        message.len(),",
        "        output_digest(&[], message.as_bytes()),",
        "    )",
    ]
    .iter()
    .map(|line| line.to_string())
    .collect();
    assert_eq!(
        find_failing_completed_without_attach(&base).len(),
        1,
        "a failing completed() that counts a message must be rejected"
    );

    // The attach continues the statement past the constructor.
    let mut fixed = base.clone();
    fixed.push("    .with_failure(message)".to_string());
    assert!(
        find_failing_completed_without_attach(&fixed).is_empty(),
        "the attach in the statement is the fix"
    );

    // A passing check that counts real output is not the shape: the exit
    // code is 0, so nothing claims a failure whose reason is missing.
    let mut passing = base.clone();
    passing[3] = "        0,".to_string();
    assert!(
        find_failing_completed_without_attach(&passing).is_empty(),
        "only failing paths are held to the reason"
    );
}
