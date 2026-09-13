//! `dispatch tick` on a corrupted queue (#4536): a queue line that names
//! work but is not a positive issue number is refused with its line and
//! reported. A queue whose lines are all refused is a stall — the tick
//! exits non-zero and says which line was refused — not a silent idle.

use std::path::PathBuf;
use std::process::Output;

#[path = "support/temp_directory.rs"]
mod temp_directory;
use temp_directory::unique as temp_dir;

const NOW: u64 = 1_800_000_000;

struct Harness {
    temp: PathBuf,
    queue: PathBuf,
    state: PathBuf,
    lifecycle: PathBuf,
}

impl Harness {
    fn new(tag: &str) -> Self {
        let temp = temp_dir(tag);
        Self {
            queue: temp.join("queue.txt"),
            state: temp.join("dispatch-liveness.json"),
            lifecycle: temp.join("dispatch-lifecycle.json"),
            temp,
        }
    }

    fn dispatch(&self, args: &[&str]) -> Output {
        let mut argv: Vec<String> = vec!["dispatch".to_string()];
        argv.extend(args.iter().map(|arg| arg.to_string()));
        argv.extend(
            [
                "--queue",
                &self.queue.display().to_string(),
                "--state-file",
                &self.state.display().to_string(),
                "--lifecycle",
                &self.lifecycle.display().to_string(),
            ]
            .iter()
            .map(|arg| arg.to_string()),
        );
        run(&argv)
    }

    fn write_queue(&self, text: &str) {
        std::fs::write(&self.queue, text).expect("queue written");
    }

    /// A stamped queue: the stamp is what keeps the tick from treating the
    /// file as frozen.
    fn write_stamped(&self, age_secs: u64, entries: &[&str]) {
        let mut text = format!(
            "# refreshed-at: {}\n# refreshed-by: refresh-queue\n",
            NOW - age_secs
        );
        for entry in entries {
            text.push_str(entry);
            text.push('\n');
        }
        self.write_queue(&text);
    }

    /// Beat every scheduled hop of the reference topology, so the tick's
    /// liveness check passes and the test reaches the queue itself.
    fn beat_all(&self) {
        for hop in ["refresh-queue", "topup", "dispatch-agent"] {
            let output = self.dispatch(&["beat", "--step", hop, "--at", &NOW.to_string()]);
            assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        }
    }
}

fn run(argv: &[String]) -> Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_autospec"))
        .args(argv)
        .output()
        .expect("autospec runs")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

fn combined(output: &Output) -> String {
    format!("{}{}", stdout(output), stderr(output))
}

#[test]
fn a_corrupted_queue_line_is_refused_named_and_the_tick_stalls() {
    let harness = Harness::new("tick-refusal-corrupt");
    // A valid entry, a blank line (not work, not a refusal), and a line
    // that names work but is not an issue number.
    harness.write_stamped(0, &["42", "", "12x"]);
    harness.beat_all();

    let output = harness.dispatch(&["tick", "--now", &NOW.to_string()]);
    let text = combined(&output);

    // The refused line is named with its line number and its content.
    assert!(
        text.contains("refused queue line 5: \"12x\"")
            || text.contains("refused queue line 5: 12x"),
        "the refusal must name the line and the content:\n{text}"
    );
    // The valid entry dispatched, so the tick's exit is the dispatch
    // verdict — but the refusal is still reported in the output.
    assert!(
        text.contains("#42"),
        "the valid entry still dispatches:\n{text}"
    );
}

#[test]
fn a_queue_whose_lines_are_all_refused_is_a_stall_not_an_idle() {
    let harness = Harness::new("tick-refusal-all");
    // The incident shape: a queue whose only "entry" is a value that never
    // completed writing. Nothing may be dispatched from it, and the tick
    // must not report it as an empty, idle queue.
    harness.write_stamped(0, &["abc"]);
    harness.beat_all();

    let output = harness.dispatch(&["tick", "--now", &NOW.to_string()]);
    let text = combined(&output);

    // Non-zero: a wrapper branching on the exit code sees the stall.
    assert_ne!(
        output.status.code(),
        Some(0),
        "an all-refused queue is a stall, not an idle: {text}"
    );
    assert!(
        text.contains("refused"),
        "the stall names the refusal: {text}"
    );
    // And nothing was dispatched from it.
    assert!(!text.contains("dispatched #"), "nothing dispatches: {text}");
}

#[test]
fn a_queue_of_only_blank_lines_is_idle_not_a_stall() {
    let harness = Harness::new("tick-refusal-blank");
    harness.write_stamped(0, &["", ""]);
    harness.beat_all();

    let output = harness.dispatch(&["tick", "--now", &NOW.to_string()]);
    let text = combined(&output);

    // Blank lines name no work: the queue is empty, the tick is quiet, and
    // the exit code is the idle one.
    assert_eq!(
        output.status.code(),
        Some(0),
        "blank lines are not work: {text}"
    );
    assert!(
        text.contains("queue empty"),
        "an all-blank queue reports as empty: {text}"
    );
}
