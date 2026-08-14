use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn autospec() -> Command {
    Command::new(env!("CARGO_BIN_EXE_autospec"))
}

fn fixture(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "autospec-parent-{name}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(path.join("bin")).expect("fixture directory");
    path
}

fn write_gh(bin: &Path, script: &str) {
    let path = bin.join("gh");
    let script = script.replacen(
        "#!/bin/sh",
        "#!/bin/sh\nif [ \"$1 $2\" = \"api user\" ]; then printf '%s\\n' \"${AUTOSPEC_PARENT_TEST_WRITER:-berlinguyinca}\"; exit 0; fi",
        1,
    );
    let mut file = fs::File::create(&path).expect("fake gh");
    file.write_all(script.as_bytes()).expect("fake gh body");
    let mut permissions = file.metadata().expect("fake gh metadata").permissions();
    drop(file);
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("fake gh executable");
}

fn path_with(bin: &Path) -> String {
    format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").expect("PATH is set")
    )
}

fn has(text: &str, needle: &str) -> bool {
    text.find(needle).is_some()
}

#[test]
fn concurrent_parent_records_preserve_both_records() {
    let root = fixture("concurrent-records");
    let bin = root.join("bin");
    let log = root.join("gh.log");
    write_gh(
        &bin,
        "#!/bin/sh\nsleep 0.1\nprintf '%s\\n' \"$@\" >> \"$AUTOSPEC_PARENT_GH_LOG\"\nexit 0\n",
    );

    let mut first = autospec();
    first
        .args([
            "parent",
            "record",
            "--repo",
            "testorg/testrepo",
            "--parent",
            "10",
            "--children",
            "11",
        ])
        .current_dir(&root)
        .env("PATH", path_with(&bin))
        .env("AUTOSPEC_PARENT_GH_LOG", &log);
    let mut second = autospec();
    second
        .args([
            "parent",
            "record",
            "--repo",
            "testorg/testrepo",
            "--parent",
            "20",
            "--children",
            "21",
        ])
        .current_dir(&root)
        .env("PATH", path_with(&bin))
        .env("AUTOSPEC_PARENT_GH_LOG", &log);
    let mut first = first.spawn().expect("first record starts");
    let mut second = second.spawn().expect("second record starts");
    assert!(first.wait().expect("first record exits").success());
    assert!(second.wait().expect("second record exits").success());

    let state = fs::read_to_string(root.join(".autospec/state/specs.json")).expect("shared state");
    assert!(has(&state, "\"parent_issue\":10"));
    assert!(has(&state, "\"parent_issue\":20"));
    assert!(!root.join(".autospec/state/specs.json.tmp").exists());
}

