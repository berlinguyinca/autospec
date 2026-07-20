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
fn forged_markers_do_not_suppress_trusted_lifecycle_comments() {
    let root = fixture("forged-marker");
    let bin = root.join("bin");
    let log = root.join("gh.log");
    write_gh(
        &bin,
        r#"#!/bin/sh
printf '%s\n' "$@" >> "$AUTOSPEC_PARENT_GH_LOG"
if [ "$1 $2 $3" = "issue view 11" ]; then
  if [ "$7" = comments ]; then exit 0; fi
fi
if [ "$1 $2 $3" = "issue view 10" ] && [ "$7" = comments ]; then exit 0; fi
exit 0
"#,
    );

    let output = autospec()
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
        .output()
        .expect("parent record starts");

    assert!(output.status.success());
    let calls = fs::read_to_string(log).expect("gh calls");
    assert!(has(&calls, ".author.login == \"berlinguyinca\""));
    assert!(has(&calls, "issue\ncomment\n11"));
    assert!(has(&calls, "issue\ncomment\n10"));
}

#[test]
fn parent_record_rejects_an_untrusted_authenticated_writer_before_comments() {
    let root = fixture("untrusted-writer");
    let bin = root.join("bin");
    let log = root.join("gh.log");
    write_gh(
        &bin,
        "#!/bin/sh\nprintf '%s\\n' \"$@\" >> \"$AUTOSPEC_PARENT_GH_LOG\"\nexit 0\n",
    );

    let output = autospec()
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
        .env("AUTOSPEC_PARENT_TEST_WRITER", "outsider")
        .output()
        .expect("parent record starts");

    assert!(!output.status.success());
    assert!(has(
        &String::from_utf8_lossy(&output.stderr),
        "authenticated GitHub actor outsider is not trusted"
    ));
    let calls = fs::read_to_string(log).expect("gh calls");
    assert!(!has(&calls, "issue\ncomment"));
}

#[test]
fn parent_reconcile_rejects_an_untrusted_writer_even_with_a_trusted_completion_marker() {
    let root = fixture("untrusted-close");
    let bin = root.join("bin");
    let log = root.join("gh.log");
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
  if [ "$7" = comments ] && printf '%s' "$9" | grep -q 'complete'; then
    printf '%s\n' '<!-- autospec-parent-complete:begin -->' 'All child implementation issues for parent #10 reached a terminal state:' '- #11' '' 'Closing parent issue automatically.' '<!-- autospec-parent-complete:end -->'
  fi
  if [ "$7" = state ]; then printf 'OPEN\n'; fi
  exit 0
fi
exit 0
"#,
    );

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
        .env("AUTOSPEC_PARENT_TEST_WRITER", "outsider")
        .output()
        .expect("parent reconciliation starts");

    assert!(!output.status.success());
    assert!(has(
        &String::from_utf8_lossy(&output.stderr),
        "authenticated GitHub actor outsider is not trusted"
    ));
    let calls = fs::read_to_string(log).expect("gh calls");
    assert!(!has(&calls, "issue\nclose\n10"));
}

#[test]
fn parent_reconcile_rereads_children_and_refuses_close_after_reopen() {
    let root = fixture("reopen-race");
    let bin = root.join("bin");
    let log = root.join("gh.log");
    let counter = root.join("state-count");
    write_gh(
        &bin,
        r#"#!/bin/sh
printf '%s\n' "$@" >> "$AUTOSPEC_PARENT_GH_LOG"
if [ "$1 $2 $3" = "issue view 11" ]; then
  if [ "$7" = comments ]; then printf '%s\n' '<!-- autospec-parent:10 -->'; exit 0; fi
  if [ "$7" = state ]; then
    count=0; [ -f "$AUTOSPEC_PARENT_STATE_COUNT" ] && count=$(cat "$AUTOSPEC_PARENT_STATE_COUNT")
    count=$((count + 1)); printf '%s' "$count" > "$AUTOSPEC_PARENT_STATE_COUNT"
    if [ "$count" -eq 1 ]; then printf 'CLOSED\n'; else printf 'OPEN\n'; fi
    exit 0
  fi
fi
if [ "$1 $2 $3" = "issue view 10" ] && [ "$7" = comments ] && printf '%s' "$9" | grep -q 'decomposition'; then
  printf '%s\n' '<!-- autospec-parent-decomposition:begin -->' 'Parent issue #10 was decomposed into child implementation issues:' '- #11' '<!-- autospec-parent-decomposition:end -->'
fi
exit 0
"#,
    );

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
        .env("AUTOSPEC_PARENT_STATE_COUNT", &counter)
        .output()
        .expect("parent reconciliation starts");

    assert!(output.status.success());
    let calls = fs::read_to_string(log).expect("gh calls");
    assert!(!has(&calls, "issue\nclose\n10"));
    assert!(has(
        &String::from_utf8_lossy(&output.stdout),
        "\"closed\":false"
    ));
}

