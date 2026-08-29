// Split out of claim_commands.rs, which is past the size ratchet. This module owns the
// release/retirement facet: terminal merge evidence, and the local heartbeat plus session
// binding that a terminal release has to retire. `use super::*` gives it the shared
// fixture helpers defined in the parent.
use super::*;

#[test]
fn claim_release_records_terminal_merge_before_removing_the_active_label() {
    let fixture = temp_dir("autospec-claim-release");
    let bin = fixture.join("bin");
    let log = fixture.join("gh.log");
    let comments = fixture.join("comments.json");
    std::fs::create_dir_all(&bin).expect("fake bin directory");
    let repo = claim_git_repo(&fixture);
    let claim = RunStateRecord::new(
        "testorg/testrepo",
        42,
        "worker-a",
        "claimed",
        "feat/test",
        "",
        "claimed",
        Vec::new(),
        "2026-07-14T00:00:00Z",
        "2026-07-14T00:00:00Z",
        10_800,
    )
    .with_claim_id("claim-a");
    transition_claim_ref(&repo, &claim);
    write_executable(
        &bin.join("gh"),
        "#!/bin/sh\nset -eu\nprintf '%s\\n' \"$@\" >> \"$AUTOSPEC_CLAIM_LOG\"\nif [ \"$1\" = api ] && [ \"$2\" = repos/testorg/testrepo/issues/42/comments ]; then cat \"$AUTOSPEC_CLAIM_COMMENTS\"; exit 0; fi\nif [ \"$1\" = issue ] && [ \"$2\" = comment ]; then\n  body=''; shift 2\n  while [ \"$#\" -gt 0 ]; do case \"$1\" in --body) body=\"$2\"; shift 2 ;; *) shift ;; esac; done\n  jq --arg body \"$body\" '. + [{id: ((map(.id)|max) + 1),updated_at:\"2030-01-01T00:00:00Z\",body:$body}]' \"$AUTOSPEC_CLAIM_COMMENTS\" > \"$AUTOSPEC_CLAIM_COMMENTS.tmp\"\n  mv \"$AUTOSPEC_CLAIM_COMMENTS.tmp\" \"$AUTOSPEC_CLAIM_COMMENTS\"\n  exit 0\nfi\nif [ \"$1\" = issue ] && [ \"$2\" = edit ]; then exit 0; fi\nexit 17\n",
    );
    std::fs::write(
        &comments,
        r#"[{"id":100,"updated_at":"2026-07-14T00:00:00Z","body":"<!-- autospec-run-state:begin -->\n{\"schema\":1,\"repo\":\"testorg/testrepo\",\"issue\":42,\"worker_id\":\"worker-a\",\"state\":\"claimed\",\"branch\":\"feat/test\",\"pr\":\"\",\"step\":\"claimed\",\"paths\":[],\"claimed_at\":\"2026-07-14T00:00:00Z\",\"updated_at\":\"2026-07-14T00:00:00Z\",\"ttl_seconds\":10800}\n<!-- autospec-run-state:end -->"}]"#,
    )
    .expect("comments fixture");

    let output = autospec()
        .args([
            "claim",
            "release",
            "--issue",
            "42",
            "--repo",
            "testorg/testrepo",
            "--worker-id",
            "worker-a",
            "--claim-id",
            "claim-a",
            "--state",
            "merged",
            "--branch",
            "feat/test",
            "--pr",
            "99",
        ])
        .env(
            "AUTOSPEC_CLAIM_GIT_REMOTE",
            fixture.join("claim-remote.git"),
        )
        .env("AUTOSPEC_CLAIM_GIT_STATE_DIR", fixture.join("claim-state"))
        .env("PATH", path_with(&bin))
        .env("AUTOSPEC_CLAIM_COMMENTS", &comments)
        .env("AUTOSPEC_CLAIM_LOG", &log)
        .env("AUTOSPEC_CLAIM_RETRY_SLEEP_MS", "0")
        // Pin the heartbeat root into the fixture. Without this the command
        // reads the developer's real ~/.autospec/process-heartbeats, so stray
        // directories left by earlier runs leak into this assertion.
        .env("AUTOSPEC_HEARTBEAT_DIR", fixture.join("heartbeats"))
        .output()
        .expect("autospec claim release starts");

    assert!(output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).is_empty(),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"released\":true"));
    assert!(stdout.contains("\"state\":\"merged\""));
    let calls = std::fs::read_to_string(log).expect("gh call log");
    let terminal = calls.find("issue\ncomment\n42").unwrap();
    let successor = calls[terminal + 1..]
        .find("issue\ncomment\n42")
        .map(|offset| terminal + 1 + offset)
        .unwrap();
    let labels = calls
        .find("issue\nedit\n42\n--repo\ntestorg/testrepo\n--remove-label\nin-progress-by-bot")
        .unwrap();
    assert!(terminal < successor && successor < labels);
}

