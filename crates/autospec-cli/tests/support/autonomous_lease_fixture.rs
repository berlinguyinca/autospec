use std::fs;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

/// Budget for a detached conductor to adopt its fresh lease before the hold
/// fixture panics. A bare local box is fast; a loaded CI runner (full suite in
/// parallel) needs far more headroom, so the budget widens under CI.
fn lease_adopt_budget() -> Duration {
    if std::env::var_os("GITHUB_ACTIONS").is_some() || std::env::var_os("CI").is_some() {
        Duration::from_secs(30)
    } else {
        Duration::from_secs(15)
    }
}

pub fn wait_until_owned(ready: &Path, state_dir: &Path, operator_dir: &Path, repo: &str) {
    let lease = state_dir
        .join("autonomous")
        .join(repo.replace('/', "__"))
        .join("state.json");
    let scope = repo
        .chars()
        .map(|character| match character {
            character if character.is_ascii_alphanumeric() => character,
            '.' | '_' | '-' => character,
            _ => '_',
        })
        .collect::<String>();
    let conductor = operator_dir.join(scope).join("conductor.pid");
    let budget = lease_adopt_budget();
    let deadline = Instant::now() + budget;
    while Instant::now() < deadline {
        let pid = json_u64(&conductor, "pid");
        let owner = json_u64(&lease, "lock_pid");
        let running = json_string(&lease, "status").as_deref() == Some("running");
        if ready.exists() && pid.is_some() && pid == owner && running {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!(
        "detached conductor did not adopt its fresh lease within {}s: ready={} conductor={} lease={}",
        budget.as_secs(),
        ready.exists(),
        fs::read_to_string(&conductor).unwrap_or_else(|error| format!("<{error}>")),
        fs::read_to_string(&lease).unwrap_or_else(|error| format!("<{error}>")),
    );
}

fn json(path: &Path) -> Option<serde_json::Value> {
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

fn json_u64(path: &Path, key: &str) -> Option<u64> {
    json(path)?.get(key)?.as_u64()
}

fn json_string(path: &Path, key: &str) -> Option<String> {
    json(path)?.get(key)?.as_str().map(str::to_string)
}
