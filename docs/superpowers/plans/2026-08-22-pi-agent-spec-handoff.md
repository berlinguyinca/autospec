# Pi Agent Spec and Handoff Protocol Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement a versioned, fail-closed artifact protocol that lets Pi coordinate isolated Claude and Codex planning/review calls and hand exact validated work to AutoSpec implementation models.

**Architecture:** Four strict JSON Schemas define approved specs, implementation handoffs, review handoffs, and bridge results. A dependency-light Python CLI validates lineage and derives downstream artifacts; a separate Pi bridge adapter invokes only an installed allowlisted extension through a hermetic Pi JSON process. AutoSpec skills and installers expose the protocol while retaining extension-free Pi dispatch and all existing routing and merge gates.

**Tech Stack:** Python 3 standard library, JSON Schema draft 2020-12, Pi JSONL CLI, shell installers, Python `unittest`, Bats, Rust workspace validation.

**Spec:** `docs/superpowers/specs/2026-08-22-pi-agent-spec-handoff-design.md`

## Global Constraints

- No new dependencies.
- Existing Pi dispatch remains byte-observably extension-free without an enabled bridge profile.
- AutoSpec never installs bridge packages during autonomous execution.
- Credentials never enter argv, generated artifacts, or captured output.
- Bridge roles are isolated, bounded to two planning calls, and cannot recursively delegate.
- All artifact paths are repository-relative and all unknown JSON properties fail validation.
- `SKILL.md`, `opencode/agent.md`, and `codex/prompt.md` bodies remain byte-identical.
- Preserve all unrelated worktree changes.

---

### Task 1: Versioned artifact schemas and fixtures

**Files:**
- Create: `schemas/autospec-spec-v1.schema.json`
- Create: `schemas/autospec-implementation-handoff-v1.schema.json`
- Create: `schemas/autospec-review-handoff-v1.schema.json`
- Create: `schemas/autospec-agent-handoff-result-v1.schema.json`
- Create: `tests/pi-agent-handoff/fixtures/*.json`
- Create: `tests/pi-agent-handoff/test_schemas.py`

**Interfaces:**
- Consumes: JSON Schema draft 2020-12 validation conventions used by `tests/routing-foundation/test_schemas.py`.
- Produces: strict schemas for `autospec.spec.v1`, `autospec.implementation_handoff.v1`, `autospec.review_handoff.v1`, and `autospec.agent_handoff_result.v1`.

- [ ] **Step 1: Write the failing schema tests**

```python
class HandoffSchemaTests(unittest.TestCase):
    def test_valid_fixtures_match_schemas(self):
        for kind in KINDS:
            validate(load_fixture(kind), load_schema(kind))

    def test_approved_spec_rejects_material_questions(self):
        spec = load_fixture("spec")
        spec["material_questions"] = ["Which persistence backend?"]
        with self.assertRaises(Exception):
            validate(spec, load_schema("spec"))

    def test_unknown_fields_are_rejected(self):
        result = load_fixture("result")
        result["conversation"] = "not part of the protocol"
        with self.assertRaises(Exception):
            validate(result, load_schema("result"))
```

- [ ] **Step 2: Run tests to verify RED**

Run: `python3 -m unittest discover -s tests/pi-agent-handoff -p 'test_schemas.py' -v`

Expected: FAIL because the schema and fixture files do not exist.

- [ ] **Step 3: Implement the four strict schemas and valid fixtures**

