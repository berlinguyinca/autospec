//! Rule 2 — substitution detection.
//!
//! When a criterion names a dependency and the test sources create a fixture
//! *named after that dependency* — a script at `bin/docker`, a `fake_docker`
//! helper, a PATH prepend in front of the real binary — the run is not
//! evidence about the dependency. It is a simulation of it. The suite passes;
//! the criterion is untouched.
//!
//! The detector is a tripwire, not a prover. It cannot tell a faithful
//! test-harness double from a binary written to make a green tick appear, and
//! it is not asked to: the outcome of a hit is
//! `SUBSTITUTION-SUSPECTED` and a human decision, never acceptance. That is
//! the whole inversion the incident required — the old behaviour accepted the
//! fake silently.
//!
//! Scope is deliberately narrow, which is what keeps the tripwire usable:
//! a file is only examined for capabilities the **criterion** names, and a
//! fixture is only a substitution if the file both names the real binary and
//! builds or exposes it (writes it, symlinks it, or prepends a directory to
//! `PATH`). A test that merely *calls* `docker` on a real engine produces no
//! finding.

use std::fmt;

use super::capability::{binary_capability, named_capabilities, Capability};
use super::token_present;

/// A source file offered as the implementation of an acceptance criterion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Source<'a> {
    /// Repo-relative path, used in the report and in basename detection.
    pub path: &'a str,
    /// File contents.
    pub text: &'a str,
}

impl<'a> Source<'a> {
    /// Wrap a path and its contents.
    pub fn new(path: &'a str, text: &'a str) -> Self {
        Self { path, text }
    }

    /// The file name without its directory.
    fn file_name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(self.path)
    }
}

/// How a file stands in for a real binary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SubstitutionSignal {
    /// The file writes, creates or symlinks an executable named after the
    /// real binary — the shape the incident's `fs::write(bin.join("docker"))`
    /// takes.
    ShimWritten,
    /// The file puts a fixture directory in front of `PATH`, so a bare
    /// `docker` invocation resolves to the fixture.
    PathPrepended,
    /// The file defines a stand-in symbol after the real binary:
    /// `fake_docker`, `docker_shim`, `stub_psql`, `mock_chromium`.
    ShimSymbol,
}

impl SubstitutionSignal {
    /// The wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ShimWritten => "shim-written",
            Self::PathPrepended => "path-prepended",
            Self::ShimSymbol => "shim-symbol",
        }
    }
}

impl fmt::Display for SubstitutionSignal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One suspected substitution, pinned to a file and line.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SubstitutionFinding {
    /// Repo-relative file.
    pub file: String,
    /// 1-based line number.
    pub line: usize,
    /// The real binary the fixture is named after.
    pub shim: String,
    /// The dependency the criterion named and the fixture replaced.
    pub capability: Capability,
    /// How the file stands in for it.
    pub signal: SubstitutionSignal,
}

impl SubstitutionFinding {
    /// `file:line`, the locator a reviewer opens.
    pub fn locator(&self) -> String {
        format!("{}:{}", self.file, self.line)
    }

    /// The finding as one report fragment.
    pub fn fragment(&self) -> String {
        format!(
            "file={file} shim={shim} capability={capability} signal={signal}",
            file = self.locator(),
            shim = self.shim,
            capability = self.capability.as_str(),
            signal = self.signal.as_str(),
        )
    }
}

/// The audit result for one criterion and the sources offered for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubstitutionAudit {
    substituted: Vec<SubstitutionFinding>,
    named: Vec<Capability>,
}

impl SubstitutionAudit {
    /// The wire code carried when the audit is not clean.
    pub const CODE: &'static str = "SUBSTITUTION-SUSPECTED";

    /// Findings, ordered by file then line.
    pub fn findings(&self) -> &[SubstitutionFinding] {
        &self.substituted
    }

    /// Capabilities the criterion named, whether substituted or not.
    pub fn named(&self) -> &[Capability] {
        &self.named
    }

    /// True when at least one fixture stands in for a named dependency.
    pub fn substituted(&self) -> bool {
        !self.substituted.is_empty()
    }

    /// True when nothing was substituted, i.e. the sources are acceptable on
    /// this axis. A clean audit is not proof of real execution — it is the
    /// absence of a detected fake.
    pub fn accepts(&self) -> bool {
        self.substituted.is_empty()
    }

