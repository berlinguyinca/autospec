use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
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
        "#!/bin/sh\nif [ \"$#\" -eq 4 ] && [ \"$1 $2 $3 $4\" = \"api user --jq .login\" ]; then printf '%s\\n' \"${AUTOSPEC_PARENT_TEST_WRITER:-berlinguyinca}\"; exit 0; fi",
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

const BASE_DECOMPOSITION: &str = "\
<!-- autospec-parent-decomposition:begin -->
Parent issue #10 was decomposed into child implementation issues:
- #11
- #12
<!-- autospec-parent-decomposition:end -->";

const EXTENDED_DECOMPOSITION: &str = "\
<!-- autospec-parent-decomposition:begin -->
Parent issue #10 was decomposed into child implementation issues:
- #11
- #12
- #13
- #14
State: `append-only-parent-extension`.
<!-- autospec-parent-decomposition:end -->";

fn write_remote(root: &Path, issue: u64, field: &str, value: &str) {
    fs::write(root.join("remote").join(format!("{issue}.{field}")), value)
        .expect("remote issue field");
}

const STATEFUL_PARENT_GH: &str = r#"#!/bin/sh
remote=$AUTOSPEC_PARENT_REMOTE
unexpected() {
  kind=$1
  shift
  printf '%s:' "$kind" >> "$AUTOSPEC_PARENT_GH_LOG"
  separator=
  for argument in "$@"; do
    printf '%s%s' "$separator" "$argument" >> "$AUTOSPEC_PARENT_GH_LOG"
    separator=' '
  done
  printf '\n' >> "$AUTOSPEC_PARENT_GH_LOG"
  printf 'unexpected gh command\n' >&2
  exit 64
}
if [ "$1 $2" = "issue view" ]; then
  if [ "$#" -ne 9 ] || [ "$4" != "--repo" ] || [ "$5" != "testorg/testrepo" ] ||
     [ "$6" != "--json" ] || [ "$8" != "--jq" ] || [ -z "$9" ]; then
    unexpected unexpected-command "$@"
  fi
  case "$3" in 10|11|12|13|14) ;; *) unexpected unexpected-command "$@" ;; esac
  case "$7" in
    comments|state) cat "$remote/$3.$7" 2>/dev/null || true ;;
    *) unexpected unexpected-command "$@" ;;
  esac
  exit 0
fi
if [ "$1 $2" = "issue comment" ]; then
  if [ "$#" -ne 7 ] || [ "$4" != "--repo" ] || [ "$5" != "testorg/testrepo" ] ||
     [ "$6" != "--body" ]; then
    unexpected unexpected-mutation "$@"
  fi
  case "$3" in 10|11|12|13|14) ;; *) unexpected unexpected-mutation "$@" ;; esac
  printf 'comment:%s\n' "$3" >> "$AUTOSPEC_PARENT_GH_LOG"
  printf '%s\n' "$7" > "$remote/$3.comments"
  exit 0
fi
if [ "$1 $2" = "issue reopen" ]; then
  if [ "$#" -ne 5 ] || [ "$4" != "--repo" ] || [ "$5" != "testorg/testrepo" ]; then
    unexpected unexpected-mutation "$@"
  fi
  case "$3" in 10|11|12|13|14) ;; *) unexpected unexpected-mutation "$@" ;; esac
  printf 'reopen:%s\n' "$3" >> "$AUTOSPEC_PARENT_GH_LOG"
  printf 'OPEN\n' > "$remote/$3.state"
  exit 0
fi
if [ "$1 $2" = "issue close" ]; then
  if [ "$#" -ne 5 ] || [ "$4" != "--repo" ] || [ "$5" != "testorg/testrepo" ]; then
    unexpected unexpected-mutation "$@"
  fi
  case "$3" in 10|11|12|13|14) ;; *) unexpected unexpected-mutation "$@" ;; esac
  printf 'close:%s\n' "$3" >> "$AUTOSPEC_PARENT_GH_LOG"
  printf 'CLOSED\n' > "$remote/$3.state"
  exit 0