#[test]
fn claim_release_retires_the_local_heartbeat_and_session_binding() {
    let fixture = temp_dir("autospec-claim-release-retires");
    let bin = fixture.join("bin");
    let log = fixture.join("gh.log");
    let comments = fixture.join("comments.json");
    let mode = fixture.join("labels.mode");
    let heartbeats = fixture.join("heartbeats");
    std::fs::create_dir_all(&bin).expect("fake bin directory");
    let _claim_repo = claim_git_repo(&fixture);
    std::fs::write(&comments, "[]\n").expect("comments fixture");
    std::fs::write(&mode, "ready\n").expect("label mode fixture");
    write_executable(
        &bin.join("gh"),
        "#!/bin/sh\nset -eu\nprintf '%s\\n' \"$@\" >> \"$AUTOSPEC_CLAIM_LOG\"\nif [ \"$1\" = issue ] && [ \"$2\" = view ]; then\n  if [ \"$(cat \"$AUTOSPEC_CLAIM_MODE\")\" = active ]; then labels='[\"in-progress-by-bot\",\"safety:reviewed\"]'; else labels='[\"auto-implement\",\"safety:reviewed\"]'; fi\n  jq -n --argjson labels \"$labels\" --arg body \"$AUTOSPEC_CLAIM_ISSUE_BODY\" '{labels:$labels,title:\"Add Rust claim\",body:$body,author:\"agent\"}'\n  exit 0\nfi\nif [ \"$1\" = label ] && [ \"$2\" = create ]; then exit 0; fi\nif [ \"$1\" = issue ] && [ \"$2\" = edit ]; then if [ \"$(cat \"$AUTOSPEC_CLAIM_MODE\")\" = ready ]; then printf retry > \"$AUTOSPEC_CLAIM_MODE\"; exit 23; fi; printf active > \"$AUTOSPEC_CLAIM_MODE\"; exit 0; fi\nif [ \"$1\" = issue ] && [ \"$2\" = comment ]; then\n  body=''; shift 2\n  while [ \"$#\" -gt 0 ]; do case \"$1\" in --body) body=\"$2\"; shift 2 ;; *) shift ;; esac; done\n  jq --arg body \"$body\" '. + [{id:100,updated_at:\"2026-07-14T00:00:00Z\",body:$body}]' \"$AUTOSPEC_CLAIM_COMMENTS\" > \"$AUTOSPEC_CLAIM_COMMENTS.tmp\"\n  mv \"$AUTOSPEC_CLAIM_COMMENTS.tmp\" \"$AUTOSPEC_CLAIM_COMMENTS\"\n  exit 0\nfi\nif [ \"$1\" = api ] && [ \"$2\" = repos/testorg/testrepo/issues/42/comments ]; then cat \"$AUTOSPEC_CLAIM_COMMENTS\"; exit 0; fi\nexit 0\n",
    );
    let body = "## Safety review\n\n<!-- autospec-safety:begin -->\n- **decision:** `SAFETY_PASS`\n<!-- autospec-safety:end -->\n\n## Goal\nAdd the Rust implementation.";

    let mut acquire = autospec();
    acquire
        .args([
            "claim",
            "acquire",
            "--issue",
            "42",
            "--repo",
            "testorg/testrepo",
            "--worker-id",
            "worker-a",
            "--branch",
            "feat/test",
            "--session-id",
            "session-real-7",
        ])
        .env("PATH", path_with(&bin))
        .env("AUTOSPEC_CLAIM_LOG", &log)
        .env("AUTOSPEC_CLAIM_COMMENTS", &comments)
        .env("AUTOSPEC_CLAIM_MODE", &mode)
        .env("AUTOSPEC_CLAIM_ISSUE_BODY", body)
        .env("AUTOSPEC_HEARTBEAT_DIR", &heartbeats)
        .env(
            "AUTOSPEC_CLAIM_GIT_REMOTE",
            fixture.join("claim-remote.git"),
        )
        .env("AUTOSPEC_CLAIM_GIT_STATE_DIR", fixture.join("claim-state"))
        .env("AUTOSPEC_GH_API_RETRIES", "1")
        .env("AUTOSPEC_CLAIM_CONFIRM_READS", "1")
        .env("AUTOSPEC_CLAIM_SETTLE_MILLIS", "0");
    assert_eq!(
        acquire
            .output()
            .expect("autospec claim acquire starts")
            .status
            .code(),
        Some(2)
    );
    let acquired = acquire.output().expect("autospec claim retry starts");
    assert!(acquired.status.success());

    // The heartbeat root is keyed by the length-prefixed repository progress key.
    // Pinned literally so a change to that convention fails here loudly.
    let issue_heartbeat = heartbeats.join("o7_testorg_r8_testrepo/42.json");
    let session_binding =
        heartbeats.join("o7_testorg_r8_testrepo/sessions/73657373696f6e2d7265616c2d37.json");
    assert!(
        issue_heartbeat.exists(),
        "acquire publishes the issue heartbeat"
    );
    assert!(
        session_binding.exists(),
        "acquire publishes the session binding"
    );
    let published = std::fs::read_to_string(&issue_heartbeat).expect("claim heartbeat");
    let claim_id = published
        .split_once("\"claim_id\":\"")
        .expect("heartbeat records a claim id")
        .1;
    let claim_id = claim_id.split_once('"').expect("claim id terminator").0;

    let released = autospec()
        .args([
            "claim",
            "release",
            "--issue",
            "42",
            "--repo",
            "testorg/testrepo",
            "--worker-id",
            "worker-a",
            "--claim-id",
            claim_id,
            "--state",
            "merged",
            "--branch",
            "feat/test",
            "--pr",
            "99",
        ])
        .env("PATH", path_with(&bin))
        .env("AUTOSPEC_CLAIM_LOG", &log)
        .env("AUTOSPEC_CLAIM_COMMENTS", &comments)
        .env("AUTOSPEC_CLAIM_MODE", &mode)
        .env("AUTOSPEC_CLAIM_ISSUE_BODY", body)
        .env("AUTOSPEC_HEARTBEAT_DIR", &heartbeats)
        .env(
            "AUTOSPEC_CLAIM_GIT_REMOTE",
            fixture.join("claim-remote.git"),
        )
        .env("AUTOSPEC_CLAIM_GIT_STATE_DIR", fixture.join("claim-state"))
        .env("AUTOSPEC_GH_API_RETRIES", "1")
        .env("AUTOSPEC_CLAIM_RETRY_SLEEP_MS", "0")
        .output()
        .expect("autospec claim release starts");

    assert!(
        released.status.success(),
        "release failed: {} {}",
        String::from_utf8_lossy(&released.stdout),
        String::from_utf8_lossy(&released.stderr)
    );
    assert!(String::from_utf8_lossy(&released.stdout).contains("\"released\":true"));
    assert!(
        !issue_heartbeat.exists(),
        "terminal release retires the issue heartbeat"
    );
    assert!(
        !session_binding.exists(),
        "terminal release retires the session binding so the session can claim again"
    );
}

