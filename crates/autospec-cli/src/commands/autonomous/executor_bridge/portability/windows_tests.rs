use super::*;

static TEST_ENVIRONMENT: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct DirectFixture {
    root: PathBuf,
    worktree: PathBuf,
    artifacts: PathBuf,
}

impl DirectFixture {
    fn new() -> Self {
        let root = fs::canonicalize(std::env::temp_dir())
            .expect("canonicalize fixture temp directory")
            .join(format!(
                "autospec-windows-direct-{}-{}",
                std::process::id(),
                DIRECT_TRANSACTION_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
        let worktree = root.join("worktree");
        let artifacts = root.join("artifacts");
        fs::create_dir_all(&worktree).expect("create direct fixture");
        for args in [
            &["init", "--quiet"][..],
            &["config", "user.email", "autospec@example.invalid"][..],
            &["config", "user.name", "Autospec Test"][..],
            &["commit", "--quiet", "--allow-empty", "-m", "fixture"][..],
        ] {
            assert!(Command::new("git")
                .args(args)
                .current_dir(&worktree)
                .status()
                .expect("run fixture git command")
                .success());
        }
        Self {
            root,
            worktree,
            artifacts,
        }
    }
}

impl Drop for DirectFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct RestoreEnvironment {
    path: Option<OsString>,
    pathext: Option<OsString>,
}

impl Drop for RestoreEnvironment {
    fn drop(&mut self) {
        match self.path.take() {
            Some(value) => std::env::set_var("PATH", value),
            None => std::env::remove_var("PATH"),
        }
        match self.pathext.take() {
            Some(value) => std::env::set_var("PATHEXT", value),
            None => std::env::remove_var("PATHEXT"),
        }
    }
}

struct ProvisionedWorktreeCleanup {
    repository: PathBuf,
    worktree: PathBuf,
    scope_root: PathBuf,
}

impl Drop for ProvisionedWorktreeCleanup {
    fn drop(&mut self) {
        let _ = Command::new("git")
            .args(["worktree", "remove", "--force"])
            .arg(&self.worktree)
            .current_dir(&self.repository)
            .status();
        let _ = fs::remove_dir_all(&self.scope_root);
    }
}

#[test]
fn windows_executor_worktree_root_rejects_canonicalization_failure() {
    let error = resolve_executor_worktree_root_with(PathBuf::from(r"C:\Temp"), |_| {
        Err(std::io::Error::other("injected resolver failure"))
    })
    .expect_err("canonicalization failure must fail closed");
    assert!(error.contains("injected resolver failure"), "{error}");
}

#[test]
fn windows_executor_worktree_root_rejects_non_drive_paths() {
    for rejected in [PathBuf::from(r"\Temp"), PathBuf::from(r"C:Temp")] {
        let error =
            resolve_executor_worktree_root_with(
                PathBuf::from(r"C:\Temp"),
                |_| Ok(rejected.clone()),
            )
            .expect_err("root-relative and drive-relative paths must fail closed");
        assert!(
            error.contains("not absolute") || error.contains("not drive-qualified"),
            "{error}"
        );
    }
}

#[test]
fn direct_plan_resolves_pathext_wrapper_through_canonical_cmd() {
    let _environment = TEST_ENVIRONMENT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let fixture = DirectFixture::new();
    let wrapper = fixture.worktree.join("autospec-fixture-tool.CMD");
    fs::write(&wrapper, "@echo wrapper-output\r\n").expect("write command wrapper");
    let restore = RestoreEnvironment {
        path: std::env::var_os("PATH"),
        pathext: std::env::var_os("PATHEXT"),
    };
    let mut search = vec![fixture.worktree.clone()];
    search.extend(std::env::split_paths(
        &restore.path.clone().unwrap_or_default(),
    ));
    std::env::set_var(
        "PATH",
        std::env::join_paths(search).expect("join fixture PATH"),
    );
    std::env::set_var("PATHEXT", ".CMD;.EXE");

    let plan = DirectCommandPlan {
        commands: vec![DirectCommand::success(vec![
            "autospec-fixture-tool".to_string()
        ])],
    };
    let observed = execute_direct_plan(
        &fixture.worktree,
        &plan,
        &fixture.artifacts,
        None,
        Duration::from_secs(5),
    )
    .expect("execute Windows direct wrapper plan");
    drop(restore);

    assert_eq!(observed[0].terminal, AttemptTerminal::Exited(0));
    assert!(fs::read_to_string(&observed[0].stdout_path)
        .expect("read wrapper stdout")
        .contains("wrapper-output"));
    assert_eq!(
        observed[0]
            .process_executable
            .file_name()
            .and_then(OsStr::to_str),
        Some("cmd.exe")
    );
    assert_eq!(&observed[0].process_argv[1..4], ["/d", "/s", "/c"]);
    assert!(observed[0].process_argv[4].contains(
        fs::canonicalize(wrapper)
            .expect("canonicalize wrapper")
            .to_str()
            .expect("UTF-8 wrapper path")
    ));
}

#[test]
fn windows_executor_worktree_root_is_canonical_and_provisionable() {
    // Break caught: `/tmp/autospec-executor` is root-relative on Windows and cannot match
    // the canonical drive-qualified path persisted in bridge identity.
    let _environment = TEST_ENVIRONMENT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let fixture = DirectFixture::new();
    let remote = fixture.root.join("remote.git");
    assert!(Command::new("git")
        .args(["init", "--bare", remote.to_str().expect("UTF-8 remote")])
        .status()
        .expect("initialize Windows fixture remote")
        .success());
    for args in [
        vec!["branch", "-M", "main"],
        vec![
            "remote",
            "add",
            "origin",
            remote.to_str().expect("UTF-8 remote"),
        ],
        vec!["push", "--quiet", "-u", "origin", "main"],
    ] {
        assert!(Command::new("git")
            .args(&args)
            .current_dir(&fixture.worktree)
            .status()
            .expect("prepare Windows worktree fixture")
            .success());
    }
    let root = executor_worktree_root().expect("canonical Windows executor root");
    assert!(root.is_absolute(), "executor root must be drive-qualified");
    assert!(root.starts_with(
        fs::canonicalize(std::env::temp_dir()).expect("canonical Windows temp directory")
    ));
    harden_executor_worktree_root(&fixture.worktree, &root)
        .expect("provision canonical Windows executor root");
    assert_eq!(
        fs::canonicalize(&root).expect("canonical provisioned executor root"),
        root
    );
    let base =
        resolve_base(&fixture.worktree, &BTreeMap::new()).expect("resolve Windows fixture base");
    let scope = format!(
        "windows-native-{}-{}",
        std::process::id(),
        DIRECT_TRANSACTION_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let issue = provision_issue_worktree_for_claim(
        &fixture.worktree,
        &scope,
        42,
        &base,
        Some(("claim-windows", "invocation-windows")),
    )
    .expect("provision issue worktree through the Windows bridge path");
    let _cleanup = ProvisionedWorktreeCleanup {
        repository: fixture.worktree.clone(),
        worktree: issue.path.clone(),
        scope_root: issue
            .path
            .parent()
            .expect("provisioned worktree scope")
            .to_path_buf(),
    };
    assert!(issue.path.is_absolute());
    assert!(issue.path.starts_with(&root));
    assert_eq!(
        fs::canonicalize(&issue.path).expect("canonical Windows issue worktree"),
        issue.path
    );
}
