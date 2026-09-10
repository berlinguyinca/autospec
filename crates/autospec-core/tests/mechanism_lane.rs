//! The bounded fast lane for delivery-mechanism work (issue #3795).
//!
//! Nine of twenty-two open issues fix the pipeline they are queued behind, and
//! the pipeline is the slowest consumer of its own fixes: the work that would
//! shorten the queue is served by the queue. The four acceptance criteria:
//!
//! * mechanism work is **distinguishable** and scheduled separately — tests 1-5;
//! * the reserved capacity is **stated policy and bounded** — tests 6-9;
//! * a mechanism change is gated by the **mechanism's own tests** — tests 10-12;
//! * the system **reports its improvement rate**, and a fixed point is a
//!   number rather than an impression — tests 13-19.

use autospec_core::mechanism_lane::{
    gate_set, schedule, ChangeClass, Classification, ImprovementLedger, ImprovementVerdict,
    LandedClass, LanePlan, LanePolicy, MechanismComponent, MechanismSurface, WorkItem, Worklist,
    FIXTURE_GATE_BUDGET_SECS, FULL_GATE_BUDGET_SECS, MECHANISM_LABEL,
};

/// A fixed "now" so no test races the wall clock.
const NOW: u64 = 1_800_000_000;
/// One hour, the window the reservation is stated over.
const HOUR: u64 = 3_600;

fn surface() -> MechanismSurface {
    MechanismSurface::repository()
}

fn item(issue: u64, labels: &[&str], paths: &[&str]) -> WorkItem {
    WorkItem::new(issue).with(labels, paths)
}

fn worklist(items: &[WorkItem]) -> Worklist {
    Worklist {
        items: items.to_vec(),
        malformed: Vec::new(),
    }
}

// ── AC1: mechanism work is distinguishable ───────────────────────────────

#[test]
fn a_change_to_the_pipeline_is_mechanism_work() {
    let classification = Classification::classify(
        &surface(),
        ["automation:auto-implement"],
        ["crates/autospec-core/src/dispatch_pipeline.rs"],
    );
    assert!(classification.is_mechanism());
    assert_eq!(classification.class, ChangeClass::Mechanism);
    assert_eq!(classification.components, vec!["pipeline"]);
    assert_eq!(
        classification.matched_paths,
        vec!["crates/autospec-core/src/dispatch_pipeline.rs"]
    );
    assert!(!classification.label_declared);
    assert_eq!(classification.line(), "mechanism [pipeline]");
}

#[test]
fn a_change_to_a_products_file_is_payload() {
    let classification = Classification::classify(
        &surface(),
        ["automation:auto-implement"],
        ["crates/autospec-core/src/rag/retriever.rs"],
    );
    assert_eq!(classification.class, ChangeClass::Payload);
    assert!(classification.components.is_empty());
    assert_eq!(classification.line(), "payload");
}

#[test]
fn a_label_declares_mechanism_work_a_path_rule_cannot_reach() {
    // A fix to the monitor's prompt text has no path rule; the label is the
    // declaration. The class stands, and the report says no path matched.
    let classification = Classification::classify(
        &surface(),
        [MECHANISM_LABEL],
        ["skills/autospec-run/SKILL.md"],
    );
    assert!(classification.is_mechanism());
    assert!(classification.label_declared);
    assert!(classification.label_without_paths());
    assert!(classification
        .line()
        .contains("label only: no changed path is on the declared surface"));
}

#[test]
fn one_change_touching_two_components_names_both() {
    let classification = Classification::classify(
        &surface(),
        Vec::<String>::new(),
        [
            "crates/autospec-cli/src/commands/dispatch.rs",
            "crates/autospec-core/src/dispatch_pipeline.rs",
            "crates/autospec-core/src/rag/retriever.rs",
        ],
    );
    assert!(classification.is_mechanism());
    // Components are reported in declaration order, payload paths dropped.
    assert_eq!(classification.components, vec!["pipeline", "dispatcher"]);
    assert_eq!(classification.matched_paths.len(), 2);
}

