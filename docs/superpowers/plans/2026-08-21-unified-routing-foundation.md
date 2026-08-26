# Unified Routing Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the complete routing foundation specified by `2026-08-21-unified-routing-foundation-design.md`, including schemas, deterministic resolution, Pi dispatch, caller compatibility, documentation, and validation.

**Architecture:** A dependency-light Python resolver loads strict YAML, validates InferWeave capability fixtures, selects harness and inference independently, and emits schema-shaped deterministic envelopes. A separate Pi adapter consumes envelopes and structured JSONL. Existing routing remains authoritative whenever the opt-in router returns fallback exit code `3`.

**Tech Stack:** Python 3 standard library, optional PyYAML already used by AutoSpec, JSON Schema documents, Bash/Bats integration tests, existing harness alias generator, existing AutoSpec validation.

**Spec:** `docs/superpowers/specs/2026-08-21-unified-routing-foundation-design.md`

## Global Constraints

- No new dependencies.
- Unknown configuration and capability keys fail validation.
- Vision is eligible only on advertised `mac` and `rtx6000` node classes.
- Vision never downgrades to text.
- Missing or unsafe new routing exits `3` and preserves the existing routing path.
- Secrets are passed only through environment references and never serialized.
- Existing `model-profiles.yml`, `TIER_A`/`TIER_B`, and reviewer-tier behavior remain intact.
- Every code change follows TDD and every commit follows Conventional Commit plus Lore trailers.

---

### Task 1: Contract schemas and fixtures

**Files:**
- Create: `schemas/autospec-routing-v1.schema.json`
- Create: `schemas/inferweave-capabilities-v1.schema.json`
- Create: `schemas/autospec-dispatch-envelope-v1.schema.json`
- Create: `tests/routing-foundation/fixtures/routing-valid.yml`
- Create: `tests/routing-foundation/fixtures/capabilities-text.json`
- Create: `tests/routing-foundation/fixtures/capabilities-vision.json`
- Create: `tests/routing-foundation/test_schemas.py`

**Interfaces:**
- Produces: Draft 2020-12 schemas with `additionalProperties: false` at every object boundary.
- Produces: canonical fixtures consumed by every later task.

- [ ] **Step 1: Write failing schema tests**

  Add `unittest` cases that load all schemas and fixtures, run `jsonschema` when available, and otherwise assert required keys plus `additionalProperties: false`. Include negative fixtures generated in memory for unknown keys, bad version, unsafe vision nodes, duplicate route IDs, and `max_input_tokens > context_window`.

- [ ] **Step 2: Run the tests and confirm failure**

  Run: `python3 -m unittest discover -s tests/routing-foundation -p 'test_schemas.py' -v`  
  Expected: failure because schema files do not exist.

- [ ] **Step 3: Add the three schemas and canonical fixtures**

  Define exact version-1 objects from spec sections 5–7. Use enums for harness transport, provider protocol, fallback mode, modalities, and vision node classes.

- [ ] **Step 4: Run schema tests**

  Run: `python3 -m unittest discover -s tests/routing-foundation -p 'test_*.py' -v`  
  Expected: all Task 1 tests pass.

- [ ] **Step 5: Commit**

  Commit intent: `feat: make routing contracts machine-verifiable`.

### Task 2: Deterministic configuration and capability validation

**Files:**
- Create: `scripts/autospec_route_lib.py`
- Create: `tests/routing-foundation/test_validation.py`

**Interfaces:**
- Produces: `RoutingError(reason: str, message: str, exit_code: int)`.
- Produces: `load_routing_config(path: Path) -> dict`.
- Produces: `load_capabilities(path: Path, *, now: datetime, maximum_age_seconds: int) -> dict`.
- Produces: `validate_routing_config(data: object) -> dict` and `validate_capabilities(data: object, ...) -> dict`.

- [ ] **Step 1: Write validation tests**

  Cover all spec section 5.1 rejection cases, absolute environment override validation, RFC 3339 freshness, duplicate capability IDs, HTTPS/loopback endpoint rules, size bounds, and stable `ROUTING_*` errors.

- [ ] **Step 2: Run and confirm failure**

  Run: `python3 -m unittest tests/routing-foundation/test_validation.py -v`  
  Expected: import failure for `autospec_route_lib`.

