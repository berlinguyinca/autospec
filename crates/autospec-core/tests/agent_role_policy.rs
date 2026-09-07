//! Issue #3318: provider-neutral role tool policies, the Pi `--tools`
//! mapping, and bounded task capsules.

mod agent_role_policy {
    use autospec_core::aar::capsule::{
        RolePolicy, TaskCapsule, MAX_GOAL_CHARS, MAX_SECTION_ITEMS, MAX_SECTION_ITEM_CHARS,
    };
    use autospec_core::aar::pi::{build_pi_argv, pi_tools_for, PiSessionSpec};
    use autospec_core::aar::reasoning::SamplingProfile;
    use autospec_core::aar::topology::AgentRole;

    /// Pinned Pi `--tools` snapshot for every role policy.
    const TOOLS_SNAPSHOT: [(RolePolicy, &str); 6] = [
        (RolePolicy::Planner, "read,grep,glob"),
        (RolePolicy::Scout, "read,grep,glob"),
        (RolePolicy::Test, "read,grep,glob,bash"),
        (RolePolicy::Reviewer, "read,grep,glob"),
        (RolePolicy::Builder, "read,grep,glob,edit,write,bash"),
        (RolePolicy::Escalation, "read,grep,glob,bash"),
    ];

    fn spec_for(policy: RolePolicy) -> PiSessionSpec {
        let role = match policy {
            RolePolicy::Planner => AgentRole::Planner,
            RolePolicy::Scout => AgentRole::Explorer,
            RolePolicy::Test => AgentRole::Tester,
            RolePolicy::Reviewer => AgentRole::Reviewer,
            RolePolicy::Builder => AgentRole::Implementer,
            RolePolicy::Escalation => AgentRole::Coordinator,
        };
        PiSessionSpec {
            session_id: "session-1".to_string(),
            worktree: "/work/autospec".to_string(),
            role,
            policy,
            provider: "inferweave".to_string(),
            model: "qwen3.8-27b".to_string(),
            reasoning_tokens: 1_024,
            sampling: SamplingProfile::qwen_thinking(),
            extra_rules: Vec::new(),
            stable_prefix_hash: String::new(),
            max_context_tokens: 32_768,
            allow_forks: false,
        }
    }

    /// AC: planner and reviewer policies omit `edit` and `write`.
    #[test]
    fn planner_and_reviewer_policies_omit_edit_and_write() {
        for policy in [RolePolicy::Planner, RolePolicy::Reviewer] {
            let tools = policy.tools();
            assert!(!tools.contains(&"edit"), "{policy:?} must omit edit");
            assert!(!tools.contains(&"write"), "{policy:?} must omit write");
            assert!(policy.is_read_only(), "{policy:?} must be read-only");
        }
    }

    /// The issue names all four inspection policies read-only.
    #[test]
    fn planner_scout_test_and_reviewer_policies_are_read_only() {
        for policy in [
            RolePolicy::Planner,
            RolePolicy::Scout,
            RolePolicy::Test,
            RolePolicy::Reviewer,
        ] {
            assert!(policy.is_read_only(), "{policy:?} must be read-only");
        }
    }

    /// AC: builder policy includes `edit` and `bash`.
    #[test]
    fn builder_policy_includes_edit_and_bash() {
        let tools = RolePolicy::Builder.tools();
        assert!(tools.contains(&"edit"), "builder must include edit");
        assert!(tools.contains(&"bash"), "builder must include bash");
        assert!(!RolePolicy::Builder.is_read_only(), "builder mutates");
    }

    /// Pi argv snapshot for every role (issue verification step).
    #[test]
    fn pi_argv_is_snapped_for_every_role_policy() {
        for (policy, expected_tools) in TOOLS_SNAPSHOT {
            let argv = build_pi_argv(&spec_for(policy)).expect("spec is valid");
            assert_eq!(argv[0], "pi");
            let index = argv
                .iter()
                .position(|entry| entry == "--tools")
                .unwrap_or_else(|| panic!("--tools missing for {policy:?} in {argv:?}"));
            assert_eq!(
                argv[index + 1],
                expected_tools,
                "--tools value for {policy:?}"
            );
            // The neutral policy maps to exactly the argv snapshot.
            assert_eq!(pi_tools_for(policy), expected_tools);
        }
    }

