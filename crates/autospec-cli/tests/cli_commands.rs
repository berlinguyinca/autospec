use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

fn autospec() -> Command {
    Command::new(env!("CARGO_BIN_EXE_autospec"))
}

fn help_command_names(help: &str) -> Vec<&str> {
    help.lines()
        .skip_while(|line| line.trim() != "COMMANDS:")
        .skip(1)
        .take_while(|line| !line.trim().is_empty() && line.trim() != "OPTIONS:")
        .filter_map(|line| line.split_whitespace().next())
        .collect()
}

#[test]
fn help_command_table_parser_returns_command_column_only() {
    let help = "COMMANDS:\n    init           Initialize AutoSpec metadata\n    growth-report  Render metrics\n\nOPTIONS:\n    -h, --help       Print help\n";

    assert_eq!(help_command_names(help), ["init", "growth-report"]);
}

#[test]
fn cli_commands_help_lists_required_commands() {
    let output = autospec().arg("--help").output().expect("autospec runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert_eq!(
        help_command_names(&stdout),
        [
            "init",
            "doctor",
            "status",
            "autonomous",
            "plan",
            "validate",
            "run",
            "runtime",
            "resume",
            "report",
            "showcase",
            "benchmark",
            "growth-report",
        ]
    );
}

#[test]
fn autonomous_start_dry_run_includes_monitor_and_supervisor_companions() {
    let output = autospec()
        .args([
            "autonomous",
            "start",
            "--repo",
            "berlinguyinca/autospec",
            "--repo-dir",
            "/tmp/autospec",
            "--dry-run",
            "--json",
        ])
        .output()
        .expect("autospec autonomous start runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("\"command\":\"autonomous\""));
    assert!(stdout.contains("\"subcommand\":\"start\""));
    assert!(stdout.contains("\"repo\":\"berlinguyinca/autospec\""));
    assert!(stdout.contains("\"repo_dir\":\"/tmp/autospec\""));
    assert!(stdout.contains("\"conductor\""));
    assert!(stdout.contains("autospec autonomous run-foreground"));
    assert!(stdout.contains("\"companions\""));
    assert!(stdout.contains("\"monitor\""));
    assert!(stdout.contains("\"supervisor\""));
    assert!(stdout.contains("autospec autonomous monitor"));
    assert!(stdout.contains("autospec autonomous supervise"));
}

#[test]
fn autonomous_bare_command_defaults_to_start() {
    let output = autospec()
        .args([
            "autonomous",
            "--repo",
            "berlinguyinca/autospec",
            "--repo-dir",
            "/tmp/autospec",
            "--dry-run",
            "--json",
        ])
        .output()
        .expect("autospec autonomous runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("\"subcommand\":\"start\""));
    assert!(stdout.contains("\"status\":\"dry-run\""));
}

#[test]
fn autonomous_supervise_once_json_reports_observer_status() {
    let output = autospec()
        .args([
            "autonomous",
            "supervise",
            "--repo",
            "berlinguyinca/autospec",
            "--pid",
            "999999",
            "--once",
            "--json",
        ])
        .output()
        .expect("autospec autonomous supervise runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("\"command\":\"autonomous\""));
    assert!(stdout.contains("\"subcommand\":\"supervise\""));
    assert!(stdout.contains("\"repo\":\"berlinguyinca/autospec\""));
    assert!(stdout.contains("\"conductor\":\"stopped\""));
    assert!(stdout.contains("\"pid\":\"999999\""));
    assert!(stdout.contains("\"action\":\"conductor-not-running\""));
}