- [ ] **Step 3: Implement strict loaders**

  Use `yaml.safe_load` for routing YAML and standard `json` for capabilities. Apply explicit allowed-key sets in Python even when `jsonschema` is unavailable. Bound configuration and capability files before reading them.

- [ ] **Step 4: Run validation tests**

  Run: `python3 -m unittest discover -s tests/routing-foundation -p 'test_*.py' -v`  
  Expected: all validation tests pass.

- [ ] **Step 5: Commit**

  Commit intent: `feat: fail closed on invalid routing inputs`.

### Task 3: Resolver and dispatch envelope

**Files:**
- Modify: `scripts/autospec_route_lib.py`
- Create: `scripts/autospec-route.py`
- Create: `tests/routing-foundation/test_resolver.py`
- Create: `tests/routing-foundation/test_cli.py`

**Interfaces:**
- Produces: `resolve_dispatch(config: dict, capabilities: dict, kind: str, proposer: dict | None) -> dict`.
- Produces CLI: `validate`, `resolve`, and `explain`.
- Produces stable exit codes `0`, `1`, `2`, `3` and refusal identifiers from spec section 7.1.

- [ ] **Step 1: Write resolver tests**

  Assert independent harness/inference selection; modality, context, protocol, queue, local-only, and opportunistic filters; deterministic sorting; vision node restrictions; review independence; minimum strength; and byte-identical output.

- [ ] **Step 2: Run and confirm failure**

  Run: `python3 -m unittest tests/routing-foundation/test_resolver.py -v`  
  Expected: missing resolver symbols.

- [ ] **Step 3: Implement the pure resolver**

  Select the first available configured harness whose protocol set intersects each candidate. Sort candidates by `(opportunistic, queue_seconds, max_input_tokens, id)`. Derive `dispatch_id` from canonical JSON containing config, capabilities, kind, and proposer identity.

- [ ] **Step 4: Write CLI tests and confirm failure**

  Exercise success JSON, readable explain output, invalid syntax exit `1`, missing dependency exit `2`, and fallback exit `3`.

- [ ] **Step 5: Implement the CLI**

  Keep `explain` as formatting over the same resolved object. Do not duplicate selection logic.

- [ ] **Step 6: Run resolver and CLI tests**

  Run: `python3 -m unittest discover -s tests/routing-foundation -p 'test_*.py' -v`  
  Expected: all tests pass.

- [ ] **Step 7: Commit**

  Commit intent: `feat: resolve typed routing envelopes deterministically`.

### Task 4: Pi harness and adapter

**Files:**
- Modify: `config/harness-runtime-aliases.tsv`
- Modify: `scripts/lib/autospec-harness-detect.sh`
- Modify: `scripts/gen-harness-runtime-aliases.sh`
- Create: `scripts/autospec-pi-dispatch.py`
- Create: `tests/routing-foundation/test_pi_adapter.py`
- Modify: `tests/harness-runtime-alias-generation.bats`

**Interfaces:**
- Adds harness identity: `pi`, binary `pi`, default noninteractive flag `--mode json`.
- Produces: `run_pi_dispatch(envelope: dict, prompt_path: Path, env: Mapping[str, str]) -> dict`.
- Pi adapter stdout: normalized JSON `{message, usage, child_exit_status}`.

- [ ] **Step 1: Add failing harness and adapter tests**

  Extend alias generation assertions for Pi. Use a temporary executable `pi` stub that captures argv/environment and emits JSONL. Assert model/endpoint propagation, no secret in argv, final-message extraction, usage normalization, malformed output rejection, unsupported envelope rejection, and non-zero status preservation.

- [ ] **Step 2: Run and confirm failure**

  Run: `python3 -m unittest tests/routing-foundation/test_pi_adapter.py -v && bats tests/harness-runtime-alias-generation.bats`  
  Expected: failures because Pi is not registered and the adapter is absent.

- [ ] **Step 3: Register Pi and implement adapter**

  Add Pi to the canonical TSV and generated aliases. Implement JSON mode only; reject RPC envelopes in version 1. Pass endpoint/model through documented Pi environment/config overrides and preserve the existing environment except for explicit provider variables.

- [ ] **Step 4: Run adapter and alias tests**

  Run: `python3 -m unittest tests/routing-foundation/test_pi_adapter.py -v`  
  Run: `bats tests/harness-runtime-alias-generation.bats`  
  Expected: pass.

