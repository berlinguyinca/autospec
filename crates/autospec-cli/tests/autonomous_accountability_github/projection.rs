use super::*;

#[test]
fn retryable_issue_failure_keeps_the_spawned_run_active() {
    let fixture = Fixture::new("retryable-failure");
    let mut store = store(&fixture);
    store
        .bind_epic(93, "https://github.com/acme/widgets/issues/93")
        .unwrap();
    store.mark_spawned().unwrap();
    let record = store
        .append_event(
            AccountabilityEvent::new(
                EventKind::Failed,
                "Issue attempt failed",
                "The issue remains eligible for bounded retry",
                vec![Evidence::outcome("retry scheduled")],
            )
            .unwrap(),
        )
        .unwrap();

    assert_eq!(store.status().lifecycle_phase, "spawned");
    assert!(record.event.created_at > 0);
    assert_eq!(record.event.created_at, record.event.updated_at);
    assert!(store.status().created_at > 0);
    assert!(store.status().updated_at >= store.status().created_at);
}

#[test]
fn pending_projection_coalesces_later_events_without_losing_high_watermark() {
    let fixture = Fixture::new("outbox-coalesce");
    let mut store = store(&fixture);
    store
        .append_event(
            AccountabilityEvent::new(
                EventKind::RunStarted,
                "Run started",
                "The launch is durable",
                vec![Evidence::outcome("started")],
            )
            .unwrap(),
        )
        .unwrap();
    let first = store.render().unwrap();
    store
        .append_event(
            AccountabilityEvent::new(
                EventKind::IssueClaimed { issue: 41 },
                "Issue claimed",
                "The next unit of work was selected",
                vec![Evidence::github_url("https://github.com/acme/widgets/issues/41").unwrap()],
            )
            .unwrap(),
        )
        .unwrap();

    let retry = store.projection_for_delivery().unwrap();
    assert_eq!(retry.revision, first.revision + 1);
    assert_eq!(retry.desired_high_watermark, 2);
    assert!(retry.markdown.contains("#41"));
}

#[test]
fn parked_projection_carries_links_and_closes_the_epic() {
    let fixture = Fixture::new("park-close");
    let mut store = store(&fixture);
    let marker = format!(
        "<!-- autospec:run-epic repo=acme/widgets run_id={} -->",
        run().run_id()
    );
    let remote = issue(94, "OPEN", &marker);
    let mut initial = StubGithub::with([
        Ok(pages(&[remote])),
        Ok(String::new()),
        Ok("__return_last_edit__".to_string()),
    ]);
    bind_epic(&mut store, &mut initial, request(), || Ok(())).unwrap();
    store.mark_spawned().unwrap();

    store
        .append_event(
            AccountabilityEvent::new(
                EventKind::IssueClaimed { issue: 41 },
                "Issue claimed",
                "The run owns this issue",
                vec![Evidence::github_url("https://github.com/acme/widgets/issues/41").unwrap()],
            )
            .unwrap(),
        )
        .unwrap();
    store
        .append_event(
            AccountabilityEvent::new(
                EventKind::PullRequestOpened { pull_request: 52 },
                "Pull request opened",
                "The implementation is reviewable",
                vec![Evidence::github_url("https://github.com/acme/widgets/pull/52").unwrap()],
            )
            .unwrap(),
        )
        .unwrap();
    store
        .append_event(
            AccountabilityEvent::new(
                EventKind::Parked,
                "Run parked",
                "The operator requested a resumable stop",
                vec![Evidence::outcome("parked")],
            )
            .unwrap(),
        )
        .unwrap();
    assert_eq!(store.status().accountability_state, "parked");
    store.render().unwrap();
    let current = initial.last_edit.unwrap();
    let mut github = StubGithub::with([
        Ok(pages(&[issue(94, "OPEN", &current)])),
        Ok(String::new()),
        Ok("__return_last_edit__".to_string()),
        Ok(String::new()),
    ]);

    bind_epic(&mut store, &mut github, request(), || Ok(())).unwrap();

    let projected = github.last_edit.unwrap();
    assert!(projected.contains("\"recovery_state\":\"parked\""));
    assert!(projected.contains("\"linked_issues\":[41]"));
    assert!(projected.contains("\"linked_pull_requests\":[52]"));
    assert!(github
        .calls
        .iter()
        .any(|call| matches!(call, GithubCommand::CloseIssue { number: 94, .. })));
    assert!(store.status().last_projected_at.is_some());
}

