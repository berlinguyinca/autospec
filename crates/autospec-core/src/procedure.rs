//! Operating-procedure check (issue #3772).
//!
//! The reported incident: a standing operating instruction told an agent to
//! "run `refresh-queue.sh` so filed issues reach dispatch". The script did not
//! exist in the fleet — not anywhere under it. The instruction was not skipped,
//! not failed, not logged: the agent noticed the script was missing, did the
//! work by hand, and the work got done. The missing piece was masked by the
//! human who happened to be there, not caught by anything.
//!
//! An operating procedure is the standing instruction an operator or agent
//! follows. This module makes one machine-checkable:
//!
//! 1. **Each step names a command** (or is explicitly manual). A step is
//!    either [`StepAction::Run`] — it names a command to run — or
//!    [`StepAction::Manual`]. There is no third "unspecified" state a step can
//!    fall into by omission.
//! 2. **A validation pass confirms every named command resolves before the
//!    procedure is published.** [`validate`] takes a resolver and returns every
//!    step whose command does not resolve.
//! 3. **A step whose command does not resolve fails the check** with the
//!    step's name and the unresolved command ([`UnresolvedStep`]) — it is not
//!    skipped at runtime.
//! 4. **"Manual" is a declared property, not a default.** A step is manual only
//!    when it is constructed as one ([`Step::manual`]); a step that names no
//!    command and is not marked manual simply cannot be expressed, so an
//!    unimplemented step can never silently pass as manual.
//!
//! Resolution is injected: the check confirms a command "resolves to an
//! executable artifact" without running it. [`CommandResolver`] is a concrete
//! resolver that resolves a command to an existing file under a set of roots;
//! any `Fn(&str) -> bool` will do.
//!
//! The rule in one line: a procedure is published only after every step that
//! runs a command points at an artifact that actually exists, and "manual" is
//! something you write down, not something a gap defaults to.

use std::fmt;
use std::path::{Path, PathBuf};

/// The action a step performs.
///
/// A step is *either* a command it runs *or* a deliberately manual action. The
/// type makes the "no command, not manual" state unrepresentable: that is the
/// state that used to swallow the missing `refresh-queue.sh`, so it cannot be
/// written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepAction {
    /// The step runs `command`; the command must resolve to an executable
    /// artifact before the procedure is published.
    Run { command: String },
    /// The step is deliberately manual. "Manual" is declared here, never
    /// inferred from the absence of a command.
    Manual,
}

impl StepAction {
    /// True when the step names a command to resolve (a manual step names none).
    pub fn names_command(&self) -> bool {
        matches!(self, Self::Run { .. })
    }
}

/// One named step in a [`Procedure`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// The step's name, as an operator would say it.
    pub name: String,
    /// What the step does: run a command, or a deliberately manual action.
    pub action: StepAction,
}

impl Step {
    /// A step that runs `command`. The command must resolve before the
    /// procedure is published.
    pub fn run(name: impl Into<String>, command: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            action: StepAction::Run {
                command: command.into(),
            },
        }
    }

    /// A deliberately manual step. This is the only way a step is manual:
    /// "manual" is declared, not defaulted.
    pub fn manual(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            action: StepAction::Manual,
        }
    }
}

/// A standing operating procedure: an ordered list of named steps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Procedure {
    /// The procedure's name.
    pub name: String,
    /// Its steps, in the order an operator follows them.
    pub steps: Vec<Step>,
}

impl Procedure {
    pub fn new(name: impl Into<String>, steps: Vec<Step>) -> Self {
        Self {
            name: name.into(),
            steps,
        }
    }

    /// True when every named command resolves under `resolves` — the gate a
    /// publisher clears before the procedure goes out. Manual steps name no
    /// command and never block publication.
    pub fn is_publishable(&self, resolves: &impl Fn(&str) -> bool) -> bool {
        unresolved_steps(self, resolves).is_empty()
    }
}

/// A step whose named command does not resolve to an executable artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnresolvedStep {
    /// The step's name, as written in the procedure.
    pub step: String,
    /// The command the step names, which did not resolve.
    pub command: String,
}

impl fmt::Display for UnresolvedStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "step `{}` names command `{}`, which does not resolve to an executable artifact",
            self.step, self.command
        )
    }
}

impl std::error::Error for UnresolvedStep {}

/// Every step in `procedure` whose command does not resolve under `resolves`.
///
/// Manual steps are skipped — they are declared manual, so they name no command
/// to resolve. The result is in procedure order.
pub fn unresolved_steps(
    procedure: &Procedure,
    resolves: &impl Fn(&str) -> bool,
) -> Vec<UnresolvedStep> {
    procedure
        .steps
        .iter()
        .filter_map(|step| match &step.action {
            StepAction::Run { command } if !resolves(command) => Some(UnresolvedStep {
                step: step.name.clone(),
                command: command.clone(),
            }),
            _ => None,
        })
        .collect()
}