#[test]
fn concurrent_reconciliation_posts_one_completion_summary() {
    let root = fixture("concurrent-reconcile");
    let bin = root.join("bin");
    let log = root.join("gh.log");
    let completed = root.join("completed");
    let closed = root.join("closed");
    let summary_count = root.join("summary-count");
    write_gh(
        &bin,
        r#"#!/bin/sh
printf '%s\n' "$@" >> "$AUTOSPEC_PARENT_GH_LOG"
if [ "$1 $2 $3" = "issue view 11" ]; then
  if [ "$7" = comments ]; then printf '%s\n' '<!-- autospec-parent:10 -->'; fi
  if [ "$7" = state ]; then printf 'CLOSED\n'; fi
  exit 0
fi
if [ "$1 $2 $3" = "issue view 10" ]; then
  if [ "$7" = comments ] && printf '%s' "$9" | grep -q 'decomposition'; then
    printf '%s\n' '<!-- autospec-parent-decomposition:begin -->' 'Parent issue #10 was decomposed into child implementation issues:' '- #11' '<!-- autospec-parent-decomposition:end -->'
  fi
  if [ "$7" = comments ] && printf '%s' "$9" | grep -q 'complete' && [ -f "$AUTOSPEC_PARENT_COMPLETED" ]; then
    printf '%s\n' '<!-- autospec-parent-complete:begin -->' 'All child implementation issues for parent #10 reached a terminal state:' '- #11' '' 'Closing parent issue automatically.' '<!-- autospec-parent-complete:end -->'
  fi
  if [ "$7" = state ]; then if [ -f "$AUTOSPEC_PARENT_CLOSED" ]; then printf 'CLOSED\n'; else printf 'OPEN\n'; fi; fi
  exit 0
fi
if [ "$1 $2 $3" = "issue comment 10" ]; then
  if mkdir "$AUTOSPEC_PARENT_COMPLETED.lock" 2>/dev/null; then
    : > "$AUTOSPEC_PARENT_COMPLETED"
    printf '1\n' >> "$AUTOSPEC_PARENT_SUMMARY_COUNT"
  else
    exit 1
  fi
  exit 0
fi
if [ "$1 $2 $3" = "issue close 10" ]; then : > "$AUTOSPEC_PARENT_CLOSED"; fi
exit 0
"#,
    );

    let record = autospec()
        .args([
            "parent",
            "record",
            "--repo",
            "testorg/testrepo",
            "--parent",
            "10",
            "--children",
            "11",
        ])
        .current_dir(&root)
        .env("PATH", path_with(&bin))
        .env("AUTOSPEC_PARENT_GH_LOG", &log)
        .env("AUTOSPEC_PARENT_COMPLETED", &completed)
        .env("AUTOSPEC_PARENT_CLOSED", &closed)
        .env("AUTOSPEC_PARENT_SUMMARY_COUNT", &summary_count)
        .output()
        .expect("record starts");
    assert!(record.status.success());

    let make_reconcile = || {
        let mut command = autospec();
        command
            .args([
                "parent",
                "reconcile-child",
                "--repo",
                "testorg/testrepo",
                "--child",
                "11",
            ])
            .current_dir(&root)
            .env("PATH", path_with(&bin))
            .env("AUTOSPEC_PARENT_GH_LOG", &log)
            .env("AUTOSPEC_PARENT_COMPLETED", &completed)
            .env("AUTOSPEC_PARENT_CLOSED", &closed)
            .env("AUTOSPEC_PARENT_SUMMARY_COUNT", &summary_count);
        command
    };
    let mut first = make_reconcile().spawn().expect("first reconcile starts");
    let mut second = make_reconcile().spawn().expect("second reconcile starts");
    assert!(first.wait().expect("first reconcile exits").success());
    assert!(second.wait().expect("second reconcile exits").success());

    assert_eq!(
        fs::read_to_string(summary_count).expect("summary count"),
        "1\n"
    );
}

#[test]
fn github_decomposition_replaces_a_stale_local_child_list() {
    let root = fixture("remote-authoritative");
    let bin = root.join("bin");
    let log = root.join("gh.log");
    write_gh(
        &bin,
        r#"#!/bin/sh
printf '%s\n' "$@" >> "$AUTOSPEC_PARENT_GH_LOG"
if [ "$1 $2 $3" = "issue view 11" ]; then
  if [ "$7" = comments ]; then printf '%s\n' '<!-- autospec-parent:10 -->'; fi
  if [ "$7" = state ]; then printf 'CLOSED\n'; fi
fi
if [ "$1 $2 $3" = "issue view 13" ] && [ "$7" = state ]; then printf 'OPEN\n'; fi
if [ "$1 $2 $3" = "issue view 10" ] && [ "$7" = comments ]; then
  printf '%s\n' '<!-- autospec-parent-decomposition:begin -->' 'Parent issue #10 was decomposed into child implementation issues:' '- #11' '- #13' '<!-- autospec-parent-decomposition:end -->'
fi
exit 0
"#,
    );
    fs::create_dir_all(root.join(".autospec/state")).expect("state directory");
    fs::write(
        root.join(".autospec/state/specs.json"),
        r#"{"schema":1,"specs":[],"parent_issues":[{"parent_issue":10,"child_issues":[{"issue":11,"terminal":false},{"issue":12,"terminal":false}],"quarantined_parent":false,"decomposition_comment_posted":true,"parent_closed":false}]}"#,
    )
    .expect("stale state");

    let output = autospec()
        .args([
            "parent",
            "reconcile-child",
            "--repo",
            "testorg/testrepo",
            "--child",
            "11",
        ])
        .current_dir(&root)
        .env("PATH", path_with(&bin))
        .env("AUTOSPEC_PARENT_GH_LOG", &log)
        .output()
        .expect("reconcile starts");
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let state = fs::read_to_string(root.join(".autospec/state/specs.json")).expect("state");
    assert!(has(&state, "\"issue\":13"));
    assert!(!has(&state, "\"issue\":12"));
}
