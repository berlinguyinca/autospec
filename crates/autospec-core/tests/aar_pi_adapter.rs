//! AAR spec section 9: the Pi harness adapter boundary.

use autospec_core::aar::pi::{
    build_pi_argv, fold_events, parse_pi_event, pi_current_dir, thinking_level_for,
    with_skills, working_rules_block, PiEvent, PiSessionSpec,
    WORKING_RULES,
};
use autospec_core::aar::reasoning::SamplingProfile;
use autospec_core::aar::topology::AgentRole;

fn spec() -> PiSessionSpec {
    PiSessionSpec {
        session_id: "session-1".to_string(),
        worktree: "/work/autospec".to_string(),
        role: AgentRole::Implementer,
        provider: "inferweave".to_string(),
        model: "qwen3.8-27b".to_string(),
        reasoning_tokens: 2_048,
        sampling: SamplingProfile::qwen_thinking(),
        extra_rules: Vec::new(),
        stable_prefix_hash: "abc123".to_string(),
        max_context_tokens: 65_536,
        allow_forks: true,
    }
}

/// The rules block is pinned verbatim: paraphrasing it changes behaviour, and
/// the spec states the exact wording.
#[test]
fn the_working_rules_match_the_specification_text() {
    let expected = "\
Understand relevant code before editing.
Prefer small controlled edits over rewrites.
Make one logical change at a time.
Re-read affected code after meaningful edits.
Do not invent requirements or perform unrelated refactors.
For bugs: form the simplest plausible hypothesis, gather targeted evidence,
implement the smallest supported fix, test it, and stop.
Do not repeatedly re-prove an established conclusion unless new evidence contradicts it.
When acceptance criteria are satisfied, STOP.";

    assert_eq!(working_rules_block(), expected);
    assert_eq!(WORKING_RULES.len(), 9);
}

#[test]
fn a_session_appends_extra_rules_after_the_standard_block() {
    let session = PiSessionSpec {
        extra_rules: vec!["Never touch migrations in this task.".to_string()],
        ..spec()
    };

    let rules = session.rules();

    assert_eq!(rules.len(), WORKING_RULES.len() + 1);
    assert_eq!(rules.last().map(String::as_str), Some("Never touch migrations in this task."));
}

#[test]
fn argv_uses_only_flags_a_released_pi_accepts() {
    let argv = build_pi_argv(&spec()).expect("spec is valid");

    assert_eq!(argv[0], "pi");
    // Non-interactive, or the harness waits forever on a prompt.
    assert!(argv.contains(&"--print".to_string()), "argv: {argv:?}");

    for (flag, value) in [
        ("--session-id", "session-1"),
        ("--model", "qwen3.8-27b"),
        ("--provider", "inferweave"),
    ] {
        let index = argv
            .iter()
            .position(|entry| entry == flag)
            .unwrap_or_else(|| panic!("{flag} missing from {argv:?}"));
        assert_eq!(argv[index + 1], value, "{flag} value");
    }
}

#[test]
fn argv_never_emits_a_flag_pi_would_reject() {
    let argv = build_pi_argv(&spec()).expect("spec is valid");

    // Every one of these was emitted by the previous implementation and is
    // rejected by pi 0.85.0. `session` is the worst of them: it is NOT rejected,
    // it is swallowed as a positional prompt, silently contaminating the real
    // prompt with the word "session".
    for rejected in [
        "session",
        "--worktree",
        "--role",
        "--reasoning-tokens",
        "--sampling-profile",
        "--temperature",
        "--top-p",
        "--top-k",
        "--max-output-tokens",
        "--max-context-tokens",
        "--prefix-cache-key",
        "--no-forks",
    ] {
        assert!(
            !argv.iter().any(|entry| entry == rejected),
            "argv still contains {rejected}: {argv:?}"
        );
    }
}

#[test]
fn worktree_is_a_working_directory_not_a_flag() {
    let s = spec();
    assert_eq!(pi_current_dir(&s), "/work/autospec");
    assert!(!build_pi_argv(&s)
        .expect("spec is valid")
        .contains(&"/work/autospec".to_string()));
}

#[test]
fn reasoning_budget_maps_onto_a_thinking_level() {
    // Pi takes a named level, not a token count. Biased low on purpose: an agent
    // that spends its whole per-turn budget thinking emits no tool call, and the
    // harness then has nothing to execute.
    assert_eq!(thinking_level_for(0), "off");
    assert_eq!(thinking_level_for(512), "minimal");
    assert_eq!(thinking_level_for(2048), "low");
    assert_eq!(thinking_level_for(8192), "medium");
    assert_eq!(thinking_level_for(100_000), "high");

    let argv = build_pi_argv(&spec()).expect("spec is valid");
    let index = argv
        .iter()
        .position(|entry| entry == "--thinking")
        .expect("--thinking missing");
    assert_eq!(argv[index + 1], "low", "2048 reasoning tokens maps to low");
}

