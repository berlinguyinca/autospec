# LLM Operations Dashboard Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the local-model-centric landing view with a trustworthy fleet operations dashboard for loaded models, GPUs, active requests and principals, throughput, queues, and server load.

**Architecture:** Extend the existing fleet API with nullable residency and measured decode data, add a bounded in-memory current-activity API, and compose those independent sources in the framework-free dashboard. Preserve public aggregate visibility while projecting identifiable activity only to authenticated readers.

**Tech Stack:** Python 3 standard library, SQLite-backed `KeyStore`, vanilla HTML/CSS/JavaScript, pytest, shell structural checks, Go agent tests.

**Spec:** `docs/superpowers/specs/2026-08-20-llm-operations-dashboard.md`

## Global Constraints

- No new dependencies.
- Keep `models` as capability; never infer residency when upstream metadata omits it.
- Public projections use positive allow-lists and expose no client identifiers.
- Activity accounting must never block or break inference.
- Every new response body read in the browser goes through `req()`.
- Run both node pytest suites in one invocation.

---

### Task 1: Trustworthy model residency and throughput contracts

**Files:**
- Modify: `llm/linux-turing-dual/scripts/gateway.py`
- Modify: `llm/linux-turing-dual/scripts/publicview.py`
- Modify: `llm/linux-turing-dual/tests/test_unit_gateway_routing.py`
- Modify: `llm/linux-turing-dual/tests/test_unit_gateway_api.py`

**Interfaces:**
- Consumes: raw OpenAI `/v1/models` entries and `KeyStore.throughput()` rows
- Produces: `_model_inventory(data) -> (available, loaded, source)` and server fields `loaded_models`, `loaded_state_source`, freshness timestamps, and `throughput_baseline`

- [ ] **Step 1: Write failing residency parser tests**

```python
def test_model_inventory_separates_available_from_loaded():
    rows = [{"id": "a", "status": {"value": "loaded"}},
            {"id": "b", "status": {"value": "unloaded"}}]
    assert gw._model_inventory(rows) == (["a", "b"], ["a"], "router-status")

def test_model_inventory_keeps_generic_residency_unknown():
    assert gw._model_inventory([{"id": "a"}]) == (["a"], None, "unsupported")
```

- [ ] **Step 2: Run the focused tests and confirm they fail because `_model_inventory` and projected fields do not exist**

Run: `python3 -m pytest -q llm/linux-turing-dual/tests/test_unit_gateway_routing.py llm/linux-turing-dual/tests/test_unit_gateway_api.py`

- [ ] **Step 3: Implement the parser, preserve stale state with source-specific timestamps, project the new fields, and expose the nested measured throughput baseline**

- [ ] **Step 4: Run the focused tests and confirm they pass**

- [ ] **Step 5: Commit**

```bash
git add llm/linux-turing-dual/scripts/gateway.py llm/linux-turing-dual/scripts/publicview.py llm/linux-turing-dual/tests/test_unit_gateway_routing.py llm/linux-turing-dual/tests/test_unit_gateway_api.py
git commit -m "feat(llm): report trustworthy fleet residency"
```

### Task 2: Current gateway request and principal activity

**Files:**
- Create: `llm/linux-turing-dual/scripts/activity.py`
- Create: `llm/linux-turing-dual/tests/test_unit_activity.py`
- Modify: `llm/linux-turing-dual/scripts/gateway.py`
- Modify: `llm/linux-turing-dual/scripts/publicview.py`
- Modify: `llm/linux-turing-dual/nginx/qwen-turing.conf`
- Modify: `llm/linux-turing-dual/tests/test_unit_gateway_api.py`
- Modify: `llm/linux-turing-dual/tests/test_unit_gateway_routing.py`
- Modify: `llm/linux-turing-dual/tests/test_unit_head_requests.py`

**Interfaces:**
- Consumes: authenticated request identity, requested model, chosen upstream, and existing inference `finally` path
- Produces: `ActivityRegistry.begin(...)`, `set_running(request_id)`, `finish(request_id)`, `snapshot()`, and `GET /api/activity`

- [ ] **Step 1: Write failing registry tests for grouping, cleanup, bounded stale pruning, and identity-free public projection**

```python
def test_registry_groups_concurrent_requests_by_principal():
    reg = activity.ActivityRegistry()
    first = reg.begin("key-a", "sub-a", "workstation", "m1", "local", now=10)
    second = reg.begin("key-a", "sub-a", "workstation", "m1", "local", now=11)
    assert reg.snapshot(now=12)["active_principals"] == 1
    assert reg.snapshot(now=12)["active_requests"] == 2
    reg.finish(first); reg.finish(second)
    assert reg.snapshot(now=13)["active_requests"] == 0
```

