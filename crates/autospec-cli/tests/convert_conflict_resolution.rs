//! The conversion pass's certified keep-both conflict resolution (issue
//! #4560): a `git apply --3way` conflict is a hold unless every conflicted
//! file's shape is certified keep-both by the classifier, in which case the
//! strict merge parser resolves the file, the gate (fmt, clippy, test) runs
//! on the merged tree, and the PR the pass opens names every resolution it
//! made on its own.
//!
//! The fixture is a real, minimal Rust crate: the gate's three cargo stages
//! actually run against it, so a resolution that corrupts the tree fails the
//! way the issue says it must — at the gate, not in review.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn autospec_bin() -> &'static str {
    env!("CARGO_BIN_EXE_autospec")
}

fn run_git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .status()
        .expect("git runs");
    assert!(status.success(), "git {args:?} failed in {dir:?}");
}

fn run_git_output(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git runs");
    String::from_utf8_lossy(&output.stdout)
        .trim_end()
        .to_string()
}

/// A file's content at a ref in the bare origin.
fn origin_show(origin: &Path, what: &str) -> String {
    let output = Command::new("git")
        .args(["--git-dir", origin.to_str().unwrap(), "show", what])
        .output()
        .expect("git runs");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// A minimal, gate-green Rust crate: two modules at the base, one function
/// each. Everything the fixture writes is rustfmt- and clippy-clean.
/// Returns the base blobs the agent's patch will carry in its `index` lines:
/// `git apply --3way` needs them to find the pre-move version of a file,
/// exactly as it would from a real `git diff`.

/// Record the gate the pass will run: without a recorded gate the pass
/// refuses to judge at all (#4556), so a fixture that exercises the gate
/// records it the way a real checkout carries its registry file.
fn record_gate(repo: &Path) {
    let data = repo.join("data");
    fs::create_dir_all(&data).expect("data dir");
    fs::write(
        data.join("convert-gate-registry.json"),
        r#"{"schema":1,"repos":{"test/fake":{"base_ref":"main","stages":[["fmt","--check"],["build","@scope"],["clippy","--all-targets","@scope"],["test","--no-fail-fast","@scope"]]}}}"#,
    )
    .expect("registry");
}

fn init_fixture_repo(repo: &Path, origin: &Path) -> (String, String) {
    fs::create_dir_all(repo.join("src")).unwrap();
    fs::write(
        repo.join("Cargo.toml"),
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n",
    )
    .unwrap();
    fs::write(repo.join("src/lib.rs"), "pub mod a;\n").unwrap();
    // Multi-line bodies: rustfmt's `fn_single_line` is false, and the gate's
    // fmt stage is real — the fixture must be fmt-clean at the base.
    fs::write(repo.join("src/a.rs"), "pub fn a() -> u32 {\n    1\n}\n").unwrap();
    run_git(repo, &["init", "-q", "-b", "main"]);
    let status = Command::new("git")
        .args(["init", "-q", "--bare", origin.to_str().unwrap()])
        .current_dir(repo)
        .status()
        .expect("git runs");
    assert!(status.success());
    run_git(repo, &["add", "-A"]);
    run_git(repo, &["commit", "-q", "-m", "base"]);
    run_git(repo, &["remote", "add", "origin", origin.to_str().unwrap()]);
    run_git(repo, &["push", "-q", "origin", "main"]);
    (
        run_git_output(repo, &["rev-parse", "main:src/lib.rs"]),
        run_git_output(repo, &["rev-parse", "main:src/a.rs"]),
    )
}

/// Main moves after the agent's work: a module is appended to the index and
/// (in `body`) an existing function's body is rewritten. This is the shape
/// that makes the agent's same-region patch conflict.
fn move_main(repo: &Path, body: bool) {
    fs::write(repo.join("src/lib.rs"), "pub mod a;\npub mod c;\n").unwrap();
    fs::write(repo.join("src/c.rs"), "pub fn c() -> u32 {\n    3\n}\n").unwrap();
    if body {
        fs::write(repo.join("src/a.rs"), "pub fn a() -> u32 {\n    4\n}\n").unwrap();
    }
    run_git(repo, &["add", "-A"]);
    run_git(repo, &["commit", "-q", "-m", "main moves"]);
    run_git(repo, &["push", "-q", "origin", "main"]);
}

/// The agent's patch against the pre-move base: it appends its own module to
/// the same index (conflicting with main's append) and adds the module file.
/// `lib_blob` / `a_blob` are the pre-move base blobs, as a real `git diff`
/// would name them.
fn agent_patch(body: bool, lib_blob: &str, a_blob: &str) -> String {
    let a_hunk = if body {
        "diff --git a/src/a.rs b/src/a.rs\n".to_string()
            + &format!("index {a_blob}..0000000\n")
            + "--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,3 +1,3 @@\n \
             pub fn a() -> u32 {\n-    1\n+    2\n }\n"
    } else {
        String::new()
    };
    format!(
        "{a_hunk}\
         diff --git a/src/lib.rs b/src/lib.rs\nindex {lib_blob}..0000000\n\
         --- a/src/lib.rs\n+++ b/src/lib.rs\n\
         @@ -1 +1,2 @@\n pub mod a;\n+pub mod b;\n\
         diff --git a/src/b.rs b/src/b.rs\nnew file mode 100644\n\
         --- /dev/null\n+++ b/src/b.rs\n\
         @@ -0,0 +1,3 @@\n+pub fn b() -> u32 {{\n+    2\n+}}\n"
    )
}

/// The agent's patch against the pre-move base, touching only the function
/// body — the single-file shape of #4637's deadlock: its one conflicted file
/// is the unclassifiable plain-module shape, so no keep-both file can turn
/// the conflict into a gate question.
fn agent_patch_a_only(a_blob: &str) -> String {
    format!(
        "diff --git a/src/a.rs b/src/a.rs\nindex {a_blob}..0000000\n\
         --- a/src/a.rs\n+++ b/src/a.rs\n\
         @@ -1,3 +1,3 @@\n pub fn a() -> u32 {{\n-    1\n+    2\n }}\n"
    )
}

fn write_patch(llm_root: &Path, issue: u64, text: &str) {
    let issue_dir = llm_root
        .join("node-a")
        .join("out")
        .join(format!("issue-{issue}"));
    fs::create_dir_all(&issue_dir).unwrap();
    fs::write(issue_dir.join("changes.patch"), text).unwrap();
}

/// A stand-in for `gh` on PATH: it records every argument it is given (one
/// per line) and succeeds, so the pass reaches its PR step and the test can
/// read the PR body the pass would have posted.
fn install_fake_gh(work: &Path) -> PathBuf {
    let bin_dir = work.join("fakebin");
    fs::create_dir_all(&bin_dir).unwrap();
    let shim = bin_dir.join("gh");
    // Arguments are joined with the 0x1F unit separator — the body carries
    // newlines, so a line-per-arg log would split it. Octal escape: POSIX
    // printf has no \x.
    fs::write(
        &shim,
        "#!/bin/sh\n: \"${FAKE_GH_LOG:?}\"\nfor arg in \"$@\"; do printf '%s\\037' \"$arg\" >> \
         \"$FAKE_GH_LOG\"; done\nprintf '\\n' >> \"$FAKE_GH_LOG\"\nexit 0\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&shim, fs::Permissions::from_mode(0o755)).unwrap();
    }
    bin_dir
}

fn run_convert(
    repo: &Path,
    args: &[&str],
    path_dir: Option<&Path>,
    gh_log: Option<&Path>,
) -> (i32, String, String) {
    let mut command = Command::new(autospec_bin());
    command
        .args(["convert"])
        .args(args)
        .current_dir(repo)
        .env("LLM", "/nonexistent-llm-for-this-test")
        // The pass commits the converted work. The identity must not depend on the
        // ambient git config: CI runners have none, and a commit that fails for that
        // reason surfaced as a bare "PR could not be opened" with an empty stderr.
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com");
    if let Some(dir) = path_dir {
        let mut path = dir.to_string_lossy().into_owned();
        path.push(':');
        path.push_str(&std::env::var("PATH").unwrap_or_default());
        command.env("PATH", path);
    }
    if let Some(log) = gh_log {
        command.env("FAKE_GH_LOG", log);
    }
    let output = command.output().expect("autospec convert runs");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// AC1 + guardrail 2: the only conflict is an additive_declarations conflict
/// (two appends to the same module index). The pass resolves it, the gate
/// runs on the merged tree, and the PR is opened naming the resolution.
#[test]
fn an_additive_declarations_conflict_is_resolved_gated_and_named_in_the_pr() {
    let work =
        std::env::temp_dir().join(format!("autospec-conv-conflict-ok-{}", std::process::id()));
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).unwrap();
    let repo = work.join("repo");
    let origin = work.join("origin.git");
    let llm_root = work.join("llm");
    let gh_log = work.join("gh.log");
    let (lib_blob, _a_blob) = init_fixture_repo(&repo, &origin);
    record_gate(&repo);
    move_main(&repo, false);
    write_patch(&llm_root, 201, &agent_patch(false, &lib_blob, &""));
    let bin_dir = install_fake_gh(&work);

    let (code, stdout, stderr) = run_convert(
        &repo,
        &[
            "--apply",
            "--llm-root",
            llm_root.to_str().unwrap(),
            "--base",
            "main",
            "--repo",
            "test/fake",
        ],
        Some(&bin_dir),
        Some(&gh_log),
    );
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");

    // The pass converted it — not held, not skipped.
    let outcome = stdout
        .lines()
        .find(|l| l.contains("convert: examined=1"))
        .unwrap_or(&stdout);
    assert!(outcome.contains("converted=1"), "stdout:\n{stdout}");
    assert!(!stdout.contains("  HELD  #201"), "stdout:\n{stdout}");

    // Guardrail 2: the PR body names the auto-resolution — the file, the
    // certified shape, and the plan. The log is 0x1F-separated: the body
    // itself carries newlines.
    let log = fs::read_to_string(&gh_log).unwrap_or_default();
    let args: Vec<&str> = log.trim_end().split('\u{1f}').collect();
    let body_idx = args
        .iter()
        .position(|a| *a == "--body")
        .expect("gh pr create was called with --body:\n{log}");
    let body = args[body_idx + 1];
    assert!(body.contains("Auto-resolved conflicts"), "body:\n{body}");
    assert!(body.contains("src/lib.rs"), "body:\n{body}");
    assert!(body.contains("additive_declarations"), "body:\n{body}");
    assert!(body.contains("keep both, deduplicated"), "body:\n{body}");

    // The pushed branch carries the union: main's module and the patch's
    // module, both present. The byte-for-byte union property itself is
    // asserted against the merge function in the core's tests.
    let branch_lib = origin_show(&origin, "conv-201:src/lib.rs");
    assert!(
        branch_lib.contains("pub mod c;"),
        "branch lib.rs:\n{branch_lib}"
    );
    assert!(
        branch_lib.contains("pub mod b;"),
        "branch lib.rs:\n{branch_lib}"
    );
    // Both module files exist on the branch: the merge kept the patch's new
    // file alongside main's.
    let branch_b = origin_show(&origin, "conv-201:src/b.rs");
    assert!(branch_b.contains("pub fn b()"), "branch b.rs:\n{branch_b}");
    let _ = fs::remove_dir_all(&work);
}

/// AC2: one additive_declarations conflict and one conflict the classifier
/// cannot certify (a function body in an ordinary file). The patch is HELD,
/// and the reason names both files and both shapes.
#[test]
fn a_mixed_conflict_holds_and_names_every_file() {
    let work = std::env::temp_dir().join(format!(
        "autospec-conv-conflict-mixed-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).unwrap();
    let repo = work.join("repo");
    let origin = work.join("origin.git");
    let llm_root = work.join("llm");
    let gh_log = work.join("gh.log");
    let (lib_blob, a_blob) = init_fixture_repo(&repo, &origin);
    record_gate(&repo);
    move_main(&repo, true); // main rewrites fn a's body, too
    write_patch(&llm_root, 202, &agent_patch(true, &lib_blob, &a_blob)); // the patch rewrites it differently
    let bin_dir = install_fake_gh(&work);

    let (code, stdout, stderr) = run_convert(
        &repo,
        &[
            "--apply",
            "--llm-root",
            llm_root.to_str().unwrap(),
            "--base",
            "main",
            "--repo",
            "test/fake",
        ],
        Some(&bin_dir),
        Some(&gh_log),
    );
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");

    // HELD — the unresolvable file holds the whole patch.
    let held = stdout
        .lines()
        .find(|l| l.contains("  HELD  #202:"))
        .unwrap_or(&stdout);
    assert!(
        held.contains("refusing auto-resolution"),
        "stdout:\n{stdout}"
    );
    // Both files are named, with their shapes: the body conflict is
    // unknown/refused, the index conflict is certified keep-both.
    assert!(held.contains("src/a.rs"), "stdout:\n{stdout}");
    assert!(held.contains("unknown"), "stdout:\n{stdout}");
    assert!(held.contains("src/lib.rs"), "stdout:\n{stdout}");
    assert!(held.contains("additive_declarations"), "stdout:\n{stdout}");

    // No PR was opened for a held patch. (`gh pr list` is the pass's
    // live-PR check and may run.)
    let log = fs::read_to_string(&gh_log).unwrap_or_default();
    assert!(
        !log.trim_end().split('\u{1f}').any(|a| a == "create"),
        "no gh pr create may run for a hold:\n{log}"
    );
    // And nothing was pushed.
    let branches = run_git_output(&repo, &["ls-remote", "--heads", "origin"]);
    assert!(
        !branches.contains("conv-202"),
        "no conversion branch may exist for a held patch:\n{branches}"
    );
    let _ = fs::remove_dir_all(&work);
}

/// The terminal disposition (issue #4637): a patch whose every conflicted
/// file is a shape the pass will never merge (here: the unclassifiable
/// plain-module shape) is not held. A hold leaves the patch on disk, and a
/// patch on disk suppresses re-dispatch of its issue forever — 193 queue
/// entries and 27 idle agent slots were measured in exactly that state. The
/// patch is invalidated instead: the disposition is recorded where the patch
/// lived, the patch is archived (never deleted), and the issue returns to
/// dispatch for regeneration against the current base.
#[test]
fn a_structurally_unconvertible_patch_is_invalidated_and_archived() {
    let work = std::env::temp_dir().join(format!(
        "autospec-conv-conflict-invalid-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).unwrap();
    let repo = work.join("repo");
    let origin = work.join("origin.git");
    let llm_root = work.join("llm");
    let gh_log = work.join("gh.log");
    let (_lib_blob, a_blob) = init_fixture_repo(&repo, &origin);
    record_gate(&repo);
    move_main(&repo, true); // main rewrites fn a's body
    write_patch(&llm_root, 303, &agent_patch_a_only(&a_blob)); // the patch rewrites it differently
    let bin_dir = install_fake_gh(&work);

    let (code, stdout, stderr) = run_convert(
        &repo,
        &[
            "--apply",
            "--llm-root",
            llm_root.to_str().unwrap(),
            "--base",
            "main",
            "--repo",
            "test/fake",
        ],
        Some(&bin_dir),
        Some(&gh_log),
    );
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");

    // Invalidated, not held: the line names the shape the pass will never
    // merge.
    let invalid = stdout
        .lines()
        .find(|l| l.contains("  INVALIDATED #303:"))
        .unwrap_or(&stdout);
    assert!(
        invalid.contains("refusing auto-resolution"),
        "stdout:\n{stdout}"
    );
    assert!(invalid.contains("src/a.rs"), "stdout:\n{stdout}");
    assert!(invalid.contains("unknown"), "stdout:\n{stdout}");
    assert!(!stdout.contains("  HELD  #303"), "stdout:\n{stdout}");
    assert!(
        stdout.contains("  DONE   #303: invalidated"),
        "stdout:\n{stdout}"
    );

    // The counter reports the category on its own: folded into `skipped` it
    // would be invisible, which is the deadlock #4637 describes.
    let outcome = stdout
        .lines()
        .find(|l| l.contains("convert: examined=1"))
        .unwrap_or(&stdout);
    assert!(outcome.contains("invalidated=1"), "stdout:\n{stdout}");
    assert!(outcome.contains("held=0"), "stdout:\n{stdout}");

    // The patch left the produced path — topup's `already produced` test can
    // no longer see it, so the issue re-enters dispatch. Archived, never
    // deleted.
    let issue_dir = llm_root.join("node-a").join("out").join("issue-303");
    assert!(
        !issue_dir.join("changes.patch").exists(),
        "the invalidated patch must leave the produced path"
    );
    let archived: Vec<String> = fs::read_dir(issue_dir.join("superseded"))
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        archived.len(),
        1,
        "the patch is archived, not deleted: {archived:?}"
    );

    // The disposition stands in for the patch: status, reason, and the base
    // the verdict was made against.
    let disposition = fs::read_to_string(issue_dir.join("disposition.txt")).unwrap();
    assert!(
        disposition.starts_with("status: invalidated\n"),
        "{disposition}"
    );
    assert!(
        disposition.contains("reason: conflict (refusing auto-resolution): src/a.rs"),
        "{disposition}"
    );
    assert!(disposition.contains("base: "), "{disposition}");

    // Nothing was offered: no PR, no branch.
    let log = fs::read_to_string(&gh_log).unwrap_or_default();
    assert!(
        !log.trim_end().split('\u{1f}').any(|a| a == "create"),
        "no gh pr create may run for an invalidation:\n{log}"
    );
    let branches = run_git_output(&repo, &["ls-remote", "--heads", "origin"]);
    assert!(
        !branches.contains("conv-303"),
        "no conversion branch may exist for an invalidated patch:\n{branches}"
    );
    let _ = fs::remove_dir_all(&work);
}

/// A module list in a file the path classifier does not recognise (not
/// `mod.rs`/`lib.rs`): before #4463 a conflict here was `Unknown` and held;
/// a declaration-only conflict now resolves by canonical union.
#[test]
fn a_declaration_only_conflict_in_a_non_index_file_is_resolved_not_held() {
    let work =
        std::env::temp_dir().join(format!("autospec-conv-declaration-{}", std::process::id()));
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).unwrap();
    let repo = work.join("repo");
    let origin = work.join("origin.git");
    let llm_root = work.join("llm");
    let gh_log = work.join("gh.log");

    fs::create_dir_all(repo.join("src")).unwrap();
    fs::write(
        repo.join("Cargo.toml"),
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n",
    )
    .unwrap();
    // `index.rs` lists modules; its declarations resolve under `src/index/`.
    fs::write(repo.join("src/lib.rs"), "mod index;\n").unwrap();
    fs::write(repo.join("src/index.rs"), "pub mod alpha;\n").unwrap();
    fs::create_dir_all(repo.join("src/index")).unwrap();
    fs::write(
        repo.join("src/index/alpha.rs"),
        "pub fn alpha() -> u32 {\n    1\n}\n",
    )
    .unwrap();
    run_git(&repo, &["init", "-q", "-b", "main"]);
    run_git(&repo, &["init", "-q", "--bare", origin.to_str().unwrap()]);
    run_git(&repo, &["add", "-A"]);
    run_git(&repo, &["commit", "-q", "-m", "base"]);
    run_git(
        &repo,
        &["remote", "add", "origin", origin.to_str().unwrap()],
    );
    run_git(&repo, &["push", "-q", "origin", "main"]);
    let index_blob = run_git_output(&repo, &["rev-parse", "main:src/index.rs"]);
    record_gate(&repo);

    // Main appends its own module to the index after the agent's base.
    fs::write(
        repo.join("src/index.rs"),
        "pub mod alpha;\npub mod gamma;\n",
    )
    .unwrap();
    fs::write(
        repo.join("src/index/gamma.rs"),
        "pub fn gamma() -> u32 {\n    3\n}\n",
    )
    .unwrap();
    run_git(&repo, &["add", "-A"]);
    run_git(&repo, &["commit", "-q", "-m", "main moves"]);
    run_git(&repo, &["push", "-q", "origin", "main"]);

    // The agent's patch appends its own module to the same index, plus its
    // file. Both sides add a `pub mod` to the one shared list.
    write_patch(
        &llm_root,
        401,
        &format!(
            "diff --git a/src/index.rs b/src/index.rs\nindex {index_blob}..0000000\n\
             --- a/src/index.rs\n+++ b/src/index.rs\n\
             @@ -1 +1,2 @@\n pub mod alpha;\n+pub mod beta;\n\
             diff --git a/src/index/beta.rs b/src/index/beta.rs\nnew file mode 100644\n\
             --- /dev/null\n+++ b/src/index/beta.rs\n\
             @@ -0,0 +1,3 @@\n+pub fn beta() -> u32 {{\n+    2\n+}}\n"
        ),
    );

    let bin_dir = install_fake_gh(&work);
    let (code, stdout, stderr) = run_convert(
        &repo,
        &[
            "--apply",
            "--llm-root",
            llm_root.to_str().unwrap(),
            "--base",
            "main",
            "--repo",
            "test/fake",
        ],
        Some(&bin_dir),
        Some(&gh_log),
    );
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");

    // Converted, not held: the conflict was confined to declarations and the
    // pass resolved it by canonical union.
    let outcome = stdout
        .lines()
        .find(|l| l.contains("convert: examined=1"))
        .unwrap_or(&stdout);
    assert!(outcome.contains("converted=1"), "stdout:\n{stdout}");
    assert!(!stdout.contains("  HELD  #401"), "stdout:\n{stdout}");

    // The merged index is the sorted union: both additions survive, and the
    // order is canonical (alpha, beta, gamma) rather than merge order.
    let merged = origin_show(&origin, "conv-401:src/index.rs");
    assert!(merged.contains("pub mod alpha;"), "merged:\n{merged}");
    assert!(merged.contains("pub mod beta;"), "merged:\n{merged}");
    assert!(merged.contains("pub mod gamma;"), "merged:\n{merged}");
    assert!(
        merged.find("pub mod alpha;").unwrap() < merged.find("pub mod beta;").unwrap(),
        "index is not canonically sorted:\n{merged}"
    );
    assert!(
        merged.find("pub mod beta;").unwrap() < merged.find("pub mod gamma;").unwrap(),
        "index is not canonically sorted:\n{merged}"
    );

    let _ = fs::remove_dir_all(&work);
}
