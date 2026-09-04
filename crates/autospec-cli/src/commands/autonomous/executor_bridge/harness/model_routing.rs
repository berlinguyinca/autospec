//! Which model runs a dispatch, independent of which harness runs it.
//!
//! Extracted from `harness.rs` rather than added to it: routing is a self
//! contained decision with its own vocabulary (client context floors, budget
//! parsing, per-CLI argv shapes), and `harness.rs` is the file every harness
//! change touches. Keeping it here also keeps that file inside the size ratchet.

use super::*;

/// Which model runs a dispatch, independent of which harness runs it.
///
/// This started as two OpenCode-only fields, which meant the two-tier selection
/// in AGENTS.md — and the local-model guardrails in #3344 — applied to one
/// harness out of four. The routing decision is not a property of OpenCode, so
/// it is modelled once here and each harness is asked to express it in its own
/// argv.
///
/// `variant` stays OpenCode-shaped on purpose: it is a reasoning tier, not a
/// model id, and no other harness has an equivalent to generalise over.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ModelRouting {
    pub(super) model: Option<String>,
    pub(super) provider: Option<String>,
    pub(super) variant: Option<String>,
    pub(super) context_tokens: Option<u32>,
}

/// Tokens a client carries before any work begins — system prompt, project
/// instructions, memory, skill and MCP schemas — measured from real sessions by
/// `llm/linux-qwen38/scripts/analyze-session-contexts.py` and recorded in
/// `docs/memory/feedback_context_floor_kills_small_tiers.md` (2026-08-18).
///
/// A window below this cannot start a session: it is full before the first
/// question. On a shared KV pool with no admission control that is worse than
/// slow, because the doomed dispatch still reserves its slot and the failure
/// lands on every live session rather than the greedy one. Refusing here is what
/// makes the pool shareable.
///
/// Deliberately absent for Codex and Pi. The source memo is explicit that the
/// floor is client-specific — Claude Code's is nearly 3x OpenCode's because its
/// prompt and skill set are larger — so an unmeasured client gets no floor
/// rather than a borrowed one.
fn measured_context_floor(kind: HarnessKind) -> Option<u32> {
    match kind {
        HarnessKind::Claude => Some(39_655), // floor p50; no p90 measured
        HarnessKind::OpenCode => Some(37_873), // floor p90
        HarnessKind::Codex | HarnessKind::Pi => None,
    }
}

/// Accept `131072` and `128k` alike; an operator writing a budget by hand writes
/// the second. Anything else is refused rather than guessed — a misparsed budget
/// silently becomes a wrong reservation.
fn parse_context_budget(raw: &str) -> Result<u32, String> {
    let text = raw.trim().to_ascii_lowercase();
    let (digits, multiplier) = match text.strip_suffix('k') {
        Some(head) => (head, 1024_u32),
        None => (text.as_str(), 1_u32),
    };
    digits
        .trim()
        .parse::<u32>()
        .ok()
        .and_then(|value| value.checked_mul(multiplier))
        .filter(|value| *value > 0)
        .ok_or_else(|| {
            format!(
                "executor_routing_context_unparseable: AUTOSPEC_EXECUTOR_CONTEXT={raw} is not                  a token count; write it as `131072` or `128k`"
            )
        })
}