#[test]
fn classification_survives_a_json_round_trip() {
    let classification = Classification::classify(
        &surface(),
        [MECHANISM_LABEL],
        ["scripts/lint-implementation.sh"],
    );
    let json = serde_json::to_string(&classification).expect("classification serializes");
    assert!(json.contains("\"class\":\"mechanism\""));
    let back: Classification = serde_json::from_str(&json).expect("classification deserializes");
    assert_eq!(back, classification);
}

// ── AC1: the class is scheduled separately ───────────────────────────────

#[test]
fn the_plan_puts_mechanism_work_in_the_lane_and_payload_in_the_queue() {
    let plan = schedule(
        &worklist(&[
            item(101, &[], &["crates/autospec-core/src/rag/retriever.rs"]),
            item(102, &[], &["crates/autospec-core/src/dispatch_pipeline.rs"]),
            item(103, &[], &["crates/autospec-cli/src/main.rs"]),
            item(104, &[], &["crates/autospec-cli/src/commands/dispatch.rs"]),
            item(105, &[], &["crates/autospec-core/src/rag/scorer.rs"]),
        ]),
        &surface(),
        &LanePolicy::default(),
        &ImprovementLedger::new(),
        NOW,
    );
    assert_eq!(plan.lane_slots, 2);
    assert_eq!(lane_issues(&plan), vec![102, 104]);
    // Payload keeps its worklist order; nothing is re-shuffled but nothing
    // waits behind the lane either.
    assert_eq!(queue_issues(&plan), vec![101, 103, 105]);
    assert!(plan.deferred.is_empty());
    assert_eq!(plan.len(), 5);
}

#[test]
fn an_empty_worklist_schedules_nothing() {
    let plan = schedule(
        &worklist(&[]),
        &surface(),
        &LanePolicy::default(),
        &ImprovementLedger::new(),
        NOW,
    );
    assert!(plan.is_empty());
    assert_eq!(plan.lane_slots, 0);
    assert_eq!(plan.lane_remaining, 0);
}

#[test]
fn a_lanes_only_worklist_still_leaves_the_reserve_alone() {
    // Five mechanism entries, capacity 2, reserve 1: the lane takes 2 of the
    // 5 slots it is allowed, and the other three ride the normal queue rather
    // than waiting for a fresh window.
    let plan = schedule(
        &worklist(&mechanism_items(&[201, 202, 203, 204, 205])),
        &surface(),
        &LanePolicy::default(),
        &ImprovementLedger::new(),
        NOW,
    );
    assert_eq!(lane_issues(&plan), vec![201, 202]);
    assert_eq!(queue_issues(&plan), vec![203, 204, 205]);
    assert_eq!(plan.deferred, vec![203, 204, 205]);
    assert_eq!(plan.lane_remaining, 0);
}

// ── AC2: the reservation is stated policy and bounded ────────────────────

#[test]
fn the_lane_never_takes_the_payload_reserve() {
    let policy = LanePolicy::default();
    // A batch of 1 with a reserve of 1 leaves the lane nothing.
    assert_eq!(policy.lane_slots(0), 0);
    assert_eq!(policy.lane_slots(1), 0);
    assert_eq!(policy.lane_slots(2), 1);
    assert_eq!(policy.lane_slots(3), 2);
    // ... and it is capped however deep the batch gets.
    assert_eq!(policy.lane_slots(10_000), 2);
}

#[test]
fn a_batch_of_one_ships_payload_not_the_lane() {
    // The reserve doing its job: with one slot available the mechanism entry
    // is reported as deferred instead of taking the only slot.
    let plan = schedule(
        &worklist(&[item(
            301,
            &[],
            &["crates/autospec-core/src/dispatch_pipeline.rs"],
        )]),
        &surface(),
        &LanePolicy::default(),
        &ImprovementLedger::new(),
        NOW,
    );
    assert_eq!(plan.lane_slots, 0);
    assert!(plan.lane.is_empty());
    assert_eq!(queue_issues(&plan), vec![301]);
    assert_eq!(plan.deferred, vec![301]);
    assert!(plan
        .lines()
        .iter()
        .any(|line| line.contains("not starved, not lost")));
}

