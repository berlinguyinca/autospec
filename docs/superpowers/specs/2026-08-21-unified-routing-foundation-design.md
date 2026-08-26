# Unified Routing Foundation Design

**Date:** 2026-08-21  
**Status:** Approved design; implementation pending  
**Scope:** AutoSpec routing foundation, Pi harness adapter, InferWeave capability routing, compatibility behavior

## 1. Goal

Provide one deterministic routing contract that selects a harness and an inference
capability independently for every AutoSpec dispatch. The first implementation must
support Pi, Codex, OpenCode, and Claude harness identities; resolve live inference
through InferWeave; preserve existing profile and tier behavior as a fail-closed
fallback; and advertise vision only when InferWeave reports an eligible Mac or RTX
6000 route.

This design is complete when routing decisions can be generated, validated, explained,
and consumed as typed dispatch envelopes without requiring a skill prompt to interpret
model tiers or hardware availability.

## 2. Problem

AutoSpec currently has two partially independent routing systems:

1. `~/.autospec/model-profiles.yml` and `--profile` determine issue admission and can
   resolve an implementer model.
2. Harness-specific `TIER_A` and `TIER_B` prose determines most actual subagent
   execution, including planning and review.

The split prevents an operator from expressing a complete policy such as “plan with Pi,
execute with Pi through InferWeave, and review independently with Codex.” It also makes
hardware names leak into workflow prompts, leaves Pi outside the canonical harness
registry, and allows vision availability to be inferred from a model name rather than
advertised by the serving network.

## 3. Scope

### 3.1 In scope

- A versioned YAML routing configuration.
- A deterministic validator and resolver.
- A versioned JSON dispatch-envelope contract.
- First-class harness identities for `pi`, `codex`, `opencode`, and `claude`.
- A Pi adapter using non-interactive JSON or RPC mode.
- InferWeave discovery through a bounded HTTP capability request.
- Independent harness and inference selection.
- Text and vision capability requirements.
- Review independence and minimum-strength constraints.
- Compatibility fallback to the existing model-profile and tier path.
- Explainable routing decisions and stable refusal identifiers.
- Hermetic tests with fixture capability documents; no live GPU is required in CI.

### 3.2 Out of scope

- Consolidating the public AutoSpec skill catalog.
- Replacing large skills with thin command routers.
- Context manifests, task capsules, source retrieval, or prompt-token enforcement.
- InferWeave control-plane implementation or changes to the private InferWeave repo.
- Automatically proving that Pi is more stable than OpenCode.
- Removing `model-profiles.yml`, `TIER_A`, `TIER_B`, or existing environment hatches.
- Changing security, merge, or review quality policy.

Those items depend on this foundation and require separate specifications.

## 4. Design principles

1. **Semantic work belongs to AutoSpec.** AutoSpec chooses the dispatch kind, harness
   policy, modality, context class, and independence requirements.
2. **Physical capacity belongs to InferWeave.** InferWeave chooses the live endpoint,
   served model, node, measured context ceiling, and queue placement.
3. **Harness and inference are independent.** A Pi process may consume an InferWeave
   route, as may Codex or OpenCode, provided the adapter supports the returned protocol.
4. **Measured capability beats model-name inference.** Hardware, context, vision, and
   concurrency are accepted only from a valid capability document.
5. **Compatibility is fail-closed.** Missing, stale, malformed, or unsuitable discovery
   data leaves current routing unchanged.
6. **Normal text work cannot require scarce accelerators.** H100 and RTX 6000 routes are
   opportunistic unless a task explicitly requires a capability available nowhere else.
7. **Vision is explicit.** Only routes advertised by policy as Mac or RTX 6000 vision
   capacity are eligible, even if another node is technically capable.
8. **Review remains independent.** A reviewer cannot silently use the same harness and
   inference profile as the implementation it evaluates when independence is required.

## 5. Configuration

The default operator path is `~/.autospec/routing.yml`. Tests and automation may set
`AUTOSPEC_ROUTING_CONFIG` to another absolute path.

