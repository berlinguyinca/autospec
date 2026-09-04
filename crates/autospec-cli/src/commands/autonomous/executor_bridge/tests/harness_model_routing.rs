// executor_bridge tests: harness-neutral model routing.
//
// Context: model selection existed here for OpenCode alone. `ResolvedHarness`
// carried `opencode_model`/`opencode_variant` from `AUTOSPEC_OPENCODE_MODEL` and
// `AUTOSPEC_OPENCODE_VARIANT`, and `opencode_model_args()` was reachable only
// from the two OpenCode arms. Claude, Codex and Pi had no way to be routed at
// all, so the two-tier selection AGENTS.md describes — and the local-model
// guardrail wave in #3344 — applied to one harness out of four.
//
// Two implementations of the same concept had grown up outside Rust to fill the
// gap: `scripts/executor-dispatch.sh` (the §16 envelope, which parses `model`
// and `provider` and has no executable caller) and PR #3493 on
// `scripts/dispatch-implementer.sh` (closed in favour of this). ADR 0001 puts
// live dispatch in `executor_bridge`, so this is where it belongs.
//
// Contract pinned here:
//   * `AUTOSPEC_EXECUTOR_MODEL` / `AUTOSPEC_EXECUTOR_PROVIDER` are harness-neutral.
//   * Each harness emits the flag its own CLI accepts, immediately before the
//     prompt; `AUTOSPEC_OPENCODE_MODEL`/`_VARIANT` keep working for OpenCode and
//     take precedence there, so no existing operator config changes behaviour.
//   * A provider directive a harness cannot express is an error, never a silent
//     drop — the routing ledger must not record a dispatch as routed when the
//     argv did not carry the routing.
//   * With none of the variables set, every harness's argv is unchanged.

use super::super::{HarnessConfig, HarnessKind};
use super::support_base::{environment, test_root, write_alias_table};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::Path;

const WORKTREE: &str = "/safe/worktree";
const ARTIFACT: &str = "/safe/state/unused.txt";
const PROMPT: &str = "implement issue 42";

/// The shared `installed_aliases()` covers Claude, Codex and OpenCode only, and
/// several tests assert on its exact alias ordering — so Pi is added here rather
/// than there, where it would perturb suites that are not about routing.
fn routable_aliases() -> &'static str {
    "claude\ttrue\t--dangerously-skip-permissions\tClaude Code\n\
     codex\tsh\t--yolo\tCodex CLI\n\
     opencode\tfalse\t\tOpenCode\n\
     pi\ttrue\t\tPi\n"
}

/// Build an env for one harness kind, with the containment adapter OpenCode
/// requires already present so every kind reaches argv construction.
fn env_for(root: &Path, kind: HarnessKind) -> BTreeMap<String, OsString> {
    let table = write_alias_table(root, routable_aliases());
    let mut env = environment(&table);
    env.insert(
        "AUTOSPEC_HANDOFF_DISPATCHER_KIND".to_string(),
        OsString::from(kind.as_str()),
    );
    env.insert(
        "AUTOSPEC_OPENCODE_CONTAINMENT_ADAPTER".to_string(),
        OsString::from("/usr/bin/true"),
    );
    env
}

fn args_for(root: &Path, env: &BTreeMap<String, OsString>) -> Result<Vec<String>, String> {
    let resolved = HarnessConfig::load(root, env)
        .expect("load harness config")
        .resolve(env)?;
    resolved
        .invocation(Path::new(WORKTREE), Path::new(ARTIFACT), PROMPT)
        .map(|invocation| invocation.args)
}

/// Assert `--model <id>` appears exactly once, immediately before the prompt.
/// Position matters: a model flag placed after the positional prompt is parsed
/// by every one of these CLIs as part of the prompt, not as routing.
fn assert_model_precedes_prompt(args: &[String], model: &str) {
    let at = args
        .iter()
        .position(|arg| arg == "--model")
        .unwrap_or_else(|| panic!("no --model in {args:?}"));
    assert_eq!(
        args.iter().filter(|arg| *arg == "--model").count(),
        1,
        "--model emitted more than once in {args:?}"
    );
    assert_eq!(args.get(at + 1).map(String::as_str), Some(model));
    assert_eq!(
        args.last().map(String::as_str),
        Some(PROMPT),
        "prompt must remain the final argument in {args:?}"
    );
}

#[test]
fn autonomous_executor_bridge_routes_claude_with_the_neutral_model_variable() {
    let root = test_root("routing-claude-model");
    let mut env = env_for(&root, HarnessKind::Claude);
    env.insert(
        "AUTOSPEC_EXECUTOR_MODEL".to_string(),
        OsString::from("claude-opus-5[1m]"),
    );

    let args = args_for(&root, &env).expect("build Claude invocation");
    assert_model_precedes_prompt(&args, "claude-opus-5[1m]");
}