#[test]
fn an_unbounded_or_total_reservation_is_rejected() {
    assert!(LanePolicy::default().validate().is_ok());
    assert!(LanePolicy::new(0, 1, HOUR).validate().is_err());
    assert!(LanePolicy::new(2, 0, HOUR).validate().is_err());
    assert!(LanePolicy::new(2, 1, 0).validate().is_err());
    // The message says why, because the failure mode is invisible otherwise.
    let message = LanePolicy::new(2, 0, HOUR).validate().unwrap_err();
    assert!(message.contains("inverts the queue"));
}

#[test]
fn the_window_already_spent_is_taken_off_the_lane() {
    // The lane is a budget per window, not per tick: work that landed this
    // hour has already spent part of it.
    let mut ledger = ImprovementLedger::new();
    ledger.record(900, LandedClass::Mechanism, NOW - 60);
    let plan = schedule(
        &worklist(&mechanism_items(&[401, 402, 403])),
        &surface(),
        &LanePolicy::default(),
        &ledger,
        NOW,
    );
    assert_eq!(plan.lane_slots, 2);
    assert_eq!(lane_issues(&plan), vec![401]);
    assert_eq!(plan.lane_remaining, 0);
    assert_eq!(queue_issues(&plan), vec![402, 403]);
}

#[test]
fn the_policy_is_stated_in_the_report() {
    let policy = LanePolicy::default();
    let line = policy.line();
    assert!(line.contains("up to 2 slot(s) per batch"));
    assert!(line.contains("1 reserved for payload"));
    assert!(line.contains("3600s"));
    assert!(line.contains("2 mechanism change(s)/hour reserved"));
}

// ── AC3: the mechanism is gated by its own tests ─────────────────────────

#[test]
fn a_pipeline_change_runs_the_pipelines_fixture_tests() {
    let classification = Classification::classify(
        &surface(),
        Vec::<String>::new(),
        ["crates/autospec-core/src/dispatch_pipeline.rs"],
    );
    let gate = gate_set(&surface(), &classification);
    assert!(gate.is_fixture());
    assert_eq!(gate.name, "mechanism-fixture");
    assert!(gate
        .commands
        .iter()
        .any(|command| command == "cargo test -p autospec-core --test dispatch_pipeline"));
    assert_eq!(gate.budget_secs, FIXTURE_GATE_BUDGET_SECS);
    assert!(gate.reason.contains("pipeline"));
}

#[test]
fn the_fixture_gate_is_seconds_not_the_candidate_gate() {
    let mechanism = Classification::classify(
        &surface(),
        Vec::<String>::new(),
        ["crates/autospec-cli/src/commands/dispatch.rs"],
    );
    let payload = Classification::classify(
        &surface(),
        Vec::<String>::new(),
        ["crates/autospec-core/src/rag/retriever.rs"],
    );
    let mechanism_gate = gate_set(&surface(), &mechanism);
    let payload_gate = gate_set(&surface(), &payload);
    assert_eq!(payload_gate.budget_secs, FULL_GATE_BUDGET_SECS);
    assert!(!payload_gate.is_fixture());
    // Mechanism work waits at least an order of magnitude less at the gate.
    assert!(mechanism_gate.budget_secs * 5 <= payload_gate.budget_secs);
}

