use autospec_core::coordination::{
    commit_shares, estimate_touch_set, parse_declared_hotspots, predict_collisions, CommitHistory,
    HotspotLedger, RepoSignals,
};

struct FakeHistory {
    commits: Vec<Vec<String>>,
}

impl CommitHistory for FakeHistory {
    fn touched_files_per_commit(&self, max_commits: usize) -> Vec<Vec<String>> {
        self.commits.iter().take(max_commits).cloned().collect()
    }
}

fn history(commits: &[&[&str]]) -> FakeHistory {
    FakeHistory {
        commits: commits
            .iter()
            .map(|commit| commit.iter().map(|file| (*file).to_string()).collect())
            .collect(),
    }
}

fn issue(number: u64, text: &str) -> (u64, String) {
    (number, format!("#{number}: {text}"))
}

fn to_refs(issues: &[(u64, String)]) -> Vec<(u64, &str)> {
    issues
        .iter()
        .map(|(number, text)| (*number, text.as_str()))
        .collect()
}

#[test]
fn estimates_touch_sets_from_paths_file_line_references_and_directories() {
    let paths = estimate_touch_set(
        "Rename `loadLedger` in `src/store/ledger.go:42` under `cmd/gateway/` and \
         touch README.md and main.rs too; https://example.com/x and and/or are not paths.",
    );
    assert!(paths.contains("src/store/ledger.go"));
    assert!(paths.contains("cmd/gateway/"));
    assert!(paths.contains("README.md"));
    assert!(paths.contains("main.rs"));
    assert!(!paths.iter().any(|path| path.contains("example.com")));
    assert!(!paths.contains("and/or"));
}

#[test]
fn disjoint_batch_dispatches_fully_in_parallel_unchanged() {
    let issues = vec![
        issue(1, "add `src/parser/token.rs` support for generics"),
        issue(2, "fix `docs/cli-reference.md` typo table"),
        issue(3, "extend `tests/store_test.go` coverage"),
    ];
    let plan = predict_collisions(&to_refs(&issues), &RepoSignals::default());
    assert!(plan.is_fully_parallel());
    assert_eq!(plan.waves, vec![vec![1, 2, 3]]);
    assert!(plan.warnings.is_empty());
    assert!(plan.colliding_files.is_empty());
}

#[test]
fn colliding_batch_serialises_colliders_and_warns_with_count_and_file() {
    let mut issues = vec![issue(1, "rename `loadLedger` in `internal/cache.go`")];
    for number in 2..=3 {
        issues.push(issue(
            number,
            &format!("wire flag in `cmd/gateway/main.go:42` for variant {number}"),
        ));
    }
    issues.push(issue(
        4,
        "add route registration beside `cmd/gateway/main.go` handler",
    ));
    issues.push(issue(5, "docs-only change in `docs/faq.md`"));

    let plan = predict_collisions(&to_refs(&issues), &RepoSignals::default());

    assert!(!plan.is_fully_parallel());
    // Breadth first: the first main.go collider still fits wave 0 alongside
    // the disjoint issues; every later collider waits its own wave.
    assert_eq!(plan.waves[0], vec![1, 2, 5]);
    assert_eq!(plan.waves[1], vec![3]);
    assert_eq!(plan.waves[2], vec![4]);
    let warning = plan
        .warnings
        .iter()
        .find(|warning| warning.path == "cmd/gateway/main.go")
        .expect("hotspot warning names the colliding file");
    assert_eq!(warning.issue_count, 3);
    assert_eq!(warning.batch_size, 5);
    assert_eq!(
        warning.message,
        "3 of 5 issues are likely to touch cmd/gateway/main.go; \
         consider serialising or splitting that file first"
    );
    assert!(plan
        .colliding_files
        .contains(&"cmd/gateway/main.go".to_string()));
}

