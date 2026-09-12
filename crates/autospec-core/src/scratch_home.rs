//! Home of recurring automation (issue #4387).
//!
//! The incident: the patch-to-PR conversion pass — selecting candidate
//! patches, applying them, running the gate, opening PRs, recording HELD
//! reasons — lived as a set of shell scripts in a session temp directory,
//! beside ~60 git worktrees and dozens of ad-hoc logs, so the real tooling
//! was indistinguishable from debris. When the scratch directory was
//! cleared, the capability was simply gone, along with every lesson
//! embedded in it.
//!
//! The invariant: automation that will run more than once does not live in
//! a temp directory. As soon as a script is invoked a second time on a
//! different input, it moves into a repository with a test — or it is
//! deliberately thrown away after use. A spec for a recurring operational
//! process must name where the implementation lives and what tests it
//! carries; "a script that does X" with no home is a design defect.
//!
//! The checkable core here is that spec rule: [`lint_spec_scratch_home`]
//! reports a spec that names a scratch *tool* path — a literal temp path
//! with a recognized tool extension — without also naming a
//! repository-relative implementation home and a test.

/// Rule ID emitted by [`lint_spec_scratch_home`].
pub const SCRATCH_HOME_RULE_ID: &str = "SCRATCH_HOME";

/// Prefixes that mark a path as scratch. The same list the
/// scratch-promotion ratchet (`scripts/lint-scratch-promotion.sh`) uses,
/// so a path the ratchet treats as scratch is scratch here.
pub const SCRATCH_PREFIXES: &[&str] = &[
    "/tmp/",
    "/var/tmp/",
    "/private/tmp/",
    "${TMPDIR}/",
    "$TMPDIR/",
];

/// Extensions that mark a scratch path as a *tool* rather than a log or
/// other debris. The same list the scratch-promotion ratchet counts.
pub const TOOL_EXTENSIONS: &[&str] = &[
    ".sh", ".bash", ".py", ".bats", ".rb", ".pl", ".js", ".mjs", ".ts",
];

/// Escape-hatch marker: a spec line carrying `linter:allow-SCRATCH_HOME`
/// followed by a mandatory reason names a deliberately one-shot script.
const ESCAPE_HATCH: &str = "linter:allow-SCRATCH_HOME";

/// One design defect found in a spec document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScratchHomeFinding {
    /// Scratch tool paths the spec names, deduplicated, in order of first
    /// appearance.
    pub scratch_tools: Vec<String>,
    /// Whether the spec names a repository-relative implementation home.
    pub has_repo_home: bool,
    /// Whether the spec names a test path under a `tests` segment.
    pub has_test: bool,
}

impl ScratchHomeFinding {
    /// Stable rule identifier for the finding.
    pub const fn rule_id(&self) -> &'static str {
        SCRATCH_HOME_RULE_ID
    }

    /// The pieces the spec fails to name, in a stable order.
    pub fn missing(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if !self.has_repo_home {
            out.push("no repository home");
        }
        if !self.has_test {
            out.push("no test");
        }
        out
    }

    /// Human-readable diagnostic for the finding.
    pub fn message(&self) -> String {
        let tools = self.scratch_tools.join(", ");
        let missing = self.missing().join(" and ");
        format!(
            "spec names scratch tool path(s) {tools} as the home of recurring \
             automation but names {missing} (issue #4387: recurring automation \
             does not live in a temp directory — a spec for a recurring process \
             must name where the implementation lives and what tests it carries)"
        )
    }
}

/// Whether a path token sits under a scratch prefix.
pub fn is_scratch_path(token: &str) -> bool {
    SCRATCH_PREFIXES
        .iter()
        .any(|prefix| token.starts_with(prefix))
}

/// Whether a scratch path has a one-task lifetime: an mktemp template,
/// unique per invocation. Writing there is the *correct* use of scratch,
/// and nothing holds it to the repository-home standard.
pub fn is_scratch_template(token: &str) -> bool {
    is_scratch_path(token) && (token.contains("XXXXXX") || token.contains("$$"))
}

/// Scratch tool paths named in one line: literal scratch paths with a
/// recognized tool extension, excluding mktemp templates.
pub fn scratch_tool_paths(line: &str) -> Vec<String> {
    path_tokens(line)
        .into_iter()
        .filter(|token| {
            is_scratch_path(token) && !is_scratch_template(token) && has_tool_extension(token)
        })
        .collect()
}

