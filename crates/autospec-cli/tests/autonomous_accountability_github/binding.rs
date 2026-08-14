use super::*;

#[test]
fn zero_matches_creates_once_then_reconciles_exact_marker() {
    let fixture = Fixture::new("create");
    let mut store = store(&fixture);
    let projection = store.render().unwrap();
    let marker = format!(
        "<!-- autospec:run-epic repo=acme/widgets run_id={} -->",
        run().run_id()
    );
    let remote = issue(42, "OPEN", &format!("{marker}\n{}", projection.markdown));
    let mut github = StubGithub::with([
        Ok("[[]]".to_string()),
        Ok(String::new()),
        Ok(String::new()),
        Ok(String::new()),
        Ok(String::new()),
        Ok("https://github.com/acme/widgets/issues/42\n".to_string()),
        Ok(pages(&[remote])),
        Ok(String::new()),
        Ok("__return_last_edit__".to_string()),
    ]);
    let mut renewals = 0;

    let binding = bind_epic(&mut store, &mut github, request(), || {
        renewals += 1;
        Ok(())
    })
    .unwrap();

    assert_eq!(binding.number, 42);
    assert!(
        renewals >= 3,
        "lease must be renewed across remote boundaries"
    );
    assert_eq!(
        github
            .calls
            .iter()
            .filter(|call| matches!(call, GithubCommand::CreateIssue { .. }))
            .count(),
        1
    );
    assert_eq!(store.status().epic_number, Some(42));
    assert_eq!(store.status().pending_projection_count, 0);
}

#[test]
fn ambiguous_create_response_never_permits_a_second_create() {
    let fixture = Fixture::new("ambiguous");
    let mut store = store(&fixture);
    let marker = format!(
        "<!-- autospec:run-epic repo=acme/widgets run_id={} -->",
        run().run_id()
    );
    let remote = issue(43, "OPEN", &marker);
    let mut github = StubGithub::with([
        Ok("[[]]".to_string()),
        Ok(String::new()),
        Ok(String::new()),
        Ok(String::new()),
        Ok(String::new()),
        Err(GithubFailure::Ambiguous(
            "connection reset after request body".to_string(),
        )),
        Err(GithubFailure::RetryAfter {
            message: "Retry-After: 0".to_string(),
            delay: Duration::from_millis(1),
        }),
        Ok("[[]]".to_string()),
        Ok(pages(&[remote])),
        Ok(String::new()),
        Ok("__return_last_edit__".to_string()),
    ]);

    bind_epic(&mut store, &mut github, request(), || Ok(())).unwrap();
    let projected = github.last_edit.clone().unwrap();
    assert_eq!(
        github
            .calls
            .iter()
            .filter(|call| matches!(call, GithubCommand::CreateIssue { .. }))
            .count(),
        1
    );

    let mut second = StubGithub::with([Ok(pages(&[issue(43, "OPEN", &projected)]))]);
    bind_epic(&mut store, &mut second, request(), || Ok(())).unwrap();
    assert!(second
        .calls
        .iter()
        .all(|call| !matches!(call, GithubCommand::CreateIssue { .. })));
}

#[test]
fn multiple_exact_markers_fail_closed() {
    let fixture = Fixture::new("duplicates");
    let mut store = store(&fixture);
    let marker = format!(
        "<!-- autospec:run-epic repo=acme/widgets run_id={} -->",
        run().run_id()
    );
    let mut github = StubGithub::with([Ok(pages(&[
        issue(42, "OPEN", &marker),
        issue(43, "CLOSED", &marker),
    ]))]);

    let error = bind_epic(&mut store, &mut github, request(), || Ok(())).unwrap_err();
    assert!(error.to_string().contains("multiple"));
    assert!(github
        .calls
        .iter()
        .all(|call| !matches!(call, GithubCommand::CreateIssue { .. })));
}

#[test]
fn lease_loss_during_reconciliation_fails_before_spawn_binding() {
    let fixture = Fixture::new("lease-loss");
    let mut store = store(&fixture);
    let mut github = StubGithub::with([
        Ok("[[]]".to_string()),
        Ok(String::new()),
        Ok(String::new()),
        Ok(String::new()),
        Ok(String::new()),
        Err(GithubFailure::Ambiguous("timeout".to_string())),
        Ok("[[]]".to_string()),
    ]);

    let error = bind_epic(&mut store, &mut github, request(), || {
        Err("lifecycle lease token mismatch".to_string())
    })
    .unwrap_err();
    assert!(error.to_string().contains("lease"));
    assert_eq!(store.status().epic_number, None);
}
