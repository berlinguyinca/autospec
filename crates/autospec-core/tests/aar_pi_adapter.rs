//! AAR spec section 9: the Pi harness adapter boundary.

use autospec_core::aar::pi::{
    build_pi_argv, fold_events, parse_pi_event, role_tools, thinking_level, working_rules_block,
    PiEvent, PiSessionSpec, WORKING_RULES,
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
    assert_eq!(
        rules.last().map(String::as_str),
        Some("Never touch migrations in this task.")
    );
}

/// The argv must stay on the released headless surface: no `session`
/// subcommand (Pi has none, and it would be swallowed as a positional
/// message), and no flag the CLI rejects outright.
#[test]
fn argv_uses_only_flags_a_released_pi_accepts() {
    let argv = build_pi_argv(&spec()).expect("spec is valid");

    assert_eq!(argv[0], "pi");
    for (flag, value) in [
        ("--mode", "json"),
        ("--session-id", "session-1"),
        ("--provider", "inferweave"),
        ("--model", "qwen3.8-27b"),
        // 2048 tokens bands onto `medium`.
        ("--thinking", "medium"),
        ("--tools", "read,edit,write,bash"),
    ] {
        let index = argv
            .iter()
            .position(|entry| entry == flag)
            .unwrap_or_else(|| panic!("{flag} missing from {argv:?}"));
        assert_eq!(argv[index + 1], value, "{flag} value");
    }
    assert!(
        argv.iter().any(|entry| entry == "--print"),
        "--print (headless) missing from {argv:?}"
    );
    let prompt_index = argv
        .iter()
        .position(|entry| entry == "--append-system-prompt")
        .expect("--append-system-prompt present");
    let prompt = &argv[prompt_index + 1];
    assert!(
        prompt.contains("Role: implementer"),
        "role mapped into prompt: {prompt}"
    );
    assert!(
        prompt.contains(working_rules_block().as_str()),
        "working rules mapped into prompt: {prompt}"
    );

    // Every flag the old argv built that released Pi rejects must be gone.
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
            "{rejected} must not be emitted: {argv:?}"
        );
    }
}

/// The token budget -> `--thinking` enum mapping is an explicit, documented
/// banding function, not a rename of a token-count flag.
#[test]
fn reasoning_tokens_band_onto_the_thinking_enum() {
    assert_eq!(thinking_level(0), "off");
    assert_eq!(thinking_level(1), "minimal");
    assert_eq!(thinking_level(512), "minimal");
    assert_eq!(thinking_level(513), "low");
    assert_eq!(thinking_level(1_024), "low");
    assert_eq!(thinking_level(1_025), "medium");
    assert_eq!(thinking_level(2_048), "medium");
    assert_eq!(thinking_level(2_049), "high");
    assert_eq!(thinking_level(4_096), "high");
    assert_eq!(thinking_level(4_097), "xhigh");
    assert_eq!(thinking_level(8_192), "xhigh");
    assert_eq!(thinking_level(65_536), "max");
}

/// Separation of duties as tool policy: producers can write, everyone else
/// can read and run but never edit or write.
#[test]
fn tool_policy_follows_separation_of_duties() {
    assert_eq!(role_tools(AgentRole::Implementer), "read,edit,write,bash");
    assert_eq!(
        role_tools(AgentRole::DocumentationWriter),
        "read,edit,write,bash"
    );
    for role in [
        AgentRole::Coordinator,
        AgentRole::Explorer,
        AgentRole::Planner,
        AgentRole::Tester,
        AgentRole::Reviewer,
        AgentRole::SecurityReviewer,
        AgentRole::PerformanceReviewer,
        AgentRole::UiEvaluator,
    ] {
        assert_eq!(role_tools(role), "read,bash", "role {role:?}");
    }
}