// Linux-only: the fixture forges a predecessor heartbeat, which means reproducing the
// host and boot identity the record is keyed by from /proc. `retire_terminal` itself is
// cfg'd per platform, so the behaviour under test is the Linux one.
#[cfg(target_os = "linux")]
#[test]
fn claim_acquire_reports_the_predecessor_claim_id_when_retirement_fails() {
    // Reproduces the wedge seen in production: a released predecessor whose
    // on-disk heartbeat carries a different claim identity (a legacy record, or
    // one from an earlier generation) makes retirement fail on every subsequent
    // acquire. Recovery needs `claim release --claim-id <predecessor>`, so the
    // refusal must name that claim id instead of forcing the operator to scrape
    // it out of a GitHub comment.
    let fixture = temp_dir("autospec-claim-predecessor-id");
    let bin = fixture.join("bin");
    let log = fixture.join("gh.log");
    let heartbeats = fixture.join("heartbeats");
    let heartbeat_repo = heartbeats.join("o7_testorg_r8_testrepo");
    let repo = claim_git_repo(&fixture);
    transition_claim_ref(
        &repo,
        &RunStateRecord::new(
            "testorg/testrepo",
            42,
            "worker-a",
            "released",
            "feat/test",
            "",
            "released",
            Vec::new(),
            "2026-07-14T00:00:00Z",
            "2026-07-14T00:00:00Z",
            10_800,
        )
        .with_claim_id("claim-predecessor"),
    );
    std::fs::create_dir_all(&heartbeat_repo).expect("heartbeat repository");
    std::fs::set_permissions(&heartbeats, std::fs::Permissions::from_mode(0o700))
        .expect("private heartbeat root");
    std::fs::set_permissions(&heartbeat_repo, std::fs::Permissions::from_mode(0o700))
        .expect("private heartbeat repository");
    let host = std::fs::read_to_string("/proc/sys/kernel/hostname").expect("host identity");
    let boot = std::fs::read_to_string("/proc/sys/kernel/random/boot_id").expect("boot identity");
    // A heartbeat whose claim identity does not match the released record.
    let heartbeat = heartbeat_repo.join("42.json");
    std::fs::write(
        &heartbeat,
        format!(
            "{{\"issue\":\"42\",\"branch\":\"feat/test\",\"step\":\"claimed\",\"ts\":1,\"ttl_seconds\":10800,\"pid\":2147483647,\"nonce\":\"cb2fb10be6aeeaa790206bdd149beaf909af1587ff0f794c1a88d479f39f1ded\",\"host\":{:?},\"boot_id\":{:?},\"process_start\":\"1\",\"pr\":\"\",\"repo\":\"testorg/testrepo\",\"worker_id\":\"worker-a\",\"claim_id\":\"claim-from-an-older-generation\"}}\n",
            host.trim(),
            boot.trim()
        ),
    )
    .expect("mismatched predecessor heartbeat");
    std::fs::set_permissions(&heartbeat, std::fs::Permissions::from_mode(0o600))
        .expect("private predecessor heartbeat");
    std::fs::create_dir_all(&bin).expect("fake bin directory");
    write_executable(
        &bin.join("gh"),
        "#!/bin/sh\nset -eu\nprintf '%s\\n' \"$@\" >> \"$AUTOSPEC_CLAIM_LOG\"\nif [ \"$1\" = issue ] && [ \"$2\" = view ]; then\n  jq -n --arg body \"$AUTOSPEC_CLAIM_ISSUE_BODY\" '{labels:[\"auto-implement\",\"safety:reviewed\"],title:\"Add Rust claim\",body:$body,author:\"agent\"}'\n  exit 0\nfi\nif [ \"$1\" = api ]; then printf '[]\\n'; exit 0; fi\nexit 0\n",
    );
    let body = "## Safety review\n\n<!-- autospec-safety:begin -->\n- **decision:** `SAFETY_PASS`\n<!-- autospec-safety:end -->\n\n## Goal\nAdd the Rust implementation.";

    let output = autospec()
        .args([
            "claim",
            "acquire",
            "--issue",
            "42",
            "--repo",
            "testorg/testrepo",
            "--worker-id",
            "worker-b",
            "--branch",
            "feat/next",
            "--session-id",
            "session-next",
        ])
        .env("PATH", path_with(&bin))
        .env("AUTOSPEC_CLAIM_LOG", &log)
        .env("AUTOSPEC_CLAIM_ISSUE_BODY", body)
        .env("AUTOSPEC_HEARTBEAT_DIR", &heartbeats)
        .env(
            "AUTOSPEC_CLAIM_GIT_REMOTE",
            fixture.join("claim-remote.git"),
        )
        .env("AUTOSPEC_CLAIM_GIT_STATE_DIR", fixture.join("claim-state"))
        .env("AUTOSPEC_GH_API_RETRIES", "1")
        .env("AUTOSPEC_CLAIM_RETRY_SLEEP_MS", "0")
        .output()
        .expect("autospec claim acquire starts");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(output.status.code(), Some(2), "stdout: {stdout}");
    assert!(
        stdout.contains("\"reason\":\"predecessor_heartbeat_retirement_failed\""),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("\"claim_id\":\"claim-predecessor\""),
        "the refusal must name the predecessor claim id needed to release it: {stdout}"
    );
}