#[test]
fn declared_hotspot_is_honoured_as_an_input() {
    let document = concat!(
        "# Contributing\n\n",
        "- A large edit to `cmd/gateway/main.go` will conflict with almost anything.\n",
        "- Unrelated note about `docs/faq.md` which nobody cites.\n",
    );
    let declared = parse_declared_hotspots(document);
    assert!(declared.contains("cmd/gateway/main.go"));
    assert!(!declared.contains("docs/faq.md"));

    // Neither issue names the hotspot file; both name its directory, which
    // the declared hotspot upgrades into a predicted touch of the file.
    let issues = vec![
        issue(1, "add auth middleware under `cmd/gateway/`"),
        issue(2, "add rate limiter under `cmd/gateway/`"),
    ];
    let signals = RepoSignals {
        declared_hotspots: declared,
        commit_share: Default::default(),
    };
    let plan = predict_collisions(&to_refs(&issues), &signals);
    assert!(!plan.is_fully_parallel());
    assert_eq!(plan.waves, vec![vec![1], vec![2]]);
    assert_eq!(
        plan.warnings
            .iter()
            .map(|warning| warning.message.as_str())
            .collect::<Vec<_>>(),
        vec![
            "2 of 2 issues are likely to touch cmd/gateway/main.go; \
             consider serialising or splitting that file first"
        ]
    );
}

#[test]
fn commit_history_share_flags_undocumented_hotspots() {
    let commits = history(&[
        &["cmd/gateway/main.go", "a.go"],
        &["cmd/gateway/main.go"],
        &["cmd/gateway/main.go", "b.go"],
        &["c.go"],
    ]);
    let shares = commit_shares(&commits, 100);
    assert_eq!(shares.get("cmd/gateway/main.go"), Some(&0.75));
    let signals = RepoSignals {
        declared_hotspots: Default::default(),
        commit_share: shares,
    };
    let issues = vec![
        issue(1, "extend the server under `cmd/gateway/`"),
        issue(2, "extend graceful shutdown under `cmd/gateway/`"),
    ];
    let plan = predict_collisions(&to_refs(&issues), &signals);
    assert!(!plan.is_fully_parallel());
    assert!(plan
        .colliding_files
        .contains(&"cmd/gateway/main.go".to_string()));
}

#[test]
fn repeated_hotspot_across_batches_becomes_a_refactoring_suggestion() {
    let mut ledger = HotspotLedger::new();
    ledger.record("101", &["cmd/gateway/main.go".to_string()]);
    // Re-polling the same batch does not inflate the count.
    ledger.record("101", &["cmd/gateway/main.go".to_string()]);
    assert!(ledger.suggestions().is_empty());

    ledger.record("102+103", &["cmd/gateway/main.go".to_string()]);
    let suggestions = ledger.suggestions();
    assert_eq!(suggestions.len(), 1);
    assert_eq!(suggestions[0].path, "cmd/gateway/main.go");
    assert_eq!(suggestions[0].batch_count, 2);
    assert!(suggestions[0].message.contains("cmd/gateway/main.go"));
    assert!(suggestions[0].message.contains("splitting"));

    // A file hot in only one batch earns no suggestion.
    ledger.record("104", &["src/other.rs".to_string()]);
    assert!(ledger
        .suggestions()
        .iter()
        .all(|suggestion| suggestion.path != "src/other.rs"));
}

#[test]
fn hotspot_ledger_round_trips_through_json() {
    let mut ledger = HotspotLedger::new();
    ledger.record("1+2", &["cmd/gateway/main.go".to_string()]);
    ledger.record("3+4", &["src/a.rs".to_string(), "src/b.rs".to_string()]);
    let json = ledger.to_json();
    let restored = HotspotLedger::from_json(&json).expect("ledger json parses back");
    assert_eq!(restored, ledger);
    assert_eq!(restored.suggestions().len(), 0);
    assert!(HotspotLedger::from_json("{\"batches\":{}}").is_err());
    assert!(HotspotLedger::from_json("{}").is_err());
}