#[test]
fn mechanism_work_that_names_no_fixture_runs_the_full_gate() {
    // A fixture-less component must not buy a free pass: the lane is a
    // scheduling decision, a gate is evidence, and evidence that cannot
    // answer is never counted as a pass.
    let fixtureless = MechanismSurface::new([MechanismComponent {
        name: "agent-runner".to_string(),
        prefixes: vec!["scripts/run-agent.sh".to_string()],
        fixture_commands: Vec::new(),
    }]);
    let classification =
        Classification::classify(&fixtureless, Vec::<String>::new(), ["scripts/run-agent.sh"]);
    assert!(classification.is_mechanism());
    let gate = gate_set(&fixtureless, &classification);
    assert_eq!(gate.name, "full-candidate");
    assert!(gate.reason.contains("naming no fixture"));

    // The same for a label with no declared component behind it.
    let repo = MechanismSurface::repository();
    let label_only = Classification::classify(&repo, [MECHANISM_LABEL], ["docs/notes.md"]);
    let gate = gate_set(&repo, &label_only);
    assert_eq!(gate.name, "full-candidate");
    assert!(gate.reason.contains("no declared component matched"));
}

#[test]
fn a_gate_that_cannot_answer_never_takes_a_reserved_slot() {
    // The fixture-less mechanism entry goes to the normal queue with the full
    // gate; the reservation is not spent on a change that still waits on it.
    let surface = MechanismSurface::new([MechanismComponent {
        name: "pipeline".to_string(),
        prefixes: vec!["crates/autospec-core/src/dispatch_pipeline.rs".to_string()],
        fixture_commands: Vec::new(),
    }]);
    let plan = schedule(
        &worklist(&[
            item(501, &[], &["crates/autospec-core/src/dispatch_pipeline.rs"]),
            item(502, &[], &["crates/autospec-core/src/rag/retriever.rs"]),
            item(503, &[], &["crates/autospec-core/src/rag/scorer.rs"]),
        ]),
        &surface,
        &LanePolicy::default(),
        &ImprovementLedger::new(),
        NOW,
    );
    assert!(plan.lane.is_empty());
    assert_eq!(plan.deferred, vec![501]);
    assert_eq!(plan.queue[0].gate.name, "full-candidate");
}

// ── AC4: the improvement rate ────────────────────────────────────────────

#[test]
fn the_rate_is_mechanism_changes_per_unit_time() {
    let mut ledger = ImprovementLedger::new();
    ledger.record(601, LandedClass::Mechanism, NOW - 300);
    ledger.record(602, LandedClass::Mechanism, NOW - 120);
    ledger.record(603, LandedClass::Payload, NOW - 60);
    let rate = ledger.improvement_rate(&LanePolicy::default(), 9, NOW);
    assert_eq!(rate.mechanism_landed, 2);
    assert_eq!(rate.payload_landed, 1);
    assert_eq!(rate.per_hour, 2.0);
    assert_eq!(rate.reservation_per_hour, 2.0);
    assert_eq!(rate.verdict, ImprovementVerdict::Improving);
    assert!(!rate.verdict.is_fault());
    assert!(ledger
        .improvement_rate(&LanePolicy::default(), 9, NOW)
        .line()
        .contains("2 mechanism change(s) in 3600s"));
}

#[test]
fn a_backlog_with_no_mechanism_work_landed_is_a_fixed_point() {
    // The #3795 state: work waiting, including work that would fix the
    // pipeline, and nothing mechanism-side landed in the window. Payload still
    // ships, which is exactly why the queue never notices.
    let mut ledger = ImprovementLedger::new();
    for issue in 700..709 {
        ledger.record(issue, LandedClass::Payload, NOW - 30);
    }
    let rate = ledger.improvement_rate(&LanePolicy::default(), 9, NOW);
    assert_eq!(rate.mechanism_landed, 0);
    assert_eq!(rate.payload_landed, 9);
    assert_eq!(rate.per_hour, 0.0);
    assert_eq!(rate.verdict, ImprovementVerdict::FixedPoint);
    assert!(rate.verdict.is_fault());
    assert!(rate.line().contains("fixed-point"));
}

#[test]
fn an_empty_backlog_is_not_a_fixed_point() {
    let rate = ImprovementLedger::new().improvement_rate(&LanePolicy::default(), 0, NOW);
    assert_eq!(rate.verdict, ImprovementVerdict::NoBacklog);
    assert!(!rate.verdict.is_fault());
}

