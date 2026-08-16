use std::fs;
use std::path::Path;
use std::thread;
use std::time::Duration;

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
    for _ in 0..200 {
        let pid = json_u64(&conductor, "pid");
        let owner = json_u64(&lease, "lock_pid");
        let running = json_string(&lease, "status").as_deref() == Some("running");
        if ready.exists() && pid.is_some() && pid == owner && running {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("detached conductor did not adopt its fresh lease before the hold fixture");
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