```yaml
version: 1

harnesses:
  pi:
    command: [pi, --mode, json]
    transport: json
    provider_protocols: [openai-compatible]

  codex:
    command: [codex, exec]
    transport: cli
    provider_protocols: [openai-compatible, native]

  opencode:
    command: [opencode, run]
    transport: cli
    provider_protocols: [openai-compatible, native]

  claude:
    command: [claude, --print]
    transport: cli
    provider_protocols: [native]

routes:
  explore:
    harnesses: [pi, codex]
    inference_class: text.compact

  planning:
    harnesses: [pi, codex, claude]
    inference_class: text.standard

  execution:
    harnesses: [pi, codex, opencode]
    inference_class: text.standard

  review:
    harnesses: [codex, pi, opencode]
    inference_class: text.standard
    independent_from: execution
    minimum_strength: execution

  visual_qa:
    harnesses: [pi]
    inference_class: vision.standard

inference_classes:
  text.compact:
    modalities: [text]
    max_input_tokens: 24000
    reserve_output_tokens: 8000
    max_queue_seconds: 30

  text.standard:
    modalities: [text]
    max_input_tokens: 48000
    reserve_output_tokens: 12000
    max_queue_seconds: 90

  text.extended:
    modalities: [text]
    max_input_tokens: 72000
    reserve_output_tokens: 16000
    max_queue_seconds: 180

  vision.standard:
    modalities: [text, image]
    max_input_tokens: 48000
    reserve_output_tokens: 12000
    max_queue_seconds: 120
    eligible_node_classes: [mac, rtx6000]

inferweave:
  discovery_url: https://inferweave.invalid/v1/capabilities
  timeout_seconds: 3
  maximum_age_seconds: 60
  local_only: false

fallback:
  mode: existing-routing
```

### 5.1 Validation

The validator rejects:

- unknown schema versions;
- duplicate or empty route names;
- unsupported transports or provider protocols;
- missing harness commands;
- missing inference classes;
- non-positive context, reserve, timeout, queue, or maximum-age values;
- a reserve greater than or equal to the serving context window when a serving
  context is supplied;
- `image` classes without `eligible_node_classes`;
- `vision.standard` eligibility outside `mac` and `rtx6000`;
- independence references to missing routes;
- unsupported fallback modes;
- relative `AUTOSPEC_ROUTING_CONFIG` paths.

Unknown keys fail validation. This prevents misspelled safety or routing controls from
being silently ignored.

## 6. InferWeave capability contract

AutoSpec performs one bounded `GET` request per resolution cycle. The response is JSON:

```json
{
  "version": 1,
  "generated_at": "2026-08-21T12:00:00Z",
  "routes": [
    {
      "id": "qwen-text-48k-a",
      "endpoint": "https://inferweave.example/v1",
      "protocol": "openai-compatible",
      "model": "qwen3.8-27b-48k",
      "node_class": "dual-1080ti",
      "modalities": ["text"],
      "context_window": 60000,
      "max_input_tokens": 48000,
      "strength": 20,
      "queue_seconds": 5,
      "available": true,
      "opportunistic": false
    }
  ]
}
```

`generated_at` must parse as RFC 3339 and be no older than
`inferweave.maximum_age_seconds`. Every candidate must have a unique non-empty `id`, an
HTTPS endpoint unless loopback HTTP is explicitly enabled for local development, a
supported protocol, non-empty model and node class, positive context values, and a
`max_input_tokens` no greater than `context_window`.

The resolver ignores unavailable candidates. It rejects the entire capability document,
rather than partially trusting it, when required top-level fields or candidate safety
fields are malformed.

### 6.1 Candidate eligibility

A candidate is eligible only when:

- it supplies every modality required by the inference class;
- its `max_input_tokens` meets or exceeds the class input budget;
- its protocol is supported by the selected harness;
- its queue estimate does not exceed the class limit;
- its node class is allowed by the class when an allowlist is present;
- it is not opportunistic when the request forbids opportunistic capacity;
- it satisfies `local_only` when that policy is enabled;
- review independence and minimum-strength constraints remain satisfied.

Eligible candidates sort deterministically by:

1. non-opportunistic before opportunistic;
2. lowest queue estimate;
3. smallest sufficient `max_input_tokens`;
4. route ID as the stable final tie-breaker.

The first eligible candidate wins.

## 7. Dispatch envelope

The resolver writes a JSON envelope to stdout. Dynamic secrets never appear in it.

```json
{
  "version": 1,
  "dispatch_id": "sha256:...",
  "kind": "execution",
  "harness": {
    "id": "pi",
    "command": ["pi", "--mode", "json"],
    "transport": "json"
  },
  "inference": {
    "source": "inferweave",
    "route_id": "qwen-text-48k-a",
    "endpoint": "https://inferweave.example/v1",
    "protocol": "openai-compatible",
    "model": "qwen3.8-27b-48k",
    "modalities": ["text"],
    "context_window": 60000,
    "max_input_tokens": 48000,
    "reserve_output_tokens": 12000,
    "node_class": "dual-1080ti",
    "strength": 20,
    "opportunistic": false
  },
  "policy": {
    "independent_from": null,
    "fallback": "existing-routing"
  },
  "explain": [
    "selected harness pi: first available configured harness",
    "selected route qwen-text-48k-a: eligible text.standard candidate"
  ]
}
```

`dispatch_id` is a SHA-256 digest over the canonicalized decision inputs and therefore
contains no random or secret material. The same configuration, capability document,
kind, and independence inputs produce the same envelope.

### 7.1 Refusal envelope

