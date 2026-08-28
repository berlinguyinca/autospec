use super::*;

#[derive(Default)]
struct StubProjectAssignment {
    target: Option<ProjectTarget>,
    pending: Vec<String>,
    acknowledged: Vec<String>,
}

impl ProjectAssignment for StubProjectAssignment {
    fn target(&self) -> Result<ProjectTarget, String> {
        self.target
            .clone()
            .ok_or_else(|| "managed Project binding is unavailable".to_string())
    }

    fn enqueue_issue(&mut self, issue_url: &str) -> Result<String, String> {
        let key = format!(
            "project:item-add:{}:{issue_url}",
            self.target().unwrap().node_id
        );
        self.pending.push(key.clone());
        Ok(key)
    }

    fn acknowledge_issue(&mut self, projection: &str) -> Result<(), String> {
        self.pending.retain(|pending| pending != projection);
        self.acknowledged.push(projection.to_string());
        Ok(())
    }
}

#[test]
fn managed_project_assignment_uses_bound_owner_and_retains_failed_projection() {
    let fixture = Fixture::new("managed-project-warning");
    let mut store = store(&fixture);
    let marker = format!(
        "<!-- autospec:run-epic repo=acme/widgets run_id={} -->",
        run().run_id()
    );
    let remote = issue(188, "OPEN", &marker);
    let mut github = StubGithub::with([
        Ok(pages(std::slice::from_ref(&remote))),
        Ok(String::new()),
        Ok("__return_last_edit__".to_string()),
        Err(GithubFailure::Definitive(
            "missing project scope".to_string(),
        )),
    ]);
    let mut project = StubProjectAssignment {
        target: Some(ProjectTarget {
            owner: "product-owner".to_string(),
            node_id: "PVT_managed".to_string(),
            number: 17,
        }),
        ..StubProjectAssignment::default()
    };

    let mut binding_request = request();
    binding_request.project_number = Some(9);
    let binding = bind_epic_with_project(
        &mut store,
        &mut github,
        binding_request,
        &mut project,
        || Ok(()),
    )
    .unwrap();

    assert_eq!(binding.number, 188);
    assert_eq!(
        binding.project_warning.as_deref(),
        Some("missing project scope")
    );
    assert_eq!(project.pending.len(), 1, "failed assignment stays durable");
    assert!(project.acknowledged.is_empty());
    assert!(github.calls.iter().any(|call| matches!(
        call,
        GithubCommand::AddToProject { owner, project_number: 17, .. }
            if owner == "product-owner"
    )));
    assert!(
        !github.calls.iter().any(|call| matches!(
            call,
            GithubCommand::AddToProject {
                project_number: 9,
                ..
            }
        )),
        "the managed binding must win over the legacy numeric map"
    );
}

#[test]
fn managed_project_assignment_acknowledges_successful_projection() {
    let fixture = Fixture::new("managed-project-ack");
    let mut store = store(&fixture);
    let marker = format!(
        "<!-- autospec:run-epic repo=acme/widgets run_id={} -->",
        run().run_id()
    );
    let remote = issue(189, "OPEN", &marker);
    let mut github = StubGithub::with([
        Ok(pages(std::slice::from_ref(&remote))),
        Ok(String::new()),
        Ok("__return_last_edit__".to_string()),
        Ok(String::new()),
    ]);
    let mut project = StubProjectAssignment {
        target: Some(ProjectTarget {
            owner: "product-owner".to_string(),
            node_id: "PVT_managed".to_string(),
            number: 17,
        }),
        ..StubProjectAssignment::default()
    };

    bind_epic_with_project(&mut store, &mut github, request(), &mut project, || Ok(())).unwrap();

    assert!(project.pending.is_empty());
    assert_eq!(project.acknowledged.len(), 1);
}

#[test]
fn zero_matches_binds_the_create_response_without_waiting_for_list_visibility() {
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
        Ok(remote),
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
    assert_eq!(
        github
            .calls
            .iter()
            .filter(|call| matches!(call, GithubCommand::ListAccountabilityIssues { .. }))
            .count(),
        1,
        "the successful create response is authoritative; no read-after-write list is needed"
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