/// The worktree is the caller's job: it never appears as a `--worktree`
/// flag, and disabling forks changes nothing on the released surface.
#[test]
fn fields_without_a_cli_equivalent_do_not_leak_into_the_argv() {
    let base = build_pi_argv(&spec()).expect("spec is valid");
    let no_forks = build_pi_argv(&PiSessionSpec {
        allow_forks: false,
        ..spec()
    })
    .expect("spec is valid");
    assert_eq!(base, no_forks);

    let other_worktree = build_pi_argv(&PiSessionSpec {
        worktree: "/other/worktree".to_string(),
        ..spec()
    })
    .expect("spec is valid");
    assert!(
        !other_worktree
            .iter()
            .any(|entry| entry.contains("/other/worktree") && entry.starts_with("--")),
        "worktree must not become its own flag: {other_worktree:?}"
    );
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
        assert!(
            error.contains(expected),
            "{error} should mention {expected}"
        );
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
        parse_pi_event(
            "context_measurement prompt_tokens=1000 cached_tokens=800 free_tokens=64000"
        )
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
    let event =
        parse_pi_event(r#"result success=true summary="fixed the parser panic""#).expect("parses");

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
    assert_eq!(
        result.files_read,
        vec!["src/a.rs"],
        "reads are deduplicated"
    );
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

/// Locate the `pi` binary on PATH without assuming any particular install
/// path (no hardcoded locations, identical behaviour on Linux and macOS).
fn find_pi() -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join("pi");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Parse `pi --version` output like `0.85.0` into its major and minor parts.
fn parse_version(output: &std::process::Output) -> Option<(u32, u32)> {
    let text = String::from_utf8_lossy(&output.stdout);
    let digits: Vec<u32> = text
        .trim()
        .split('.')
        .take(2)
        .filter_map(|part| part.trim().parse().ok())
        .collect();
    Some((digits[0], digits[1]))
}

/// Real-CLI gate for the argv this adapter builds: spawn released Pi with the
/// exact flags `build_pi_argv` emits and fail if Pi rejects any of them.
///
/// A test that only re-states the argv cannot catch a flag Pi does not accept,
/// which is exactly how the `session` subcommand and the eleven rejected flags
/// shipped. Instead this spawns `pi --print` and checks that the failure it
/// reports (a bogus provider/model makes startup fail fast without a network
/// round trip) is not an unknown-option rejection.
///
/// Skip, do not mock, when unreachable (the convention #3173 set): Pi absent
/// from PATH, and Pi minor versions outside the set this surface was
/// validated against (0.84.4 as reported in the issue; 0.85.0 locally).
#[test]
fn a_released_pi_accepts_every_flag_the_adapter_builds() {
    let pi = match find_pi() {
        Some(path) => path,
        None => {
            eprintln!("skip: pi not found on PATH");
            return;
        }
    };

    let version_output = match std::process::Command::new(&pi).arg("--version").output() {
        Ok(output) => output,
        Err(error) => {
            eprintln!("skip: cannot run {pi:?} --version: {error}");
            return;
        }
    };
    let (major, minor) = match parse_version(&version_output) {
        Some(parsed) => parsed,
        None => {
            eprintln!("skip: unparseable pi version output");
            return;
        }
    };
    if major != 0 || !(84..=85).contains(&minor) {
        eprintln!(
            "skip: pi {major}.{minor} is outside the 0.84..0.85 surface this argv was validated against; re-validate before trusting the flags"
        );
        return;
    }

    // Same flags as a production spec, but a bogus provider/model so startup
    // fails immediately on model lookup instead of reaching a live endpoint.
    let spec = PiSessionSpec {
        provider: "autospec-smoke-nonexistent-provider".to_string(),
        model: "no-such-model".to_string(),
        ..spec()
    };
    let argv = build_pi_argv(&spec).expect("spec is valid");
    let output = std::process::Command::new(&argv[0])
        .args(&argv[1..])
        .output()
        .unwrap_or_else(|error| panic!("failed to spawn pi for smoke test: {error}"));
    let transcript = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !transcript.to_ascii_lowercase().contains("unknown option"),
        "pi {major}.{minor} rejected a flag build_pi_argv emits.\nargv: {argv:?}\npi said:\n{transcript}"
    );
}
