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
fn parent_record_posts_child_list_and_links_each_child_idempotently() {
    let root = fixture("record");
    let bin = root.join("bin");
    let log = root.join("gh.log");
    write_gh(
        &bin,
        r#"#!/bin/sh
printf '%s\n' "$@" >> "$AUTOSPEC_PARENT_GH_LOG"
if [ "$1 $2 $3" = "issue view 10" ]; then
  exit 0
fi
if [ "$1 $2 $3" = "issue view 11" ]; then
  printf 'first child\n'
  exit 0
fi
if [ "$1 $2 $3" = "issue view 12" ]; then
  printf 'second child\n<!-- autospec-parent:10 -->\n'
  exit 0
fi
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
            "11,12",
            "--quarantined",
        ])
        .current_dir(&root)
        .env("PATH", path_with(&bin))
        .env("AUTOSPEC_PARENT_GH_LOG", &log)
        .output()
        .expect("parent record starts");

    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let calls = fs::read_to_string(log).expect("gh calls");
    assert!(has(&calls, "issue\ncomment\n11"));
    assert!(has(&calls, "<!-- autospec-parent:10 -->"));
    assert!(!has(&calls, "issue\nedit\n11"));
    assert!(!has(&calls, "issue\nedit\n12"));
    assert!(has(&calls, "issue\ncomment\n10"));
    assert!(has(&calls, "#11"));
    assert!(has(&calls, "#12"));
    assert!(has(&calls, "quarantined-parent-decomposed"));
}

#[test]
fn parent_record_rejects_a_child_already_linked_to_another_parent() {
    let root = fixture("conflicting-parent");
    let bin = root.join("bin");
    let log = root.join("gh.log");
    write_gh(
        &bin,
        r#"#!/bin/sh
printf '%s\n' "$@" >> "$AUTOSPEC_PARENT_GH_LOG"
if [ "$1 $2 $3" = "issue view 11" ]; then
  printf '%s\n' '<!-- autospec-parent:9 -->'
fi
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

    assert!(!output.status.success());
    assert!(has(
        &String::from_utf8_lossy(&output.stderr),
        "already linked to parent #9"
    ));
    let calls = fs::read_to_string(log).expect("gh calls");
    assert!(!has(&calls, "issue\ncomment\n11"));
    assert!(!has(&calls, "issue\ncomment\n10"));
}

#[test]
fn parent_reconcile_closes_parent_only_after_every_linked_child_is_closed() {
    let root = fixture("reconcile");
    let bin = root.join("bin");
    let log = root.join("gh.log");
    write_gh(
        &bin,
        r#"#!/bin/sh
printf '%s\n' "$@" >> "$AUTOSPEC_PARENT_GH_LOG"
if [ "$1 $2 $3" = "issue view 11" ] && [ "$4" = "--repo" ]; then
  if [ "$7" = comments ]; then printf '%s\n' '<!-- autospec-parent:10 -->'; fi
  if [ "$7" = state ]; then printf 'CLOSED\n'; fi
  exit 0
fi
if [ "$1 $2 $3" = "issue view 12" ]; then
  printf 'CLOSED\n'
  exit 0
fi
if [ "$1 $2 $3" = "issue view 10" ]; then
  if [ "$7" = comments ] && printf '%s' "$9" | grep -q 'decomposition'; then
    printf '%s\n' '<!-- autospec-parent-decomposition:begin -->' 'Parent issue #10 was decomposed into child implementation issues:' '- #11' '- #12' '<!-- autospec-parent-decomposition:end -->'
  fi
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
        .output()
        .expect("parent reconciliation starts");

    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let calls = fs::read_to_string(log).expect("gh calls");
    assert!(has(&calls, "issue\ncomment\n10"));
    assert!(has(&calls, "autospec-parent-complete:begin"));
    assert!(has(&calls, "issue\nclose\n10"));
    assert!(has(
        &String::from_utf8_lossy(&output.stdout),
        "\"closed\":true"
    ));
}

#[test]
fn parent_reconcile_is_pending_when_a_sibling_is_open() {
    let root = fixture("pending");
    let bin = root.join("bin");
    let log = root.join("gh.log");
    write_gh(
        &bin,
        r#"#!/bin/sh
printf '%s\n' "$@" >> "$AUTOSPEC_PARENT_GH_LOG"
if [ "$1 $2 $3" = "issue view 11" ] && [ "$7" = comments ]; then printf '%s\n' '<!-- autospec-parent:10 -->'; fi
if [ "$1 $2 $3" = "issue view 10" ]; then printf '%s\n' '<!-- autospec-parent-decomposition:begin -->' 'Parent issue #10 was decomposed into child implementation issues:' '- #11' '- #12' '<!-- autospec-parent-decomposition:end -->'; fi
if [ "$1 $2 $3" = "issue view 11" ] && [ "$7" = state ]; then printf 'CLOSED\n'; fi
if [ "$1 $2 $3" = "issue view 12" ]; then printf 'OPEN\n'; fi
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
fn parent_reconcile_is_idempotent_after_parent_is_already_closed() {
    let root = fixture("already-closed");
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
if [ "$1 $2 $3" = "issue view 12" ]; then
  printf 'CLOSED\n'
  exit 0
fi
if [ "$1 $2 $3" = "issue view 10" ]; then
  if [ "$7" = comments ] && printf '%s' "$9" | grep -q 'decomposition'; then printf '%s\n' '<!-- autospec-parent-decomposition:begin -->' 'Parent issue #10 was decomposed into child implementation issues:' '- #11' '- #12' '<!-- autospec-parent-decomposition:end -->'; fi
  if [ "$7" = comments ] && printf '%s' "$9" | grep -q 'complete'; then printf '%s\n' '<!-- autospec-parent-complete:begin -->' 'All child implementation issues for parent #10 reached a terminal state:' '- #11' '- #12' '' 'Closing parent issue automatically.' '<!-- autospec-parent-complete:end -->'; fi
  if [ "$7" = state ]; then printf 'CLOSED\n'; fi
  exit 0
fi
exit 0
"#,
    );

    for _ in 0..2 {
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
            .expect("parent reconciliation starts");
        assert!(
            output.status.success(),
            "stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let calls = fs::read_to_string(log).expect("gh calls");
    assert!(!has(&calls, "issue\ncomment\n10"));
    assert!(!has(&calls, "issue\nclose\n10"));
}