/// Whether a token looks like a repository-relative implementation home:
/// no leading `/` or `$`, at least one directory separator, a dotted final
/// segment, and not a documentation file — docs describe an implementation,
/// they are not its home.
pub fn is_repo_home_candidate(token: &str) -> bool {
    if token.starts_with('/') || token.starts_with('$') || !token.contains('/') {
        return false;
    }
    let segment = last_segment(token);
    if !segment.contains('.') {
        return false;
    }
    let lowered = segment.to_ascii_lowercase();
    !(lowered.ends_with(".md") || lowered.ends_with(".markdown"))
}

/// Whether a token looks like a test path: a repository-relative path that
/// carries a `tests` segment (`tests/...`, `crates/.../tests/...`).
pub fn is_test_candidate(token: &str) -> bool {
    !token.starts_with('/')
        && !token.starts_with('$')
        && token.split('/').any(|segment| segment == "tests")
}

/// Review a spec and return the design defect, if any.
///
/// A spec that names one or more scratch tool paths must also name a
/// repository home and a test; otherwise it is a recurring process with no
/// home. At most one finding is returned, listing every scratch tool the
/// spec named and exactly which piece is missing (home, test, or both). A
/// `linter:allow-SCRATCH_HOME <reason>` line with a non-empty reason
/// suppresses the finding; a bare marker is rejected and the finding
/// stands.
pub fn lint_spec_scratch_home(source: &str) -> Vec<ScratchHomeFinding> {
    let mut tools: Vec<String> = Vec::new();
    let mut has_home = false;
    let mut has_test = false;
    let mut escaped = false;

    for line in source.lines() {
        let trimmed = line.trim();
        if let Some(reason) = escape_reason(trimmed) {
            if !reason.is_empty() {
                escaped = true;
            }
            continue;
        }
        for token in path_tokens(line) {
            if is_scratch_path(&token)
                && !is_scratch_template(&token)
                && has_tool_extension(&token)
                && !tools.contains(&token)
            {
                tools.push(token.clone());
            }
            // A `tests/` path names a test, not an implementation home.
            if is_test_candidate(&token) {
                has_test = true;
            } else if is_repo_home_candidate(&token) {
                has_home = true;
            }
        }
    }

    if tools.is_empty() || escaped || (has_home && has_test) {
        return Vec::new();
    }
    vec![ScratchHomeFinding {
        scratch_tools: tools,
        has_repo_home: has_home,
        has_test: has_test,
    }]
}

/// The remainder of an escape-hatch line after the marker, or `None` when
/// the line carries no marker.
fn escape_reason(line: &str) -> Option<&str> {
    line.split(ESCAPE_HATCH).nth(1).map(str::trim)
}

fn last_segment(token: &str) -> &str {
    token.rsplit('/').next().unwrap_or(token)
}

fn has_tool_extension(token: &str) -> bool {
    let segment = last_segment(token).to_ascii_lowercase();
    TOOL_EXTENSIONS.iter().any(|ext| segment.ends_with(ext))
}

fn is_path_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '$' | '{' | '}' | '/' | '-')
}