#[test]
fn pending_projection_rejects_tampered_existing_managed_content() {
    let fixture = Fixture::new("pending-tamper");
    let mut store = store(&fixture);
    let marker = format!(
        "<!-- autospec:run-epic repo=acme/widgets run_id={} -->",
        run().run_id()
    );
    let mut initial = StubGithub::with([
        Ok(pages(&[issue(99, "OPEN", &marker)])),
        Ok(String::new()),
        Ok("__return_last_edit__".to_string()),
    ]);
    bind_epic(&mut store, &mut initial, request(), || Ok(())).unwrap();
    store
        .append_event(
            AccountabilityEvent::new(
                EventKind::WorkSelected { issue: Some(41) },
                "Work selected",
                "The queue selected the next issue",
                vec![Evidence::outcome("selected")],
            )
            .unwrap(),
        )
        .unwrap();
    store.render().unwrap();
    let tampered = initial
        .last_edit
        .unwrap()
        .replacen("Build the requested", "Tampered remote", 1);
    let mut github = StubGithub::with([Ok(pages(&[issue(99, "OPEN", &tampered)]))]);

    let error = bind_epic(&mut store, &mut github, request(), || Ok(())).unwrap_err();

    assert!(error.to_string().contains("digest"));
    assert!(github
        .calls
        .iter()
        .all(|call| !matches!(call, GithubCommand::EditIssue { .. })));
}

#[test]
fn ambiguous_edit_persists_a_degradable_retry_schedule() {
    let fixture = Fixture::new("edit-retry-schedule");
    let mut store = store(&fixture);
    let marker = format!(
        "<!-- autospec:run-epic repo=acme/widgets run_id={} -->",
        run().run_id()
    );
    let mut initial = StubGithub::with([
        Ok(pages(&[issue(100, "OPEN", &marker)])),
        Ok(String::new()),
        Ok("__return_last_edit__".to_string()),
    ]);
    bind_epic(&mut store, &mut initial, request(), || Ok(())).unwrap();
    store
        .append_event(
            AccountabilityEvent::new(
                EventKind::Verified,
                "Verification recorded",
                "The proof must project durably",
                vec![Evidence::outcome("verified")],
            )
            .unwrap(),
        )
        .unwrap();
    store.render().unwrap();
    let current = initial.last_edit.unwrap();
    let mut github = StubGithub::with([
        Ok(pages(&[issue(100, "OPEN", &current)])),
        Err(GithubFailure::Ambiguous("HTTP 502 after edit".to_string())),
    ]);

    let error = bind_epic(&mut store, &mut github, request(), || Ok(())).unwrap_err();

    assert_eq!(
        error.projection_disposition(),
        Some(ProjectionDisposition::DegradableTransport)
    );
    assert!(store.status().next_projection_retry_at.is_some());
}

#[test]
fn marker_integrity_failure_is_typed_separately_from_transport_degradation() {
    let fixture = Fixture::new("typed-integrity");
    let mut store = store(&fixture);
    let marker = format!(
        "<!-- autospec:run-epic repo=acme/widgets run_id={} -->",
        run().run_id()
    );
    let mut github = StubGithub::with([Ok(pages(&[
        issue(95, "OPEN", &marker),
        issue(96, "CLOSED", &marker),
    ]))]);

    let error = bind_epic(&mut store, &mut github, request(), || Ok(())).unwrap_err();
    assert_eq!(
        error.projection_disposition(),
        Some(ProjectionDisposition::IntegrityBlock)
    );
}

#[test]
fn retryable_transport_failure_is_typed_as_degradable() {
    let fixture = Fixture::new("typed-transport");
    let mut store = store(&fixture);
    let mut github = StubGithub::with([Err(GithubFailure::Retryable(
        "temporary network outage".to_string(),
    ))]);

    let error = bind_epic(&mut store, &mut github, request(), || Ok(())).unwrap_err();
    assert_eq!(
        error.projection_disposition(),
        Some(ProjectionDisposition::DegradableTransport)
    );
}

#[test]
fn sanitizer_redacts_pem_payloads_and_embedded_absolute_paths() {
    let fixture = Fixture::new("sanitizer-followup");
    let mut store = store(&fixture);
    store
        .append_event(
            AccountabilityEvent::new(
                EventKind::Verified,
                "Checked path=/Users/alice/private and home=~/secret",
                "-----BEGIN PRIVATE KEY----- c2VjcmV0LXBheWxvYWQ= -----END PRIVATE KEY-----",
                vec![Evidence::outcome(
                    "config=C:\\Users\\alice\\secret share=\\\\server\\private",
                )],
            )
            .unwrap(),
        )
        .unwrap();

    let markdown = store.render().unwrap().markdown;
    for forbidden in [
        "/Users/alice/private",
        "~/secret",
        "c2VjcmV0LXBheWxvYWQ=",
        "C:\\Users\\alice\\secret",
        "\\\\server\\private",
    ] {
        assert!(
            !markdown.contains(forbidden),
            "leaked {forbidden}: {markdown}"
        );
    }
}
