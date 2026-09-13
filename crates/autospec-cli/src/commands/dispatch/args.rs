//! Shared argument discipline for the dispatch subcommands (#4568).
//!
//! Asking a command what it does must not make it do the thing. `dispatch
//! stamp --help` used to stamp the queue, because `stamp` takes no required
//! arguments, so the unrecognized flag fell through to execution rather
//! than to an error. The protection its siblings had was accidental —
//! `guard` and `freshness` were safe only because a missing required
//! argument stopped the run, not because they validated flags. Any future
//! subcommand with no required arguments would have inherited the same
//! behavior.
//!
//! So the discipline lives at the dispatcher, where every subcommand goes
//! through it and none can opt out:
//!
//! - `-h`/`--help` is recognized before any argument interpretation, in
//!   every subcommand, and prints usage — exit 0, nothing written.
//! - A flag no dispatch subcommand accepts is an error that names the flag
//!   — never an empty option set, never a silent mutation.
//!
//! `print_help` lives here with the discipline: the help text is what
//! `--help` prints, and it belongs to the layer that guarantees `--help` is
//! answered, not to a command that used to ignore it.

use super::{HOLD_EXIT, OK_EXIT, SUBCOMMANDS, CommandFailure};
use autospec_core::dispatch_pipeline::{
    DEFAULT_INTERVAL_SECS, DEFAULT_MAX_DISPATCH_ATTEMPTS, DEFAULT_MAX_STALE_INTERVALS,
    QUEUE_ARTIFACT,
};

/// Every flag any dispatch subcommand — including the spec subcommands in
/// `dispatch_spec` — accepts, so the dispatcher can tell a flag from a
/// typo. A flag outside this set names something no subcommand reads, and
/// executing the subcommand with it is not what was asked.
const KNOWN_FLAGS: &[&str] = &[
    "--admitted-file",
    "--at",
    "--body-file",
    "--body-updated-at",
    "--by",
    "--comments-json",
    "--container-runtime",
    "--covered-file",
    "--database",
    "--dry-run",
    "--duration-floor",
    "--fault-threshold",
    "--gate",
    "--help",
    "--issue",
    "--issue-json",
    "--interval",
    "--jq",
    "--json",
    "--lifecycle",
    "--live-json",
    "--live-updated-at",
    "--manifest",
    "--max-attempts",
    "--max-intervals",
    "--method",
    "--no-probe",
    "--now",
    "--out",
    "--out-dir",
    "--patch-name",
    "--prompt-file",
    "--queue",
    "--quote-bytes",
    "--reason",
    "--registry",
    "--repo",
    "--require-step",
    "--runs",
    "--source-updated-at",
    "--staged",
    "--staged-at",
    "--state-file",
    "--step",
    "--status-file",
    "--title",
    "--topology",
    "--transcript-dir",
];

/// Flags that take no value; every other known flag consumes the next
/// argument as its value, so the scan stays on flag boundaries and a value
/// that merely looks like a flag is not mistaken for one.
const BOOLEAN_FLAGS: &[&str] = &["--dry-run", "--json", "--no-probe"];

/// What a validated subcommand invocation is.
#[derive(Debug)]
pub(super) enum Invocation {
    /// `-h`/`--help` was given: print usage, write nothing.
    Help,
    /// Every flag was recognized: the subcommand may run.
    Run,
}

/// The one gate every dispatch subcommand goes through. Recognizes
/// `-h`/`--help` before any argument interpretation and rejects a flag no
/// dispatch subcommand accepts, naming it.
pub(super) fn classify(subcommand: &str, args: &[String]) -> Result<Invocation, CommandFailure> {
    let mut index = 0;
    while index < args.len() {
        let arg = args[index].as_str();
        if arg == "-h" {
            return Ok(Invocation::Help);
        }
        if let Some(stripped) = arg.strip_prefix("--") {
            let name = format!("--{}", stripped.split('=').next().unwrap_or(""));
            if name == "--help" {
                return Ok(Invocation::Help);
            }
            if !KNOWN_FLAGS.contains(&name.as_str()) {
                return Err(CommandFailure::diagnostic(format!(
                    "dispatch {subcommand}: unknown flag {arg} (no dispatch subcommand accepts it; known: {})",
                    KNOWN_FLAGS.join(", ")
                )));
            }
            // A value-taking flag given without `=` reads the next argument
            // as its value; consume it so the value is not scanned as a flag.
            if !BOOLEAN_FLAGS.contains(&name.as_str()) && !stripped.contains('=') {
                index += 1;
            }
        } else if arg.starts_with('-') {
            return Err(CommandFailure::diagnostic(format!(
                "dispatch {subcommand}: unknown flag {arg} (the only short flag is -h)"
            )));
        }
        index += 1;
    }
    Ok(Invocation::Run)
}