#[test]
fn autonomous_executor_bridge_routes_codex_with_the_neutral_model_variable() {
    let root = test_root("routing-codex-model");
    let mut env = env_for(&root, HarnessKind::Codex);
    env.insert(
        "AUTOSPEC_EXECUTOR_MODEL".to_string(),
        OsString::from("gpt-5-codex"),
    );

    let args = args_for(&root, &env).expect("build Codex invocation");
    assert_model_precedes_prompt(&args, "gpt-5-codex");
}

#[test]
fn autonomous_executor_bridge_routes_pi_with_both_provider_and_model() {
    // Pi is the harness the two-node split needs: `--provider` selects the
    // endpoint (a tunnelled remote node vs the local one) and `--model` the
    // weights on it. Both must reach argv or the split silently collapses onto
    // whichever model the harness happens to default to.
    let root = test_root("routing-pi-provider-model");
    let mut env = env_for(&root, HarnessKind::Pi);
    env.insert(
        "AUTOSPEC_EXECUTOR_PROVIDER".to_string(),
        OsString::from("qwen-bender"),
    );
    env.insert(
        "AUTOSPEC_EXECUTOR_MODEL".to_string(),
        OsString::from("qwen3.8-27b-q6"),
    );

    let args = args_for(&root, &env).expect("build Pi invocation");
    assert_eq!(
        args,
        vec![
            "--provider".to_string(),
            "qwen-bender".to_string(),
            "--model".to_string(),
            "qwen3.8-27b-q6".to_string(),
            PROMPT.to_string(),
        ]
    );
}

#[test]
fn autonomous_executor_bridge_opencode_legacy_model_variable_still_wins() {
    // Backward compatibility: an operator whose config predates the neutral
    // variable must see byte-identical argv. The OpenCode-specific variable is
    // more specific, so it takes precedence rather than being overridden.
    let root = test_root("routing-opencode-legacy");
    let mut env = env_for(&root, HarnessKind::OpenCode);
    env.insert(
        "AUTOSPEC_OPENCODE_MODEL".to_string(),
        OsString::from("anthropic/claude-opus-5"),
    );
    env.insert(
        "AUTOSPEC_OPENCODE_VARIANT".to_string(),
        OsString::from("high"),
    );
    env.insert(
        "AUTOSPEC_EXECUTOR_MODEL".to_string(),
        OsString::from("should-not-be-used"),
    );

    let args = args_for(&root, &env).expect("build OpenCode invocation");
    assert!(
        args.iter().all(|arg| arg != "should-not-be-used"),
        "neutral variable overrode the OpenCode-specific one: {args:?}"
    );
    assert_model_precedes_prompt(&args, "anthropic/claude-opus-5");
    assert!(args.iter().any(|arg| arg == "--variant"));
}

#[test]
fn autonomous_executor_bridge_opencode_accepts_the_neutral_model_variable() {
    let root = test_root("routing-opencode-neutral");
    let mut env = env_for(&root, HarnessKind::OpenCode);
    env.insert(
        "AUTOSPEC_EXECUTOR_MODEL".to_string(),
        OsString::from("anthropic/claude-sonnet-5"),
    );

    let args = args_for(&root, &env).expect("build OpenCode invocation");
    assert_model_precedes_prompt(&args, "anthropic/claude-sonnet-5");
}

#[test]
fn autonomous_executor_bridge_refuses_a_provider_the_harness_cannot_express() {
    // Claude, Codex and OpenCode select a provider through their own
    // configuration, not an invocation flag. Honouring the model while dropping
    // the provider would report a routed dispatch that was never routed, which
    // is the failure the guardrails in #3344 exist to prevent — so refuse.
    for kind in [
        HarnessKind::Claude,
        HarnessKind::Codex,
        HarnessKind::OpenCode,
    ] {
        let root = test_root(&format!("routing-refuse-provider-{}", kind.as_str()));
        let mut env = env_for(&root, kind);
        env.insert(
            "AUTOSPEC_EXECUTOR_PROVIDER".to_string(),
            OsString::from("qwen-bender"),
        );

        let error = args_for(&root, &env).expect_err(&format!(
            "{} must refuse a provider directive",
            kind.as_str()
        ));
        assert!(
            error.contains("executor_routing_provider_unsupported"),
            "unexpected error for {}: {error}",
            kind.as_str()
        );
    }
}