impl ModelRouting {
    /// `AUTOSPEC_EXECUTOR_MODEL` / `_PROVIDER` are harness-neutral.
    /// `AUTOSPEC_OPENCODE_MODEL` / `_VARIANT` predate them and stay authoritative
    /// for OpenCode: an operator whose configuration names the specific harness
    /// meant that harness, and must not have it silently overridden by a broader
    /// setting. Absent variables leave the harness default untouched.
    pub(super) fn load(kind: HarnessKind, env: &BTreeMap<String, OsString>) -> Self {
        let read = |key: &str| {
            env.get(key)
                .and_then(|value| value.to_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        };
        let neutral_model = read("AUTOSPEC_EXECUTOR_MODEL");
        let model = if kind == HarnessKind::OpenCode {
            read("AUTOSPEC_OPENCODE_MODEL").or(neutral_model)
        } else {
            neutral_model
        };
        Self {
            model,
            provider: read("AUTOSPEC_EXECUTOR_PROVIDER"),
            // Named for OpenCode and only ever consumed by it. Reading it for
            // other harnesses would turn an exported variable into a hard
            // failure for a dispatch it was never meant to describe.
            variant: if kind == HarnessKind::OpenCode {
                read("AUTOSPEC_OPENCODE_VARIANT")
            } else {
                None
            },
            context_tokens: None,
        }
    }

    /// Parsed and floor-checked separately from `load` so the failure is a
    /// `Result` the caller surfaces, not a silently dropped field.
    pub(super) fn with_context(
        mut self,
        kind: HarnessKind,
        env: &BTreeMap<String, OsString>,
    ) -> Result<Self, String> {
        let raw = env
            .get("AUTOSPEC_EXECUTOR_CONTEXT")
            .and_then(|value| value.to_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let Some(raw) = raw else {
            return Ok(self);
        };

        let tokens = parse_context_budget(raw)?;
        if let Some(floor) = measured_context_floor(kind) {
            if tokens < floor {
                return Err(format!(
                    "executor_routing_context_below_floor: {tokens} tokens is under the                      {floor}-token session floor measured for {}; a window this small is                      full before the first question and would hold a slot in the shared KV                      pool without ever starting (docs/memory/\
                     feedback_context_floor_kills_small_tiers.md)",
                    kind.as_str()
                ));
            }
        }
        self.context_tokens = Some(tokens);
        Ok(self)
    }
}

impl ResolvedHarness {
    /// The routing flags this harness's CLI accepts, to be spliced in
    /// immediately before the positional prompt. Every one of these CLIs treats
    /// a flag after the prompt as part of the prompt, so position is part of the
    /// contract, not a formatting preference.
    ///
    /// A directive the harness cannot express is an error rather than a silent
    /// drop. Emitting the model while discarding the provider would let the
    /// routing ledger record a dispatch as routed when the argv never carried
    /// the routing — the precise failure the #3344 guardrails exist to catch.
    pub(super) fn routing_args(&self) -> Result<Vec<String>, String> {
        let mut args = Vec::new();

        if let Some(provider) = &self.routing.provider {
            match self.kind {
                // Pi selects the endpoint by provider name; see its --provider flag.
                HarnessKind::Pi => {
                    args.push("--provider".into());
                    args.push(provider.clone());
                }
                // Claude, Codex and OpenCode bind a provider through their own
                // configuration, not an invocation flag.
                other => {
                    return Err(format!(
                        "executor_routing_provider_unsupported: {} cannot express \
                         AUTOSPEC_EXECUTOR_PROVIDER={provider} as an invocation flag; \
                         configure the provider in the harness itself or dispatch \
                         through a harness that accepts one",
                        other.as_str()
                    ));
                }
            }
        }

        if let Some(model) = &self.routing.model {
            args.push("--model".into());
            args.push(model.clone());
        }

        // Only Codex exposes the window as a per-invocation setting. The others
        // take it from the model or the server, so the budget cannot reach their
        // argv — and unlike a provider directive, that is not a
        // misrepresentation: a context budget asserts capacity, it does not
        // redirect the work somewhere the caller did not ask for. Its real
        // enforcement is the floor check in `ModelRouting::with_context`, which
        // has already run by the time we get here.
        if let Some(tokens) = self.routing.context_tokens {
            if self.kind == HarnessKind::Codex {
                args.push("-c".into());
                args.push(format!("model_context_window={tokens}"));
            }
        }

        if let Some(variant) = &self.routing.variant {
            match self.kind {
                HarnessKind::OpenCode => {
                    args.push("--variant".into());
                    args.push(variant.clone());
                }
                other => {
                    return Err(format!(
                        "executor_routing_variant_unsupported: {} has no reasoning-variant \
                         flag; AUTOSPEC_OPENCODE_VARIANT applies to OpenCode only",
                        other.as_str()
                    ));
                }
            }
        }

        Ok(args)
    }
}