#[test]
fn a_reservation_that_is_not_used_is_its_own_verdict() {
    // One landed of two reserved over a deep backlog: the lane exists, and
    // something other than its cap is holding mechanism work back.
    let mut ledger = ImprovementLedger::new();
    ledger.record(801, LandedClass::Mechanism, NOW - 30);
    let rate = ledger.improvement_rate(&LanePolicy::new(2, 1, HOUR), 40, NOW);
    assert_eq!(rate.verdict, ImprovementVerdict::ReservationUnused);
    assert!(!rate.verdict.is_fault());
}

#[test]
fn work_outside_the_window_is_not_counted() {
    let mut ledger = ImprovementLedger::new();
    ledger.record(851, LandedClass::Mechanism, NOW - HOUR);
    let rate = ledger.improvement_rate(&LanePolicy::default(), 3, NOW);
    assert_eq!(rate.mechanism_landed, 0);
    assert_eq!(rate.verdict, ImprovementVerdict::FixedPoint);
    // One second inside the window, and the verdict changes.
    let mut inside = ImprovementLedger::new();
    inside.record(852, LandedClass::Mechanism, NOW - HOUR + 1);
    assert_eq!(
        inside
            .improvement_rate(&LanePolicy::default(), 3, NOW)
            .mechanism_landed,
        1
    );
}

#[test]
fn a_record_from_the_future_does_not_inflate_the_rate() {
    let mut ledger = ImprovementLedger::new();
    ledger.record(861, LandedClass::Mechanism, NOW + 10 * HOUR);
    assert_eq!(
        ledger
            .improvement_rate(&LanePolicy::default(), 3, NOW)
            .mechanism_landed,
        0
    );
}

#[test]
fn an_issue_lands_once_and_cannot_be_reclassified() {
    let mut ledger = ImprovementLedger::new();
    assert!(ledger.record(871, LandedClass::Mechanism, NOW - 10));
    // Re-recording the same issue — as the other class, to move the number —
    // is refused; the first record stands.
    assert!(!ledger.record(871, LandedClass::Payload, NOW - 5));
    assert_eq!(ledger.len(), 1);
    assert_eq!(ledger.class_of(871), Some(LandedClass::Mechanism));
    assert_eq!(ledger.class_of(872), None);
}

#[test]
fn the_ledger_survives_a_json_round_trip() {
    let mut ledger = ImprovementLedger::new();
    ledger.record(881, LandedClass::Mechanism, NOW - 30);
    ledger.record(882, LandedClass::Payload, NOW - 20);
    let json = ledger.to_json();
    let back = ImprovementLedger::from_json(&json).expect("ledger parses");
    assert_eq!(back, ledger);
    assert_eq!(back.records()[0].issue, 881);
    assert!(ImprovementLedger::from_json("not json").is_err());
}

#[test]
fn the_ledger_is_kept_in_landing_order() {
    let mut ledger = ImprovementLedger::new();
    ledger.record(891, LandedClass::Payload, NOW - 5);
    ledger.record(892, LandedClass::Mechanism, NOW - 500);
    ledger.record(893, LandedClass::Mechanism, NOW - 100);
    assert_eq!(
        ledger
            .records()
            .iter()
            .map(|record| record.issue)
            .collect::<Vec<_>>(),
        vec![892, 893, 891]
    );
}

// ── The declared surface and the worklist format ─────────────────────────

#[test]
fn the_repository_declares_its_mechanism_surface() {
    // The lane has to be declared against something: an empty surface makes
    // every change payload and restores the fixed point silently.
    let surface = surface();
    let names: Vec<&str> = surface
        .components()
        .iter()
        .map(|component| component.name.as_str())
        .collect();
    assert_eq!(
        names,
        vec!["pipeline", "dispatcher", "gate", "agent-runner"]
    );
    for component in surface.components() {
        assert!(
            !component.prefixes.is_empty(),
            "{} declares no paths",
            component.name
        );
        assert!(
            !component.fixture_commands.is_empty(),
            "{} declares no fixture tests",
            component.name
        );
    }
}