    /// The code, present only when the audit is not clean.
    pub fn code(&self) -> Option<&'static str> {
        self.substituted().then_some(Self::CODE)
    }

    /// Capabilities that were substituted, deduplicated.
    pub fn substituted_capabilities(&self) -> Vec<Capability> {
        let mut out: Vec<Capability> = self
            .substituted
            .iter()
            .map(|finding| finding.capability)
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// One-line report naming the code, the dependencies and every finding.
    pub fn report(&self) -> String {
        if self.accepts() {
            return "substitution=clean".to_string();
        }
        let dependencies = self
            .substituted_capabilities()
            .iter()
            .map(|capability| capability.as_str())
            .collect::<Vec<_>>()
            .join(",");
        let findings = self
            .substituted
            .iter()
            .map(SubstitutionFinding::fragment)
            .collect::<Vec<_>>()
            .join(" ");
        format!(
            "{code} dependencies={dependencies} not-accepted=review-required {findings}",
            code = Self::CODE
        )
    }
}

/// Audit `sources` for fixtures standing in for dependencies `criterion` names.
pub fn audit_substitution(criterion: &str, sources: &[Source<'_>]) -> SubstitutionAudit {
    let named = named_capabilities(criterion);
    if named.is_empty() {
        // Nothing was named, so nothing can be substituted for it. A fixture
        // for an unnamed helper is ordinary test hygiene.
        return SubstitutionAudit {
            substituted: Vec::new(),
            named,
        };
    }
    let mut findings = Vec::new();
    for source in sources {
        audit_source(&named, source, &mut findings);
    }
    findings.sort();
    findings.dedup();
    SubstitutionAudit {
        substituted: findings,
        named,
    }
}

/// Cue words that mean "this file builds an executable".
const WRITE_CUES: &[&str] = &[
    "fs::write",
    "write(",
    "write_all",
    "create(",
    "touch ",
    "chmod",
    "mkdir",
    "cat >",
    "printf ",
    "echo ",
    "ln -s",
    "symlink",
    "tee ",
];

/// Cue words that mean "this file exposes a directory ahead of the real one".
const PREPEND_CUES: &[&str] = &[
    ":$", ":${", ":{", "prepend", "insert(0", "unshift", "path:", "path=",
];

fn audit_source(named: &[Capability], source: &Source<'_>, out: &mut Vec<SubstitutionFinding>) {
    let lower = source.text.to_ascii_lowercase();
    let lines: Vec<String> = lower.lines().map(|line| line.to_string()).collect();
    let builds = lines
        .iter()
        .any(|line| WRITE_CUES.iter().any(|cue| line.contains(cue)));
    let prepend_line = lines.iter().position(|line| path_prepend_cue(line));

    for &capability in named {
        for binary in capability.binaries() {
            let name = binary.to_ascii_lowercase();
            // A committed fixture named after the binary, e.g.
            // `tests/fake-bin/docker`, is a substitution on its own.
            if source.file_name() == name {
                out.push(SubstitutionFinding {
                    file: source.path.to_string(),
                    line: 1,
                    shim: name.clone(),
                    capability,
                    signal: SubstitutionSignal::ShimWritten,
                });
                continue;
            }

            let mut mentions = false;
            for (index, line) in lines.iter().enumerate() {
                if token_present(line, &name) {
                    mentions = true;
                    if line_has_write_cue(line) || (builds && !line_is_call_only(line, &name)) {
                        out.push(SubstitutionFinding {
                            file: source.path.to_string(),
                            line: index + 1,
                            shim: name.clone(),
                            capability,
                            signal: SubstitutionSignal::ShimWritten,
                        });
                    }
                }
                if shim_symbol_present(line, &name) {
                    out.push(SubstitutionFinding {
                        file: source.path.to_string(),
                        line: index + 1,
                        shim: name.clone(),
                        capability,
                        signal: SubstitutionSignal::ShimSymbol,
                    });
                }
            }

            if mentions {
                // The PATH prepend is only attributable to this binary because
                // the file also names it; that link is what makes the fixture
                // the thing a bare invocation would resolve to.
                if let Some(line) = prepend_line {
                    out.push(SubstitutionFinding {
                        file: source.path.to_string(),
                        line: line + 1,
                        shim: name.clone(),
                        capability,
                        signal: SubstitutionSignal::PathPrepended,
                    });
                }
            }
        }
    }
}

fn line_has_write_cue(line: &str) -> bool {
    WRITE_CUES.iter().any(|cue| line.contains(cue))
}

/// A line that only invokes the binary (a real call, or an assertion on the
/// command string) does not build a fixture. Attribution then needs a write
/// cue on the same line, which keeps `assert!(cmd.contains("docker"))` in a
/// clean test from becoming a finding.
fn line_is_call_only(line: &str, name: &str) -> bool {
    let trimmed = line.trim_start();
    let invokes = trimmed.starts_with(&format!("{name} "))
        || trimmed.contains(&format!(" \"{name} "))
        || trimmed.contains(&format!("\"{name} "))
        || trimmed.contains(&format!("({name} "));
    invokes && !line_has_write_cue(line)
}

/// True when the line repoints `PATH` at a directory, the shape of
/// `env.set_var("PATH", format!("{}/bin:{}", tmp, env::var("PATH")))`.
fn path_prepend_cue(line: &str) -> bool {
    if !token_present(line, "path") {
        return false;
    }
    PREPEND_CUES.iter().any(|cue| line.contains(cue))
}

/// Detects `fake_docker`, `docker_shim`, `stub_psql`, `mock_chromium` and the
/// reversed orders, in either separator style.
fn shim_symbol_present(line: &str, binary: &str) -> bool {
    let stem = binary.replace('-', "_");
    ["fake_", "shim_", "stub_", "mock_", "dummy_"]
        .iter()
        .flat_map(|prefix| [format!("{prefix}{stem}"), format!("{stem}_{prefix}")])
        .any(|candidate| token_present(line, &candidate))
}

/// The capability a fixture path substitutes for, when the file is named after
/// one of the known binaries.
///
/// Exposed for callers that audit a diff's added paths rather than their
/// contents: `tests/fixtures/bin/docker` is a finding even before it is read.
pub fn substituted_capability_for_path(path: &str) -> Option<Capability> {
    let file_name = path.rsplit('/').next().unwrap_or(path);
    binary_capability(file_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CRITERION: &str = "A fresh container starts the compose stack and answers 200.";

    // The incident, reproduced as a test source: a script named `docker`,
    // made executable, put in front of PATH, and a suite that runs against it.
    const INCIDENT_SUITE: &str = concat!(
        "#[test]\n",
        "fn compose_stack_starts() {\n",
        "    let bin = tempdir().join(\"bin\");\n",
        "    fs::create_dir_all(&bin).unwrap();\n",
        "    fs::write(bin.join(\"docker\"), \"#!/bin/sh\\necho 'compose version: v2'\\n\").unwrap();\n",
        "    fs::set_permissions(&bin, Permissions::from_mode(0o755)).unwrap();\n",
        "    env::set_var(\"PATH\", format!(\"{}:{}\", bin.display(), env::var(\"PATH\").unwrap()));\n",
        "    let out = Command::new(\"docker\").arg(\"compose\").arg(\"up\").output().unwrap();\n",
        "    assert!(out.status.success());\n",
        "}\n",
    );

    fn audit(text: &str, path: &str) -> SubstitutionAudit {
        audit_substitution(CRITERION, &[Source::new(path, text)])
    }

    #[test]
    fn the_incident_suite_is_flagged_not_accepted() {
        let audit = audit(INCIDENT_SUITE, "tests/compose_smoke.rs");

        assert!(audit.substituted());
        assert!(!audit.accepts());
        assert_eq!(audit.code(), Some(SubstitutionAudit::CODE));
        assert_eq!(
            audit.substituted_capabilities(),
            vec![Capability::ContainerRuntime]
        );

        let written = audit
            .findings()
            .iter()
            .find(|finding| finding.signal == SubstitutionSignal::ShimWritten)
            .expect("the write of the shim is a finding");
        assert_eq!(written.shim, "docker");
        assert_eq!(written.locator(), "tests/compose_smoke.rs:5");

        assert!(audit
            .findings()
            .iter()
            .any(|finding| finding.signal == SubstitutionSignal::PathPrepended));
    }

    #[test]
    fn the_report_names_the_code_the_binary_and_the_line() {
        let report = audit(INCIDENT_SUITE, "tests/compose_smoke.rs").report();
        assert!(report.starts_with(SubstitutionAudit::CODE));
        assert!(report.contains("dependencies=container-runtime"));
        assert!(report.contains("file=tests/compose_smoke.rs:5"));
        assert!(report.contains("shim=docker"));
        assert!(report.contains("signal=shim-written"));
        assert!(report.contains("signal=path-prepended"));
        assert!(report.contains("not-accepted=review-required"));
    }

    #[test]
    fn a_shim_symbol_is_a_finding() {
        let source = concat!(
            "fn fake_docker(args: &[String]) -> Output {\n",
            "    Output::success()\n",
            "}\n",
        );
        let audit = audit(source, "tests/compose.rs");
        assert_eq!(audit.findings().len(), 1);
        assert_eq!(audit.findings()[0].signal, SubstitutionSignal::ShimSymbol);
        assert_eq!(audit.findings()[0].line, 1);
    }

    #[test]
    fn a_committed_fixture_named_after_the_binary_is_a_finding() {
        let audit = audit("#!/bin/sh\\necho ok\\n", "tests/fixtures/bin/docker");
        assert_eq!(audit.findings().len(), 1);
        assert_eq!(audit.findings()[0].signal, SubstitutionSignal::ShimWritten);
        assert_eq!(audit.findings()[0].line, 1);
    }

    #[test]
    fn a_real_invocation_of_a_real_binary_is_not_a_finding() {
        // The engine is present and the test drives it; there is no fixture.
        let source = concat!(
            "let out = Command::new(\"docker\").arg(\"compose\").arg(\"up\").output()?;\n",
            "assert!(out.status.success());\n",
        );
        let audit = audit(source, "tests/compose.rs");
        assert!(audit.accepts(), "{}", audit.report());
        assert_eq!(audit.code(), None);
        assert_eq!(audit.report(), "substitution=clean");
    }

    #[test]
    fn an_assertion_mentioning_the_binary_is_not_a_finding() {
        let source = "assert!(plan.command().contains(\"docker compose up\"));\n";
        assert!(audit(source, "tests/cli.rs").accepts());
    }

    #[test]
    fn a_fixture_for_an_unnamed_dependency_is_not_a_substitution() {
        // The criterion names no external dependency, so writing a fake binary
        // is ordinary test hygiene, not a fake of something required.
        let source = INCIDENT_SUITE;
        let audit = audit_substitution(
            "The parser rejects a malformed frontmatter line with exit code 3.",
            &[Source::new("tests/parser.rs", source)],
        );
        assert!(audit.accepts());
        assert!(audit.named().is_empty());
    }

    #[test]
    fn detection_is_scoped_to_the_dependencies_the_criterion_names() {
        // The file fakes a database; the criterion asked for a container.
        let source = "fs::write(bin.join(\"psql\"), \"#!/bin/sh\\n\").unwrap();\n";
        assert!(audit(source, "tests/compose.rs").accepts());

        let audit = audit_substitution(
            "The suite connects to a real postgres server and reads a row.",
            &[Source::new("tests/compose.rs", source)],
        );
        assert!(audit.substituted());
        assert_eq!(audit.findings()[0].shim, "psql");
        assert_eq!(audit.findings()[0].capability, Capability::Database);
    }

    #[test]
    fn path_prepend_without_a_named_binary_is_not_attributed() {
        let source =
            "env::set_var(\"PATH\", format!(\"{}:{}\", dir.display(), env::var(\"PATH\")?));\n";
        assert!(audit(source, "tests/toolchain.rs").accepts());
    }

    #[test]
    fn a_browser_criterion_catches_a_fake_chromedriver() {
        let source = "fs::write(tmp.join(\"chromedriver\"), script).unwrap();\n";
        let audit = audit_substitution(
            "The checkout flow completes in a real browser.",
            &[Source::new("tests/checkout.rs", source)],
        );
        assert!(audit.substituted());
        assert_eq!(audit.findings()[0].capability, Capability::Browser);
        assert_eq!(audit.findings()[0].shim, "chromedriver");
    }

    #[test]
    fn docker_compose_is_its_own_binary_and_not_a_docker_hit() {
        let source = "fs::write(bin.join(\"docker-compose\"), script).unwrap();\n";
        let audit = audit(source, "tests/compose.rs");
        assert!(audit
            .findings()
            .iter()
            .all(|finding| finding.shim == "docker-compose"));
    }

    #[test]
    fn path_lookup_resolves_a_fixture_path_to_its_capability() {
        assert_eq!(
            substituted_capability_for_path("tests/fixtures/bin/docker"),
            Some(Capability::ContainerRuntime)
        );
        assert_eq!(substituted_capability_for_path("src/main.rs"), None);
    }
}