/// The full usage: what `-h`/`--help` answers with, in every subcommand.
pub(super) fn print_help() {
    println!("USAGE: autospec dispatch <subcommand> [options]");
    println!();
    println!("SUBCOMMANDS:");
    for (name, description) in SUBCOMMANDS {
        println!("    {name:<7} {description}");
    }
    println!("OPTIONS:");
    println!("    --queue <PATH>        Queue artifact (check/status/tick default $HOME/.autospec/{QUEUE_ARTIFACT}; stamp requires it)");
    println!("    --admitted-file <PATH>  check/reconcile/queue-gap: the tracker's admitted (eligible) set, one issue number per line");
    println!("    --covered-file <PATH>   queue-gap: issues a branch or PR already covers, one issue number per line (required)");
    println!("    --require-step <NAME=COMMAND>  queue-gap: a component the step needs; repeatable. An unresolved COMMAND fails the run.");
    println!("    --state-file <PATH>   Liveness ledger (default $HOME/.autospec/dispatch-liveness.json)");
    println!(
        "    --topology <PATH>     Topology JSON (default: built-in filing-to-dispatch chain)"
    );
    println!("    --step <NAME>         Hop the beat is for (required for beat)");
    println!(
        "    --issue <N>           Issue the guard, stage, freshness, or mark command acts on (required for mark)"
    );
    println!("    --issue-json <PATH>   stage: `gh api` issue payload (body, updatedAt, comments)");
    println!("    --comments-json <PATH> stage: `gh api .../comments` payload merged into the discussion");
    println!("    --body-file <PATH>    stage: verbatim body when no --issue-json is given");
    println!("    --source-updated-at <T> stage: live issue updatedAt (epoch or RFC 3339)");
    println!("    --out <PATH>          stage: staged spec to write (default $HOME/.autospec/dispatch/specs/<N>.md)");
    println!("    --staged <PATH>       freshness/preflight: staged spec to check (same default)");
    println!("    --prompt-file <PATH>  preflight: the assembled prompt; the dispatch is refused if it carries no issue text");
    println!("    --status-file <PATH>  freshness/preflight: append the spec receipt (byte count + sha256) to the run's status.txt");
    println!(
        "    --live-updated-at <T> freshness: the live issue updatedAt, when the caller read it"
    );
    println!("    --live-json <PATH>    freshness: issue payload to read updatedAt from");
    println!("    --repo <OWNER/NAME>   stage: read issue + comments live via `gh api` (else $AUTOSPEC_REPO)");
    println!("                          freshness: repository to query with `gh api`");
    println!("    --container-runtime <P> stage: runtime path, or 'absent' / 'not probed'");
    println!("    --database <VALUE>    stage: database availability, or 'absent' / 'not probed'");
    println!("    --registry <VALUE>    stage: registry reachability (never probed unless given)");
    println!("    --no-probe            stage: declare every environment fact as not probed");
    println!("    --out-dir <PATH>      Runner output root (default $HOME/.autospec/dispatch/out)");
    println!("    --patch-name <NAME>   Patch file under issue-<N> (default changes.patch)");
    println!("    --dry-run             guard: report the decision without touching the directory");
    println!(
        "    --by <NAME>           Producer named in the stamp (default: declared queue producer)"
    );
    println!(
        "    --runs <PATH>         runs: JSON of the batch (array of runs, or {{runs, subfleets}})"
    );
    println!(
        "    --duration-floor <S>  runs: seconds below which a run is LAUNCH-FAIL (default 30)"
    );
    println!("    --quote-bytes <N>     runs: transcripts of at most N bytes are quoted verbatim (default 4096)");
    println!("    --fault-threshold <N> runs: identical failures at/above N in one batch raise a fleet fault (default 3)");
    println!("    --out <PATH>          runs: where to write the agent-status.tsv record (default stdout)");
    println!("    --lifecycle <PATH>    Lifecycle ledger (default $HOME/.autospec/dispatch-lifecycle.json)");
    println!("    --action <A>          mark: produced / converted / hold / release / failed (required)");
    println!(
        "    --max-attempts <N>    tick: fresh dispatches without a patch tolerated before an entry is held, not redispatched (default {DEFAULT_MAX_DISPATCH_ATTEMPTS}, #4451)"
    );
    println!("    --reason <TEXT>       mark hold: why the entry is blocked (required for hold)");
    println!("    --at <EPOCH>          Beat timestamp in epoch seconds (default: current time)");
    println!("    --now <EPOCH>         Evaluate against this instant instead of the clock");
    println!("    --interval <SECONDS>  Interval for hops that declare none (default {DEFAULT_INTERVAL_SECS})");
    println!("    --max-intervals <N>   Missed intervals tolerated before a hold (default {DEFAULT_MAX_STALE_INTERVALS})");
    println!("    --json                Emit JSON");
    println!("    -h, --help            Print help (recognized in every subcommand; it is never an action, #4568)");
    println!();
    println!("EXIT CODES:");
    println!("    {OK_EXIT}  ok        queue fresh (proceed or genuinely idle) and every hop live");
    println!("    {HOLD_EXIT}  hold      queue missing/unstamped/stale, clock rewind, silent hop, or topology defect");
    println!("    {HOLD_EXIT}  stall     tick: queue non-empty but nothing dispatched (every skip named); mark: stamp refused");
    println!("    {HOLD_EXIT}  gap       queue-gap: eligible work in neither the queue nor a branch/PR, or a required component is absent");
    println!("    2  diagnostic usage error, unreadable artifact, or unparseable ledger");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_string).collect()
    }

    #[test]
    fn help_is_recognized_before_anything_else() {
        for text in ["--help", "-h", "--help --queue /q", "--queue /q --help", "-h --bogus"] {
            assert!(
                matches!(classify("stamp", &args(text)).unwrap(), Invocation::Help),
                "{text}"
            );
        }
    }

    #[test]
    fn an_unknown_flag_is_an_error_that_names_it() {
        let error = classify("stamp", &args("--bogus")).unwrap_err();
        let message = error.to_string();
        assert!(message.contains("--bogus"), "{message}");
        assert!(message.contains("dispatch stamp"), "{message}");
    }

    #[test]
    fn known_flags_pass_and_their_values_are_not_flags() {
        for text in [
            "",
            "--queue /tmp/q",
            "--json",
            "--dry-run",
            "--by refresh-queue --at 1789254032",
            "--require-step a=one --require-step b=two",
        ] {
            assert!(
                matches!(classify("guard", &args(text)).unwrap(), Invocation::Run),
                "{text}"
            );
        }
    }

    #[test]
    fn a_value_that_looks_like_a_flag_is_not_mistaken_for_one() {
        // `--by -x`: the value follows the value-taking flag, so it is
        // consumed, not validated as a flag.
        assert!(
            matches!(classify("stamp", &args("--by -x")).unwrap(), Invocation::Run)
        );
        // A bare short flag with no flag before it to own it is an error.
        assert!(classify("stamp", &args("-x")).is_err());
    }

    #[test]
    fn the_reported_invocation_answers_help_not_a_stamp() {
        // The exact shape from the report: a help request mixed with other
        // arguments. Help wins, because it is recognized before any
        // argument interpretation.
        let invocation = classify("stamp", &args("--help --bogus")).unwrap();
        assert!(matches!(invocation, Invocation::Help));
    }
}