#[test]
fn a_directory_prefix_owns_its_descendants_and_a_file_prefix_does_not() {
    let surface = surface();
    assert!(surface
        .components_for("crates/autospec-core/src/validation/catalog.rs")
        .iter()
        .any(|component| component.name == "gate"));
    // A file prefix claims the file, not a sibling of the same name prefix.
    assert!(surface
        .components_for("crates/autospec-core/src/conversion_gate_helpers.rs")
        .is_empty());
    // `./` and backslashes normalise away; the classifier sees repo paths.
    assert!(!surface
        .components_for("./crates/autospec-core/src/dispatch_pipeline.rs")
        .is_empty());
}

#[test]
fn the_worklist_reads_a_manifest_and_names_what_it_cannot_read() {
    let worklist = Worklist::parse(
        "# issue\tlabels\tpaths\n\
         901\tautomation:auto-implement\tcrates/autospec-core/src/dispatch_pipeline.rs\n\
         902\t\tcrates/autospec-core/src/rag/retriever.rs,crates/autospec-core/src/rag/scorer.rs\n\
         903\n\
         not-an-issue\t\t\n",
    );
    assert_eq!(worklist.len(), 3);
    assert_eq!(worklist.items[0].labels, vec!["automation:auto-implement"]);
    assert_eq!(worklist.items[0].paths.len(), 1);
    assert_eq!(worklist.items[1].labels, Vec::<String>::new());
    assert_eq!(worklist.items[1].paths.len(), 2);
    assert_eq!(worklist.items[2], WorkItem::new(903));
    // A line the scheduler cannot read is reported, never dropped: silently
    // losing a candidate is the #3927 failure in a smaller hat.
    assert_eq!(worklist.malformed.len(), 1);
    assert!(worklist.malformed[0].starts_with("5: "));
}

#[test]
fn the_worklist_renders_back_to_the_manifest_format() {
    let worklist = worklist(&[item(
        911,
        &["delivery-mechanism"],
        &["crates/autospec-core/src/dispatch_pipeline.rs"],
    )]);
    let text = worklist.render();
    assert_eq!(
        text,
        "911\tdelivery-mechanism\tcrates/autospec-core/src/dispatch_pipeline.rs\n"
    );
    assert_eq!(Worklist::parse(&text), worklist);
}

#[test]
fn a_reported_plan_names_the_gate_next_to_every_entry() {
    let plan = schedule(
        &worklist(&[
            item(921, &[], &["crates/autospec-core/src/dispatch_pipeline.rs"]),
            item(922, &[], &["crates/autospec-core/src/rag/retriever.rs"]),
            item(923, &[], &["crates/autospec-cli/src/commands/dispatch.rs"]),
            item(924, &[], &["crates/autospec-core/src/rag/scorer.rs"]),
        ]),
        &surface(),
        &LanePolicy::default(),
        &ImprovementLedger::new(),
        NOW,
    );
    let lines = plan.lines().join("\n");
    assert!(lines.contains("lane: 2/2 slot(s) used, 0 remaining; queue: 2 entry(s)"));
    assert!(lines.contains("lane  #921 mechanism [pipeline] — gate mechanism-fixture"));
    assert!(lines.contains("queue #922 payload — gate full-candidate"));
    assert_eq!(plan.scheduled().len(), 4);
}

// ── helpers ──────────────────────────────────────────────────────────────

fn mechanism_items(issues: &[u64]) -> Vec<WorkItem> {
    issues
        .iter()
        .map(|issue| {
            item(
                *issue,
                &[],
                &["crates/autospec-core/src/dispatch_pipeline.rs"],
            )
        })
        .collect()
}

fn lane_issues(plan: &LanePlan) -> Vec<u64> {
    plan.lane.iter().map(|entry| entry.issue).collect()
}

fn queue_issues(plan: &LanePlan) -> Vec<u64> {
    plan.queue.iter().map(|entry| entry.issue).collect()
}