When the new router cannot make a safe decision, it exits `3` and emits:

```json
{
  "version": 1,
  "status": "fallback_required",
  "reason": "ROUTING_CAPABILITY_UNAVAILABLE",
  "fallback": "existing-routing",
  "explain": ["no text.standard candidate satisfied the configured route"]
}
```

Stable refusal identifiers are:

- `ROUTING_CONFIG_MISSING`
- `ROUTING_CONFIG_INVALID`
- `ROUTING_HARNESS_UNAVAILABLE`
- `ROUTING_DISCOVERY_FAILED`
- `ROUTING_CAPABILITY_STALE`
- `ROUTING_CAPABILITY_INVALID`
- `ROUTING_CAPABILITY_UNAVAILABLE`
- `ROUTING_INDEPENDENCE_UNSATISFIED`
- `ROUTING_ADAPTER_UNSUPPORTED`

Configuration syntax errors exit `1`. Missing local dependencies exit `2`. Safe
fallback requests exit `3`. Successful decisions exit `0`.

## 8. Pi harness adapter

Pi is a first-class harness ID, not a model provider. The adapter consumes an envelope
whose protocol is `openai-compatible` and launches Pi non-interactively.

The first implementation uses `pi --mode json` because it provides a finite process and
structured JSONL suitable for one AutoSpec dispatch. RPC remains a declared transport
for a later persistent-session optimization.

The adapter:

1. validates the dispatch envelope;
2. refuses a non-Pi harness or non-OpenAI-compatible inference protocol;
3. verifies `pi` is installed;
4. passes the InferWeave endpoint and model through Pi's supported provider/model
   configuration mechanism without writing credentials to arguments or artifacts;
5. passes one prompt file;
6. captures JSONL output;
7. returns the final assistant result and normalized usage fields;
8. preserves Pi's exit status and emits a stable adapter error on malformed output.

The adapter receives endpoint credentials only through an environment-variable name
configured by the operator. Routing files and dispatch envelopes contain the variable
name, never its value.

Pi children do not receive nested-agent capability in this slice. AutoSpec owns
parallelism and process isolation.

## 9. Compatibility

The new resolver is additive. Existing skill paths invoke it before the current model
selector only after an operator creates `routing.yml` or sets
`AUTOSPEC_ROUTING_CONFIG`.

When routing is not configured, invalid, unavailable, or unsuitable, callers receive
exit `3` and must execute the current path unchanged:

1. existing `select-model-profile.sh` behavior for implementation;
2. existing harness-detected `TIER_B` fallback;
3. existing `TIER_A` fallback-up behavior;
4. existing reviewer tier and `AUTOSPEC_REVIEWER_TIER` behavior.

No migration rewrites `~/.autospec/model-profiles.yml`. A later migration may generate
new route profiles from it, but this slice does not.

## 10. Security and privacy

- Capability discovery has a three-second default timeout and no retry loop.
- Redirects are rejected by the default client invocation.
- Only HTTPS endpoints are accepted, except explicit loopback development endpoints.
- Configuration and capability documents have bounded file/response sizes.
- Endpoint credentials are environment references, never serialized values.
- Resolver output is safe to record in telemetry.
- `local_only: true` excludes non-local routes and fails closed rather than falling back
  to a cloud inference endpoint.
- Vision requests never downgrade to text-only.
- A malformed candidate invalidates discovery instead of being silently repaired.

## 11. Observability

Every decision exposes an `explain` array. When a dispatch completes, the existing
routing ledger gains enough information to associate the outcome with:

- dispatch ID and kind;
- harness ID;
- InferWeave route ID;
- model and node class;
- context class and token limits;
- queue estimate;
- fallback or escalation status;
- outcome and normalized usage.

The resolver itself does not mutate the ledger. The owning workflow records dispatch
and outcome events so a read-only resolution remains deterministic.

## 12. Files and interfaces

The implementation should use focused files:

- `schemas/autospec-routing-v1.schema.json` — operator configuration schema.
- `schemas/inferweave-capabilities-v1.schema.json` — discovery document schema.
- `schemas/autospec-dispatch-envelope-v1.schema.json` — successful envelope schema.
- `scripts/autospec-route.py` — validate and resolve commands.
- `scripts/autospec-pi-dispatch.py` — Pi JSON adapter.
- `config/harness-runtime-aliases.tsv` — add canonical Pi runtime alias.
- `examples/routing.yml` — documented example.
- `tests/routing-foundation/` — hermetic fixtures and behavioral tests.
- `docs/CONFIG_REFERENCE.md` — operator-facing configuration contract.
- `docs/API_REFERENCE.md` — command and envelope contract.

CLI surface:

