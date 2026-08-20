# Contributor-owned Slurm Worker Registration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let uniquely named, contributor-owned Slurm allocations add and remove GPU capacity from the public OpenAI-compatible gateway through direct outbound agent connections, with no SSH inference tunnel.

**Architecture:** Each allocation starts llama.cpp on compute-node loopback and runs `qwen-turing-agent` as a lifecycle-coupled child. The agent opens one control WebSocket and several request pipes to the gateway; control disconnect is the online lease. A workstation-side scheduler adapter retains setup, GPU selection and queue management, but replaces the SSH proxy/provider path with private slot credentials, atomic slot leases and an allocation-owned agent.

**Tech Stack:** Bash 4.3+, Python 3.11+ standard library, Go standard library with `CGO_ENABLED=0`, SQLite, nginx, Slurm CLI, llama.cpp OpenAI-compatible HTTP API.

**Spec:** [`docs/superpowers/specs/2026-08-19-agent-tunnel-self-registration-design.md` §13](../specs/2026-08-19-agent-tunnel-self-registration-design.md#13-amendment-contributor-owned-scheduler-workers)

## Global Constraints

- The repository is public. No real cluster hostname, account, storage path, IP, user name, enrolment token, server credential or API key may enter a committed file.
- The active inference path is `client -> gateway -> reverse WSS pipe -> compute-node loopback`. It contains no SSH forward, workstation holding proxy, inbound compute-node listener or client-specific provider configuration.
- Server IDs match `^[a-z0-9][a-z0-9-]{0,30}$`. Every simultaneously active allocation has one unique ID and one `qts_` credential.
- Credentials live only in private mode-`0600` JSON files. They are never printed, exported with `sbatch --export`, written to job logs or passed on a process command line.
- The agent target is exactly `http://127.0.0.1:<port>` with no path and no upstream API key.
- Initial scheduler workers advertise a non-empty subset of the established seven primary Qwen IDs. This is a Slurm acceptance rule, not a gateway-wide model allow-list.
- A worker identity remains registered while offline. The control connection is its online lease; job termination requires no detach API call.
- Usage rows are accounting history and are never deleted to reset routing measurements. Hardware changes advance a per-server measurement cutoff.
- Older agents that omit `hardware_fingerprint` remain compatible; they simply cannot invalidate stale performance history.
- No new runtime dependency. New Python helpers use the standard library; `llm/agent/go.mod` remains dependency-free.
- Every implementation task follows red-green TDD and ends in a conventional commit. Never amend an existing commit and never bypass hooks.
- Final verification includes `cargo test --workspace --no-fail-fast` plus both node suites and the agent tests.

## File Structure

| File | Responsibility |
|---|---|
| `llm/linux-turing-dual/nginx/qwen-turing.conf` | Expose both the server collection and its enrol/pool/priority/detach subpaths to the gateway. |
| `llm/agent/gpus.go` | Produce stable, privacy-preserving hardware fingerprints from immutable GPU capability fields. |
| `llm/linux-turing-dual/scripts/keystore.py` | Persist hardware generation and exclude pre-change usage from scheduler throughput. |
| `llm/linux-turing-dual/scripts/gateway.py` | Observe agent fingerprint changes and refresh live scheduling evidence immediately. |
| `llm/linux-qwen38/slurm/federation_contract.py` | Validate private agent config and the seven-model initial federation contract. |
| `llm/linux-qwen38/slurm/slot_lock.py` | Atomic shared-filesystem lease for one logical worker slot and Slurm job. |
| `llm/linux-qwen38/slurm/qwen_slurm` | Site-neutral enrol/setup/up/status/logs/stop/slots/verify CLI; no inference transport. |
| `llm/linux-qwen38/slurm/serve-qwen.sbatch` | Allocation-owned llama.cpp plus agent lifecycle. |
| `llm/linux-qwen38/slurm/probe_federation.py` | Online, routed-completion and offline acceptance probe. |
| `llm/linux-qwen38/slurm/opencode_slurm` | One-release deprecation shim to `qwen_slurm`. |
| `llm/linux-qwen38/slurm/legacy/` | Historical SSH proxy and supervisor, never synced or invoked by the active launcher. |

## Execution Preflight

- [ ] Fetch current refs and merge `origin/main` into `feat/turing-dual-qwen-node` before Task 1. Resolve conflicts by preserving both the already-deployed gateway behavior and the current `main` validation contracts; do not force-push or rewrite existing commits.
- [ ] Run the baseline gates before changing production code:

```bash
(cd llm/agent && go test ./...)
bash llm/linux-turing-dual/tests/test_structural.sh
bash llm/linux-qwen38/tests/test_structural.sh
python3 -m pytest -q -p no:cacheprovider \
  llm/linux-turing-dual/tests llm/linux-qwen38/tests
```

- [ ] Record any baseline failure separately. Do not weaken an assertion to make a pre-existing failure disappear.

---

### Task 1: Expose server-management subpaths through nginx

**Files:**
- Modify: `llm/linux-turing-dual/nginx/qwen-turing.conf:197`
- Modify: `llm/linux-turing-dual/tests/test_structural.sh:428`
- Modify: `llm/linux-turing-dual/README.md` current implementation-status note

**Interfaces:**
- Consumes: existing gateway handlers for `/api/servers/enrol` and `/api/servers/<id>/{pool,priority,detach}`.
- Produces: nginx prefix `location ^~ /api/servers/` with the same gateway and forwarded-proto headers as the collection route.

- [ ] **Step 1: Add the failing structural assertion**

```bash
# tests/test_structural.sh
server_prefix=$(awk '/location \^~ \/api\/servers\//,/^    }/' "${NODE}/nginx/qwen-turing.conf")
if ! printf '%s' "$server_prefix" | grep -q 'proxy_pass http://qwen_gateway' \
  || ! printf '%s' "$server_prefix" | grep -q 'proxy_set_header Host \$host' \
  || ! printf '%s' "$server_prefix" | grep -q 'proxy_set_header X-Forwarded-Proto \$scheme'; then
  bad "nginx does not route the whole /api/servers/ management prefix to the gateway"
else
  ok "nginx routes the whole /api/servers/ management prefix to the gateway"
fi
```

- [ ] **Step 2: Run the test and verify the current exact-only route fails**

Run: `bash llm/linux-turing-dual/tests/test_structural.sh`

Expected: one failure containing `whole /api/servers/ management prefix`.

- [ ] **Step 3: Add the minimal nginx prefix location**

```nginx
location ^~ /api/servers/ {
    proxy_pass http://qwen_gateway;
    proxy_set_header Host $host;
    proxy_set_header X-Forwarded-Proto $scheme;
}
```

Keep `location = /api/servers` separate so `/api/serversx` does not match.

- [ ] **Step 4: Run the structural suite**

Run: `bash llm/linux-turing-dual/tests/test_structural.sh`

Expected: `OK -- all structural checks passed`.

- [ ] **Step 5: Commit**

```bash
git add llm/linux-turing-dual/nginx/qwen-turing.conf \
        llm/linux-turing-dual/tests/test_structural.sh \
        llm/linux-turing-dual/README.md
git commit -m "fix(llm): route server management through the gateway"
```

---

### Task 2: Add a stable hardware fingerprint to agent telemetry

**Files:**
- Modify: `llm/agent/gpus.go`
- Create: `llm/agent/gpus_test.go`
- Modify: `llm/agent/README.md`

**Interfaces:**
- Consumes: `[]card` from `readCards()`.
- Produces: `hardwareFingerprint(cards []card) string` and JSON field `hardware_fingerprint` on `capabilities` messages.

The digest input is the sorted multiset of `name + "\x00" + decimal(mem_total_mib)`. It excludes index, UUID, serial, memory used, utilization, temperature and power. Identical card classes/counts keep scheduler history; card class, capacity or count changes invalidate it.

- [ ] **Step 1: Write failing Go tests**

```go
func TestHardwareFingerprintIgnoresVolatileTelemetryAndIndex(t *testing.T) {
	a := []card{{Index: 0, Name: "RTX 6000", MemTotal: 97887, MemUsed: 10, Util: 2}}
	b := []card{{Index: 7, Name: "RTX 6000", MemTotal: 97887, MemUsed: 90000, Util: 99}}
	if hardwareFingerprint(a) != hardwareFingerprint(b) { t.Fatal("volatile telemetry changed fingerprint") }
}

func TestHardwareFingerprintChangesWithCardClassCapacityOrCount(t *testing.T) {
	one := []card{{Name: "RTX 6000", MemTotal: 97887}}
	two := append(append([]card{}, one...), card{Name: "RTX 6000", MemTotal: 97887})
	other := []card{{Name: "A100", MemTotal: 81920}}
	if hardwareFingerprint(one) == hardwareFingerprint(two) { t.Fatal("count was ignored") }
	if hardwareFingerprint(one) == hardwareFingerprint(other) { t.Fatal("hardware change was ignored") }
}

func TestHardwareFingerprintOmitsAnEmptyObservation(t *testing.T) {
	if hardwareFingerprint(nil) != "" { t.Fatal("a failed GPU probe became a hardware generation") }
}

func TestHardwareFingerprintIsIndependentOfCardOrder(t *testing.T) {
	a := []card{{Name: "A100", MemTotal: 81920}, {Name: "L40S", MemTotal: 46068}}
	b := []card{a[1], a[0]}
	if hardwareFingerprint(a) != hardwareFingerprint(b) { t.Fatal("card order changed generation") }
}
```

- [ ] **Step 2: Run the tests and verify the symbol is missing**

Run: `(cd llm/agent && go test ./...)`

Expected: build failure containing `undefined: hardwareFingerprint`.

- [ ] **Step 3: Implement the digest and telemetry field**

```go
type capabilities struct {
	Type                string `json:"type"`
	Cards               []card `json:"cards"`
	HardwareFingerprint string `json:"hardware_fingerprint,omitempty"`
}

func hardwareFingerprint(cards []card) string {
	if len(cards) == 0 { return "" }
	parts := make([]string, 0, len(cards))
	for _, c := range cards { parts = append(parts, c.Name+"\x00"+strconv.Itoa(c.MemTotal)) }
	sort.Strings(parts)
	sum := sha256.Sum256([]byte(strings.Join(parts, "\x00")))
	return "v1:" + hex.EncodeToString(sum[:])
}
```

`sendCapabilities()` reads cards once per interval and sends the cards and digest from the same snapshot.

- [ ] **Step 4: Format, vet and test**

Run: `(cd llm/agent && gofmt -w gpus.go gpus_test.go && go vet ./... && go test ./...)`

Expected: all commands exit 0.

- [ ] **Step 5: Commit**

```bash
git add llm/agent/gpus.go llm/agent/gpus_test.go llm/agent/README.md
git commit -m "feat(llm): fingerprint agent hardware generations"
```

---

### Task 3: Invalidate routing measurements when a worker changes hardware

**Files:**
- Modify: `llm/linux-turing-dual/scripts/keystore.py`
- Modify: `llm/linux-turing-dual/scripts/gateway.py`
- Modify: `llm/linux-turing-dual/tests/test_unit_keystore_servers.py`
- Modify: `llm/linux-turing-dual/tests/test_unit_agent_endpoints.py`
- Modify: `llm/linux-turing-dual/README.md`

**Interfaces:**
- Consumes: optional `hardware_fingerprint` from agent capabilities.
- Produces:

```python
def KeyStore.observe_hardware(server_id: str, fingerprint: str,
                              *, now: float | None = None) -> bool
# True only when a previously established non-empty fingerprint changes.
```

SQLite `servers` gains nullable `hardware_fingerprint` and `throughput_valid_after`. A first observation establishes a baseline. A changed observation advances the cutoff. `throughput()` retains usage rows for accounting but ignores samples before that server-specific cutoff.

- [ ] **Step 1: Add failing migration and throughput tests**

```python
def test_hardware_columns_are_added_to_an_older_database(tmp_path):
    db = tmp_path / "old.sqlite3"
    con = sqlite3.connect(db)
    con.executescript(
        "CREATE TABLE servers (server_id TEXT PRIMARY KEY, sub TEXT, kind TEXT NOT NULL, "
        "base_url TEXT, note TEXT, gpus TEXT, priority INTEGER NOT NULL DEFAULT 0, "
        "pool_member INTEGER NOT NULL DEFAULT 0, secret_hash TEXT, upstream_key TEXT, "
        "created_at TEXT NOT NULL, revoked_at TEXT, last_seen TEXT);"
    )
    con.close()
    store = KeyStore(str(db)); store.migrate_local()
    cols = {r[1] for r in sqlite3.connect(db).execute("PRAGMA table_info(servers)")}
    assert {"hardware_fingerprint", "throughput_valid_after"} <= cols

def test_hardware_change_invalidates_only_that_servers_old_samples(store):
    attach(store, "boxa"); attach(store, "boxb")
    now = time.time()
    _usage(store, "boxa", "m1", 1000, 1000, 10, 100, ts=now - 20)
    _usage(store, "boxb", "m1", 1000, 1000, 10, 100, ts=now - 20)
    assert store.observe_hardware("boxa", "v1:first", now=now - 10) is False
    assert store.observe_hardware("boxa", "v1:second", now=now) is True
    out = store.throughput(now=now + 1)
    assert ("boxa", "m1") not in out and ("boxb", "m1") in out
```

- [ ] **Step 2: Run targeted tests and verify they fail**

Run: `python3 -m pytest -q -p no:cacheprovider llm/linux-turing-dual/tests/test_unit_keystore_servers.py`

Expected: failures for missing columns and `observe_hardware`.

- [ ] **Step 3: Implement the schema migration and cutoff-aware query**

Add both columns to `_SCHEMA`; in `migrate_local()` add missing columns individually. Implement `observe_hardware()` in one SQLite transaction. Change `throughput()` to `LEFT JOIN servers s ON s.server_id=e.upstream` and require `(s.throughput_valid_after IS NULL OR e.ts >= s.throughput_valid_after)`.

Initialize `throughput_valid_after=created_at` when a previously revoked ID is re-enrolled, so a new owner/generation cannot inherit old routing evidence.
Add a test that records old usage, revokes and re-enrols the ID, and proves the
new record has zero scheduler samples while the historical usage row remains.

- [ ] **Step 4: Add the failing live-cache refresh test**

```python
def test_hardware_change_refreshes_live_scheduler_cache(monkeypatch, node):
    _srv, store, _target = node
    attach(_srv, session("sub-a"), "box")
    store.observe_hardware("box", "v1:old")
    called = []
    monkeypatch.setattr(gw, "_refresh_throughput", lambda: called.append(True))
    _said("box", {
        "type": "capabilities", "hardware_fingerprint": "v1:new", "cards": []
    }, monkeypatch)
    assert called == [True]
```

- [ ] **Step 5: Wire `_agent_said()` and run both targeted modules**

Only non-empty fingerprints reach `observe_hardware`. Refresh `THROUGHPUT` immediately only when it returns `True`; ordinary 20-second capability reports remain no-ops.

Run: `python3 -m pytest -q -p no:cacheprovider llm/linux-turing-dual/tests/test_unit_keystore_servers.py llm/linux-turing-dual/tests/test_unit_agent_endpoints.py`

Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
git add llm/linux-turing-dual/scripts/keystore.py \
        llm/linux-turing-dual/scripts/gateway.py \
        llm/linux-turing-dual/tests/test_unit_keystore_servers.py \
        llm/linux-turing-dual/tests/test_unit_agent_endpoints.py \
        llm/linux-turing-dual/README.md
git commit -m "feat(llm): reset routing evidence on hardware changes"
```

---

### Task 4: Define the initial scheduler federation contract

**Files:**
- Create: `llm/linux-qwen38/slurm/federation_contract.py`
- Create: `llm/linux-qwen38/tests/test_unit_slurm_federation.py`
- Modify: `llm/linux-qwen38/slurm/gen-preset.py`
- Modify: `llm/linux-qwen38/tests/test_structural.sh`
- Modify: `llm/linux-qwen38/slurm/README.md`

**Interfaces:**

```python
PRIMARY_MODELS = frozenset({
    "qwen3.8-27b", "qwen3.8-27b-100k", "qwen3.8-27b-vision",
    "qwen3.8-27b-uncensored", "qwen3.8-27b-uncensored-vision",
    "qwen3.5-9b", "qwen3.5-9b-vision",
})
def model_ids(payload: bytes | str | dict) -> set[str]
def validate_scheduler_subset(ids: set[str]) -> set[str]
def load_agent_config(path: Path, expected_port: int) -> dict  # redacted return
```

`load_agent_config` validates mode `0600`, ID regex, non-empty node/credential, and exact loopback target. It returns node, server_id, target and credential presence, never the credential value.

- [ ] **Step 1: Write tests for the exact seven IDs, subset rule and private config**

```python
def test_nonempty_subset_is_accepted():
    assert fc.validate_scheduler_subset({"qwen3.8-27b"}) == {"qwen3.8-27b"}

def test_primary_model_contract_is_exactly_the_established_seven():
    assert fc.PRIMARY_MODELS == {
        "qwen3.8-27b", "qwen3.8-27b-100k", "qwen3.8-27b-vision",
        "qwen3.8-27b-uncensored", "qwen3.8-27b-uncensored-vision",
        "qwen3.5-9b", "qwen3.5-9b-vision",
    }

@pytest.mark.parametrize("ids", [set(), {"qwen3.8-27b-40k"}, {"other-model"}])
def test_empty_alias_or_unknown_model_is_rejected(ids):
    with pytest.raises(ValueError): fc.validate_scheduler_subset(ids)

def test_agent_config_must_be_private_and_loopback_targeted(tmp_path):
    path = write_config(tmp_path, mode=0o644, target="http://0.0.0.0:8080")
    with pytest.raises(ValueError): fc.load_agent_config(path, 8080)
```

Also cover bytes, string and dict model payloads; malformed JSON; non-list
`data`; duplicate IDs; non-string IDs; and every supported-card output from
`gen-preset.py --federated`, asserting a non-empty subset with no aliases.

- [ ] **Step 2: Run the new module and verify collection fails**

Run: `python3 -m pytest -q -p no:cacheprovider llm/linux-qwen38/tests/test_unit_slurm_federation.py`

Expected: import failure for `federation_contract`.

- [ ] **Step 3: Implement the pure validator and `gen-preset.py --federated`**

In federated mode, `emit()` writes primary sections only and omits context aliases. The default output remains byte-compatible with existing tests.

- [ ] **Step 4: Run unit and structural tests**

Run: `python3 -m pytest -q -p no:cacheprovider llm/linux-qwen38/tests/test_unit_slurm_federation.py && bash llm/linux-qwen38/tests/test_structural.sh`

Expected: all pass, including generation for every supported card.

- [ ] **Step 5: Commit**

```bash
git add llm/linux-qwen38/slurm/federation_contract.py \
        llm/linux-qwen38/slurm/gen-preset.py \
        llm/linux-qwen38/tests/test_unit_slurm_federation.py \
        llm/linux-qwen38/tests/test_structural.sh \
        llm/linux-qwen38/slurm/README.md
git commit -m "feat(llm): constrain initial scheduler model supply"
```

---

### Task 5: Add secure revisioned slot state and exact CAS

**Files:**
- Create: `llm/linux-qwen38/slurm/slot_lock.py`
- Create: `llm/linux-qwen38/tests/test_unit_slot_lock.py`
- Modify: `llm/linux-qwen38/slurm/README.md`

**Interfaces:**

```python
@dataclass(frozen=True)
class ExpectedState:
    revision: int
    claim_id: str
    attempt_id: str | None
    serve_job_id: str | None

def claim(root: Path, slot: str, claim_id: str, owner_uid: int, owner_name: str) -> Lease
def mutate(root: Path, slot: str, expected: ExpectedState | None,
           transition: Callable[[dict], dict]) -> Lease
def release_if_current(root: Path, slot: str, claim_id: str,
                       attempt_id: str, serve_job_id: str) -> str
# "released" | "stale_cleanup"; unsafe/corrupt state raises LeaseError.

def main(argv: list[str] | None = None) -> int
# CLI includes: cleanup --root PATH --slot ID --claim ID --attempt ID --job-id ID
```

`expected=None` is legal only for the initial absent -> claimed creation under an
atomic slot-directory `mkdir`. Every mutation of an existing lease supplies its
current revision and the nullable attempt/job values exactly as stored; empty
strings are invalid.

State is `qwen-slurm-slot/v1` and contains `revision`, phase, claim, active and
retiring attempt IDs, and bounded attempt records with unique tags, serve job ID
and cleanup job IDs. Release matches the full tuple `(revision, claim_id,
active_attempt_id, serve_job_id)`.

```json
{
  "schema": "qwen-slurm-slot/v1",
  "slot": "cluster-user-01",
  "claim_id": "32-hex",
  "owner": {"uid": 1000, "user": "user"},
  "revision": 7,
  "phase": "claimed|submitting|active|replacing|stopping|blocked",
  "active_attempt_id": "32-hex|null",
  "retiring_attempt_id": "32-hex|null",
  "attempts": [{
    "attempt_id": "32-hex",
    "ordinal": 0,
    "gpu": "scheduler-gres-name",
    "state": "intent|submit_unknown|bound|cancel_requested|terminal",
    "serve_tag": "qwen-slurm-v1:slot=...,claim=...,attempt=...,role=serve",
    "serve_job_id": "12345|null",
    "cleanup": {"tag": "qwen-slurm-v1:slot=...,claim=...,attempt=...,role=cleanup", "state": "needed|submit_unknown|bound|observed|not_needed", "job_ids": ["12346"]}
  }],
  "created_at": "RFC3339",
  "updated_at": "RFC3339"
}
```

The cleanup process supplies claim/attempt/job IDs, then `release_if_current`
reads the current revision under the mutation mutex and performs the full CAS.
It never embeds a revision captured before its own cleanup job was bound.

- [ ] **Step 1: Write process-concurrency, CAS and stale-cleanup tests**

Use `multiprocessing`, not threads. Assert one winner for one slot, independent
slots proceed, each CAS field rejects independently, an old attempt cannot release
a replacement, an old claim cannot release a new claim, and duplicate cleanup is
an idempotent `stale_cleanup`.

- [ ] **Step 2: Add fail-closed filesystem tests**

Cover symlinks at every path level, wrong owner/mode, hard-linked state,
non-regular state, oversized/truncated/duplicate-key JSON, unknown fields/schema,
crashes before and after `os.replace`, tombstones, and an unprovably stale mutex.
Exercise `main(["cleanup", ...])` for current release, stale-attempt no-op,
duplicate invocation, wrong claim/job, malformed argv, and redacted output.

- [ ] **Step 3: Run and verify the missing module failure**

Run: `python3 -m pytest -q -p no:cacheprovider llm/linux-qwen38/tests/test_unit_slot_lock.py`

- [ ] **Step 4: Implement atomic filesystem semantics**

Validate paths with `lstat` and `O_NOFOLLOW`; require `0700` directories and a
single-link `0600` regular state file owned by the effective UID. Serialize
mutations with atomic sibling `mkdir(.<slot>.txn)`, write a unique same-directory
temp using `O_EXCL`, `fsync`, `os.replace`, and directory `fsync`. Release renames
the exact lease to a generation-named tombstone before deletion.

- [ ] **Step 5: Run tests and commit**

```bash
python3 -m pytest -q -p no:cacheprovider llm/linux-qwen38/tests/test_unit_slot_lock.py
git add llm/linux-qwen38/slurm/slot_lock.py \
        llm/linux-qwen38/tests/test_unit_slot_lock.py \
        llm/linux-qwen38/slurm/README.md
git commit -m "feat(llm): fence scheduler slot generations"
```

---

### Task 6: Accept enrolment tokens from private files

**Files:**
- Modify: `llm/agent/main.go`
- Modify: `llm/agent/config.go`
- Modify: `llm/agent/testsupport_test.go`
- Create: `llm/agent/enrol_test.go`
- Modify: `llm/agent/README.md`

**Interfaces:**
- `enrol --token-file PATH --config PATH`; `--token` remains deprecated for one release.
- `prepareConfigPath(path) error` creates/validates `0700` parent, rejects an
  existing config, and proves a same-directory atomic `0600` write before token redemption.
- `saveConfig` writes a unique temp, fsyncs, renames and fsyncs the parent.

- [ ] **Step 1: Write tests for secret-safe argv and persistence**

Assert mode-`0600` token input, refusal of symlink/wrong-mode token/config paths,
no token or `qts_` in stdout/stderr, atomic config save, existing-config refusal,
and a distinct post-redemption persistence error that names only `server_id` and
instructs the owner to detach before retrying.

- [ ] **Step 2: Run Go tests and verify missing `--token-file` behavior**

Run: `(cd llm/agent && go test ./...)`

- [ ] **Step 3: Implement file input and preflight**

Read at most 8 KiB, trim once, reject empty/non-`qte_` content, and never retain
the token beyond the enrol request. Retry the atomic config save three times after
a successful redemption; if all fail, return the distinct orphaned-enrolment error.

- [ ] **Step 4: Format, vet, test and commit**

```bash
(cd llm/agent && gofmt -w main.go config.go enrol_test.go && go vet ./... && go test ./...)
git add llm/agent/main.go llm/agent/config.go llm/agent/testsupport_test.go \
        llm/agent/enrol_test.go llm/agent/README.md
git commit -m "fix(llm): keep enrolment secrets out of process arguments"
```

---

### Task 7: Add site-neutral config, remote enrolment and atomic artifact deployment

**Files:**
- Create: `llm/linux-qwen38/slurm/qwen_slurm`
- Modify: `llm/linux-qwen38/slurm/site.conf.example`
- Create: `llm/linux-qwen38/tests/test_unit_qwen_slurm.py`
- Modify: `llm/linux-qwen38/tests/test_structural.sh`
- Modify: `llm/linux-qwen38/slurm/README.md`

**Interfaces:**
- Commands implemented here: `enrol`, `slots`, and `sync`.
- Private remote layout: `${QWEN_SLURM_PRIVATE_DIR}/slots/<id>/agent.json` mode
  `0600`; parent directories mode `0700` on contributor-owned shared storage.
- Authoritative lease state: `${QWEN_SLURM_LEASE_DIR}`, defaulting to
  `${QWEN_SLURM_PRIVATE_DIR}/leases`; cleanup receives this path explicitly.
- Enrol input: local mode-`0600` token file, copied to a unique remote private
  temp and consumed by the staged agent's `--token-file` option.

Canonical configuration uses environment over site file over default:

| Variable | Required/default |
|---|---|
| `QWEN_SLURM_SITE_CONF` | `${XDG_CONFIG_HOME:-$HOME/.config}/qwen-slurm/site.conf` |
| `QWEN_SLURM_HOST`, `QWEN_SLURM_USER`, `QWEN_SLURM_ROOT` | required login/shared-root values |
| `QWEN_SLURM_PRIVATE_DIR` | `${QWEN_SLURM_ROOT}/private/${QWEN_SLURM_USER}/qwen-turing-agent` |
| `QWEN_SLURM_LEASE_DIR` | `${QWEN_SLURM_PRIVATE_DIR}/leases` |
| `QWEN_SLURM_ACCOUNT`, `QWEN_SLURM_PARTITION` | required; example partition `low` |
| `QWEN_SLURM_GPU`, `QWEN_SLURM_GPU_CANDIDATES`, `QWEN_SLURM_NGPU` | `auto`, documented candidates, `1` |
| `QWEN_SLURM_TIME`, `QWEN_SLURM_REEVAL_AFTER` | `12:00:00`, `600` seconds |
| `QWEN_SLURM_SLOT`, `QWEN_SLURM_GATEWAY` | required by enrol/up/verify as applicable |
| `QWEN_SLURM_AGENT_ARCH`, `QWEN_SLURM_PORT` | `amd64`, `8080` |
| `QWEN_SLURM_API_KEY_FILE` | required only by verify; mode `0600` |

- [ ] **Step 1: Add fake-SSH/SCP/Go tests**

Assert the token value never enters captured argv/output, the remote directory is
created `0700`, config is written remotely `0600`, existing config is refused,
returned ID equals requested slot, and a post-redemption save failure reports the
detach/retry recovery without printing a credential.

- [ ] **Step 2: Add artifact deployment tests**

Assert source build for selected `linux/{amd64,arm64}`, unique temp names under a
deployment lock, atomic rename to `${ROOT}/bin/qwen-turing-agent`, and sync of only
active helpers. Concurrent syncs must not share `.qwen-turing-agent.tmp`.

- [ ] **Step 3: Implement canonical `QWEN_SLURM_*` config and remote enrolment**

Use existing `OPENCODE_SLURM_*` only as read-only one-release fallbacks. Remote
commands use multiplexed SSH, `bash -lc`, a 45-second outer timeout, and never
embed token/config contents. Remove the remote one-use token file after the agent
returns, regardless of success.

- [ ] **Step 4: Run tests and commit**

```bash
python3 -m pytest -q -p no:cacheprovider llm/linux-qwen38/tests/test_unit_qwen_slurm.py
bash llm/linux-qwen38/tests/test_structural.sh
(cd llm/agent && go test ./...)
git add llm/linux-qwen38/slurm/qwen_slurm llm/linux-qwen38/slurm/site.conf.example \
        llm/linux-qwen38/tests/test_unit_qwen_slurm.py \
        llm/linux-qwen38/tests/test_structural.sh llm/linux-qwen38/slurm/README.md
git commit -m "feat(llm): enrol contributor scheduler slots privately"
```

---

### Task 8: Add authoritative Slurm observation and attempt tags

**Files:**
- Modify: `llm/linux-qwen38/slurm/qwen_slurm`
- Modify: `llm/linux-qwen38/tests/test_unit_qwen_slurm.py`
- Modify: `llm/linux-qwen38/slurm/README.md`

**Interfaces:**
- `serve_tag(slot, claim, attempt)` and `cleanup_tag(...)` use the shell-safe
  grammar `qwen-slurm-v1:slot=ID,claim=HEX,attempt=HEX,role=serve|cleanup`.
- `observe_tag(tag) -> PRESENT(rows) | ABSENT | UNKNOWN | BLOCKED`.
- A terminal absence requires five consecutive authoritative negative snapshots,
  two seconds apart; positive, transport failure, timeout, malformed row or tag
  mismatch resets the sequence.

- [ ] **Step 1: Add a capability preflight for `--comment` and `%k`**

The CLI refuses live scheduling unless `sbatch --comment` and `squeue -o '%k'`
work on the configured site. Test supported and unsupported fake Slurm versions.

- [ ] **Step 2: Write strict observation tests**

Cover four negatives insufficient, fifth negative terminal, transport failure
reset, malformed row unknown, exact bound job with wrong comment blocked, and
PENDING/RUNNING/REQUEUED/SUSPENDED/COMPLETING all present.

- [ ] **Step 3: Implement bounded multiplexed observation**

Run `squeue -h -u USER -n qwen-SLOT -o '%i|%T|%k'` through the existing SSH
control socket and parse every row. Never infer absence from empty stdout unless
SSH, timeout and `squeue` all exit zero.

Every remote argv is assembled by one tested serializer and each dynamic value
must first match its narrow regex. Add a test that the complete tag reaches fake
`sbatch` as one argument; semicolons and unquoted shell metacharacters are refused.

- [ ] **Step 4: Run and commit**

```bash
python3 -m pytest -q -p no:cacheprovider llm/linux-qwen38/tests/test_unit_qwen_slurm.py
git add llm/linux-qwen38/slurm/qwen_slurm \
        llm/linux-qwen38/tests/test_unit_qwen_slurm.py \
        llm/linux-qwen38/slurm/README.md
git commit -m "feat(llm): reconcile scheduler attempts by durable tags"
```

---

### Task 9: Fence serving submission and CPU-only cleanup

**Files:**
- Modify: `llm/linux-qwen38/slurm/qwen_slurm`
- Create: `llm/linux-qwen38/slurm/cleanup-slot.sbatch`
- Modify: `llm/linux-qwen38/tests/test_unit_qwen_slurm.py`
- Modify: `llm/linux-qwen38/tests/test_unit_slot_lock.py`
- Modify: `llm/linux-qwen38/tests/test_structural.sh`

**Interfaces:**
- One physical `sbatch` per attempt ID. Unknown output is reconciled by tag and
  adopted; it is never blindly resubmitted.
- `up` returns success only after one exact serve job and at least one confirmed
  cleanup job are bound in the lease.
- Cleanup calls `release_if_current` with the full generation key.
- Serving `sbatch` uses an explicit allow-list, never `--export=ALL`:
  `QWEN_AGENT_CONFIG`, `QWEN_SLOT_ID`, `QWEN_SLOT_CLAIM`,
  `QWEN_CLUSTER_ROOT`, `QWEN_PORT`, `QWEN_SITE`, and `QWEN_SLURM_NGPU`.

- [ ] **Step 1: Write normal, ambiguous and crash-point tests**

Cover claim -> intent -> submit -> adopt/bind -> cleanup; lost serving response;
zero/one/multiple tagged jobs; lost cleanup response; crash after each durable
transition; and no secret in comments, exports, captured commands or logs.
Assert captured serving submissions contain every required mapping, contain no
`ALL`, and contain no token, credential, API key, or unrelated caller variable.
Add a structural assertion that the active launcher contains no `--export=ALL`.

- [ ] **Step 2: Specify and test the no-GPU cleanup job**

Require configured account/partition, `--dependency=afterany:J`, one task, one
CPU, `--mem=256M`, `--time=00:05:00`, explicit log path, minimal export, no GRES,
and only `QWEN_SLURM_LEASE_DIR` plus slot/claim/attempt/job identifiers. Duplicate
cleanup jobs are harmless. `cleanup-slot.sbatch` invokes staged
`slot_lock.py cleanup --root "$QWEN_SLURM_LEASE_DIR" ...`.

- [ ] **Step 3: Implement submission and reconciliation**

Use `sbatch --parsable --job-name=qwen-<slot> --comment=<serve-tag>`. On ambiguous
result, scan the tag: adopt exactly one; block and report redacted exact IDs for
multiple; create a new attempt only after five negatives. If cleanup cannot be
confirmed, cancel only the exact serve ID, prove absence five times, then release.

- [ ] **Step 4: Run tests and commit**

```bash
python3 -m pytest -q -p no:cacheprovider \
  llm/linux-qwen38/tests/test_unit_qwen_slurm.py \
  llm/linux-qwen38/tests/test_unit_slot_lock.py
git add llm/linux-qwen38/slurm/qwen_slurm \
        llm/linux-qwen38/slurm/cleanup-slot.sbatch \
        llm/linux-qwen38/tests/test_unit_qwen_slurm.py \
        llm/linux-qwen38/tests/test_unit_slot_lock.py \
        llm/linux-qwen38/tests/test_structural.sh
git commit -m "feat(llm): fence serving jobs and cleanup generations"
```

---

### Task 10: Make GPU reevaluation and repair generation-safe

**Files:**
- Modify: `llm/linux-qwen38/slurm/qwen_slurm`
- Modify: `llm/linux-qwen38/tests/test_unit_qwen_slurm.py`
- Modify: `llm/linux-qwen38/tests/test_unit_slot_lock.py`
- Modify: `llm/linux-qwen38/slurm/README.md`

**Interfaces:**
- Commands implemented here: `up`, `status`, `logs`, `stop`, `repair`.
- Initial attempt plus at most two replacements. Requeue retains attempt/job ID.
- Replacement order: cancel all old cleanup IDs -> prove them absent -> recheck
  old serve job -> publish replacement generation -> cancel exact old serve job ->
  prove absent -> submit/bind replacement and cleanup.

- [ ] **Step 1: Write the stale-cleanup and running-race tests**

Assert an old cleanup invoked after replacement bind is a no-op; reevaluation
never submits B while A cleanup or serve exists; if A becomes RUNNING after its
cleanup is canceled, restore A cleanup and abort the switch.

- [ ] **Step 2: Write repair and ambiguous-cancel tests**

Cover unused claim, submit-unknown adoption, missing cleanup resubmission, dead
serve synchronous release, stopping reconciliation, corrupt-state archival,
unique tagged reconstruction, multiple tagged jobs blocking, exact orphan
reporting, and explicit `repair --fix-permissions` only for same-owner mode defects.

- [ ] **Step 3: Implement exact cancellation and repair**

Never use `scancel -n` or user-wide cancellation. `stop` and repair reissue exact
`scancel JOB`, then require five authoritative negatives. No Slurm/SSH call occurs
while the state mutation mutex is held. Symlink or wrong-owner state is never
auto-repaired.

- [ ] **Step 4: Run tests and commit**

```bash
python3 -m pytest -q -p no:cacheprovider \
  llm/linux-qwen38/tests/test_unit_qwen_slurm.py \
  llm/linux-qwen38/tests/test_unit_slot_lock.py
git add llm/linux-qwen38/slurm/qwen_slurm \
        llm/linux-qwen38/tests/test_unit_qwen_slurm.py \
        llm/linux-qwen38/tests/test_unit_slot_lock.py \
        llm/linux-qwen38/slurm/README.md
git commit -m "fix(llm): fence scheduler replacement and repair"
```

---

### Task 11: Couple llama.cpp and the reverse agent to the allocation lifecycle

**Files:**
- Modify: `llm/linux-qwen38/slurm/serve-qwen.sbatch`
- Create: `llm/linux-qwen38/tests/test_unit_serve_job.py`
- Modify: `llm/linux-qwen38/tests/test_structural.sh`
- Modify: `llm/linux-qwen38/slurm/README.md`

**Interfaces:**
- Inputs: `QWEN_AGENT_CONFIG`, `QWEN_SLOT_ID`, `QWEN_SLOT_CLAIM`,
  `QWEN_CLUSTER_ROOT`, `QWEN_PORT`, `QWEN_SITE`, `QWEN_SLURM_NGPU`.
- Children: one llama-server PID and one agent PID. Exit of either terminates the other and fails the job.

- [ ] **Step 1: Add a fake-process lifecycle test**

The test supplies fake `module`, `nvidia-smi`, llama-server, curl, Python and agent executables, runs the job script outside Slurm with a temporary root, and records invocation order.

```python
def test_runtime_is_verified_before_agent_and_binds_loopback(job):
    result = job.run()
    assert result.events.index("models-verified") < result.events.index("agent-start")
    assert result.llama_args.contains("--host", "127.0.0.1")
    assert "0.0.0.0" not in result.llama_args

def test_exit_of_either_child_terminates_the_other(job):
    result = job.run(agent_exit=7)
    assert result.returncode != 0 and result.llama_received_term
```

Add symmetric llama-exit coverage plus TERM/preemption, requeue restart,
TLS-preflight failure, bounded model-poll timeout, malformed/unknown models, and
proof that none of those pre-registration failures starts the agent.

- [ ] **Step 2: Run the test and verify the legacy public-bind/tunnel script fails it**

Run: `python3 -m pytest -q -p no:cacheprovider llm/linux-qwen38/tests/test_unit_serve_job.py`

- [ ] **Step 3: Replace endpoint publication and API-key logic**

Validate the staged agent and private config first. Extract only the gateway host through `federation_contract.load_agent_config()`. Run bounded HTTPS-only `curl --proto '=https' --tlsv1.2 -fsSI --connect-timeout 10 --max-time 20 https://HOST/api/gateway-health`. Remove `QWEN_REQUIRE_KEY`, `api-key.txt`, SSH command publication and `--api-key`; bind llama.cpp to `127.0.0.1`.

Before registration, query allocation-visible `nvidia-smi` directly and require
exactly `QWEN_SLURM_NGPU` cards. After the agent connects, Task 13 compares the
gateway-observed card count to the same value. Node-wide telemetry that includes
unallocated GPUs is a hard failure.

- [ ] **Step 4: Verify models before registration**

Generate `gen-preset.py --federated`, start llama.cpp, poll `http://127.0.0.1:${PORT}/v1/models`, and pass the live response to `validate_scheduler_subset`. Do not start the agent on empty, malformed or unknown model output.

- [ ] **Step 5: Supervise both children**

Install TERM/INT/EXIT traps that signal both PIDs. Start the agent only after validation, then use `wait -n "$srv" "$agent"`; record the first exit status, terminate the surviving child, wait for both, and exit non-zero when either dies unexpectedly.

- [ ] **Step 6: Run tests and parse checks**

Run: `python3 -m pytest -q -p no:cacheprovider llm/linux-qwen38/tests/test_unit_serve_job.py && bash -n llm/linux-qwen38/slurm/serve-qwen.sbatch && bash llm/linux-qwen38/tests/test_structural.sh`

- [ ] **Step 7: Commit**

```bash
git add llm/linux-qwen38/slurm/serve-qwen.sbatch \
        llm/linux-qwen38/tests/test_unit_serve_job.py \
        llm/linux-qwen38/tests/test_structural.sh \
        llm/linux-qwen38/slurm/README.md
git commit -m "feat(llm): register scheduler capacity from the allocation"
```

---

### Task 12: Retire the SSH inference path from active tooling

**Files:**
- Modify: `llm/linux-qwen38/slurm/opencode_slurm`
- Move: `llm/linux-qwen38/slurm/slurm-proxy.py` -> `llm/linux-qwen38/slurm/legacy/slurm-proxy.py`
- Move: `llm/linux-qwen38/slurm/tunnel-supervisor.sh` -> `llm/linux-qwen38/slurm/legacy/tunnel-supervisor.sh`
- Modify: `llm/linux-qwen38/tests/test_structural.sh`
- Modify: `llm/linux-qwen38/slurm/README.md`
- Modify: `llm/QWEN-NODE-SPEC.md`

**Interfaces:**
- `opencode_slurm` prints one deprecation line to stderr and `exec`s sibling `qwen_slurm` with unchanged arguments that remain valid.
- Legacy files are documentation artifacts only: active sync and launcher code never reference them.

- [ ] **Step 1: Add structural assertions that active paths contain no legacy transport**

```bash
active="${SLURM}/qwen_slurm ${SLURM}/serve-qwen.sbatch"
if grep -Eq 'slurm-proxy|tunnel-supervisor|ssh -N|-L .*:|configure-opencode|127\.0\.0\.1:1111[12]' $active; then
  bad "active scheduler path still contains the superseded SSH inference transport"
else
  ok "active scheduler path contains no SSH inference transport"
fi
```

- [ ] **Step 2: Run structural tests and verify they fail before retirement**

Run: `bash llm/linux-qwen38/tests/test_structural.sh`

- [ ] **Step 3: Move historical helpers and replace the launcher with the shim**

```bash
#!/usr/bin/env bash
set -euo pipefail
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
echo "WARN: opencode_slurm is superseded; using qwen_slurm" >&2
exec "${here}/qwen_slurm" "$@"
```

Remove legacy files from artifact sync and active parse/invariant checks; add `bash -n`/Python parse checks under the explicitly historical section.

- [ ] **Step 4: Run structural and CLI tests**

Run: `bash llm/linux-qwen38/tests/test_structural.sh && python3 -m pytest -q -p no:cacheprovider llm/linux-qwen38/tests/test_unit_qwen_slurm.py`

- [ ] **Step 5: Commit**

```bash
git add llm/linux-qwen38/slurm llm/linux-qwen38/tests/test_structural.sh \
        llm/linux-qwen38/slurm/README.md llm/QWEN-NODE-SPEC.md
git commit -m "refactor(llm): retire the scheduler SSH inference path"
```

---

### Task 13: Prove online routing and automatic offline removal

**Files:**
- Create: `llm/linux-qwen38/slurm/probe_federation.py`
- Create: `llm/linux-qwen38/tests/test_unit_probe_federation.py`
- Modify: `llm/linux-qwen38/slurm/qwen_slurm`
- Modify: `llm/linux-qwen38/slurm/README.md`
- Modify: `docs/superpowers/specs/2026-08-19-agent-tunnel-self-registration-design.md` status/evidence

**Interfaces:**
- `probe_federation.py online --gateway URL --slot ID --api-key-file PATH`.
- `probe_federation.py offline --gateway URL --slot ID --timeout-seconds 60`.
- `qwen_slurm verify --slot ID --cancel-after` orchestrates `up`, online probe, exact-job cancellation and offline probe. It never enrols, promotes or detaches a server.

- [ ] **Step 1: Write fake-gateway tests**

```python
def test_online_requires_card_subset_and_routing_header(fake_gateway):
    fake_gateway.servers(slot="cluster-user-01", state="online", cards=[{"name": "RTX 6000"}], models=["qwen3.8-27b"])
    fake_gateway.completion(headers={"X-Routed-To": "cluster-user-01"})
    assert probe.online(fake_gateway.url, "cluster-user-01", fake_gateway.key).ok

@pytest.mark.parametrize("cards", [[], [{"name": "A"}, {"name": "B"}]])
def test_online_requires_exactly_the_allocated_gpu_count(fake_gateway, cards):
    fake_gateway.servers(slot="cluster-user-01", state="online", cards=cards,
                         models=["qwen3.8-27b"])
    with pytest.raises(ProbeError):
        probe.online(fake_gateway.url, "cluster-user-01", fake_gateway.key,
                     expected_gpus=1)

def test_online_rejects_a_completion_served_by_another_worker(fake_gateway):
    fake_gateway.completion(headers={"X-Routed-To": "local"})
    with pytest.raises(ProbeError): probe.online(fake_gateway.url, "cluster-user-01", fake_gateway.key)

def test_offline_requires_state_transition_not_detach(fake_gateway):
    fake_gateway.sequence("cluster-user-01", ["online", "offline"])
    assert probe.offline(fake_gateway.url, "cluster-user-01", timeout=2).ok
```

Also test disappeared server records, online timeout, pinned wrong-model refusal,
routing failure followed by exact cancellation, offline-probe timeout while lease
reconciliation still runs, and duplicate cleanup idempotence.

- [ ] **Step 2: Run tests and verify the probe is absent**

Run: `python3 -m pytest -q -p no:cacheprovider llm/linux-qwen38/tests/test_unit_probe_federation.py`

- [ ] **Step 3: Implement bounded probes with redacted errors**

Online requires the exact slot online, gateway-observed card count exactly equal
to `QWEN_SLURM_NGPU`, a non-empty allowed model subset, a successful pinned
completion, and an ordinary `/v1/chat/completions` response with `X-Routed-To`
equal to the slot. Offline polls `/api/servers` until the same durable record
reports `offline`; disappearance is a failure because it would mean detach, not
lifecycle deregistration.

The ordinary balanced assertion runs only when the test slot has a **strictly
higher admin priority than every other online server eligible for the chosen
model**. `verify` checks this from `/api/servers` and refuses an ambiguous test.
Priority change is an explicit authenticated-dashboard prerequisite, not a CLI
feature. Before allocation, the implementation operator records the prior value
and sets a temporary higher value. After offline/lease cleanup, the operator
restores it through the same dashboard even when verification failed. If no
authenticated admin session exists, stop before allocating. The CLI never
auto-promotes or changes priority.

When `--cancel-after` is present, orchestration wraps every online assertion in a
`finally` path that cancels only the lease-bound job and runs the offline/lease
cleanup checks even when routing verification fails. A failed assertion must not
leave a paid GPU allocation running.

- [ ] **Step 4: Run all local suites**

Run:

```bash
(cd llm/agent && go vet ./... && go test ./...)
bash llm/linux-turing-dual/tests/test_structural.sh
bash llm/linux-qwen38/tests/test_structural.sh
python3 -m pytest -q -p no:cacheprovider \
  llm/linux-turing-dual/tests llm/linux-qwen38/tests
cargo test --workspace --no-fail-fast
```

Expected: every command exits 0.

- [ ] **Step 5: Set the temporary deterministic routing priority**

Using the authenticated admin dashboard, record the slot's current priority and
set it strictly above every other online server eligible for the chosen model.
Re-read `/api/servers` and stop before allocation unless `verify` confirms the
precondition. Store the prior numeric value in the private run log, not Git.

- [ ] **Step 6: Run the authorized live scheduler acceptance test**

Prerequisites: a privately enrolled and administrator-promoted test slot with a
recorded temporary uniquely-highest priority for the chosen model, a private
gateway user-key file, current site config, and no active job bound to that slot.

Run:

```bash
QWEN_SLURM_SLOT=cluster-user-01 \
QWEN_SLURM_TIME=00:15:00 \
QWEN_SLURM_API_KEY_FILE="$HOME/.config/qwen-slurm/probe.key" \
llm/linux-qwen38/slurm/qwen_slurm verify --slot cluster-user-01 --cancel-after
```

Expected evidence, in order: slot lease claimed; exact Slurm job ID submitted; agent appears online with observed GPU and allowed models; pinned completion routes to the slot; ordinary `/v1` completion has `X-Routed-To: cluster-user-01`; exact job is cancelled; durable server becomes offline; slot lease is released.

Also record active attempt ID, serve and cleanup job IDs, five authoritative
negative observations, and a deliberate stale-cleanup call that returns
`stale_cleanup` without releasing the current generation. Restore the prior
server priority after offline and lease checks.

`qwen_slurm verify` performs the stale-cleanup proof while the lease is active by
remotely invoking staged `slot_lock.py cleanup` with the current slot/claim and
deliberately nonmatching attempt/job IDs, then re-reading the unchanged lease.
This is the declared invocation path and never targets another slot.

- [ ] **Step 7: Restore the prior priority**

Use the authenticated admin dashboard to restore the exact value recorded in
Step 5, then re-read `/api/servers`. Perform this closeout even when Step 6 fails;
if restoration is blocked, report that production control-plane change explicitly.

- [ ] **Step 8: Update measured evidence and status without site identifiers**

Record GPU class, model IDs, timestamps-to-online/offline, routed headers, completion result, job cleanup and any scheduler retry. Do not record the real cluster hostname, account, storage path, username, token, key or server credential.

- [ ] **Step 9: Commit**

```bash
git add llm/linux-qwen38/slurm/probe_federation.py \
        llm/linux-qwen38/tests/test_unit_probe_federation.py \
        llm/linux-qwen38/slurm/qwen_slurm \
        llm/linux-qwen38/slurm/README.md \
        docs/superpowers/specs/2026-08-19-agent-tunnel-self-registration-design.md
git commit -m "test(llm): prove scheduler workers join and leave the fleet"
```

---

## Completion Gate

Spec coverage:

| Spec requirement | Implemented by |
|---|---|
| Unique contributor-owned slot identity and duplicate exclusion | Tasks 5, 7, 8, 9 and 10 |
| Allocation-owned agent, loopback runtime and disconnect liveness | Task 11 |
| Private per-slot credential and multi-user isolation | Tasks 4, 5, 6 and 7 |
| Transparent model discovery and initial seven-Qwen subset | Tasks 4, 11 and 13 |
| nginx management-prefix reachability | Task 1 |
| Hardware-generation measurement invalidation | Tasks 2 and 3 |
| No SSH inference transport in the active path | Task 12 |
| Online, routed and offline live evidence | Task 13 |

- [ ] `git status --short` is empty in `/tmp/wt-turing`.
- [ ] `git diff origin/feat/turing-dual-qwen-node...HEAD --check` exits 0.
- [ ] Agent `go vet ./...` and `go test ./...` pass.
- [ ] Both structural suites pass, including management-prefix, no-SSH-active-path, loopback, slot and model-contract assertions.
- [ ] Combined Python node suites pass in one invocation.
- [ ] `cargo test --workspace --no-fail-fast` passes.
- [ ] Independent code review returns no Critical or Important findings.
- [ ] Live acceptance evidence proves online registration, public balanced routing to the contributed GPU, automatic offline transition and exact lease cleanup.
- [ ] No live server credential, API key, site identifier or scheduler account appears in Git history or test output.
