//! Retrying idempotent GitHub reads.
//!
//! A read that fails once — a TLS handshake that comes back with an unusable
//! certificate under concurrency, a rate limit, a 502 — must cost a retry, not the
//! conductor process. When one of these was fatal it killed the conductor mid-claim,
//! and the dead worker's claim then stranded the issue: `claim release` validates the
//! caller's identity, so nothing could ever release it.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, Output};

use super::CommandFailure;

fn env_u64(name: &str, fallback: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(fallback)
}

/// Run an idempotent `gh` read, retrying a failed attempt before giving up.
///
/// Shares `AUTOSPEC_GH_API_RETRIES` and `AUTOSPEC_CLAIM_RETRY_SLEEP_MS` with the
/// claim path's `run_gh_with_retry`, and returns the captured output that helper
/// discards. Only use this for reads: a retried mutation would not be safe.
pub(crate) fn run_gh_read_with_retry(
    arguments: &[&str],
    action: &str,
) -> Result<Output, CommandFailure> {
    run_command_read_with_retry(
        || {
            let mut command = Command::new("gh");
            command.args(arguments);
            command
        },
        action,
    )
}

pub(crate) fn run_command_read_with_retry(
    mut command: impl FnMut() -> Command,
    action: &str,
) -> Result<Output, CommandFailure> {
    let attempts = env_u64("AUTOSPEC_GH_API_RETRIES", 3);
    let sleep_ms = env_u64("AUTOSPEC_CLAIM_RETRY_SLEEP_MS", 1_000);
    let mut last_error = String::new();
    for attempt in 0..attempts {
        let output = command()
            .output()
            .map_err(|error| CommandFailure::diagnostic(format!("cannot run {action}: {error}")))?;
        if output.status.success() {
            return Ok(output);
        }
        last_error = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if attempt + 1 < attempts {
            std::thread::sleep(std::time::Duration::from_millis(sleep_ms));
        }
    }
    Err(CommandFailure::diagnostic(format!(
        "{action} failed after {attempts} attempts: {last_error}"
    )))
}

pub(crate) fn run_gh_read_with_retry_in(
    program: &Path,
    arguments: &[&str],
    environment: &BTreeMap<OsString, OsString>,
    action: &str,
) -> Result<Output, CommandFailure> {
    run_command_read_with_retry(
        || {
            let mut command = Command::new(program);
            command.args(arguments).envs(environment);
            command
        },
        action,
    )
}

#[cfg(test)]
mod tests {
    use super::{env_u64, run_command_read_with_retry};
    use std::process::Command;

    #[test]
    fn an_unset_variable_falls_back() {
        assert_eq!(env_u64("AUTOSPEC_TEST_UNSET_RETRY_KNOB", 3), 3);
    }

    #[test]
    fn a_zero_or_unparseable_value_falls_back() {
        // SAFETY: single-threaded test process mutating its own environment.
        unsafe {
            std::env::set_var("AUTOSPEC_TEST_ZERO_RETRY_KNOB", "0");
            std::env::set_var("AUTOSPEC_TEST_JUNK_RETRY_KNOB", "many");
        }
        assert_eq!(env_u64("AUTOSPEC_TEST_ZERO_RETRY_KNOB", 3), 3);
        assert_eq!(env_u64("AUTOSPEC_TEST_JUNK_RETRY_KNOB", 3), 3);
        unsafe {
            std::env::remove_var("AUTOSPEC_TEST_ZERO_RETRY_KNOB");
            std::env::remove_var("AUTOSPEC_TEST_JUNK_RETRY_KNOB");
        }
    }

    #[test]
    fn retries_a_transient_read_until_it_succeeds() {
        let root =
            std::env::temp_dir().join(format!("autospec-gh-read-retry-{}", std::process::id()));
        std::fs::create_dir_all(&root).expect("create temp directory");
        let counter = root.join("counter");
        let script = format!(
            "n=$(cat '{}' 2>/dev/null || echo 0); n=$((n+1)); echo $n > '{}'; [ $n -ge 2 ]",
            counter.display(),
            counter.display()
        );
        unsafe {
            std::env::set_var("AUTOSPEC_GH_API_RETRIES", "2");
            std::env::set_var("AUTOSPEC_CLAIM_RETRY_SLEEP_MS", "1");
        }
        let output = run_command_read_with_retry(
            || {
                let mut command = Command::new("sh");
                command.args(["-c", &script]);
                command
            },
            "test read",
        )
        .expect("second attempt succeeds");
        assert!(output.status.success());
        assert_eq!(std::fs::read_to_string(counter).expect("counter"), "2\n");
        let _ = std::fs::remove_dir_all(root);
        unsafe {
            std::env::remove_var("AUTOSPEC_GH_API_RETRIES");
            std::env::remove_var("AUTOSPEC_CLAIM_RETRY_SLEEP_MS");
        }
    }
}

#[cfg(test)]
mod guard {
    use std::path::Path;

    /// Markers that identify an idempotent GitHub read.
    ///
    /// `"GET"` alone missed `gh issue view`, which goes over GraphQL POST and
    /// carries no method argument at all. That read stayed unretried, and a TLS
    /// handshake failure on it crash-looped a live conductor eighteen times.
    const READ_MARKERS: [&str; 5] = [
        "\"GET\"",
        "\"view\"",
        "\"list\"",
        "\"checks\"",
        "\"graphql\"",
    ];