- [ ] **Step 2: Run activity/API/routing tests and confirm the endpoint and registry are missing**

Run: `python3 -m pytest -q llm/linux-turing-dual/tests/test_unit_activity.py llm/linux-turing-dual/tests/test_unit_gateway_api.py llm/linux-turing-dual/tests/test_unit_gateway_routing.py llm/linux-turing-dual/tests/test_unit_head_requests.py`

- [ ] **Step 3: Implement paired detail/overflow tokens, waiting-to-running lifecycle, public/member/admin projections, routing, and HEAD parity**

- [ ] **Step 4: Add a blocking-upstream integration test proving activity exists only while the real request is in flight**

- [ ] **Step 5: Run focused tests and confirm cleanup holds on success, refusal, and disconnect paths**

- [ ] **Step 6: Commit**

```bash
git add llm/linux-turing-dual/scripts/activity.py llm/linux-turing-dual/tests/test_unit_activity.py llm/linux-turing-dual/scripts/gateway.py llm/linux-turing-dual/scripts/publicview.py llm/linux-turing-dual/nginx/qwen-turing.conf llm/linux-turing-dual/tests/test_unit_gateway_api.py llm/linux-turing-dual/tests/test_unit_gateway_routing.py llm/linux-turing-dual/tests/test_unit_head_requests.py
git commit -m "feat(llm): expose active gateway clients"
```

### Task 3: Consolidated Operations view

**Files:**
- Modify: `llm/linux-turing-dual/web/index.html`
- Create: `llm/linux-turing-dual/tests/test_unit_operations_page.py`
- Modify: `llm/linux-turing-dual/README.md`
- Modify: `llm/linux-turing-dual/docs/measured-ceilings.md`

**Interfaces:**
- Consumes: `/api/stats`, `/api/servers`, `/api/activity`, and existing public/authenticated audience state
- Produces: `renderOperations()`, loaded-model/server/activity summaries, source freshness, and fleet sidebar status

- [ ] **Step 1: Write failing page-contract tests for the Operations route, required panels, privacy copy, and measured/live labels**

```python
def test_operations_page_names_the_signal_boundaries():
    page = INDEX.read_text()
    assert ">Operations<" in page
    assert 'id="loaded-models"' in page
    assert 'id="active-work"' in page
    assert "measured over" in page
    assert "through this gateway" in page
```

- [ ] **Step 2: Run the page test and confirm it fails on the missing Operations markup**

Run: `python3 -m pytest -q llm/linux-turing-dual/tests/test_unit_operations_page.py`

- [ ] **Step 3: Replace the local-model sidebar summary, add the Operations panels, and compose cached source payloads with a non-overlapping poll and no extra raw `fetch()` calls**

- [ ] **Step 4: Update operator documentation with public/private fields and live-versus-measured semantics**

- [ ] **Step 5: Run page, structural, and joint node suites**

Run: `bash llm/linux-turing-dual/tests/test_structural.sh`

Run: `python3 -m pytest -q -p no:cacheprovider llm/linux-turing-dual/tests llm/linux-qwen38/tests`

- [ ] **Step 6: Render the page against representative fixture payloads and inspect desktop and 880 px layouts**

- [ ] **Step 7: Commit**

```bash
git add llm/linux-turing-dual/web/index.html llm/linux-turing-dual/tests/test_unit_operations_page.py llm/linux-turing-dual/README.md llm/linux-turing-dual/docs/measured-ceilings.md llm/linux-turing-dual/DESIGN.md docs/superpowers/specs/2026-08-20-llm-operations-dashboard.md docs/superpowers/plans/2026-08-20-llm-operations-dashboard.md
git commit -m "feat(llm): add fleet operations dashboard"
```

### Task 4: Repository-level verification

**Files:**
- Verify only

**Interfaces:**
- Consumes: completed feature branch
- Produces: fresh evidence for node, agent, Rust workspace, and autospec validation

- [ ] **Step 1: Run the Go agent suite**

Run: `(cd llm/agent && go test ./... && go vet ./...)`

- [ ] **Step 2: Run the Rust workspace without fail-fast masking**

Run: `cargo test --workspace --no-fail-fast`

- [ ] **Step 3: Run repository validation**

Run: `cargo run -p autospec-cli -- validate`

- [ ] **Step 4: Review the final diff for public-data leaks, unsupported claims, and unrelated files**

- [ ] **Step 5: Record the pre-existing baseline failure separately if it remains reproducible**