Each root schema uses:

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "additionalProperties": false,
  "required": ["version", "artifact_id", "created_at", "repository", "producer"]
}
```

The approved-spec conditional is:

```json
{
  "if": {"properties": {"status": {"const": "approved"}}},
  "then": {"properties": {"material_questions": {"maxItems": 0}}}
}
```

- [ ] **Step 4: Run tests to verify GREEN**

Run: `python3 -m unittest discover -s tests/pi-agent-handoff -p 'test_schemas.py' -v`

Expected: all schema tests pass.

- [ ] **Step 5: Commit**

```bash
git add schemas/autospec-*-v1.schema.json tests/pi-agent-handoff
git commit -m "feat: make agent handoffs machine-verifiable"
```

### Task 2: Artifact validation, reconciliation, and derivation CLI

**Files:**
- Create: `scripts/autospec-handoff.py`
- Create: `tests/pi-agent-handoff/test_handoff_cli.py`

**Interfaces:**
- Consumes: the four Task 1 schemas and repository-relative input artifacts.
- Produces: `validate_artifact(kind, path)`, `reconcile_spec(proposal, critique, repo)`, `derive_implementation(spec, issue)`, `derive_review(implementation, closeout, base, head, repo)` plus the documented CLI commands.

- [ ] **Step 1: Write failing behavioral tests**

```python
def test_reconcile_approves_grounded_spec_and_records_source_digests(self):
    result = run_cli("reconcile-spec", "--proposal", proposal, "--critique", critique,
                     "--repo", repo, "--output", output)
    self.assertEqual(result.returncode, 0, result.stderr)
    artifact = load(output)
    self.assertEqual(artifact["status"], "approved")
    self.assertEqual(len(artifact["sources"]), 2)

def test_reconcile_rejects_missing_existing_path(self):
    proposal["affected_surfaces"][0]["state"] = "existing"
    proposal["affected_surfaces"][0]["path"] = "missing.py"
    self.assertError("HANDOFF_EVIDENCE_INSUFFICIENT")

def test_implementation_rejects_unapproved_spec_and_empty_write_scope(self):
    self.assertError("HANDOFF_UNRESOLVED_MATERIAL_QUESTION")
    self.assertError("HANDOFF_SCOPE_INVALID")

def test_review_rejects_lineage_and_scope_mismatch(self):
    self.assertError("HANDOFF_LINEAGE_MISMATCH")
    self.assertError("HANDOFF_SCOPE_INVALID")
```

- [ ] **Step 2: Run tests to verify RED**

Run: `python3 -m unittest discover -s tests/pi-agent-handoff -p 'test_handoff_cli.py' -v`

Expected: FAIL because `scripts/autospec-handoff.py` does not exist.

- [ ] **Step 3: Implement minimal dependency-light artifact engine**

Core digest and path validation:

```python
def digest(value: dict[str, Any]) -> str:
    payload = json.dumps(value, sort_keys=True, separators=(",", ":")).encode()
    return "sha256:" + hashlib.sha256(payload).hexdigest()

def repository_path(root: Path, relative: str) -> Path:
    if Path(relative).is_absolute() or ".." in Path(relative).parts:
        raise HandoffError("HANDOFF_SCOPE_INVALID", relative)
    resolved = (root / relative).resolve()
    if root.resolve() not in (resolved, *resolved.parents):
        raise HandoffError("HANDOFF_SCOPE_INVALID", relative)
    return resolved
```

The CLI returns `0` on success, `1` on malformed data, `3` on fail-closed protocol refusal, and prints exactly one stable error category to stderr.

- [ ] **Step 4: Run tests to verify GREEN**

Run: `python3 -m unittest discover -s tests/pi-agent-handoff -p 'test_handoff_cli.py' -v`

Expected: all artifact CLI tests pass.

- [ ] **Step 5: Commit**

```bash
git add scripts/autospec-handoff.py tests/pi-agent-handoff/test_handoff_cli.py
git commit -m "feat: preserve planning lineage through typed handoffs"
```

### Task 3: Allowlisted Pi bridge adapter

**Files:**
- Create: `scripts/autospec-pi-bridge-dispatch.py`
- Create: `examples/pi-agent-handoff.yml`
- Create: `tests/pi-agent-handoff/test_pi_bridge_adapter.py`

**Interfaces:**
- Consumes: a strict bridge config, role, validated input artifact, installed Pi package inventory, and environment.
- Produces: one validated `autospec-agent-handoff-result-v1` artifact and normalized usage; stable adapter errors otherwise.

- [ ] **Step 1: Write failing adapter tests with a Pi executable stub**

```python
def test_claude_planner_is_isolated_and_read_only(self):
    result = run_adapter(role="intent_planner", tool="AskClaude")
    argv = captured_argv()
    self.assertIn("--mode", argv)
    self.assertIn("json", argv)
    self.assertNotIn("--no-extensions", argv)
    self.assertIn('"mode":"read"', captured_prompt())
    self.assertIn('"isolated":true', captured_prompt())