#[test]
fn parent_reconcile_reopens_parent_when_a_child_reopens_across_the_close_window() {
    let root = fixture("post-close-reopen-race");
    let bin = root.join("bin");
    let log = root.join("gh.log");
    let counter = root.join("state-count");
    let closed = root.join("parent-closed");
    write_gh(
        &bin,
        r#"#!/bin/sh
printf '%s\n' "$@" >> "$AUTOSPEC_PARENT_GH_LOG"
if [ "$1 $2 $3" = "issue view 11" ]; then
  if [ "$7" = comments ]; then printf '%s\n' '<!-- autospec-parent:10 -->'; exit 0; fi
  if [ "$7" = state ]; then
    count=0; [ -f "$AUTOSPEC_PARENT_STATE_COUNT" ] && count=$(cat "$AUTOSPEC_PARENT_STATE_COUNT")
    count=$((count + 1)); printf '%s' "$count" > "$AUTOSPEC_PARENT_STATE_COUNT"
    if [ "$count" -le 2 ]; then printf 'CLOSED\n'; else printf 'OPEN\n'; fi
    exit 0
  fi
fi
if [ "$1 $2 $3" = "issue view 10" ]; then
  if [ "$7" = comments ] && printf '%s' "$9" | grep -q 'decomposition'; then
    printf '%s\n' '<!-- autospec-parent-decomposition:begin -->' 'Parent issue #10 was decomposed into child implementation issues:' '- #11' '<!-- autospec-parent-decomposition:end -->'
  fi
  if [ "$7" = state ]; then
    if [ -f "$AUTOSPEC_PARENT_CLOSED" ]; then printf 'CLOSED\n'; else printf 'OPEN\n'; fi
  fi
  exit 0
fi
if [ "$1 $2 $3" = "issue close 10" ]; then : > "$AUTOSPEC_PARENT_CLOSED"; exit 0; fi
if [ "$1 $2 $3" = "issue reopen 10" ]; then rm -f "$AUTOSPEC_PARENT_CLOSED"; exit 0; fi
exit 0
"#,
    );

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
        .env("AUTOSPEC_PARENT_STATE_COUNT", &counter)
        .env("AUTOSPEC_PARENT_CLOSED", &closed)
        .output()
        .expect("parent reconciliation starts");

    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let calls = fs::read_to_string(log).expect("gh calls");
    assert!(has(&calls, "issue\nclose\n10"));
    assert!(has(&calls, "issue\nreopen\n10"));
    assert!(has(
        &String::from_utf8_lossy(&output.stdout),
        "\"closed\":false"
    ));
    assert!(!closed.exists());
    let state = fs::read_to_string(root.join(".autospec/state/specs.json"))
        .expect("reconciled parent state");
    assert!(has(&state, "\"issue\":11,\"terminal\":false"));
    assert!(has(&state, "\"parent_closed\":false"));
}

#[test]
fn parent_sweep_closes_a_parent_when_children_closed_outside_autospec() {
    let root = fixture("sweep");
    let bin = root.join("bin");
    let log = root.join("gh.log");
    write_gh(
        &bin,
        r#"#!/bin/sh
printf '%s\n' "$@" >> "$AUTOSPEC_PARENT_GH_LOG"
if [ "$1 $2 $3" = "issue view 10" ] && [ "$7" = comments ] && printf '%s' "$9" | grep -q 'decomposition'; then
  printf '%s\n' '<!-- autospec-parent-decomposition:begin -->' 'Parent issue #10 was decomposed into child implementation issues:' '- #11' '<!-- autospec-parent-decomposition:end -->'
fi
if [ "$1 $2 $3" = "issue view 11" ] && [ "$7" = state ]; then printf 'CLOSED\n'; fi
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
        .output()
        .expect("parent record starts");
    assert!(record.status.success());
    let sweep = autospec()
        .args(["parent", "sweep", "--repo", "testorg/testrepo"])
        .current_dir(&root)
        .env("PATH", path_with(&bin))
        .env("AUTOSPEC_PARENT_GH_LOG", &log)
        .output()
        .expect("parent sweep starts");

    assert!(
        sweep.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&sweep.stderr)
    );
    let calls = fs::read_to_string(log).expect("gh calls");
    assert!(has(&calls, "issue\nclose\n10"));
}

#[test]
fn parent_sweep_reopens_a_closed_parent_when_a_child_is_open_again() {
    let root = fixture("sweep-reopened-child");
    let bin = root.join("bin");
    let log = root.join("gh.log");
    fs::create_dir_all(root.join(".autospec/state")).expect("state directory");
    fs::write(
        root.join(".autospec/state/specs.json"),
        r#"{"schema":1,"specs":[],"parent_issues":[{"parent_issue":10,"child_issues":[{"issue":11,"terminal":true}],"quarantined_parent":false,"decomposition_comment_posted":true,"parent_closed":true}]}"#,
    )
    .expect("closed parent state");
    write_gh(
        &bin,
        r#"#!/bin/sh
printf '%s\n' "$@" >> "$AUTOSPEC_PARENT_GH_LOG"
if [ "$1 $2 $3" = "issue view 10" ]; then
  if [ "$7" = comments ] && printf '%s' "$9" | grep -q 'decomposition'; then
    printf '%s\n' '<!-- autospec-parent-decomposition:begin -->' 'Parent issue #10 was decomposed into child implementation issues:' '- #11' '<!-- autospec-parent-decomposition:end -->'
  fi
  if [ "$7" = state ]; then printf 'CLOSED\n'; fi
  exit 0
fi
if [ "$1 $2 $3" = "issue view 11" ] && [ "$7" = state ]; then printf 'OPEN\n'; exit 0; fi
exit 0
"#,
    );

    let output = autospec()
        .args(["parent", "sweep", "--repo", "testorg/testrepo"])
        .current_dir(&root)
        .env("PATH", path_with(&bin))
        .env("AUTOSPEC_PARENT_GH_LOG", &log)
        .output()
        .expect("parent sweep starts");

    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let calls = fs::read_to_string(log).expect("gh calls");
    assert!(has(&calls, "issue\nreopen\n10"));
    let state = fs::read_to_string(root.join(".autospec/state/specs.json"))
        .expect("reconciled parent state");
    assert!(has(&state, "\"issue\":11,\"terminal\":false"));
    assert!(has(&state, "\"parent_closed\":false"));
}
