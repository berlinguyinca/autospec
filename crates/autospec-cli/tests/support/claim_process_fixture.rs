pub(super) fn current_process_start() -> String {
    // Linux heartbeat ownership is keyed by PID plus /proc start time. Test
    // fixtures must use the same identity or they represent a dead process.
    let stat = std::fs::read_to_string("/proc/self/stat").expect("process identity");
    stat.rsplit_once(") ")
        .expect("process stat fields")
        .1
        .split_whitespace()
        .nth(19)
        .expect("process start")
        .to_string()
}

pub(super) fn bind_to_current_process(document: String) -> String {
    document
        .replace(
            "\"pid\":2147483647,",
            &format!("\"pid\":{},", std::process::id()),
        )
        .replace(
            "\"process_start\":\"1\"",
            &format!("\"process_start\":\"{}\"", current_process_start()),
        )
}