#[test]
fn autonomous_executor_bridge_argv_is_unchanged_when_no_routing_is_set() {
    // The whole surface is opt-in. With none of the variables set, every harness
    // must build exactly the argv it built before routing existed.
    for kind in [
        HarnessKind::Claude,
        HarnessKind::Codex,
        HarnessKind::OpenCode,
        HarnessKind::Pi,
    ] {
        let root = test_root(&format!("routing-absent-{}", kind.as_str()));
        let env = env_for(&root, kind);
        let args = args_for(&root, &env)
            .unwrap_or_else(|error| panic!("{} invocation: {error}", kind.as_str()));

        assert!(
            args.iter()
                .all(|arg| arg != "--model" && arg != "--provider"),
            "{} emitted routing flags with no routing set: {args:?}",
            kind.as_str()
        );
        assert_eq!(args.last().map(String::as_str), Some(PROMPT));
    }
}

// ── context budget ───────────────────────────────────────────────────────────

#[test]
fn autonomous_executor_bridge_codex_receives_the_context_budget_as_a_config_override() {
    // Codex is the only one of the four with a real per-invocation knob for the
    // window (`-c key=value`), so it is the only one where the budget reaches argv.
    let root = test_root("routing-codex-context");
    let mut env = env_for(&root, HarnessKind::Codex);
    env.insert(
        "AUTOSPEC_EXECUTOR_CONTEXT".to_string(),
        OsString::from("64k"),
    );

    let args = args_for(&root, &env).expect("build Codex invocation");
    let at = args
        .iter()
        .position(|arg| arg == "-c")
        .unwrap_or_else(|| panic!("no -c in {args:?}"));
    assert_eq!(
        args.get(at + 1).map(String::as_str),
        Some("model_context_window=65536"),
        "64k must be expanded to tokens, not passed through verbatim: {args:?}"
    );
    assert_eq!(args.last().map(String::as_str), Some(PROMPT));
}

#[test]
fn autonomous_executor_bridge_refuses_a_context_below_the_measured_client_floor() {
    // docs/memory/feedback_context_floor_kills_small_tiers.md: Claude Code carries
    // 39,655 tokens before any work begins. A 32k window is full before the first
    // question, so the dispatch cannot start — and on a shared KV pool with no
    // admission control it takes the slot anyway. Refusing is what makes the pool
    // shareable; the alternative is a reservation that is certain to be wasted.
    let root = test_root("routing-context-below-floor");
    let mut env = env_for(&root, HarnessKind::Claude);
    env.insert(
        "AUTOSPEC_EXECUTOR_CONTEXT".to_string(),
        OsString::from("32k"),
    );

    let error = args_for(&root, &env).expect_err("32k is below the Claude Code floor");
    assert!(
        error.contains("executor_routing_context_below_floor"),
        "unexpected error: {error}"
    );
    assert!(
        error.contains("39655"),
        "the refusal must cite the measured floor: {error}"
    );
}

#[test]
fn autonomous_executor_bridge_accepts_a_context_above_the_measured_floor() {
    let root = test_root("routing-context-above-floor");
    let mut env = env_for(&root, HarnessKind::Claude);
    env.insert(
        "AUTOSPEC_EXECUTOR_CONTEXT".to_string(),
        OsString::from("120k"),
    );

    // Claude's window is not operator-selectable, so nothing reaches argv — but
    // the floor check has run, which is the part that protects the pool.
    let args = args_for(&root, &env).expect("120k clears the Claude Code floor");
    assert!(args.iter().all(|arg| arg != "-c"));
    assert_eq!(args.last().map(String::as_str), Some(PROMPT));
}

#[test]
fn autonomous_executor_bridge_applies_no_floor_to_an_unmeasured_client() {
    // Only OpenCode and Claude Code have measured floors. The source memo is
    // explicit that a number measured on one client must not be carried to
    // another, so Pi and Codex get no invented floor.
    let root = test_root("routing-context-unmeasured");
    let mut env = env_for(&root, HarnessKind::Pi);
    env.insert(
        "AUTOSPEC_EXECUTOR_CONTEXT".to_string(),
        OsString::from("8k"),
    );

    let args = args_for(&root, &env).expect("no floor is known for Pi");
    assert_eq!(args.last().map(String::as_str), Some(PROMPT));
}

#[test]
fn autonomous_executor_bridge_refuses_an_unparseable_context_budget() {
    let root = test_root("routing-context-garbage");
    let mut env = env_for(&root, HarnessKind::Codex);
    env.insert(
        "AUTOSPEC_EXECUTOR_CONTEXT".to_string(),
        OsString::from("lots"),
    );

    let error = args_for(&root, &env).expect_err("an unparseable budget must not be guessed");
    assert!(
        error.contains("executor_routing_context_unparseable"),
        "unexpected error: {error}"
    );
}