def test_codex_critic_is_isolated_and_read_only(self):
    result = run_adapter(role="repository_critic", tool="AskCodex")
    self.assertIn('"sandbox":"read-only"', captured_prompt())

def test_missing_or_unpinned_package_fails_closed(self):
    self.assertError("HANDOFF_BRIDGE_UNAVAILABLE")

def test_secret_never_appears_in_argv_prompt_or_output(self):
    self.assertNotIn("super-secret", combined_capture())
```

- [ ] **Step 2: Run tests to verify RED**

Run: `python3 -m unittest discover -s tests/pi-agent-handoff -p 'test_pi_bridge_adapter.py' -v`

Expected: FAIL because the bridge adapter does not exist.

- [ ] **Step 3: Implement adapter and strict example configuration**

The generated Pi invocation uses a private config directory and a logical tool request:

```python
command = [pi, "--mode", "json", "--print", "--no-session", "--no-skills",
           "--no-prompt-templates", "--tools", configured_tool, f"@{prompt_path}"]
```

It validates installed package/version metadata before launch, refuses write-capable modes for planning/review roles, and validates the final result through `autospec-handoff.py` before atomically replacing the output file.

- [ ] **Step 4: Run tests to verify GREEN**

Run: `python3 -m unittest discover -s tests/pi-agent-handoff -p 'test_pi_bridge_adapter.py' -v`

Expected: all adapter tests pass.

- [ ] **Step 5: Commit**

```bash
git add scripts/autospec-pi-bridge-dispatch.py examples/pi-agent-handoff.yml tests/pi-agent-handoff/test_pi_bridge_adapter.py
git commit -m "feat: let Pi consult pinned planning agents safely"
```

### Task 4: End-to-end artifact exchange and compatibility fallback

**Files:**
- Create: `tests/pi-agent-handoff/test_end_to_end.py`
- Create: `tests/pi-agent-handoff/test.sh`
- Modify: `tests/routing-foundation/test_pi_adapter.py`

**Interfaces:**
- Consumes: Tasks 1-3 CLIs and fixtures.
- Produces: a hermetic proof that Claude planning and Codex grounding results become an approved spec, implementation handoff, review handoff, and independent verdict while the legacy Pi invocation remains unchanged.

- [ ] **Step 1: Write failing end-to-end and compatibility tests**

```python
def test_planning_to_independent_review_lineage(self):
    proposal = dispatch_stub("intent_planner", provider="anthropic")
    critique = dispatch_stub("repository_critic", provider="openai")
    spec = reconcile(proposal, critique)
    implementation = derive_implementation(spec, issue)
    review = derive_review(implementation, closeout)
    verdict = dispatch_stub("reviewer", provider="anthropic", input=review)
    self.assert_lineage(spec, implementation, review, verdict)

def test_same_provider_review_fails_required_independence(self):
    self.assertError("HANDOFF_INDEPENDENCE_UNSATISFIED")