    /// AC: reviewer capsules omit the `builder_reasoning` transcript.
    #[test]
    fn reviewer_capsule_omits_the_builder_reasoning_transcript() {
        let builder = TaskCapsule::new(
            AgentRole::Implementer,
            "Fix the parser panic on CRLF input",
        )
        .with_acceptance_criteria(["cargo test -p autospec-core parser"])
        .with_constraints(["no new dependencies"])
        .with_relevant_files(["crates/autospec-core/src/state/json.rs"])
        .with_tests(["cargo test -p autospec-core"])
        .with_non_goals(["repository indexing"])
        .with_builder_reasoning(
            "I reproduced the panic, then bisected to the quote handler, then chose the fix.",
        );
        builder.validate().expect("builder capsule is bounded");
        assert!(builder.builder_reasoning.is_some());
        assert!(builder.render().contains("## Builder reasoning"));

        let reviewer = builder.for_reviewer();

        assert!(reviewer.builder_reasoning.is_none());
        assert!(!reviewer.render().contains("Builder reasoning"));
        assert_eq!(reviewer.role, AgentRole::Reviewer);
        // Scope survives the handoff; only the transcript is removed.
        assert_eq!(reviewer.acceptance_criteria, builder.acceptance_criteria);
        assert_eq!(reviewer.constraints, builder.constraints);
        assert_eq!(reviewer.relevant_files, builder.relevant_files);
        assert_eq!(reviewer.tests, builder.tests);
        assert_eq!(reviewer.non_goals, builder.non_goals);
        reviewer.validate().expect("reviewer capsule is bounded");
    }

    /// Capsules are bounded: over-long or over-full sections are rejected.
    #[test]
    fn capsule_validation_enforces_the_bounds() {
        let ok = TaskCapsule::new(AgentRole::Reviewer, "review the diff");
        ok.validate().expect("a bounded capsule validates");

        assert!(TaskCapsule::new(AgentRole::Reviewer, "   ")
            .validate()
            .unwrap_err()
            .contains("goal"));

        let long_goal = "x".repeat(MAX_GOAL_CHARS + 1);
        assert!(TaskCapsule::new(AgentRole::Reviewer, long_goal)
            .validate()
            .unwrap_err()
            .contains("goal"));

        let too_many = vec!["item".to_string(); MAX_SECTION_ITEMS + 1];
        assert!(TaskCapsule::new(AgentRole::Reviewer, "goal")
            .with_acceptance_criteria(too_many)
            .validate()
            .unwrap_err()
            .contains("acceptance criteria"));

        let too_long = vec!["x".repeat(MAX_SECTION_ITEM_CHARS + 1)];
        assert!(TaskCapsule::new(AgentRole::Reviewer, "goal")
            .with_constraints(too_long)
            .validate()
            .unwrap_err()
            .contains("constraints"));

        assert!(TaskCapsule::new(AgentRole::Reviewer, "goal")
            .with_non_goals(["   "])
            .validate()
            .unwrap_err()
            .contains("non-goals"));
    }

    /// The rendered capsule carries every section, in order.
    #[test]
    fn rendered_capsule_contains_every_section_in_order() {
        let capsule = TaskCapsule::new(AgentRole::Planner, "plan the rollout")
            .with_acceptance_criteria(["plan lists each PR"])
            .with_constraints(["stay on the feat branch"])
            .with_relevant_files(["crates/autospec-core/src/aar/"])
            .with_tests(["cargo test -p autospec-core"])
            .with_non_goals(["telemetry"]);

        capsule.validate().expect("bounded capsule validates");
        let rendered = capsule.render();

        let mut positions = Vec::new();
        for section in [
            "## Goal",
            "## Acceptance criteria",
            "## Constraints",
            "## Relevant files",
            "## Tests",
            "## Non-goals",
        ] {
            positions.push(
                rendered
                    .find(section)
                    .unwrap_or_else(|| panic!("{section} missing from rendered capsule")),
            );
        }
        assert!(
            positions.windows(2).all(|pair| pair[0] < pair[1]),
            "sections out of order: {rendered}"
        );
        assert!(!rendered.contains("Builder reasoning"));
    }

    /// Every dispatchable role resolves to exactly one policy.
    #[test]
    fn every_dispatchable_role_maps_to_one_policy() {
        assert_eq!(
            RolePolicy::for_role(AgentRole::Planner),
            RolePolicy::Planner
        );
        assert_eq!(RolePolicy::for_role(AgentRole::Explorer), RolePolicy::Scout);
        assert_eq!(RolePolicy::for_role(AgentRole::Tester), RolePolicy::Test);
        assert_eq!(
            RolePolicy::for_role(AgentRole::Reviewer),
            RolePolicy::Reviewer
        );
        assert_eq!(
            RolePolicy::for_role(AgentRole::Implementer),
            RolePolicy::Builder
        );
        assert_eq!(
            RolePolicy::for_role(AgentRole::Coordinator),
            RolePolicy::Escalation
        );

        // The builder is the only policy a producer role may hold.
        for role in [
            AgentRole::Planner,
            AgentRole::Explorer,
            AgentRole::Tester,
            AgentRole::Reviewer,
            AgentRole::UiEvaluator,
            AgentRole::SecurityReviewer,
            AgentRole::PerformanceReviewer,
            AgentRole::Coordinator,
        ] {
            assert!(RolePolicy::for_role(role).is_read_only(), "{role:?}");
        }
        for role in [AgentRole::Implementer, AgentRole::DocumentationWriter] {
            assert!(
                !RolePolicy::for_role(role).is_read_only(),
                "{role:?} is a producer"
            );
        }
    }
}