/// The validation pass a procedure must clear before it is published.
///
/// Returns `Ok(())` when every named command resolves. Otherwise returns
/// `Err` carrying every unresolved step, in procedure order, each naming its
/// step and command — so the failure says *which* step and *which* command
/// broke, rather than the procedure being skipped at runtime.
pub fn validate(
    procedure: &Procedure,
    resolves: &impl Fn(&str) -> bool,
) -> Result<(), Vec<UnresolvedStep>> {
    let bad = unresolved_steps(procedure, resolves);
    if bad.is_empty() {
        Ok(())
    } else {
        Err(bad)
    }
}

/// A resolver that resolves a named command to an existing file under a set
/// of roots. It never runs the command; it only checks that the artifact the
/// command names is present.
#[derive(Debug, Clone, Default)]
pub struct CommandResolver {
    roots: Vec<PathBuf>,
}

impl CommandResolver {
    /// Resolve commands against the given roots (directories to search, or
    /// file prefixes to join onto).
    pub fn new(roots: Vec<PathBuf>) -> Self {
        Self { roots }
    }

    /// Resolve commands under a single directory.
    pub fn in_dir(root: impl Into<PathBuf>) -> Self {
        Self {
            roots: vec![root.into()],
        }
    }

    /// The command resolves when the file it names exists under one of the
    /// roots. `command` may be a path relative to a root (e.g.
    /// `scripts/refresh-queue.sh`) or an absolute path (checked as itself). An
    /// empty or blank command never resolves.
    pub fn resolves(&self, command: &str) -> bool {
        let command = command.trim();
        if command.is_empty() {
            return false;
        }
        let target = Path::new(command);
        if target.is_absolute() {
            target.is_file()
        } else {
            self.roots.iter().any(|root| root.join(target).is_file())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// A resolver over an in-memory set of "present" commands, standing in for
    /// the contents of a directory without touching the filesystem.
    fn present<'a>(cmds: impl IntoIterator<Item = &'a str>) -> BTreeSet<&'a str> {
        cmds.into_iter().collect()
    }

    /// The incident, in miniature. The standing instruction's first step names
    /// `refresh-queue.sh`; the fleet has no such script. The procedure must
    /// fail validation, naming both the step and the command.
    #[test]
    fn regression_missing_command_fails_validation() {
        let fleet = present([]); // the fleet directory has no such script
        let resolves = |cmd: &str| fleet.contains(cmd);

        let procedure = Procedure::new(
            "reconcile-queue",
            vec![Step::run("refresh dispatch queue", "refresh-queue.sh")],
        );

        let err = validate(&procedure, &resolves).expect_err("the fleet has no refresh-queue.sh");
        assert_eq!(err.len(), 1, "exactly one step is broken");
        assert_eq!(err[0].step, "refresh dispatch queue");
        assert_eq!(err[0].command, "refresh-queue.sh");
    }

    /// Same regression, driven by a real filesystem resolver: the crate dir has
    /// no `refresh-queue.sh`, so a step naming one fails.
    #[test]
    fn regression_missing_script_in_dir_fails() {
        let fleet = CommandResolver::in_dir(Path::new(env!("CARGO_MANIFEST_DIR")));
        let procedure = Procedure::new(
            "reconcile-queue",
            vec![Step::run("refresh dispatch queue", "refresh-queue.sh")],
        );

        let err = validate(&procedure, &|cmd| fleet.resolves(cmd))
            .expect_err("no refresh-queue.sh under the crate dir");
        assert_eq!(err[0].step, "refresh dispatch queue");
        assert_eq!(err[0].command, "refresh-queue.sh");
    }

    /// AC1: a procedure is machine-checkable — a validation pass confirms every
    /// named command resolves before publication.
    #[test]
    fn every_command_resolves_passes_validation() {
        let fleet = present(["refresh-queue.sh", "topup.sh", "dispatch-agent.sh"]);
        let procedure = Procedure::new(
            "reconcile-queue",
            vec![
                Step::run("refresh dispatch queue", "refresh-queue.sh"),
                Step::run("top up the pool", "topup.sh"),
                Step::run("start the dispatch agent", "dispatch-agent.sh"),
            ],
        );

        assert!(validate(&procedure, &|c| fleet.contains(c)).is_ok());
        assert!(procedure.is_publishable(&|c| fleet.contains(c)));
        assert!(unresolved_steps(&procedure, &|c| fleet.contains(c)).is_empty());
    }