/// Path-shaped tokens on a line: maximal runs of path characters, with
/// trailing sentence punctuation stripped. A static text scan, not an
/// interpreter — variables and quoted fragments are invisible to it.
fn path_tokens(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    for run in line.split(|c: char| !is_path_char(c)) {
        let token = run.trim_end_matches(['.', ',', ';', ':']);
        if !token.is_empty() {
            out.push(token.to_owned());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scratch_tool_paths_are_recognized_per_prefix() {
        for line in [
            "the pass runs /tmp/conv/convpass.sh daily",
            "gate: /var/tmp/gate.sh",
            "copy ${TMPDIR}/x/run.py into the worktree",
            "$TMPDIR/once.sh",
        ] {
            assert!(!scratch_tool_paths(line).is_empty(), "{line}");
        }
    }

    #[test]
    fn scratch_debris_is_not_a_tool() {
        assert!(scratch_tool_paths("notes go to /tmp/conv.log").is_empty());
        assert!(scratch_tool_paths("worktrees under /tmp/wt-feat-x").is_empty());
    }

    #[test]
    fn mktemp_templates_are_one_task_lifetime() {
        let line = "worker writes /tmp/conv.XXXXXX/run.sh per invocation";
        assert!(scratch_tool_paths(line).is_empty());
        assert!(is_scratch_template("/tmp/conv.XXXXXX/run.sh"));
        assert!(is_scratch_template("$TMPDIR/$$.sh"));
        assert!(!is_scratch_template("/tmp/conv/convpass.sh"));
    }

    #[test]
    fn repo_home_candidates() {
        assert!(is_repo_home_candidate("scripts/convpass.sh"));
        assert!(is_repo_home_candidate(
            "crates/autospec-core/src/scratch_home.rs"
        ));
        assert!(is_repo_home_candidate("tests/convpass.bats"));
        assert!(!is_repo_home_candidate("docs/design.md"));
        assert!(!is_repo_home_candidate("design.markdown"));
        assert!(!is_repo_home_candidate("/tmp/conv/convpass.sh"));
        assert!(!is_repo_home_candidate("main"));
    }

    #[test]
    fn test_candidates() {
        assert!(is_test_candidate("tests/convpass.bats"));
        assert!(is_test_candidate(
            "crates/autospec-core/tests/scratch_home.rs"
        ));
        assert!(!is_test_candidate("scripts/convpass.sh"));
        assert!(!is_test_candidate("in tests of the gate"));
    }

    #[test]
    fn spec_without_scratch_is_clean() {
        let source = "Implementation: scripts/convpass.sh, tested by tests/convpass.bats.\n";
        assert!(lint_spec_scratch_home(source).is_empty());
    }

    #[test]
    fn scratch_home_without_repo_home_or_test_is_a_finding() {
        let findings =
            lint_spec_scratch_home("The pass is /tmp/conv/convpass.sh and /tmp/conv/gate.sh.\n");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].rule_id(), SCRATCH_HOME_RULE_ID);
        assert_eq!(
            findings[0].scratch_tools,
            vec![
                "/tmp/conv/convpass.sh".to_owned(),
                "/tmp/conv/gate.sh".to_owned()
            ]
        );
        assert!(!findings[0].has_repo_home);
        assert!(!findings[0].has_test);
        assert_eq!(findings[0].missing(), vec!["no repository home", "no test"]);
    }

    #[test]
    fn repo_home_without_test_names_only_the_test_as_missing() {
        let findings =
            lint_spec_scratch_home("Move /tmp/conv/convpass.sh to scripts/convpass.sh.\n");
        assert_eq!(findings.len(), 1);
        assert!(findings[0].has_repo_home);
        assert!(!findings[0].has_test);
        assert_eq!(findings[0].missing(), vec!["no test"]);
    }

    #[test]
    fn scratch_path_with_named_home_and_test_is_clean() {
        let source = "\
The session copies /tmp/conv/convpass.sh for the first throwaway run;
the implementation lives at scripts/convpass.sh and tests/convpass.bats pins it.
";
        assert!(lint_spec_scratch_home(source).is_empty());
    }

    #[test]
    fn escape_hatch_requires_a_reason() {
        let with_reason =
            "linter:allow-SCRATCH_HOME one-shot migration, discarded after use\n/tmp/once.sh\n";
        assert!(lint_spec_scratch_home(with_reason).is_empty());
        let bare = "linter:allow-SCRATCH_HOME\n/tmp/once.sh\n";
        let findings = lint_spec_scratch_home(bare);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].scratch_tools, vec!["/tmp/once.sh".to_owned()]);
    }

    #[test]
    fn repeated_scratch_paths_are_deduplicated() {
        let source = "/tmp/conv/convpass.sh runs, then /tmp/conv/convpass.sh again.\n";
        let findings = lint_spec_scratch_home(source);
        assert_eq!(
            findings[0].scratch_tools,
            vec!["/tmp/conv/convpass.sh".to_owned()]
        );
    }

    #[test]
    fn message_names_tools_and_missing_pieces() {
        let findings = lint_spec_scratch_home("Run /var/tmp/gate.sh\n");
        let message = findings[0].message();
        assert!(message.contains("/var/tmp/gate.sh"), "{message}");
        assert!(message.contains("no repository home"), "{message}");
        assert!(message.contains("no test"), "{message}");
        assert!(message.contains("#4387"), "{message}");
    }
}
