use super::*;

#[test]
fn explicit_resume_reconstructs_manifest_reopens_and_records_resume() {
    let fixture = Fixture::new("resume");
    let empty = AccountabilityStore::open(fixture.path()).unwrap();
    drop(empty);
    let projection = "Existing run overview";
    let manifest = accountability::RecoveryManifest::new(
        run(),
        77,
        "https://github.com/acme/widgets/issues/77",
        4,
        autospec_core::autonomous::waterfall::sha256_hex(format!("{projection}\n").as_bytes()),
        12,
        2,
    )
    .unwrap()
    .with_recovery_state(accountability::RecoveryState::Parked, vec![], vec![])
    .unwrap();
    let marker = format!(
        "<!-- autospec:run-epic repo=acme/widgets run_id={} -->",
        run().run_id()
    );
    let body = accountability::github::compose_managed_body(
        &marker,
        projection,
        &manifest,
        "human-authored tail",
    );
    let mut github = StubGithub::with([
        Ok(issue(77, "CLOSED", &body)),
        Ok(String::new()),
        Ok(issue(77, "OPEN", &body)),
        Ok(String::new()),
        Ok("__return_last_edit__".to_string()),
    ]);
    let mut store = AccountabilityStore::open(fixture.path()).unwrap();
    let mut resume_request = request();
    resume_request.explicit_epic = Some(77);
    resume_request.resume_policy = ResumePolicy::ReopenClosed;

    let binding = bind_epic(&mut store, &mut github, resume_request, || Ok(())).unwrap();

    assert_eq!(binding.number, 77);
    assert_eq!(store.status().journal_segment, 3);
    assert_eq!(store.status().event_count, 1);
    assert!(github
        .calls
        .iter()
        .any(|call| matches!(call, GithubCommand::ReopenIssue { number: 77, .. })));
    let edited = github.calls.iter().find_map(|call| match call {
        GithubCommand::EditIssue { body, .. } => Some(body),
        _ => None,
    });
    assert!(edited.unwrap().contains("human-authored tail"));
}

#[test]
fn explicit_epic_rejects_wrong_labels_or_duplicate_markers() {
    let fixture = Fixture::new("reject-explicit");
    let mut store = AccountabilityStore::open(fixture.path()).unwrap();
    let marker = format!(
        "<!-- autospec:run-epic repo=acme/widgets run_id={} -->",
        run().run_id()
    );
    let invalid_labels = format!(
        r#"{{"number":55,"url":"https://github.com/acme/widgets/issues/55","state":"OPEN","body":{},"labels":[{{"name":"epic"}}]}}"#,
        serde_json::to_string(&marker).unwrap()
    );
    let mut github = StubGithub::with([Ok(invalid_labels)]);
    let mut explicit = request();
    explicit.explicit_epic = Some(55);
    let error = bind_epic(&mut store, &mut github, explicit, || Ok(())).unwrap_err();
    assert!(error.to_string().contains("labels"));

    let duplicate = format!("{marker}\n{marker}");
    let mut github = StubGithub::with([Ok(issue(56, "OPEN", &duplicate))]);
    let mut explicit = request();
    explicit.explicit_epic = Some(56);
    let error = bind_epic(&mut store, &mut github, explicit, || Ok(())).unwrap_err();
    assert!(error.to_string().contains("exactly one"));
}