#[test]
fn skills_are_appended_as_repeated_skill_flags() {
    let argv = with_skills(
        build_pi_argv(&spec()).expect("spec is valid"),
        &[
            "/home/u/.agents/skills/autospec".to_string(),
            String::new(), // empty entries are skipped, not passed as ""
            "/opt/skills/extra".to_string(),
        ],
    );
    let count = argv.iter().filter(|e| *e == "--skill").count();
    assert_eq!(count, 2, "argv: {argv:?}");
    assert!(argv.contains(&"/home/u/.agents/skills/autospec".to_string()));
}

#[test]
fn an_invalid_session_spec_is_rejected_before_launch() {
    for (session, expected) in [
        (
            PiSessionSpec {
                model: String::new(),
                ..spec()
            },
            "model",
        ),
        (
            PiSessionSpec {
                worktree: String::new(),
                ..spec()
            },
            "worktree",
        ),
        (
            PiSessionSpec {
                max_context_tokens: 0,
                ..spec()
            },
            "context ceiling",
        ),
    ] {
        let error = build_pi_argv(&session).unwrap_err();
        assert!(error.contains(expected), "{error} should mention {expected}");
    }
}

#[test]
fn events_parse_from_the_wire_format() {
    assert_eq!(
        parse_pi_event("tool_call name=read").expect("parses"),
        PiEvent::ToolCall {
            name: "read".to_string()
        }
    );
    assert_eq!(
        parse_pi_event("file_edit path=src/a.rs lines=42").expect("parses"),
        PiEvent::FileEdit {
            path: "src/a.rs".to_string(),
            lines: 42
        }
    );
    assert_eq!(
        parse_pi_event("context_measurement prompt_tokens=1000 cached_tokens=800 free_tokens=64000")
            .expect("parses"),
        PiEvent::ContextMeasurement {
            prompt_tokens: 1_000,
            cached_tokens: 800,
            free_tokens: 64_000
        }
    );
}

#[test]
fn quoted_values_preserve_spaces() {
    let event = parse_pi_event(r#"result success=true summary="fixed the parser panic""#)
        .expect("parses");

    assert_eq!(
        event,
        PiEvent::Result {
            success: true,
            summary: "fixed the parser panic".to_string()
        }
    );
}

/// An unknown field means the harness changed. Failing loudly beats silently
/// dropping telemetry that the optimizer depends on.
#[test]
fn an_unknown_field_or_kind_is_an_error() {
    assert!(parse_pi_event("tool_call name=read extra=1")
        .unwrap_err()
        .contains("unknown field extra"));
    assert!(parse_pi_event("teleport path=x")
        .unwrap_err()
        .contains("unknown pi event kind"));
}

#[test]
fn a_missing_required_field_is_an_error() {
    assert!(parse_pi_event("file_edit path=src/a.rs")
        .unwrap_err()
        .contains("missing field lines"));
}

#[test]
fn a_non_numeric_token_count_is_an_error() {
    assert!(parse_pi_event("file_edit path=src/a.rs lines=many")
        .unwrap_err()
        .contains("not a number"));
}

#[test]
fn an_event_stream_folds_into_a_structured_result() {
    let events = vec![
        PiEvent::ToolCall {
            name: "read".to_string(),
        },
        PiEvent::FileRead {
            path: "src/a.rs".to_string(),
        },
        PiEvent::FileRead {
            path: "src/a.rs".to_string(),
        },
        PiEvent::ToolCall {
            name: "edit".to_string(),
        },
        PiEvent::FileEdit {
            path: "src/a.rs".to_string(),
            lines: 12,
        },
        PiEvent::ContextMeasurement {
            prompt_tokens: 5_000,
            cached_tokens: 4_000,
            free_tokens: 50_000,
        },
        PiEvent::Fork {
            child_session_id: "session-1-fork".to_string(),
        },
        PiEvent::Result {
            success: true,
            summary: "done".to_string(),
        },
    ];

    let result = fold_events("session-1", AgentRole::Implementer, &events);

    assert!(result.success);
    assert_eq!(result.tool_calls, 2);
    assert_eq!(result.files_read, vec!["src/a.rs"], "reads are deduplicated");
    assert_eq!(result.files_edited, vec!["src/a.rs"]);
    assert_eq!(result.lines_changed, 12);
    assert_eq!(result.prompt_tokens, 5_000);
    assert_eq!(result.cached_prompt_tokens, 4_000);
    assert_eq!(result.forks, vec!["session-1-fork"]);
    assert!(result.errors.is_empty());
}

#[test]
fn errors_in_the_stream_are_collected_without_masking_the_result() {
    let events = vec![
        PiEvent::Error {
            message: "tool timed out".to_string(),
        },
        PiEvent::Result {
            success: false,
            summary: "gave up".to_string(),
        },
    ];

    let result = fold_events("session-1", AgentRole::Implementer, &events);

    assert!(!result.success);
    assert_eq!(result.errors, vec!["tool timed out"]);
}
