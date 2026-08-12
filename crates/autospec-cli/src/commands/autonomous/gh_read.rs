//! Retrying idempotent GitHub reads.
//!
//! A read that fails once — a TLS handshake that comes back with an unusable
//! certificate under concurrency, a rate limit, a 502 — must cost a retry, not the
//! conductor process. When one of these was fatal it killed the conductor mid-claim,
//! and the dead worker's claim then stranded the issue: `claim release` validates the
//! caller's identity, so nothing could ever release it.

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
    let attempts = env_u64("AUTOSPEC_GH_API_RETRIES", 3);
    let sleep_ms = env_u64("AUTOSPEC_CLAIM_RETRY_SLEEP_MS", 1_000);
    let mut last_error = String::new();
    for attempt in 0..attempts {
        let output = Command::new("gh")
            .args(arguments)
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

#[cfg(test)]
mod tests {
    use super::env_u64;

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
}

#[cfg(test)]
mod guard {
    use std::path::Path;

    const CONDUCTOR_PATH_FILES: [&str; 3] = ["queue.rs", "autonomous.rs", "claim.rs"];

    fn unretried_reads(file: &str, source: &str) -> Vec<String> {
        source
            .lines()
            .enumerate()
            .filter(|(_, line)| line.contains("run_gh(") && line.contains("\"GET\""))
            .map(|(index, _)| format!("{file}:{}", index + 1))
            .collect()
    }

    /// Every idempotent GitHub read in the conductor path must retry.
    ///
    /// Each un-retried read is a single point of failure: one handshake that comes
    /// back unusable under concurrency kills the conductor, and the claim it was
    /// holding strands its issue. Two were fixed reactively, each only after it was
    /// caught killing a live run. This fails the build instead.
    #[test]
    fn no_unretried_read_survives_in_the_conductor_path() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands");
        let offenders = CONDUCTOR_PATH_FILES
            .iter()
            .flat_map(|file| {
                let source = std::fs::read_to_string(root.join(file)).expect("read source");
                unretried_reads(file, &source)
            })
            .collect::<Vec<_>>();

        assert!(
            offenders.is_empty(),
            "these reads bypass run_gh_read_with_retry and can kill the conductor: {}",
            offenders.join(", ")
        );
    }

    #[test]
    fn the_guard_recognises_an_unretried_read() {
        let source = "let output = run_gh(&[\"api\", \"--method\", \"GET\", &endpoint])?;";

        assert_eq!(unretried_reads("sample.rs", source), ["sample.rs:1"]);
    }
}