#[test]
fn resume_policy_rejects_parked_open_and_active_closed_epics() {
    for (name, state, recovery_state, policy) in [
        (
            "parked-open",
            "OPEN",
            accountability::RecoveryState::Parked,
            ResumePolicy::ActiveOnly,
        ),
        (
            "active-closed",
            "CLOSED",
            accountability::RecoveryState::Active,
            ResumePolicy::ReopenClosed,
        ),
        (
            "active-open-unowned",
            "OPEN",
            accountability::RecoveryState::Active,
            ResumePolicy::ActiveOnly,
        ),
    ] {
        let fixture = Fixture::new(name);
        let mut store = AccountabilityStore::open(fixture.path()).unwrap();
        let projection = "Existing run overview";
        let manifest = accountability::RecoveryManifest::new(
            run(),
            79,
            "https://github.com/acme/widgets/issues/79",
            4,
            autospec_core::autonomous::waterfall::sha256_hex(format!("{projection}\n").as_bytes()),
            12,
            2,
        )
        .unwrap()
        .with_recovery_state(recovery_state, vec![], vec![])
        .unwrap();
        let marker = format!(
            "<!-- autospec:run-epic repo=acme/widgets run_id={} -->",
            run().run_id()
        );
        let body = accountability::github::compose_managed_body(&marker, projection, &manifest, "");
        let mut github = StubGithub::with([Ok(issue(79, state, &body))]);
        let mut explicit = request();
        explicit.explicit_epic = Some(79);
        explicit.resume_policy = policy;

        let error = bind_epic(&mut store, &mut github, explicit, || Ok(())).unwrap_err();
        assert!(error.to_string().contains("policy"));
    }
}

#[test]
fn optional_project_failure_does_not_unbind_verified_epic() {
    let fixture = Fixture::new("project-warning");
    let mut store = store(&fixture);
    let marker = format!(
        "<!-- autospec:run-epic repo=acme/widgets run_id={} -->",
        run().run_id()
    );
    let remote = issue(88, "OPEN", &marker);
    let mut project_request = request();
    project_request.project_number = Some(9);
    let mut github = StubGithub::with([
        Ok(pages(std::slice::from_ref(&remote))),
        Ok(String::new()),
        Ok("__return_last_edit__".to_string()),
        Err(GithubFailure::Definitive(
            "missing project scope".to_string(),
        )),
    ]);

    let binding = bind_epic(&mut store, &mut github, project_request, || Ok(())).unwrap();
    assert_eq!(binding.number, 88);
    assert_eq!(store.status().epic_number, Some(88));
    assert_eq!(
        binding.project_warning.as_deref(),
        Some("missing project scope")
    );
}

#[test]
fn definitive_create_failure_is_not_reclassified_as_unknown_or_retried() {
    let fixture = Fixture::new("definitive-create");
    let mut store = store(&fixture);
    let mut github = StubGithub::with([
        Ok("[[]]".to_string()),
        Ok(String::new()),
        Ok(String::new()),
        Ok(String::new()),
        Ok(String::new()),
        Err(GithubFailure::Definitive("validation failed".to_string())),
    ]);

    let error = bind_epic(&mut store, &mut github, request(), || Ok(())).unwrap_err();
    assert!(error.to_string().contains("validation failed"));
    assert_eq!(
        github
            .calls
            .iter()
            .filter(|call| matches!(call, GithubCommand::CreateIssue { .. }))
            .count(),
        1
    );
}

#[test]
fn crash_after_local_binding_projects_missing_manifest_without_creating_again() {
    let fixture = Fixture::new("bound-before-manifest");
    let mut store = store(&fixture);
    let projection = store.render().unwrap();
    let marker = format!(
        "<!-- autospec:run-epic repo=acme/widgets run_id={} -->",
        run().run_id()
    );
    store
        .bind_epic(91, "https://github.com/acme/widgets/issues/91")
        .unwrap();
    assert_eq!(store.status().pending_projection_count, 1);
    let remote_without_manifest = issue(91, "OPEN", &format!("{marker}\n{}", projection.markdown));
    let mut github = StubGithub::with([
        Ok(pages(&[remote_without_manifest])),
        Ok(String::new()),
        Ok("__return_last_edit__".to_string()),
    ]);

    let binding = bind_epic(&mut store, &mut github, request(), || Ok(())).unwrap();
    assert_eq!(binding.number, 91);
    assert_eq!(store.status().pending_projection_count, 0);
    assert!(github
        .calls
        .iter()
        .all(|call| !matches!(call, GithubCommand::CreateIssue { .. })));
}

