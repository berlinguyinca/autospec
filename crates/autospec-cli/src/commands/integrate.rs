//! `autospec integrate` — the integration phase (issue #3565).
//!
//! Rebase and land a batch of parallel-implementer branches onto a base in
//! dependency order, gating each conflict resolution on symbol preservation
//! (not on a green build), and re-verifying after every rebase.
//!
//! Usage:
//!   autospec integrate --repo <path> --base <trunk> --branch <b> [--branch <b> ...] \
//!                      [--verify-cmd '<shell>'] [--json]
//!
//! Branches are listed in the order they should land. With `--verify-cmd`,
//! the command is executed (via `/bin/sh -c`) in the repo after each
//! rebase; a non-zero exit fails the integration of that branch, naming the
//! branch. Without it, no local check runs and the symbol-preservation gate
//! is the only gate.

use crate::commands::CommandFailure;
use autospec_core::integration::{
    BatchReport, BranchOutcome, BranchResult, GitVcs, IntegrationPhase, NoopVerifier,
    UnionResolver, Verifier, VerifyOutcome,
};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::Command;

pub fn run(args: &[String]) -> Result<(), CommandFailure> {
    let mut repo: Option<String> = None;
    let mut base: Option<String> = None;
    let mut branches: Vec<String> = Vec::new();
    let mut verify_cmd: Option<String> = None;
    let as_json = super::is_json(args);

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--help" | "-h" => {
                print_usage();
                return Ok(());
            }
            "--repo" => {
                i += 1;
                repo = Some(
                    args.get(i)
                        .ok_or("integrate: --repo requires a value")
                        .map_err(CommandFailure::diagnostic)?
                        .clone(),
                );
            }
            "--base" => {
                i += 1;
                base = Some(
                    args.get(i)
                        .ok_or("integrate: --base requires a value")
                        .map_err(CommandFailure::diagnostic)?
                        .clone(),
                );
            }
            "--branch" => {
                i += 1;
                branches.push(
                    args.get(i)
                        .ok_or("integrate: --branch requires a value")
                        .map_err(CommandFailure::diagnostic)?
                        .clone(),
                );
            }
            "--verify-cmd" => {
                i += 1;
                verify_cmd = Some(
                    args.get(i)
                        .ok_or("integrate: --verify-cmd requires a value")
                        .map_err(CommandFailure::diagnostic)?
                        .clone(),
                );
            }
            "--json" => {}
            other => {
                return Err(CommandFailure::diagnostic(format!(
                    "integrate: unknown argument: {other} (see --help)"
                )))
            }
        }
        i += 1;
    }

    let repo: PathBuf = PathBuf::from(
        repo.ok_or_else(|| CommandFailure::diagnostic("integrate: --repo <path> is required"))?,
    );
    let base = base.ok_or_else(|| {
        CommandFailure::diagnostic("integrate: --base <trunk-branch> is required")
    })?;
    if branches.is_empty() {
        return Err(CommandFailure::diagnostic(
            "integrate: at least one --branch <name> is required",
        ));
    }

    let vcs = GitVcs::new(repo.clone(), base, branches);
    let verifier: Box<dyn Verifier> = match verify_cmd {
        Some(cmd) => Box::new(ShellVerifier {
            cmd,
            repo: repo.clone(),
        }),
        None => Box::new(NoopVerifier),
    };
    let phase = IntegrationPhase::new(vcs, UnionResolver, verifier);
    let report = phase.run();

    if as_json {
        println!("{}", report_json(&report).to_string());
    } else {
        print_report(&report);
    }

    if report.ok() {
        Ok(())
    } else {
        Err(CommandFailure::diagnostic(format!(
            "integration incomplete: {} of {} branch(es) not landed",
            report.halted().len(),
            report.results.len()
        )))
    }
}