```text
python3 scripts/autospec-route.py validate --config <path>
python3 scripts/autospec-route.py resolve --config <path> --capabilities <path> \
  --kind <kind> [--proposer-envelope <path>]
python3 scripts/autospec-route.py explain --config <path> --capabilities <path> \
  --kind <kind> [--proposer-envelope <path>]
python3 scripts/autospec-pi-dispatch.py --envelope <path> --prompt-file <path>
```

`resolve` emits JSON. `explain` emits the same decision as readable text and must not
implement a second resolver.

## 13. Testing

Development is test-driven. Tests use temporary files and executable stubs, not live
InferWeave, Pi, GPUs, or cloud APIs.

### 13.1 Schema and validation tests

- Accept the complete example configuration.
- Reject every invalid condition listed in section 5.1.
- Accept a valid text-only capability document.
- Accept a valid Mac or RTX 6000 vision route.
- Reject stale, malformed, duplicate-ID, oversized, and unsafe-endpoint documents.
- Prove all emitted success and refusal envelopes validate against their schemas.

### 13.2 Resolution tests

- Select harness and inference independently.
- Prefer non-opportunistic capacity.
- Select the lowest-queue sufficient route.
- Use route ID as a deterministic tie-breaker.
- Exclude candidates with insufficient context, modality, protocol, or queue budget.
- Never select 1080 Ti, 4090, or H100 for vision under the configured policy.
- Select Mac or RTX 6000 for vision.
- Preserve text operation when scarce nodes are absent.
- Refuse rather than downgrade vision to text.
- Enforce reviewer independence and strength.
- Produce byte-identical envelopes for identical inputs.

### 13.3 Pi adapter tests

- Resolve `pi` from `PATH` and invoke JSON mode.
- Pass model and endpoint without placing secret values in argv.
- Consume JSONL and return the final assistant message.
- Normalize usage fields.
- Reject malformed JSONL, wrong harness, unsupported protocol, and missing Pi.
- Preserve non-zero child status.
- Never enable nested-agent capability.

### 13.4 Compatibility tests

- Missing routing configuration requests existing routing.
- Discovery timeout, invalid JSON, stale data, and no eligible candidate request existing
  routing.
- Existing `select-model-profile.sh` tests remain unchanged and green.
- Existing reviewer-tier tests remain unchanged and green.
- Harness alias generation remains deterministic after adding Pi.

### 13.5 Repository validation

The completed implementation must pass:

```bash
cargo test --workspace
bash tests/routing-foundation/test.sh
bats tests/select-model-profile.bats
bats tests/routing-decision.bats
bats tests/harness-runtime-alias-generation.bats
autospec validate
```

If a command is unavailable in the test environment, the closeout must identify the
missing executable and must not claim that gate passed.

## 14. Rollout

1. Ship schemas, resolver, example, and tests without changing callers.
2. Ship the Pi adapter and canonical harness alias.
3. Add opt-in caller integration guarded by the presence of routing configuration.
4. Record routing outcomes alongside existing telemetry.
5. After representative Pi, Codex, and OpenCode runs, compare first-pass success,
   retries, total tokens, latency, and failures before considering a new default.

Rollback is removal or unsetting of `AUTOSPEC_ROUTING_CONFIG` and
`~/.autospec/routing.yml`. Existing routing then remains authoritative.

## 15. Acceptance criteria

- [ ] A version-1 routing configuration validates deterministically with unknown keys
  rejected.
- [ ] Pi, Codex, OpenCode, and Claude are valid harness identities.
- [ ] Harness and inference route are selected independently.
- [ ] InferWeave capability data is bounded, freshness-checked, and fail-closed.
- [ ] The resolver emits schema-valid, deterministic dispatch envelopes.
- [ ] Pi can consume an OpenAI-compatible envelope in JSON mode without serializing
  credentials.
- [ ] Text planning and execution do not require H100 or RTX 6000 availability.
- [ ] Vision routes only to advertised Mac or RTX 6000 capacity.
- [ ] Vision never silently downgrades to text.
- [ ] Review independence and minimum-strength policy are enforced.
- [ ] Missing or invalid new routing preserves the existing selection path.
- [ ] Existing model-profile, tier fallback, and reviewer-tier behavior remain green.
- [ ] The example configuration, configuration reference, and API reference agree with
  the schemas and CLI.
- [ ] Every command in section 13.5 passes before completion is claimed.

## 16. Follow-up specifications

After this foundation is proven:

1. **Context manifests and task capsules** — role-specific contracts, hard prompt
   budgets, source retrieval, and typed phase artifacts.
2. **Skill-surface consolidation** — approximately seven public skills, thin aliases,
   capability packs, and CLI-owned workflow state.
3. **Measured routing optimization** — select harness and inference class from total
   retry-adjusted cost rather than static preference.

These follow-ups must not be folded into this implementation, because each changes a
different public contract and requires independent rollout evidence.