    fn call_is_a_read(source: &str, start: usize, command_builder: bool) -> bool {
        let terminator = if command_builder { ".output()" } else { "])" };
        let end = source[start..]
            .find(terminator)
            .map_or(source.len(), |offset| start + offset + terminator.len());
        let call = &source[start..end];
        READ_MARKERS.iter().any(|marker| call.contains(marker))
    }

    fn inside_retry_builder(source: &str, start: usize) -> bool {
        let prefix = &source[..start];
        prefix
            .rfind("run_command_read_with_retry(")
            .is_some_and(|retry| prefix.rfind("},").is_none_or(|end| retry > end))
    }

    fn unretried_pattern(file: &str, source: &str, needle: &str) -> Vec<String> {
        let mut offenders = Vec::new();
        for (start, _) in source.match_indices(needle) {
            if !inside_retry_builder(source, start)
                && call_is_a_read(source, start, needle.starts_with("Command::"))
            {
                offenders.push(format!("{file}:{needle}"));
            }
        }
        offenders
    }

    fn unretried_reads(file: &str, source: &str) -> Vec<String> {
        let compact = source.split_whitespace().collect::<Vec<_>>().join(" ");
        let mut offenders = Vec::new();
        for needle in [
            "run_gh(",
            "Command::new(\"gh\")",
            "Command::new(&adapter.gh)",
        ] {
            offenders.extend(unretried_pattern(file, &compact, needle));
        }
        offenders
    }

    fn rust_files(root: &Path) -> Vec<std::path::PathBuf> {
        let mut files = Vec::new();
        let mut pending = vec![root.to_path_buf()];
        while let Some(path) = pending.pop() {
            if path.extension().and_then(|value| value.to_str()) == Some("rs") {
                files.push(path);
                continue;
            }
            if !path.is_dir() {
                continue;
            }
            pending.extend(directory_entries(&path));
        }
        files
    }

    fn directory_entries(path: &Path) -> Vec<std::path::PathBuf> {
        std::fs::read_dir(path)
            .expect("read source directory")
            .map(|entry| entry.expect("read source entry").path())
            .collect()
    }

    fn is_conductor_source(path: &Path) -> bool {
        path.ends_with("queue.rs")
            || path.ends_with("claim.rs")
            || path
                .components()
                .any(|part| part.as_os_str() == "autonomous")
    }

    fn scan_source(path: &Path) -> Vec<String> {
        let source = std::fs::read_to_string(path).expect("read source");
        unretried_reads(&path.display().to_string(), &source)
    }

    /// Every idempotent GitHub read in the conductor path must retry.
    ///
    /// Each un-retried read is a single point of failure: one handshake that comes
    /// back unusable under concurrency kills the conductor, and the claim it was
    /// holding strands its issue. Two were fixed reactively, each only after it was
    /// caught killing a live run. This fails the build instead.
    /// Files this guard does not cover yet, and why.
    ///
    /// `executor_bridge.rs` raises `BridgeRunFailure` rather than `CommandFailure`,
    /// so its seven reads need an error-mapping wrapper before they can adopt
    /// `run_gh_read_with_retry`. Tracked separately; named here so the gap is
    /// visible in the source rather than silently absent from the guard.
    const PENDING_RETRY_ADOPTION: [&str; 1] = ["executor_bridge.rs"];

    fn is_pending_adoption(path: &Path) -> bool {
        PENDING_RETRY_ADOPTION
            .iter()
            .any(|pending| path.ends_with(pending))
    }

    #[test]
    fn no_unretried_read_survives_in_the_conductor_path() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands");
        let offenders = rust_files(&root)
            .iter()
            .filter(|path| is_conductor_source(path))
            .filter(|path| !path.ends_with("gh_read.rs"))
            .filter(|path| !is_pending_adoption(path))
            .flat_map(|path| scan_source(path))
            .collect::<Vec<_>>();

        assert!(
            offenders.is_empty(),
            "these reads bypass run_gh_read_with_retry and can kill the conductor: {}",
            offenders.join(", ")
        );
    }

    /// `gh issue view` goes over GraphQL POST and carries no method argument, so a
    /// guard keyed on `"GET"` never saw it. That read stayed unretried and a TLS
    /// failure on it crash-looped a live conductor eighteen times.
    #[test]
    fn the_guard_recognises_a_read_that_carries_no_method() {
        let source = "let output = Command::new(\"gh\")\n.args([\"issue\", \"view\", \"51\", \"--repo\", repo]).output();";

        assert_eq!(
            unretried_reads("sample.rs", source),
            ["sample.rs:Command::new(\"gh\")"]
        );
    }

    #[test]
    fn the_guard_recognises_a_graphql_read() {
        let source = "let output = run_gh(&[\"api\", \"graphql\", \"-f\", query]);";

        assert_eq!(unretried_reads("sample.rs", source), ["sample.rs:run_gh("]);
    }

    #[test]
    fn the_guard_recognises_an_unretried_read() {
        let source = "let mut command = Command::new(&adapter.gh);\ncommand.args([\"api\", \"--method\", \"GET\"]);";

        assert_eq!(
            unretried_reads("sample.rs", source),
            ["sample.rs:Command::new(&adapter.gh)"]
        );
    }
}