fn print_usage() {
    eprintln!(
        "autospec integrate — rebase and land a batch with symbol-preservation gates\n\
         \n\
         Usage:\n\
         \x20 autospec integrate --repo <path> --base <trunk> --branch <b> [--branch <b> ...]\n\
         \x20                  [--verify-cmd '<shell command>'] [--json]\n\
         \n\
         --repo <path>        path to the git repository\n\
         --base <trunk>       branch to rebase onto and land into (e.g. main)\n\
         --branch <name>      a branch of the batch; repeat in dependency (landing) order\n\
         --verify-cmd <cmd>   run after every rebase via /bin/sh -c in the repo;\n\
         \x20                  non-zero exit fails that branch's integration (named in output)\n\
         --json               emit the batch report as JSON\n\
         \n\
         Exit codes: 0 all landed; 2 one or more branches halted (see output for which\n\
         and why).\n\
         \n\
         A branch that conflicts additively is union-resolved and gated on\n\
         symbol preservation; a branch whose sides genuinely disagree is halted\n\
         with the offending hunk reported; no branch is ever skipped."
    );
}

/// Runs the configured shell command in the repo after each rebase.
struct ShellVerifier {
    cmd: String,
    repo: PathBuf,
}

impl Verifier for ShellVerifier {
    fn verify(&self, _branch: &str) -> VerifyOutcome {
        let output = match Command::new("/bin/sh")
            .arg("-c")
            .arg(&self.cmd)
            .current_dir(&self.repo)
            .output()
        {
            Ok(output) => output,
            Err(e) => {
                return VerifyOutcome::Failed {
                    reason: format!("verify command failed to start: {e}"),
                }
            }
        };
        if output.status.success() {
            VerifyOutcome::Passed
        } else {
            let mut text = String::from_utf8_lossy(&output.stderr).to_string();
            if text.is_empty() {
                text = String::from_utf8_lossy(&output.stdout).to_string();
            }
            let code = output.status.code().unwrap_or(-1);
            VerifyOutcome::Failed {
                reason: format!("exit {code}: {}", truncate(text.trim(), 400)),
            }
        }
    }
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        let cut: String = text.chars().take(max).collect();
        format!("{cut}…")
    }
}

fn branch_result_json(result: &BranchResult) -> Value {
    let (outcome, extra) = match &result.outcome {
        BranchOutcome::Landed => ("landed", json!({})),
        BranchOutcome::RejectedSymbolLoss { missing } => (
            "rejected_symbol_loss",
            json!({
                "missing_symbols": missing
                    .iter()
                    .map(|m| json!({ "file": m.file, "symbol": m.symbol }))
                    .collect::<Vec<_>>(),
            }),
        ),
        BranchOutcome::HaltedSemantic { conflicts } => (
            "halted_semantic",
            json!({
                "conflicts": conflicts
                    .iter()
                    .map(|c| json!({
                        "file": c.file,
                        "line": c.start + 1,
                        "reason": c.reason,
                        "ancestor": c.ancestor,
                        "ours": c.ours,
                        "theirs": c.theirs,
                    }))
                    .collect::<Vec<_>>(),
            }),
        ),
        BranchOutcome::VerificationFailed { reason } => {
            ("verification_failed", json!({ "reason": reason }))
        }
        BranchOutcome::ResolveError { message } => ("resolve_error", json!({ "message": message })),
        BranchOutcome::VcsError { message } => ("vcs_error", json!({ "message": message })),
    };
    json!({
        "branch": result.branch,
        "outcome": outcome,
        "detail": result.outcome.describe(),
        "extra": extra,
    })
}

fn report_json(report: &BatchReport) -> Value {
    let mut value = json!({
        "ok": report.ok(),
        "results": report
            .results
            .iter()
            .map(branch_result_json)
            .collect::<Vec<_>>(),
        "landed": report.landed(),
        "halted": report.halted(),
    });
    if let Some(error) = &report.error {
        value["error"] = json!(error);
    }
    value
}

fn print_report(report: &BatchReport) {
    if let Some(error) = &report.error {
        println!("error: {error}");
        return;
    }
    for result in &report.results {
        println!("  {}: {}", result.branch, result.outcome.describe());
    }
    println!(
        "\nintegration: {} of {} branch(es) landed",
        report.landed().len(),
        report.results.len()
    );
}