fi
if [ "$1 $2" = "api graphql" ]; then
  case "$*" in *mutation*) unexpected unexpected-mutation "$@" ;; esac
  if [ "$#" -ne 10 ] || [ "$3" != "-f" ] || [ "$5" != "-F" ] ||
     [ "$6" != "owner=testorg" ] || [ "$7" != "-F" ] ||
     [ "$8" != "name=testrepo" ] || [ "$9" != "-F" ]; then
    unexpected unexpected-command "$@"
  fi
  case "$4" in query=query*) ;; *) unexpected unexpected-command "$@" ;; esac
  case "${10}" in number=*) number=${10#number=} ;; *) unexpected unexpected-command "$@" ;; esac
  case "$number" in 11|12|13|14) ;; *) unexpected unexpected-command "$@" ;; esac
  cat "$remote/$number.graphql"
  exit 0
fi
case "$1" in issue|api) unexpected unexpected-mutation "$@" ;; esac
unexpected unexpected-command "$@"
"#;

fn stateful_parent_fixture(name: &str) -> PathBuf {
    let root = fixture(name);
    fs::create_dir(root.join("remote")).expect("remote fixture directory");
    write_remote(&root, 10, "comments", BASE_DECOMPOSITION);
    write_remote(&root, 10, "state", "OPEN\n");
    write_gh(&root.join("bin"), STATEFUL_PARENT_GH);
    root
}

fn parent_command(root: &Path, arguments: &[&str]) -> Output {
    autospec()
        .args(arguments)
        .current_dir(root)
        .env("PATH", path_with(&root.join("bin")))
        .env("AUTOSPEC_PARENT_GH_LOG", root.join("gh.log"))
        .env("AUTOSPEC_PARENT_REMOTE", root.join("remote"))
        .output()
        .expect("parent command starts")
}

fn extend(root: &Path, children: &str) -> Output {
    parent_command(
        root,
        &[
            "parent",
            "extend",
            "--repo",
            "testorg/testrepo",
            "--parent",
            "10",
            "--children",
            children,
        ],
    )
}

fn mutation_log(root: &Path) -> String {
    fs::read_to_string(root.join("gh.log")).unwrap_or_default()
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn parent_extend_fake_gh_rejects_unexpected_mutations() {
    let root = stateful_parent_fixture("extend-harness-guard");
    let issue_edit = Command::new(root.join("bin/gh"))
        .args(["issue", "edit", "13"])
        .env("AUTOSPEC_PARENT_GH_LOG", root.join("gh.log"))
        .env("AUTOSPEC_PARENT_REMOTE", root.join("remote"))
        .output()
        .expect("unexpected gh mutation starts");
    let graphql_mutation = Command::new(root.join("bin/gh"))
        .args([
            "api",
            "graphql",
            "-f",
            "query=mutation{deleteIssue(input:{}){clientMutationId}}",
        ])
        .env("AUTOSPEC_PARENT_GH_LOG", root.join("gh.log"))
        .env("AUTOSPEC_PARENT_REMOTE", root.join("remote"))
        .output()
        .expect("unexpected GraphQL mutation starts");
    let rest_mutation = Command::new(root.join("bin/gh"))
        .args([
            "api",
            "user",
            concat!("--", "method"),
            "PATCH",
            "-f",
            "name=x",
        ])
        .env("AUTOSPEC_PARENT_GH_LOG", root.join("gh.log"))
        .env("AUTOSPEC_PARENT_REMOTE", root.join("remote"))
        .output()
        .expect("unexpected REST mutation starts");

    assert!(!issue_edit.status.success());
    assert!(!graphql_mutation.status.success());
    assert!(!rest_mutation.status.success());
    assert_eq!(
        mutation_log(&root),
        concat!(
            "unexpected-mutation:issue edit 13\n",
            "unexpected-mutation:api graphql -f ",
            "query=mutation{deleteIssue(input:{}){clientMutationId}}\n",
            "unexpected-mutation:api user --",
            "method PATCH -f name=x\n"
        )
    );
}

#[test]
fn parent_extend_publishes_exact_markers_full_list_and_typed_output() {
    let root = stateful_parent_fixture("extend-first");

    let output = extend(&root, "11,12,13,14");

    assert_success(&output);
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "{\"changed\":true,\"parent\":10,\"children\":[11,12,13,14],\"added\":[13,14]}\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("remote/13.comments")).expect("child 13 marker"),
        "<!-- autospec-parent:10 -->\nChild issue #13 belongs to parent issue #10.\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("remote/14.comments")).expect("child 14 marker"),
        "<!-- autospec-parent:10 -->\nChild issue #14 belongs to parent issue #10.\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("remote/10.comments")).expect("parent ledger"),
        format!("{EXTENDED_DECOMPOSITION}\n")
    );
    assert_eq!(mutation_log(&root), "comment:13\ncomment:14\ncomment:10\n");
}