- [ ] **Step 5: Commit**

  Commit intent: `feat: add Pi as a routed AutoSpec harness`.

### Task 5: Opt-in compatibility integration and installation

**Files:**
- Modify: `skills/autospec-run/SKILL.md`
- Derive: `skills/autospec-run/codex/prompt.md`
- Derive: `skills/autospec-run/opencode/agent.md`
- Modify: `skills/autospec-run/install.sh`
- Modify: shared installer script inventories that validate runtime script completeness
- Create: `tests/routing-foundation/test_compatibility.bats`

**Interfaces:**
- Consumes `AUTOSPEC_ROUTING_CONFIG` or `~/.autospec/routing.yml`.
- Calls `autospec-route.py resolve` before implementer selection only when configured.
- Exit `3` continues through the byte-identical existing selector/tier path.

- [ ] **Step 1: Write failing compatibility tests**

  Assert missing config leaves old selection text unchanged, configured success names the routed envelope, every routing refusal reaches existing selection, reviewer tier remains unchanged, and installer inventories ship resolver, library, Pi adapter, and schemas.

- [ ] **Step 2: Run and confirm failure**

  Run: `bats tests/routing-foundation/test_compatibility.bats`  
  Expected: missing wiring and installer entries.

- [ ] **Step 3: Add opt-in caller instructions and installation wiring**

  Keep the current selector block intact as the fallback branch. Generate trio mirrors with `scripts/derive-trio.sh` rather than hand-editing them.

- [ ] **Step 4: Run compatibility and legacy tests**

  Run: `bats tests/routing-foundation/test_compatibility.bats`  
  Run: `bats tests/select-model-profile.bats`  
  Run: `bats tests/routing-decision.bats`  
  Run: `bats skills/autospec-run/tests/reviewer-tier.bats`.  
  Expected: all pass.

- [ ] **Step 5: Commit**

  Commit intent: `feat: route implementers through the opt-in foundation`.

### Task 6: Examples and operator/API documentation

**Files:**
- Create: `examples/routing.yml`
- Modify: `docs/CONFIG_REFERENCE.md`
- Modify: `docs/API_REFERENCE.md`
- Create: `tests/routing-foundation/test_docs.py`

**Interfaces:**
- Documents exact schema keys, commands, exit codes, refusal identifiers, Pi behavior, InferWeave payload, compatibility fallback, and rollback.

- [ ] **Step 1: Write failing documentation contract tests**

  Validate the example with the production loader. Assert every refusal ID, CLI command, environment variable, vision restriction, and rollback instruction appears in the appropriate reference.

- [ ] **Step 2: Run and confirm failure**

  Run: `python3 -m unittest tests/routing-foundation/test_docs.py -v`  
  Expected: missing example/reference content.

- [ ] **Step 3: Add example and documentation**

  Keep prose aligned with the schemas and use generated snippets where the repository already has a generator pattern.

- [ ] **Step 4: Run documentation tests**

  Run: `python3 -m unittest tests/routing-foundation/test_docs.py -v`  
  Expected: pass.

- [ ] **Step 5: Commit**

  Commit intent: `docs: explain unified routing and Pi operation`.

### Task 7: Full validation and requirement audit

**Files:**
- Modify only files required to fix failures found by the gates.
- Update: `docs/superpowers/specs/2026-08-21-unified-routing-foundation-design.md` status to Implemented only after every acceptance criterion is proven.

**Interfaces:**
- Produces: evidence for each acceptance criterion and required command.

- [ ] **Step 1: Run focused suite**

  Run: `bash tests/routing-foundation/test.sh`.

- [ ] **Step 2: Run legacy compatibility suites**

  Run the selector, routing-decision, alias-generation, and reviewer-tier commands from Task 5.

- [ ] **Step 3: Run repository validation**

  Run: `cargo test --workspace`  
  Run: `autospec validate`.

- [ ] **Step 4: Audit the spec line by line**

  For every acceptance criterion and section 13.5 command, record whether file content and fresh command output prove it. Fix every missing or indirect item before continuing.

- [ ] **Step 5: Mark the spec implemented and commit closeout**

  Commit intent: `test: prove unified routing foundation completion`.