#[test]
fn autonomous_start_live_writes_repo_scoped_pid_and_log_metadata() {
    let temp = temp_dir("autospec-autonomous-live");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let repo_dir = temp.join("repo");
    make_git_repo(&repo_dir, None);

    let output = autospec()
        .args([
            "autonomous",
            "start",
            "--repo",
            "berlinguyinca/autospec",
            "--repo-dir",
            repo_dir.to_str().unwrap(),
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .env("AUTOSPEC_AUTONOMOUS_CONDUCTOR_CMD", "sleep 20")
        .env("AUTOSPEC_AUTONOMOUS_MONITOR_CMD", "sleep 20")
        .env("AUTOSPEC_AUTONOMOUS_SUPERVISOR_CMD", "sleep 20")
        .output()
        .expect("autospec autonomous start runs");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let scope = operator_dir.join("berlinguyinca_autospec");

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("\"status\":\"started\""));
    assert!(stdout.contains("\"conductor\""));
    assert!(stdout.contains("\"monitor\""));
    assert!(stdout.contains("\"supervisor\""));
    for name in ["conductor", "monitor", "supervisor"] {
        let pid_file = scope.join(format!("{name}.pid"));
        let logpath_file = scope.join(format!("{name}.logpath"));
        assert!(pid_file.exists(), "missing {}", pid_file.display());
        assert!(logpath_file.exists(), "missing {}", logpath_file.display());
        assert!(!std::fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .is_empty());
    }

    cleanup_pids(&scope);
}

#[test]
fn autonomous_status_json_reports_companion_processes() {
    let temp = temp_dir("autospec-autonomous-status");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let repo_dir = temp.join("repo");
    make_git_repo(&repo_dir, None);

    let start = autospec()
        .args([
            "autonomous",
            "start",
            "--repo",
            "berlinguyinca/autospec",
            "--repo-dir",
            repo_dir.to_str().unwrap(),
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .env("AUTOSPEC_AUTONOMOUS_CONDUCTOR_CMD", "sleep 20")
        .env("AUTOSPEC_AUTONOMOUS_MONITOR_CMD", "sleep 20")
        .env("AUTOSPEC_AUTONOMOUS_SUPERVISOR_CMD", "sleep 20")
        .output()
        .expect("autospec autonomous start runs");
    assert!(start.status.success());

    let status = autospec()
        .args([
            "autonomous",
            "status",
            "--repo",
            "berlinguyinca/autospec",
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .output()
        .expect("autospec autonomous status runs");
    let stdout = String::from_utf8_lossy(&status.stdout);

    assert!(status.status.success());
    assert!(stdout.contains("\"conductor\":{\"running\":true"));
    assert!(stdout.contains("\"monitor\":{\"running\":true"));
    assert!(stdout.contains("\"supervisor\":{\"running\":true"));
    assert!(stdout.contains("\"pid_file\""));
    assert!(stdout.contains("\"logpath_file\""));

    cleanup_pids(&operator_dir.join("berlinguyinca_autospec"));
}

#[test]
fn autonomous_status_json_reports_state_heartbeat_and_spend_metadata() {
    let temp = temp_dir("autospec-autonomous-rich-status");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let home = temp.join("home");
    let scope = operator_dir.join("berlinguyinca_autospec");
    let state_scope = home
        .join(".autospec")
        .join("autonomous")
        .join("berlinguyinca_autospec");
    std::fs::create_dir_all(&scope).expect("scope dir");
    std::fs::create_dir_all(&state_scope).expect("state scope");
    std::fs::create_dir_all(home.join(".autospec")).expect("home autospec");
    std::fs::write(scope.join("conductor.pid"), "999999\n").expect("pid");
    std::fs::write(
        state_scope.join("state.json"),
        "{\"status\":\"parked:usage-limit\",\"heartbeat_at\":1783526400,\"cycle\":42}\n",
    )
    .expect("state");
    std::fs::write(
        home.join(".autospec").join("autonomous-spend.json"),
        "{\"tokens\":1234,\"issues\":5}\n",
    )
    .expect("spend");

    let output = autospec()
        .args([
            "autonomous",
            "status",
            "--repo",
            "berlinguyinca/autospec",
            "--json",
        ])
        .env("HOME", &home)
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .output()
        .expect("autospec autonomous status runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("\"state_status\":\"parked:usage-limit\""));
    assert!(stdout.contains("\"heartbeat_at\":1783526400"));
    assert!(stdout.contains("\"last_cycle\":\"42\""));
    assert!(stdout.contains("\"spend\""));
    assert!(stdout.contains("\"tokens\":1234"));
}

#[test]
fn autonomous_status_all_json_aliases_list_with_conductors_key() {
    let temp = temp_dir("autospec-autonomous-status-all");
    let operator_dir = temp.join("operator");
    let scope = operator_dir.join("berlinguyinca_autospec");
    std::fs::create_dir_all(&scope).expect("scope dir");
    std::fs::write(scope.join("conductor.pid"), "999999\n").expect("pid");

    let output = autospec()
        .args(["autonomous", "status", "--all", "--json"])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .output()
        .expect("autospec autonomous status all runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("\"conductors\":["));
    assert!(stdout.contains("\"slug\":\"berlinguyinca_autospec\""));
}

#[test]
fn autonomous_stop_kills_only_the_target_repo_scope() {
    let temp = temp_dir("autospec-autonomous-stop");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let stop_flag = temp.join("stop.flag");
    let target_repo_dir = temp.join("target-repo");
    let other_repo_dir = temp.join("other-repo");
    std::fs::create_dir_all(&target_repo_dir).expect("target repo dir");
    std::fs::create_dir_all(&other_repo_dir).expect("other repo dir");

    start_sleeping_autonomous(
        &operator_dir,
        &log_dir,
        &target_repo_dir,
        "berlinguyinca/autospec",
    );
    start_sleeping_autonomous(
        &operator_dir,
        &log_dir,
        &other_repo_dir,
        "metabolomics-us/go-modules",
    );

    let output = autospec()
        .args([
            "autonomous",
            "stop",
            "--immediate",
            "--repo",
            "berlinguyinca/autospec",
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .env("AUTOSPEC_STOP_FLAG_FILE", &stop_flag)
        .output()
        .expect("autospec autonomous stop runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("\"subcommand\":\"stop\""));
    assert!(stdout.contains("\"repo\":\"berlinguyinca/autospec\""));
    assert!(stdout.contains("\"stopped\":3"));

    let target_status = autonomous_status(&operator_dir, &log_dir, "berlinguyinca/autospec");
    assert!(target_status.contains("\"conductor\":{\"running\":false"));
    let other_status = autonomous_status(&operator_dir, &log_dir, "metabolomics-us/go-modules");
    assert!(other_status.contains("\"conductor\":{\"running\":true"));
    assert!(other_status.contains("\"monitor\":{\"running\":true"));
    assert!(other_status.contains("\"supervisor\":{\"running\":true"));

    cleanup_pids(&operator_dir.join("metabolomics-us_go-modules"));
}

#[test]
fn autonomous_stop_graceful_writes_sentinel_and_leaves_conductor_running() {
    let temp = temp_dir("autospec-autonomous-graceful-stop");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let stop_flag = temp.join("stop.flag");
    let repo_dir = temp.join("repo");
    std::fs::create_dir_all(&repo_dir).expect("repo dir");

    start_sleeping_autonomous(&operator_dir, &log_dir, &repo_dir, "berlinguyinca/autospec");
    let scope = operator_dir.join("berlinguyinca_autospec");
    let conductor = read_pid(&scope, "conductor");

    let output = autospec()
        .args([
            "autonomous",
            "stop",
            "--repo",
            "berlinguyinca/autospec",
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .env("AUTOSPEC_STOP_FLAG_FILE", &stop_flag)
        .output()
        .expect("autospec autonomous stop runs");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let flag = std::fs::read_to_string(&stop_flag).expect("stop flag written");

    assert!(output.status.success());
    assert!(stdout.contains("\"mode\":\"graceful\""));
    assert!(stdout.contains("\"stop_flag\""));
    assert!(flag.starts_with("graceful\n"));
    assert!(process_is_alive(&conductor));

    let target_status = autonomous_status(&operator_dir, &log_dir, "berlinguyinca/autospec");
    assert!(target_status.contains("\"conductor\":{\"running\":true"));
    assert!(target_status.contains("\"monitor\":{\"running\":false"));
    assert!(target_status.contains("\"supervisor\":{\"running\":false"));

    cleanup_pids(&scope);
}

#[test]
fn autonomous_list_json_reports_each_repo_scope_with_companions() {
    let temp = temp_dir("autospec-autonomous-list");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let repo_a = temp.join("repo-a");
    let repo_b = temp.join("repo-b");
    std::fs::create_dir_all(&repo_a).expect("repo a");
    std::fs::create_dir_all(&repo_b).expect("repo b");

    start_sleeping_autonomous(&operator_dir, &log_dir, &repo_a, "berlinguyinca/autospec");
    start_sleeping_autonomous(
        &operator_dir,
        &log_dir,
        &repo_b,
        "metabolomics-us/go-modules",
    );

    let output = autospec()
        .args(["autonomous", "list", "--json"])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .output()
        .expect("autospec autonomous list runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("\"subcommand\":\"list\""));
    assert!(stdout.contains("\"scope\":\"berlinguyinca_autospec\""));
    assert!(stdout.contains("\"scope\":\"metabolomics-us_go-modules\""));
    assert!(stdout.contains("\"conductor\":{\"running\":true"));
    assert!(stdout.contains("\"monitor\":{\"running\":true"));
    assert!(stdout.contains("\"supervisor\":{\"running\":true"));

    cleanup_pids(&operator_dir.join("berlinguyinca_autospec"));
    cleanup_pids(&operator_dir.join("metabolomics-us_go-modules"));
}

#[test]
fn autonomous_status_marks_stale_metadata_for_dead_pids() {
    let temp = temp_dir("autospec-autonomous-stale");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let scope = operator_dir.join("berlinguyinca_autospec");
    std::fs::create_dir_all(&scope).expect("scope dir");
    std::fs::write(scope.join("conductor.pid"), "999999\n").expect("pid");
    std::fs::write(
        scope.join("conductor.logpath"),
        "/tmp/missing-autospec.log\n",
    )
    .expect("logpath");

    let output = autospec()
        .args([
            "autonomous",
            "status",
            "--repo",
            "berlinguyinca/autospec",
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .output()
        .expect("autospec autonomous status runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("\"conductor\":{\"running\":false"));
    assert!(stdout.contains("\"stale_pid\":true"));
    assert!(stdout.contains("\"metadata_only\":true"));
}

#[test]
fn autonomous_logs_json_reads_recorded_conductor_log_tail() {
    let temp = temp_dir("autospec-autonomous-logs");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let scope = operator_dir.join("berlinguyinca_autospec");
    let log = temp.join("conductor.log");
    std::fs::create_dir_all(&scope).expect("scope dir");
    std::fs::write(&log, "first-line\nimplemented feature x\ndone\n").expect("log");
    std::fs::write(
        scope.join("conductor.logpath"),
        format!("{}\n", log.display()),
    )
    .expect("logpath");

    let output = autospec()
        .args([
            "autonomous",
            "logs",
            "--repo",
            "berlinguyinca/autospec",
            "--lines",
            "2",
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .output()
        .expect("autospec autonomous logs runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("\"subcommand\":\"logs\""));
    assert!(stdout.contains("implemented feature x"));
    assert!(stdout.contains("done"));
    assert!(!stdout.contains("first-line"));
}

#[test]
fn autonomous_watch_once_reads_recorded_conductor_log_tail() {
    let temp = temp_dir("autospec-autonomous-watch");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let scope = operator_dir.join("berlinguyinca_autospec");
    let log = temp.join("conductor.log");
    std::fs::create_dir_all(&scope).expect("scope dir");
    std::fs::write(&log, "alpha\nbeta\n").expect("log");
    std::fs::write(
        scope.join("conductor.logpath"),
        format!("{}\n", log.display()),
    )
    .expect("logpath");

    let output = autospec()
        .args([
            "autonomous",
            "watch",
            "--repo",
            "berlinguyinca/autospec",
            "--once",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .output()
        .expect("autospec autonomous watch runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("alpha"));
    assert!(stdout.contains("beta"));
}

#[test]
fn autonomous_timeline_summarizes_recorded_conductor_log() {
    let temp = temp_dir("autospec-autonomous-timeline");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let home = temp.join("home");
    let scope = operator_dir.join("berlinguyinca_autospec");
    let log = temp.join("conductor.log");
    std::fs::create_dir_all(&scope).expect("scope dir");
    std::fs::write(
        &log,
        "2026-07-11T04:30:00Z implemented feature x\n2026-07-11T04:45:00Z started research on y\n",
    )
    .expect("log");
    std::fs::write(
        scope.join("conductor.logpath"),
        format!("{}\n", log.display()),
    )
    .expect("logpath");

    let output = autospec()
        .args([
            "autonomous",
            "timeline",
            "--repo",
            "berlinguyinca/autospec",
            "--lines",
            "5",
        ])
        .env("HOME", &home)
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .output()
        .expect("autospec autonomous timeline runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("implemented feature x"));
    assert!(stdout.contains("started research on y"));
}

#[test]
fn autonomous_timeline_reports_forecast_and_planned_steps() {
    let temp = temp_dir("autospec-autonomous-timeline-forecast");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let home = temp.join("home");
    let log = write_conductor_log(
        &operator_dir,
        "berlinguyinca_autospec",
        "{
  \"ready\": [
    {\"number\": 1538, \"title\": \"feat: autonomous UX/UI optimization tier\"},
    {\"number\": 1539, \"title\": \"feat: autonomous accessibility standards tier\"}
  ],
  \"blocked\": [
    {\"number\": 1540, \"title\": \"feat: documentation freshness tier\"}
  ],
  \"claimed\": [
    {\"number\": 1537, \"title\": \"feat: proactive security scanning workstream\"}
  ],
  \"batch\": [
    {\"number\": 1538, \"title\": \"feat: autonomous UX/UI optimization tier\"}
  ]
}
",
    );

    let output = autospec()
        .args([
            "autonomous",
            "timeline",
            "--repo",
            "berlinguyinca/autospec",
            "--lines",
            "80",
        ])
        .env("HOME", &home)
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .output()
        .expect("autospec autonomous timeline runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success(), "log={}", log.display());
    assert!(stdout.contains("autospec-autonomous forecast"));
    assert!(stdout.contains("things left: 4 total (2 ready, 1 in progress, 1 blocked)"));
    assert!(stdout.contains("rough ETA: about 3-6 hours"));
    assert!(
        stdout.contains("planned next: finish #1537 feat: proactive security scanning workstream")
    );
    assert!(stdout.contains("then start #1538 feat: autonomous UX/UI optimization tier"));
    assert!(stdout.contains("blocked later: #1540 feat: documentation freshness tier"));
}

#[test]
fn autonomous_timeline_reports_item_timing() {
    let temp = temp_dir("autospec-autonomous-timeline-timing");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let home = temp.join("home");
    write_conductor_log(
        &operator_dir,
        "berlinguyinca_autospec",
        "{\"issue\":\"1539\",\"branch\":\"feat/issue-1539-accessibility\",\"step\":\"claimed\",\"ts\":1783440000,\"repo\":\"berlinguyinca/autospec\"}
{\"issue\":\"1539\",\"branch\":\"feat/issue-1539-accessibility\",\"step\":\"tests_started\",\"ts\":1783441800,\"repo\":\"berlinguyinca/autospec\"}
{\"issue\":\"1538\",\"branch\":\"feat/issue-1538-ux\",\"step\":\"claimed\",\"ts\":1783430000,\"repo\":\"berlinguyinca/autospec\"}
{\"issue\":\"1538\",\"branch\":\"feat/issue-1538-ux\",\"step\":\"merged\",\"ts\":1783437200,\"repo\":\"berlinguyinca/autospec\"}
",
    );

    let output = autospec()
        .args([
            "autonomous",
            "timeline",
            "--repo",
            "berlinguyinca/autospec",
            "--lines",
            "80",
        ])
        .env("HOME", &home)
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .output()
        .expect("autospec autonomous timeline runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("item timing"));
    assert!(stdout.contains("#1539 current step tests started after 30 minutes"));
    assert!(stdout.contains("#1538 completed in 2 hours"));
}

#[test]
fn autonomous_timeline_reconciles_heartbeat_active_issue() {
    let temp = temp_dir("autospec-autonomous-timeline-heartbeat");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let home = temp.join("home");
    write_conductor_log(
        &operator_dir,
        "berlinguyinca_autospec",
        "{
  \"ready\": [
    {\"number\": 1543, \"title\": \"feat: autonomy guardrails\"},
    {\"number\": 1544, \"title\": \"feat: immutable verifier\"}
  ],
  \"blocked\": [],
  \"claimed\": [],
  \"batch\": [
    {\"number\": 1543, \"title\": \"feat: autonomy guardrails\"}
  ]
}
",
    );
    let heartbeat_dir = home
        .join(".autospec")
        .join("process-heartbeats")
        .join("berlinguyinca__autospec");
    std::fs::create_dir_all(&heartbeat_dir).expect("heartbeat dir");
    std::fs::write(
        heartbeat_dir.join("1543.json"),
        "{\"issue\":\"1543\",\"branch\":\"feat/issue-1543-autonomy-guardrails\",\"step\":\"claimed\",\"ts\":1783466193,\"repo\":\"berlinguyinca/autospec\"}\n",
    )
    .expect("heartbeat");

    let output = autospec()
        .args([
            "autonomous",
            "timeline",
            "--repo",
            "berlinguyinca/autospec",
            "--lines",
            "80",
        ])
        .env("HOME", &home)
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .output()
        .expect("autospec autonomous timeline runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("things left: 2 total (1 ready, 1 in progress, 0 blocked)"));
    assert!(stdout.contains("planned next: finish #1543 feat: autonomy guardrails"));
    assert!(stdout.contains("then start #1544 feat: immutable verifier"));
}

#[test]
fn autonomous_start_without_repo_uses_git_remote_scope() {
    let temp = temp_dir("autospec-autonomous-git-scope");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let repo_dir = temp.join("repo");
    make_git_repo(&repo_dir, Some("https://github.com/example/rust-scope.git"));

    let output = autospec()
        .args([
            "autonomous",
            "start",
            "--repo-dir",
            repo_dir.to_str().unwrap(),
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .env("AUTOSPEC_AUTONOMOUS_CONDUCTOR_CMD", "sleep 20")
        .env("AUTOSPEC_AUTONOMOUS_MONITOR_CMD", "sleep 20")
        .env("AUTOSPEC_AUTONOMOUS_SUPERVISOR_CMD", "sleep 20")
        .output()
        .expect("autospec autonomous start runs");

    assert!(output.status.success());
    assert!(operator_dir.join("example_rust-scope").exists());
    cleanup_pids(&operator_dir.join("example_rust-scope"));
}

#[test]
fn autonomous_start_non_git_repo_dir_fails_loud() {
    let temp = temp_dir("autospec-autonomous-nongit");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let repo_dir = temp.join("nongit");
    std::fs::create_dir_all(&repo_dir).expect("repo dir");

    let output = autospec()
        .args([
            "autonomous",
            "start",
            "--repo-dir",
            repo_dir.to_str().unwrap(),
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .env("AUTOSPEC_AUTONOMOUS_CONDUCTOR_CMD", "sleep 20")
        .output()
        .expect("autospec autonomous start runs");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success());
    assert!(stderr.contains("not a git checkout"));
    assert!(stderr.contains("--repo-dir"));
    assert!(!operator_dir.exists());
}

#[test]
fn autonomous_start_mismatched_repo_warns_but_launches() {
    let temp = temp_dir("autospec-autonomous-mismatch");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let repo_dir = temp.join("repo");
    make_git_repo(&repo_dir, Some("https://github.com/acme/widget.git"));

    let output = autospec()
        .args([
            "autonomous",
            "start",
            "--repo",
            "other-owner/other-repo",
            "--repo-dir",
            repo_dir.to_str().unwrap(),
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .env("AUTOSPEC_AUTONOMOUS_CONDUCTOR_CMD", "sleep 20")
        .env("AUTOSPEC_AUTONOMOUS_MONITOR_CMD", "sleep 20")
        .env("AUTOSPEC_AUTONOMOUS_SUPERVISOR_CMD", "sleep 20")
        .output()
        .expect("autospec autonomous start runs");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(output.status.success());
    assert!(stderr.contains("warning"));
    assert!(stderr.contains("other-owner/other-repo"));
    assert!(operator_dir.join("other-owner_other-repo").exists());
    cleanup_pids(&operator_dir.join("other-owner_other-repo"));
}

#[test]
fn autonomous_start_writes_launch_provenance_and_list_reports_it() {
    let temp = temp_dir("autospec-autonomous-launch");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let repo_dir = temp.join("repo");
    make_git_repo(&repo_dir, None);

    start_sleeping_autonomous(&operator_dir, &log_dir, &repo_dir, "berlinguyinca/autospec");
    let scope = operator_dir.join("berlinguyinca_autospec");
    let launch = scope.join("launch.json");
    assert!(launch.exists(), "missing {}", launch.display());
    assert!(std::fs::read_to_string(&launch)
        .expect("launch json")
        .contains("\"repo\":\"berlinguyinca/autospec\""));

    let output = autospec()
        .args(["autonomous", "list", "--json"])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .output()
        .expect("autospec autonomous list runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("\"launch\""));
    assert!(stdout.contains("\"repo\":\"berlinguyinca/autospec\""));
    cleanup_pids(&scope);
}

#[test]
fn autonomous_list_reports_empty_object_for_malformed_launch_json() {
    let temp = temp_dir("autospec-autonomous-launch-malformed");
    let operator_dir = temp.join("operator");
    let scope = operator_dir.join("berlinguyinca_autospec");
    std::fs::create_dir_all(&scope).expect("scope dir");
    std::fs::write(scope.join("launch.json"), "not-json").expect("launch json");

    let output = autospec()
        .args(["autonomous", "list", "--json"])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .output()
        .expect("autospec autonomous list runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("\"scope\":\"berlinguyinca_autospec\""));
    assert!(stdout.contains("\"launch\":{}"));
}

#[test]
fn autonomous_start_records_argv_and_passthrough_options_in_launch_provenance() {
    let temp = temp_dir("autospec-autonomous-launch-argv");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let repo_dir = temp.join("repo");
    make_git_repo(&repo_dir, None);

    let output = autospec()
        .args([
            "autonomous",
            "start",
            "--repo",
            "berlinguyinca/autospec",
            "--repo-dir",
            repo_dir.to_str().unwrap(),
            "--max-cycles",
            "3",
            "--budget-tokens",
            "1000",
            "--budget-issues",
            "4",
            "--no-digest",
            "--poll-interval-sec",
            "15",
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .env("AUTOSPEC_AUTONOMOUS_CONDUCTOR_CMD", "sleep 20")
        .env("AUTOSPEC_AUTONOMOUS_MONITOR_CMD", "sleep 20")
        .env("AUTOSPEC_AUTONOMOUS_SUPERVISOR_CMD", "sleep 20")
        .output()
        .expect("autospec autonomous start runs");
    let launch = std::fs::read_to_string(
        operator_dir
            .join("berlinguyinca_autospec")
            .join("launch.json"),
    )
    .expect("launch json");

    assert!(output.status.success());
    assert!(launch.contains("\"argv\":["));
    assert!(launch.contains("\"--max-cycles\""));
    assert!(launch.contains("\"max_cycles\":\"3\""));
    assert!(launch.contains("\"budget_tokens\":\"1000\""));
    assert!(launch.contains("\"budget_issues\":\"4\""));
    assert!(launch.contains("\"no_digest\":true"));
    assert!(launch.contains("\"poll_interval_sec\":\"15\""));
    cleanup_pids(&operator_dir.join("berlinguyinca_autospec"));
}

#[test]
fn autonomous_start_default_conductor_forwards_enforced_backend_options() {
    let output = autospec()
        .args([
            "autonomous",
            "start",
            "--repo",
            "berlinguyinca/autospec",
            "--repo-dir",
            "/tmp/autospec",
            "--max-cycles",
            "3",
            "--budget-tokens",
            "1000",
            "--budget-issues",
            "4",
            "--no-digest",
            "--poll-interval-sec",
            "15",
            "--dry-run",
            "--json",
        ])
        .output()
        .expect("autospec autonomous start runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("autospec autonomous run-foreground"));
    assert!(stdout.contains("--max-cycles 3"));
    assert!(stdout.contains("--budget-tokens 1000"));
    assert!(stdout.contains("--budget-issues 4"));
    assert!(stdout.contains("--no-digest"));
    assert!(stdout.contains("--poll-interval-sec 15"));
    assert!(stdout.contains("--dry-run"));
}

#[test]
fn autonomous_start_companion_opt_out_skips_monitor_and_supervisor_processes() {
    let temp = temp_dir("autospec-autonomous-no-companions");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let repo_dir = temp.join("repo");
    make_git_repo(&repo_dir, None);

    let output = autospec()
        .args([
            "autonomous",
            "start",
            "--repo",
            "berlinguyinca/autospec",
            "--repo-dir",
            repo_dir.to_str().unwrap(),
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .env("AUTOSPEC_AUTONOMOUS_COMPANIONS", "0")
        .env("AUTOSPEC_AUTONOMOUS_CONDUCTOR_CMD", "sleep 20")
        .output()
        .expect("autospec autonomous start runs");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let scope = operator_dir.join("berlinguyinca_autospec");

    assert!(output.status.success());
    assert!(stdout.contains("\"conductor\":{\"pid\":\""));
    assert!(stdout.contains("\"monitor\":{\"pid\":\"\""));
    assert!(stdout.contains("\"supervisor\":{\"pid\":\"\""));
    assert!(!scope.join("monitor.pid").exists());
    assert!(!scope.join("supervisor.pid").exists());
    cleanup_pids(&scope);
}

#[test]
fn autonomous_stop_defaults_to_repo_scoped_stop_flag() {
    let temp = temp_dir("autospec-autonomous-scoped-stop");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");

    let output = autospec()
        .args([
            "autonomous",
            "stop",
            "--repo",
            "berlinguyinca/autospec",
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .output()
        .expect("autospec autonomous stop runs");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let scoped_flag = operator_dir
        .join("berlinguyinca_autospec")
        .join("stop.flag");

    assert!(output.status.success());
    assert!(stdout.contains(&json_path(&scoped_flag)));
    assert_eq!(
        std::fs::read_to_string(&scoped_flag)
            .expect("scoped stop flag")
            .lines()
            .next(),
        Some("graceful")
    );
}

#[test]
fn autonomous_monitor_and_supervise_accept_shell_compatibility_options() {
    let monitor = autospec()
        .args([
            "autonomous",
            "monitor",
            "--iterations",
            "1",
            "--log",
            "/tmp/ignored.log",
        ])
        .output()
        .expect("autospec autonomous monitor runs");
    let supervisor = autospec()
        .args([
            "autonomous",
            "supervise",
            "--iterations",
            "1",
            "--force",
            "--log",
            "/tmp/ignored.log",
        ])
        .output()
        .expect("autospec autonomous supervise runs");

    assert!(monitor.status.success());
    assert!(supervisor.status.success());
}

#[test]
fn autonomous_start_rejects_unsupported_budget_hours() {
    let output = autospec()
        .args([
            "autonomous",
            "start",
            "--repo",
            "berlinguyinca/autospec",
            "--repo-dir",
            "/tmp/autospec",
            "--budget-hours",
            "2",
            "--dry-run",
            "--json",
        ])
        .output()
        .expect("autospec autonomous start runs");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success());
    assert!(stderr.contains("--budget-hours is not supported"));
}

#[test]
fn autonomous_start_uses_explicit_conductor_log_path() {
    let temp = temp_dir("autospec-autonomous-explicit-log");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let repo_dir = temp.join("repo");
    let explicit_log = temp.join("custom").join("conductor.log");
    make_git_repo(&repo_dir, None);

    let output = autospec()
        .args([
            "autonomous",
            "start",
            "--repo",
            "berlinguyinca/autospec",
            "--repo-dir",
            repo_dir.to_str().unwrap(),
            "--log",
            explicit_log.to_str().unwrap(),
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .env("AUTOSPEC_AUTONOMOUS_CONDUCTOR_CMD", "sleep 20")
        .env("AUTOSPEC_AUTONOMOUS_MONITOR_CMD", "sleep 20")
        .env("AUTOSPEC_AUTONOMOUS_SUPERVISOR_CMD", "sleep 20")
        .output()
        .expect("autospec autonomous start runs");
    let scope = operator_dir.join("berlinguyinca_autospec");

    assert!(output.status.success());
    assert_eq!(
        std::fs::read_to_string(scope.join("conductor.logpath"))
            .expect("logpath")
            .trim(),
        explicit_log.to_str().unwrap()
    );
    assert!(explicit_log.exists());
    cleanup_pids(&scope);
}

#[test]
fn autonomous_start_refuses_duplicate_conductor_without_force() {
    let temp = temp_dir("autospec-autonomous-duplicate");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let repo_dir = temp.join("repo");
    make_git_repo(&repo_dir, None);
    start_sleeping_autonomous(&operator_dir, &log_dir, &repo_dir, "berlinguyinca/autospec");
    let scope = operator_dir.join("berlinguyinca_autospec");
    let original = read_pid(&scope, "conductor");

    let output = autospec()
        .args([
            "autonomous",
            "start",
            "--repo",
            "berlinguyinca/autospec",
            "--repo-dir",
            repo_dir.to_str().unwrap(),
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .env("AUTOSPEC_AUTONOMOUS_CONDUCTOR_CMD", "sleep 20")
        .env("AUTOSPEC_AUTONOMOUS_MONITOR_CMD", "sleep 20")
        .env("AUTOSPEC_AUTONOMOUS_SUPERVISOR_CMD", "sleep 20")
        .output()
        .expect("autospec autonomous start runs");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success());
    assert!(stderr.contains("already running"));
    assert_eq!(read_pid(&scope, "conductor"), original);
    cleanup_pids(&scope);
}

#[test]
fn autonomous_start_force_replaces_existing_conductor() {
    let temp = temp_dir("autospec-autonomous-force");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let repo_dir = temp.join("repo");
    make_git_repo(&repo_dir, None);
    start_sleeping_autonomous(&operator_dir, &log_dir, &repo_dir, "berlinguyinca/autospec");
    let scope = operator_dir.join("berlinguyinca_autospec");
    let original = read_pid(&scope, "conductor");

    let output = autospec()
        .args([
            "autonomous",
            "start",
            "--repo",
            "berlinguyinca/autospec",
            "--repo-dir",
            repo_dir.to_str().unwrap(),
            "--force",
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .env("AUTOSPEC_AUTONOMOUS_CONDUCTOR_CMD", "sleep 20")
        .env("AUTOSPEC_AUTONOMOUS_MONITOR_CMD", "sleep 20")
        .env("AUTOSPEC_AUTONOMOUS_SUPERVISOR_CMD", "sleep 20")
        .output()
        .expect("autospec autonomous start runs");
    let replacement = read_pid(&scope, "conductor");

    assert!(output.status.success());
    assert_ne!(original, replacement);
    assert!(!process_is_alive(&original));
    assert!(process_is_alive(&replacement));
    cleanup_pids(&scope);
}

#[test]
fn autonomous_logs_falls_back_to_newest_legacy_flat_log() {
    let temp = temp_dir("autospec-autonomous-legacy-log");
    let operator_dir = temp.join("operator");
    let home = temp.join("home");
    let log_root = home.join(".autospec").join("logs");
    let scope = operator_dir.join("berlinguyinca_autospec");
    std::fs::create_dir_all(&scope).expect("scope dir");
    std::fs::create_dir_all(&log_root).expect("log root");
    std::fs::write(scope.join("conductor.pid"), "999999\n").expect("pid");
    std::fs::write(
        log_root.join("autospec-autonomous-20260708T100000Z.log"),
        "old legacy\n",
    )
    .expect("old log");
    std::fs::write(
        log_root.join("autospec-autonomous-20260708T110000Z.log"),
        "new legacy\n",
    )
    .expect("new log");

    let output = autospec()
        .args([
            "autonomous",
            "logs",
            "--repo",
            "berlinguyinca/autospec",
            "--lines",
            "1",
        ])
        .env("HOME", &home)
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .output()
        .expect("autospec autonomous logs runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert_eq!(stdout.trim(), "new legacy");
}

#[test]
fn autonomous_cleanup_removes_dead_metadata_without_killing_live_units() {
    let temp = temp_dir("autospec-autonomous-cleanup");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let dead_scope = operator_dir.join("berlinguyinca_autospec");
    let live_repo = temp.join("live-repo");
    std::fs::create_dir_all(&dead_scope).expect("dead scope");
    std::fs::create_dir_all(&live_repo).expect("live repo");
    std::fs::write(dead_scope.join("conductor.pid"), "999999\n").expect("dead pid");
    std::fs::write(
        dead_scope.join("conductor.logpath"),
        "/tmp/dead-autospec.log\n",
    )
    .expect("dead logpath");
    start_sleeping_autonomous(
        &operator_dir,
        &log_dir,
        &live_repo,
        "metabolomics-us/go-modules",
    );

    let output = autospec()
        .args([
            "autonomous",
            "cleanup",
            "--repo",
            "berlinguyinca/autospec",
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .output()
        .expect("autospec autonomous cleanup runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("\"removed\":2"));
    assert!(!dead_scope.join("conductor.pid").exists());
    let other_status = autonomous_status(&operator_dir, &log_dir, "metabolomics-us/go-modules");
    assert!(other_status.contains("\"conductor\":{\"running\":true"));
    cleanup_pids(&operator_dir.join("metabolomics-us_go-modules"));
}

#[test]
fn autonomous_supervise_reports_stale_metadata_action_for_dead_recorded_pid() {
    let temp = temp_dir("autospec-autonomous-supervise-stale");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let scope = operator_dir.join("berlinguyinca_autospec");
    std::fs::create_dir_all(&scope).expect("scope dir");
    std::fs::write(scope.join("conductor.pid"), "999999\n").expect("pid");

    let output = autospec()
        .args([
            "autonomous",
            "supervise",
            "--repo",
            "berlinguyinca/autospec",
            "--once",
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .output()
        .expect("autospec autonomous supervise runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("\"action\":\"stale-metadata\""));
}

#[test]
fn autonomous_restart_replaces_existing_target_companions() {
    let temp = temp_dir("autospec-autonomous-restart");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let repo_dir = temp.join("repo");
    std::fs::create_dir_all(&repo_dir).expect("repo dir");

    start_sleeping_autonomous(&operator_dir, &log_dir, &repo_dir, "berlinguyinca/autospec");
    let scope = operator_dir.join("berlinguyinca_autospec");
    let old_conductor = read_pid(&scope, "conductor");

    let output = autospec()
        .args([
            "autonomous",
            "restart",
            "--repo",
            "berlinguyinca/autospec",
            "--repo-dir",
            repo_dir.to_str().unwrap(),
            "--max-cycles",
            "7",
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .env("AUTOSPEC_AUTONOMOUS_CONDUCTOR_CMD", "sleep 20")
        .env("AUTOSPEC_AUTONOMOUS_MONITOR_CMD", "sleep 20")
        .env("AUTOSPEC_AUTONOMOUS_SUPERVISOR_CMD", "sleep 20")
        .output()
        .expect("autospec autonomous restart runs");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let new_conductor = read_pid(&scope, "conductor");

    assert!(output.status.success());
    assert!(stdout.contains("\"subcommand\":\"restart\""));
    assert_ne!(old_conductor, new_conductor);
    assert!(!process_is_alive(&old_conductor));
    assert!(process_is_alive(&new_conductor));
    let launch = std::fs::read_to_string(scope.join("launch.json")).expect("launch json");
    assert!(launch.contains("\"max_cycles\":\"7\""));

    cleanup_pids(&scope);
}

#[test]
fn autonomous_restart_clears_existing_stop_flag_before_launch() {
    let temp = temp_dir("autospec-autonomous-restart-stop-flag");
    let operator_dir = temp.join("operator");
    let log_dir = temp.join("logs");
    let stop_flag = temp.join("stop.flag");
    let repo_dir = temp.join("repo");
    make_git_repo(&repo_dir, None);
    std::fs::write(&stop_flag, "graceful\nold\n").expect("stop flag");

    let output = autospec()
        .args([
            "autonomous",
            "restart",
            "--repo",
            "berlinguyinca/autospec",
            "--repo-dir",
            repo_dir.to_str().unwrap(),
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &operator_dir)
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", &log_dir)
        .env("AUTOSPEC_STOP_FLAG_FILE", &stop_flag)
        .env("AUTOSPEC_AUTONOMOUS_CONDUCTOR_CMD", "sleep 20")
        .env("AUTOSPEC_AUTONOMOUS_MONITOR_CMD", "sleep 20")
        .env("AUTOSPEC_AUTONOMOUS_SUPERVISOR_CMD", "sleep 20")
        .output()
        .expect("autospec autonomous restart runs");

    assert!(output.status.success());
    assert!(!stop_flag.exists());
    cleanup_pids(&operator_dir.join("berlinguyinca_autospec"));
}

#[test]
fn cli_commands_json_modes_emit_json() {
    for command in [
        "doctor",
        "status",
        "plan",
        "validate",
        "report",
        "showcase",
        "growth-report",
    ] {
        let output = autospec()
            .args([command, "--json"])
            .output()
            .unwrap_or_else(|error| panic!("{command} runs: {error}"));
        let stdout = String::from_utf8_lossy(&output.stdout);

        assert!(output.status.success(), "{command} failed");
        assert!(
            stdout.trim_start().starts_with('{'),
            "{command} did not emit JSON"
        );
        assert!(stdout.contains(&format!("\"command\":\"{command}\"")));
    }
}

fn start_sleeping_autonomous(
    operator_dir: &std::path::Path,
    log_dir: &std::path::Path,
    repo_dir: &std::path::Path,
    repo: &str,
) {
    if !repo_dir.join(".git").exists() {
        make_git_repo(repo_dir, None);
    }
    let output = autospec()
        .args([
            "autonomous",
            "start",
            "--repo",
            repo,
            "--repo-dir",
            repo_dir.to_str().unwrap(),
            "--json",
        ])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", operator_dir)
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", log_dir)
        .env("AUTOSPEC_AUTONOMOUS_CONDUCTOR_CMD", "sleep 20")
        .env("AUTOSPEC_AUTONOMOUS_MONITOR_CMD", "sleep 20")
        .env("AUTOSPEC_AUTONOMOUS_SUPERVISOR_CMD", "sleep 20")
        .output()
        .expect("autospec autonomous start runs");
    assert!(output.status.success());
}

fn make_git_repo(repo_dir: &std::path::Path, remote: Option<&str>) {
    std::fs::create_dir_all(repo_dir).expect("repo dir");
    if !repo_dir.join(".git").exists() {
        assert!(Command::new("git")
            .args(["init"])
            .current_dir(repo_dir)
            .output()
            .expect("git init")
            .status
            .success());
    }
    if let Some(remote) = remote {
        let _ = Command::new("git")
            .args(["remote", "remove", "origin"])
            .current_dir(repo_dir)
            .output();
        assert!(Command::new("git")
            .args(["remote", "add", "origin", remote])
            .current_dir(repo_dir)
            .output()
            .expect("git remote add")
            .status
            .success());
    }
}

fn autonomous_status(
    operator_dir: &std::path::Path,
    log_dir: &std::path::Path,
    repo: &str,
) -> String {
    let output = autospec()
        .args(["autonomous", "status", "--repo", repo, "--json"])
        .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", operator_dir)
        .env("AUTOSPEC_AUTONOMOUS_LOG_DIR", log_dir)
        .output()
        .expect("autospec autonomous status runs");
    assert!(output.status.success());
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn read_pid(scope: &std::path::Path, name: &str) -> String {
    std::fs::read_to_string(scope.join(format!("{name}.pid")))
        .unwrap()
        .trim()
        .to_string()
}

fn process_is_alive(pid: &str) -> bool {
    Command::new("kill")
        .args(["-0", pid])
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn temp_dir(prefix: &str) -> std::path::PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("{prefix}-{}-{suffix}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn cleanup_pids(scope: &std::path::Path) {
    for name in ["conductor", "monitor", "supervisor"] {
        let pid_file = scope.join(format!("{name}.pid"));
        if let Ok(pid) = std::fs::read_to_string(pid_file) {
            let pid = pid.trim();
            if !pid.is_empty() {
                let _ = Command::new("kill").arg(pid).stderr(Stdio::null()).status();
            }
        }
    }
}

fn json_path(path: &std::path::Path) -> String {
    path.display().to_string().replace('\\', "\\\\")
}

fn write_conductor_log(
    operator_dir: &std::path::Path,
    scope: &str,
    contents: &str,
) -> std::path::PathBuf {
    let scope_dir = operator_dir.join(scope);
    let log = operator_dir.join(format!("{scope}.log"));
    std::fs::create_dir_all(&scope_dir).expect("scope dir");
    std::fs::write(&log, contents).expect("log");
    std::fs::write(
        scope_dir.join("conductor.logpath"),
        format!("{}\n", log.display()),
    )
    .expect("logpath");
    log
}

#[test]
fn doctor_readiness_json_reports_workflow_safety() {
    let output = autospec()
        .args(["doctor", "--readiness", "--json"])
        .output()
        .expect("autospec doctor runs");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("\"command\":\"doctor\""));
    assert!(stdout.contains("\"mode\":\"readiness\""));
    assert!(stdout.contains("\"workflow_recommendations\""));
    assert!(stdout.contains("\"define\""));
    assert!(stdout.contains("\"run\""));
    assert!(stdout.contains("\"autonomous\""));
}

#[test]
fn cli_commands_unimplemented_mutating_commands_are_explicit() {
    for command in ["init", "run", "resume", "benchmark"] {
        let output = autospec().arg(command).output().expect("autospec runs");
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert!(
            !output.status.success(),
            "{command} should not silently succeed"
        );
        assert!(stderr.contains("not yet implemented"));
    }
}