```

Add a routing compatibility assertion that the existing adapter still contains `--no-extensions` when no bridge config is supplied.

- [ ] **Step 2: Run tests to verify RED**

Run: `bash tests/pi-agent-handoff/test.sh`

Expected: FAIL until lineage and independence validation cover the complete exchange.

- [ ] **Step 3: Add only the missing integration behavior**

Reuse the public CLI interfaces from Tasks 2 and 3; do not add a second orchestration implementation inside the test.

- [ ] **Step 4: Run protocol and routing tests to verify GREEN**

Run: `bash tests/pi-agent-handoff/test.sh && bash tests/routing-foundation/test.sh`

Expected: both suites pass.

- [ ] **Step 5: Commit**

```bash
git add tests/pi-agent-handoff tests/routing-foundation/test_pi_adapter.py
git commit -m "test: prove specs reach independent implementation review"
```

### Task 5: Skill, installer, and documentation integration

**Files:**
- Modify: `skills/autospec-define/SKILL.md`
- Modify: `skills/autospec-define/opencode/agent.md`
- Modify: `skills/autospec-define/codex/prompt.md`
- Modify: `skills/autospec-define/install.sh`
- Modify: `skills/autospec-run/SKILL.md`
- Modify: `skills/autospec-run/opencode/agent.md`
- Modify: `skills/autospec-run/codex/prompt.md`
- Modify: `skills/autospec-run/install.sh`
- Modify: `docs/CONFIG_REFERENCE.md`
- Modify: `docs/API_REFERENCE.md`
- Modify: `docs/USER_MANUAL.md`
- Modify: `tests/routing-foundation/test_compatibility.bats`
- Create: `tests/pi-agent-handoff/test_docs.py`

**Interfaces:**
- Consumes: installed `autospec-handoff.py`, `autospec-pi-bridge-dispatch.py`, schemas, and optional `AUTOSPEC_PI_HANDOFF_CONFIG`.
- Produces: planning/spec gates in `autospec-define`, implementation/review handoffs in `autospec-run`, installed runtime files, and operator documentation.

- [ ] **Step 1: Write failing lock-step, installer, and docs tests**

```bash
run grep -F 'autospec-agent-handoff-result-v1.schema.json' skills/autospec-run/install.sh
[ "$status" -eq 0 ]
run grep -F 'AUTOSPEC_PI_HANDOFF_CONFIG' docs/CONFIG_REFERENCE.md
[ "$status" -eq 0 ]
```

Python docs tests assert every CLI command and stable error appears in `docs/API_REFERENCE.md`.

- [ ] **Step 2: Run tests to verify RED**

Run: `bats tests/routing-foundation/test_compatibility.bats && python3 -m unittest discover -s tests/pi-agent-handoff -p 'test_docs.py' -v`

Expected: FAIL because installers and docs do not reference the new protocol.

- [ ] **Step 3: Update canonical skill bodies, mirror them byte-identically, and install all files**

Add concise, executable sections to the canonical skill bodies. After editing each canonical body, mechanically copy the body to its OpenCode and Codex mirrors while preserving frontmatter. Extend installer script/schema lists and executable cases.

- [ ] **Step 4: Update operator and API documentation**

Document opt-in configuration, pinned extension inventory, commands, exit codes, artifact locations, independence semantics, rollback, and the extension-free default.

- [ ] **Step 5: Run integration tests and lock-step validation**

Run: `bash tests/pi-agent-handoff/test.sh && bash tests/routing-foundation/test.sh && autospec validate`

Expected: all protocol, compatibility, and lock-step checks pass.

- [ ] **Step 6: Commit**

```bash
git add skills/autospec-define skills/autospec-run docs tests/routing-foundation tests/pi-agent-handoff
git commit -m "feat: route approved specs through Pi agent handoffs"
```

### Task 6: Full verification and completion audit

**Files:**
- Modify only files required by failures attributable to this feature.

**Interfaces:**
- Consumes: complete implementation and repository validation contract.
- Produces: fresh evidence for every acceptance criterion in the design spec.

- [ ] **Step 1: Run syntax and focused protocol checks**

```bash
python3 -m py_compile scripts/autospec-handoff.py scripts/autospec-pi-bridge-dispatch.py
bash tests/pi-agent-handoff/test.sh
bash tests/routing-foundation/test.sh
bash -n skills/autospec-define/install.sh skills/autospec-run/install.sh
```

- [ ] **Step 2: Run repository validation**

```bash
autospec validate
cargo test --workspace
```

- [ ] **Step 3: Audit every design acceptance criterion against evidence**

Record the exact test or source proving each of the seven acceptance criteria from section 12 of the design. Treat missing, indirect, or partial evidence as incomplete and continue fixing.

- [ ] **Step 4: Inspect scoped git state**

```bash
git status --short
git diff --check HEAD~5..HEAD
```

Confirm unrelated pre-existing changes remain untouched and feature changes contain no secrets, placeholders, or generated caches.

- [ ] **Step 5: Commit any verification-only corrections**

```bash
git add scripts/autospec-handoff.py scripts/autospec-pi-bridge-dispatch.py tests/pi-agent-handoff
git commit -m "fix: keep agent handoffs compatible with repository gates"
```