    /// AC2: a step whose command does not resolve fails the check with the
    /// step's name and the unresolved command.
    #[test]
    fn unresolved_step_is_named_with_its_command() {
        let fleet = present(["topup.sh"]);
        let procedure = Procedure::new(
            "reconcile-queue",
            vec![
                Step::run("refresh dispatch queue", "refresh-queue.sh"),
                Step::run("top up the pool", "topup.sh"),
            ],
        );

        let err =
            validate(&procedure, &|c| fleet.contains(c)).expect_err("refresh-queue.sh is missing");
        assert_eq!(err.len(), 1);
        let bad = &err[0];
        assert_eq!(bad.step, "refresh dispatch queue");
        assert_eq!(bad.command, "refresh-queue.sh");
        assert!(bad.to_string().contains("refresh dispatch queue"));
        assert!(bad.to_string().contains("refresh-queue.sh"));
    }

    /// All unresolved steps are reported, in procedure order — not just the
    /// first.
    #[test]
    fn every_unresolved_step_is_reported_in_order() {
        let fleet = present([]);
        let procedure = Procedure::new(
            "reconcile-queue",
            vec![
                Step::run("first", "a.sh"),
                Step::run("second", "b.sh"),
                Step::run("third", "c.sh"),
            ],
        );

        let err = validate(&procedure, &|c| fleet.contains(c)).expect_err("nothing in the fleet");
        assert_eq!(
            err.iter().map(|s| s.command.as_str()).collect::<Vec<_>>(),
            vec!["a.sh", "b.sh", "c.sh"]
        );
    }

    /// AC3: a deliberately manual step is marked as such explicitly. A manual
    /// step names no command, so it is never resolved against and never blocks
    /// publication — it is declared, not defaulted.
    #[test]
    fn manual_step_is_declared_and_never_blocks() {
        let fleet = present([]);
        let procedure = Procedure::new(
            "reconcile-queue",
            vec![
                Step::manual("hand-carry the queue"),
                Step::run("top up the pool", "topup.sh"),
            ],
        );

        // The manual step is the only non-command step; the run step resolves.
        assert!(procedure.steps[0].action == StepAction::Manual);
        assert!(!procedure.steps[0].action.names_command());
        // Manual names nothing to resolve, so it cannot be flagged.
        let bad = unresolved_steps(&procedure, &|c| fleet.contains(c));
        assert!(
            !bad.iter().any(|s| s.step == "hand-carry the queue"),
            "a declared manual step must never be reported as unresolved"
        );
        // ... but the run step that is missing still fails the procedure.
        assert_eq!(
            bad.iter().map(|s| s.command.as_str()).collect::<Vec<_>>(),
            vec!["topup.sh"]
        );
    }

    /// A procedure that is only manual steps publishes even with an empty
    /// resolver: manual is a declared property, and there is nothing to resolve.
    #[test]
    fn all_manual_procedure_publishes_without_commands() {
        let procedure = Procedure::new(
            "hand-rollback",
            vec![
                Step::manual("pull the fleet"),
                Step::manual("reseed the queue"),
            ],
        );
        assert!(procedure.is_publishable(&|_| false));
        assert!(validate(&procedure, &|_| false).is_ok());
    }

    /// The only way a step passes validation without a resolving command is to
    /// be explicitly manual. (The "no command, not manual" state cannot be
    /// expressed, which is the point of the type.)
    #[test]
    fn only_manual_steps_skip_resolution() {
        let procedure = Procedure::new("mixed", vec![Step::run("a", "a.sh"), Step::manual("b")]);
        let commands: Vec<bool> = procedure
            .steps
            .iter()
            .map(|s| s.action.names_command())
            .collect();
        assert_eq!(commands, vec![true, false]);
    }

    #[test]
    fn command_resolver_resolves_existing_file_not_missing() {
        let resolver = CommandResolver::in_dir(Path::new(env!("CARGO_MANIFEST_DIR")));
        // The crate's own manifest exists; a made-up path and an empty command
        // do not resolve.
        assert!(resolver.resolves("Cargo.toml"), "the crate manifest exists");
        assert!(
            !resolver.resolves("no-such-file-xyz-12345"),
            "a made-up path does not resolve"
        );
        assert!(!resolver.resolves(""), "an empty command never resolves");
        assert!(!resolver.resolves("   "), "blank is empty");
    }

    #[test]
    fn command_resolver_checks_all_roots_and_absolute_paths() {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf();
        let resolver = CommandResolver::new(vec![
            manifest.clone(),
            PathBuf::from("/definitely/not/a/real/dir"),
        ]);
        assert!(
            resolver.resolves("Cargo.toml"),
            "found under the first root"
        );
        // An absolute path is checked as itself, not joined to a root.
        assert!(resolver.resolves(manifest.join("Cargo.toml").to_str().unwrap()));
        assert!(!resolver.resolves("/definitely/not/a/real/dir/ghost.sh"));
    }

    #[test]
    fn unresolved_step_display_names_step_and_command() {
        let step = UnresolvedStep {
            step: "refresh dispatch queue".to_owned(),
            command: "refresh-queue.sh".to_owned(),
        };
        let text = step.to_string();
        assert!(text.contains("refresh dispatch queue"), "{text}");
        assert!(text.contains("refresh-queue.sh"), "{text}");
    }
}