#[test]
fn parent_extend_repeat_is_a_zero_mutation_noop() {
    let root = stateful_parent_fixture("extend-repeat");
    assert_success(&extend(&root, "11,12,13,14"));
    fs::write(root.join("gh.log"), "").expect("reset mutation log");

    let output = extend(&root, "11,12,13,14");

    assert_success(&output);
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "{\"changed\":false,\"parent\":10,\"children\":[11,12,13,14],\"added\":[]}\n"
    );
    assert_eq!(mutation_log(&root), "");
}

#[test]
fn parent_extend_prevalidates_every_new_child_before_mutation() {
    let root = stateful_parent_fixture("extend-conflict");
    write_remote(&root, 14, "comments", "<!-- autospec-parent:9 -->\n");

    let output = extend(&root, "11,12,13,14");

    assert!(!output.status.success());
    assert!(has(
        &String::from_utf8_lossy(&output.stderr),
        "child issue #14 is already linked to parent #9"
    ));
    assert_eq!(mutation_log(&root), "");
}

#[test]
fn parent_extend_recovers_a_preexisting_marker_without_reposting_it() {
    let root = stateful_parent_fixture("extend-recovery");
    write_remote(
        &root,
        13,
        "comments",
        "<!-- autospec-parent:10 -->\nChild issue #13 belongs to parent issue #10.\n",
    );

    let output = extend(&root, "11,12,13");

    assert_success(&output);
    assert_eq!(mutation_log(&root), "comment:10\n");
    assert!(has(
        &fs::read_to_string(root.join("remote/10.comments")).expect("parent ledger"),
        "append-only-parent-extension"
    ));
}

#[test]
fn parent_extend_reconciliation_keeps_manual_closure_pending() {
    let root = stateful_parent_fixture("extend-manual-close");
    write_remote(&root, 10, "comments", EXTENDED_DECOMPOSITION);
    write_remote(
        &root,
        13,
        "comments",
        "<!-- autospec-parent:10 -->\nChild issue #13 belongs to parent issue #10.\n",
    );
    for issue in [11, 12, 14] {
        write_remote(
            &root,
            issue,
            "graphql",
            &format!(
                r#"{{"data":{{"repository":{{"issue":{{"number":{issue},"state":"CLOSED","closedByPullRequestsReferences":{{"nodes":[{{"mergedAt":"2026-07-29T10:00:00Z"}}]}}}}}}}}}}"#
            ),
        );
    }
    write_remote(
        &root,
        13,
        "graphql",
        r#"{"data":{"repository":{"issue":{"number":13,"state":"CLOSED","closedByPullRequestsReferences":{"nodes":[]}}}}}"#,
    );

    let output = parent_command(
        &root,
        &[
            "parent",
            "reconcile-child",
            "--repo",
            "testorg/testrepo",
            "--child",
            "13",
        ],
    );

    assert_success(&output);
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "{\"reconciled\":true,\"parent\":10,\"closed\":false,\"terminal\":3,\"total\":4}\n"
    );
    assert_eq!(mutation_log(&root), "");
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