#[test]
fn managed_projection_rejects_duplicate_blocks_and_digest_mismatch() {
    let marker = format!(
        "<!-- autospec:run-epic repo=acme/widgets run_id={} -->",
        run().run_id()
    );
    let manifest = accountability::RecoveryManifest::new(
        run(),
        92,
        "https://github.com/acme/widgets/issues/92",
        3,
        "a".repeat(64),
        0,
        1,
    )
    .unwrap();
    let body = accountability::github::compose_managed_body(&marker, "projection", &manifest, "");
    let duplicate = body.replace(
        "<!-- autospec:accountability:end -->",
        "<!-- autospec:accountability:end -->\n<!-- autospec:accountability:start -->\nextra\n<!-- autospec:accountability:end -->",
    );
    for invalid in [
        duplicate,
        body.replacen("\nprojection\n", "\ntampered\n", 1),
    ] {
        let fixture = Fixture::new("managed-integrity");
        let mut store = store(&fixture);
        let mut github = StubGithub::with([Ok(issue(92, "OPEN", &invalid))]);
        let mut explicit = request();
        explicit.explicit_epic = Some(92);
        let error = bind_epic(&mut store, &mut github, explicit, || Ok(())).unwrap_err();
        assert!(
            error.to_string().contains("managed") || error.to_string().contains("digest"),
            "unexpected error: {error}"
        );
    }
}

#[test]
fn local_parked_and_terminal_runs_resume_into_a_new_active_segment() {
    for (name, kind, recovery_state) in [
        (
            "local-parked",
            EventKind::Parked,
            accountability::RecoveryState::Parked,
        ),
        (
            "local-terminal",
            EventKind::Completed,
            accountability::RecoveryState::Terminal,
        ),
    ] {
        let fixture = Fixture::new(name);
        let mut store = store(&fixture);
        store
            .bind_epic(97, "https://github.com/acme/widgets/issues/97")
            .unwrap();
        store.mark_spawned().unwrap();
        store
            .append_event(
                AccountabilityEvent::new(
                    kind,
                    "Run boundary recorded",
                    "The run must remain resumable",
                    vec![Evidence::outcome("boundary persisted")],
                )
                .unwrap(),
            )
            .unwrap();
        let projection = store.render().unwrap();
        let manifest = accountability::RecoveryManifest::new(
            run(),
            97,
            "https://github.com/acme/widgets/issues/97",
            projection.revision,
            &projection.digest,
            projection.desired_high_watermark,
            store.status().journal_segment,
        )
        .unwrap()
        .with_recovery_state(recovery_state, vec![], vec![])
        .unwrap();

        store.resume_bound_from_manifest(manifest).unwrap();

        assert_eq!(store.status().lifecycle_phase, "bound_not_spawned");
        assert_eq!(store.status().journal_segment, 2);
        assert!(store.has_event(&EventKind::ResumedFromEpic { epic: 97 }));
        assert_eq!(
            store.recovery_projection().0,
            accountability::RecoveryState::Active
        );
        store.mark_spawned().unwrap();
    }
}

#[test]
fn active_remote_epic_requires_matching_adopted_lease_generation() {
    let fixture = Fixture::new("active-adoption-proof");
    let projection = "Existing active run";
    let manifest = accountability::RecoveryManifest::new(
        run(),
        98,
        "https://github.com/acme/widgets/issues/98",
        3,
        autospec_core::autonomous::waterfall::sha256_hex(format!("{projection}\n").as_bytes()),
        4,
        1,
    )
    .unwrap();
    let marker = format!(
        "<!-- autospec:run-epic repo=acme/widgets run_id={} -->",
        run().run_id()
    );
    let body = accountability::github::compose_managed_body(&marker, projection, &manifest, "");
    let mut github = StubGithub::with([
        Ok(issue(98, "OPEN", &body)),
        Ok(String::new()),
        Ok("__return_last_edit__".to_string()),
    ]);
    let mut store = AccountabilityStore::open(fixture.path()).unwrap();
    let mut explicit = request();
    explicit.explicit_epic = Some(98);
    explicit.adopted_lease_generation = Some(7);

    let binding = bind_epic(&mut store, &mut github, explicit, || Ok(())).unwrap();

    assert_eq!(binding.number, 98);
    assert!(store.has_event(&EventKind::ResumedFromEpic { epic: 98 }));
}
