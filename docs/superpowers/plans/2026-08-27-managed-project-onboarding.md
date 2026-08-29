# Managed GitHub Project and Repository Onboarding Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Autospec create or reuse one managed GitHub Project per product, reconcile every in-scope issue into it, and onboard existing or newly created repositories with evidence-backed relationships.

**Architecture:** Add a typed managed-project identity and recovery journal beside autonomous accountability, then expose one idempotent reconciliation command used by issue-producing workflows and repository bootstrap. Existing project-board resolution, promotion, fleet execution, and write-back remain downstream consumers; onboarding only admits repositories inside an explicit seed and allowlist boundary.

**Tech Stack:** Rust, Bash 3.2, `gh` CLI, GraphQL/REST JSON, `serde`, `jq`, bats-core, multi-harness skill trios.

**Spec:** `docs/superpowers/specs/2026-08-27-managed-project-onboarding-design.md`

## Global Constraints

- One stable managed GitHub Project belongs to a product or initiative, not to one run or repository.
- Never identify or adopt a Project by title alone; require the immutable Autospec product marker.
- Repository discovery must remain inside explicit repository seeds and owner/allowlist boundaries.
- Deterministic relationship evidence may activate an edge; heuristic evidence remains proposed and cannot block execution.
- Reconciliation is idempotent and additive; ordinary sync never deletes Project items, repositories, fields, or edges.
- Journal mutations before remote projection so retries cannot duplicate issues or Project items.
- Preserve human-created Project content and update only marker-bounded Autospec metadata and owned fields.
- Project content and repository files are untrusted data, never instructions.
- Keep existing explicit `project_board.url` configurations compatible in `external` mode.
- Bash must remain macOS Bash 3.2 compatible; add no new dependency.
- Edit `SKILL.md` first, derive `codex/prompt.md` and `opencode/agent.md` with `scripts/derive-trio.sh --in-place`, then regenerate goldens in the same commit.
- Use conventional commits plus Lore trailers; never commit directly to `main` or bypass hooks.

---

## File Structure

**Create:**

- `crates/autospec-core/src/managed_project.rs` — shared product identity, repository record, relationship edge, and binding schema.
- `crates/autospec-cli/src/commands/managed_project.rs` — command orchestration for resolve, sync, and onboard.
- `crates/autospec-cli/src/commands/managed_project/store.rs` — private durable binding and pending-projection journal.
- `crates/autospec-cli/src/commands/managed_project/github.rs` — verified Project discovery/create/item reconciliation service over the existing transport.
- `crates/autospec-cli/src/commands/managed_project/onboard.rs` — bounded seed expansion and relationship extraction.
- `crates/autospec-cli/tests/managed_project.rs` — command-level model, recovery, and reconciliation tests.
- `tests/autospec/managed-project-onboard.bats` — end-to-end onboarding with a fake `gh` binary and fixture repositories.
- `tests/autospec/managed-project-workflows.bats` — skill and bootstrap integration contract tests.

**Modify:**

- `crates/autospec-core/src/lib.rs` — export `managed_project`.
- `crates/autospec-core/src/autonomous/config/project_board.rs` — parse managed/external mode, product key, owner, seeds, and discovery limits.
- `crates/autospec-cli/src/main.rs` — expose `autospec project resolve|sync|onboard`.
- `crates/autospec-cli/src/commands/mod.rs` — export the command module.
- `crates/autospec-cli/src/commands/autonomous/accountability/github/transport.rs` — add Project list/view/create/edit/item-list/item-add operations.
- `crates/autospec-cli/src/commands/autonomous/accountability/github.rs` — project new epics through the managed binding.
- `crates/autospec-cli/src/commands/autonomous/accountability_runtime.rs` — replace numeric `project-map.yml` lookup with managed binding resolution and retain legacy fallback.
- `skills/autospec-project/SKILL.md` and derived mirrors — document and route onboarding/sync.
- `skills/autospec-define/SKILL.md`, `skills/autospec-split/SKILL.md`, `skills/autospec-classify/SKILL.md`, and derived mirrors — call the unified sync after issue creation.
- `skills/autospec/SKILL.md` and derived mirrors — register a repository immediately after verified `gh repo create`.
- `scripts/autospec-control-plane.sh` — register repositories created or adopted by the control-plane bootstrap.
- `scripts/autospec-explore.sh`, `scripts/autonomous-self-improvement.sh`, `scripts/autospec-gap-miner.sh`, `scripts/qa-finding-to-issue.sh`, and `scripts/qa-brute-force-sweep.sh` — reconcile issues created outside define/split/classify.
- `skills/autospec-shared/scripts/autospec-self-issue.sh`, `doc-freshness-tier.sh`, `gap-remediation-loop.sh`, `grow-define-file-issues.sh`, and `repo-quality-audit.sh` — reconcile at shared publisher boundaries.
- `crates/autospec-cli/src/commands/autonomous/tier2_publisher.rs` — reconcile Rust Tier 2 discoveries.
- `install.sh` and the relevant skill install manifests — install the new command/script surfaces.
- `README.md`, `docs/USER_MANUAL.md`, `docs/KNOWN_LIMITATIONS.md`, `AGENTS.md` — document managed Projects and update the old “optional” contract.

---

### Task 1: Define Managed Project Types and Configuration

**Files:**

- Create: `crates/autospec-core/src/managed_project.rs`
- Modify: `crates/autospec-core/src/lib.rs`
- Modify: `crates/autospec-core/src/autonomous/config/project_board.rs`
- Test: unit tests inside both Rust modules

**Interfaces:**

- Produces: `ProjectMode`, `ManagedProjectPolicy`, `ProductKey`, `RepositoryRecord`, `RelationshipKind`, `RelationshipState`, `RelationshipEvidence`, `RelationshipEdge`, and `ManagedProjectBinding`.
- Produces: `ProjectBoardConfig::managed_policy() -> Option<&ManagedProjectPolicy>`.
- Consumes: existing `ProjectBoardConfig` URL, allowlist, write-back, field-map, TTL, and spend-scope behavior.

- [ ] **Step 1: Write failing type and parser tests**

Add tests proving:

```rust
let config = AutonomousConfig::parse(r#"
project_board:
  mode: managed
  product_key: autospec
  owner: berlinguyinca
  repo_allowlist: ["berlinguyinca/autospec", "berlinguyinca/autospec-*" ]
  repository_seeds: ["berlinguyinca/autospec"]
  discovery_max_repos: 25
  write_back: true
"#).unwrap();

let policy = config.project_board.managed_policy().unwrap();
assert_eq!(policy.product_key.as_str(), "autospec");
assert_eq!(policy.owner, "berlinguyinca");
assert_eq!(policy.discovery_max_repos, 25);
```

Also assert that `mode: managed` rejects a missing product key, missing owner, empty seeds,
an invalid key such as `../autospec`, and `discovery_max_repos: 0`; assert that an existing
configuration containing only `url:` parses as `ProjectMode::External`.

- [ ] **Step 2: Run the focused tests and confirm failure**

Run:

```bash
cargo test -p autospec-core managed_project -- --nocapture
cargo test -p autospec-core project_board -- --nocapture
```

Expected: compilation fails because the new types and fields do not exist.

- [ ] **Step 3: Implement the minimal typed model**

Use validated newtypes and serializable records with these stable shapes:

```rust
pub enum ProjectMode { Managed, External }

pub struct ManagedProjectPolicy {
    pub product_key: ProductKey,
    pub owner: String,
    pub repository_seeds: Vec<String>,
    pub repo_allowlist: Vec<String>,
    pub discovery_max_repos: usize,
}

pub enum RelationshipKind {
    Contains,
    DependsOn,
    Implements,
    Tracks,
    SpawnedFrom,
    Blocks,
}

pub enum RelationshipState { Active, Proposed }
```

Define binding schema version `1`. Make the relationship dedupe key contain product key,
kind, normalized source, normalized target, evidence kind, and evidence location.

- [ ] **Step 4: Extend compatibility-preserving parsing**

Parse `mode`, `product_key`, `owner`, `repository_seeds`, and
`discovery_max_repos`. Preserve every existing field and default a URL-only configuration
to external mode. Reject unknown keys and malformed nested values using the module's current
line-numbered error style.

- [ ] **Step 5: Run focused validation**

```bash
cargo fmt --all -- --check
cargo test -p autospec-core managed_project -- --nocapture
cargo test -p autospec-core project_board -- --nocapture
```

Expected: all focused tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/autospec-core/src/lib.rs crates/autospec-core/src/managed_project.rs crates/autospec-core/src/autonomous/config/project_board.rs
git commit -m "feat: define managed project identity"
```

Include `Tested:` and `Scope-risk:` Lore trailers.

---

### Task 2: Add the Durable Binding and Pending-Projection Journal

**Files:**

- Create: `crates/autospec-cli/src/commands/managed_project.rs`
- Create: `crates/autospec-cli/src/commands/managed_project/store.rs`
- Modify: `crates/autospec-cli/src/commands/mod.rs`
- Test: `crates/autospec-cli/tests/managed_project.rs`

**Interfaces:**

- Consumes: `ManagedProjectBinding`, `RepositoryRecord`, and `RelationshipEdge` from Task 1.
- Produces: `ManagedProjectStore::open(root: &Path, product_key: &ProductKey)`, `record_repository`, `record_edge`, `enqueue_projection`, `ack_projection`, and `snapshot`.
- Produces: storage under `${AUTOSPEC_HOME:-~/.autospec}/projects/<product-key>/binding.json` and append-only `events.jsonl`.

- [ ] **Step 1: Write failing store tests**

Test that:

```rust
let mut store = ManagedProjectStore::open(temp.path(), &key("autospec")).unwrap();
store.record_repository(repository("berlinguyinca/autospec", "explicit-seed")).unwrap();
store.enqueue_projection(add_item("https://github.com/berlinguyinca/autospec/issues/42")).unwrap();

let reopened = ManagedProjectStore::open(temp.path(), &key("autospec")).unwrap();
assert_eq!(reopened.snapshot().repositories.len(), 1);
assert_eq!(reopened.snapshot().pending_projections.len(), 1);
```

Add negative tests for symlinked state directories, world-readable binding files on Unix,
truncated JSONL tails, mismatched product keys, and an `ack_projection` retry.

- [ ] **Step 2: Run the test and confirm failure**

```bash
cargo test -p autospec-cli --test managed_project store_ -- --nocapture
```

Expected: compilation fails because `ManagedProjectStore` does not exist.

- [ ] **Step 3: Implement private, atomic storage**

Follow the accountability store's existing private-directory, atomic-write, and fail-closed
symlink patterns. Create directories as `0700`, files as `0600`, append an event before
updating the snapshot, and rebuild the snapshot from the event journal when the JSON snapshot
is missing or stale.

- [ ] **Step 4: Implement idempotency keys**

Use stable projection keys:

```text
project:create:<product-key>
project:item-add:<project-node-id>:<issue-url>
repository:register:<product-key>:<owner/repo>
relationship:<product-key>:<kind>:<source>:<target>:<evidence-digest>
```

Appending an existing key must be a no-op; acknowledging a missing key must fail closed.

- [ ] **Step 5: Run focused tests**

```bash
cargo fmt --all -- --check
cargo test -p autospec-cli --test managed_project store_ -- --nocapture
```

Expected: all store and recovery tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/autospec-cli/src/commands/mod.rs crates/autospec-cli/src/commands/managed_project.rs crates/autospec-cli/src/commands/managed_project/store.rs crates/autospec-cli/tests/managed_project.rs
git commit -m "feat: journal managed project reconciliation"
```

---

### Task 3: Implement Verified GitHub Project Upsert and Item Reconciliation

**Files:**

- Create: `crates/autospec-cli/src/commands/managed_project/github.rs`
- Modify: `crates/autospec-cli/src/commands/autonomous/accountability/github/transport.rs`
- Modify: `crates/autospec-cli/src/commands/managed_project.rs`
- Test: `crates/autospec-cli/tests/managed_project.rs`
- Test: `crates/autospec-cli/tests/autonomous_accountability_github/contracts.rs`

**Interfaces:**

- Consumes: `ManagedProjectStore` and `ManagedProjectPolicy`.
- Produces: `resolve_or_create_project`, `verify_managed_marker`, `reconcile_issue`, and `retry_pending_projections`.
- Extends `GithubCommand` with `ListProjects`, `ViewProject`, `CreateProject`, `EditProjectMarker`, `ListProjectItems`, and owner-explicit `AddToProject`.

- [ ] **Step 1: Write failing transport-contract tests**

Assert exact argv and stdin JSON for:

```rust
GithubCommand::ListProjects { owner: "berlinguyinca".into() }
GithubCommand::CreateProject { owner: "berlinguyinca".into(), title: "Autospec".into() }
GithubCommand::ListProjectItems { owner: "berlinguyinca".into(), number: 7 }
GithubCommand::AddToProject {
    owner: "berlinguyinca".into(),
    project_number: 7,
    issue_url: "https://github.com/berlinguyinca/autospec/issues/42".into(),
}
```

Remove the current implicit `repository.split('/').next()` owner derivation from
`AddToProject`; cross-owner ambiguity must be impossible at the transport boundary.

- [ ] **Step 2: Write failing lifecycle tests with a scripted fake transport**

Cover:

- no verified match causes one create and one marker update;
- one exact product marker is adopted;
- two exact markers fail as ambiguous without mutation;
- a title-only match is ignored;
- owner mismatch fails closed;
- an already-present issue causes zero `item-add` calls;
- failure after issue creation leaves exactly one pending projection; and
- retry acknowledges that projection without creating another issue or item.

- [ ] **Step 3: Run the focused tests and confirm failure**

```bash
cargo test -p autospec-cli --test managed_project github_ -- --nocapture
cargo test -p autospec-cli --test autonomous_accountability_github contracts -- --nocapture
```

Expected: tests fail on missing command variants and lifecycle functions.

- [ ] **Step 4: Implement the immutable marker contract**

Use a marker whose payload is stable and versioned:

```text
<!-- autospec-managed-project:begin -->
schema: 1
product-key: autospec
owner: berlinguyinca
<!-- autospec-managed-project:end -->
```

Parse exactly one complete marker; preserve all content outside the managed block. Verify the
returned Project after create/edit before persisting its node ID and number.

- [ ] **Step 5: Implement idempotent item reconciliation**

List Project items once per reconciliation, normalize issue URLs, enqueue missing additions,
execute them, then acknowledge only successful calls. Treat permission/rate-limit failures as
retryable projection failures; treat marker and owner conflicts as integrity failures.

- [ ] **Step 6: Run focused tests**

```bash
cargo fmt --all -- --check
cargo test -p autospec-cli --test managed_project github_ -- --nocapture
cargo test -p autospec-cli --test autonomous_accountability_github -- --nocapture
```

- [ ] **Step 7: Commit**

```bash
git add crates/autospec-cli/src/commands/managed_project crates/autospec-cli/src/commands/managed_project.rs crates/autospec-cli/src/commands/autonomous/accountability/github/transport.rs crates/autospec-cli/tests/managed_project.rs crates/autospec-cli/tests/autonomous_accountability_github
git commit -m "feat: upsert managed GitHub Projects"
```

---

### Task 4: Add Bounded Existing-Repository Onboarding and Relationship Indexing

**Files:**

- Create: `crates/autospec-cli/src/commands/managed_project/onboard.rs`
- Modify: `crates/autospec-cli/src/commands/managed_project.rs`
- Modify: `crates/autospec-cli/src/main.rs`
- Test: `crates/autospec-cli/tests/managed_project.rs`
- Test: `tests/autospec/managed-project-onboard.bats`

**Interfaces:**

- Produces CLI:

```text
autospec project resolve --repo-dir <path>
autospec project sync --repo-dir <path> [--issue-url <url>]
autospec project onboard --repo-dir <path> [--repo <owner/name>]... [--workspace <path>] [--dry-run]
```

- Produces: `OnboardingReport` with `created`, `adopted`, `updated`, `unchanged`, `proposed`, `out_of_bound`, `inaccessible`, and `pending_projection` counts.
- Consumes: policy, store, and GitHub reconciliation service from Tasks 1–3.

- [ ] **Step 1: Write failing boundary and evidence tests**

Build fixture repositories containing a GitHub `origin`, a `.gitmodules` entry, Cargo path/git
dependencies, npm workspace/package dependencies, and issue-body references. Assert:

- explicit seeds are admitted;
- same-owner allowlisted submodule and manifest repositories are admitted;
- a linked repository outside the allowlist is reported as out-of-bound;
- discovery stops at `discovery_max_repos`;
- repeated onboarding produces the same repository and edge sets;
- exact submodule, manifest, issue, source-spec, tracker, and fleet evidence yields active edges;
- free-text name similarity yields a proposed edge; and
- proposed edges are absent from the active dependency graph.

- [ ] **Step 2: Run the focused tests and confirm failure**

```bash
cargo test -p autospec-cli --test managed_project onboard_ -- --nocapture
bats tests/autospec/managed-project-onboard.bats
```

Expected: command and onboarding tests fail because the scanner and CLI do not exist.

- [ ] **Step 3: Implement canonical repository admission**

Normalize SSH and HTTPS GitHub remotes to `owner/name`. Match only the configured owner and
allowlist patterns. Resolve workspace seeds with `git -C <path> remote get-url origin`; skip
directories without a verified GitHub remote and report them as inaccessible.

- [ ] **Step 4: Implement bounded deterministic scanners**

Read only these sources in the first implementation:

- `.gitmodules`;
- Cargo `git =` and workspace members;
- npm/pnpm/yarn workspace package metadata with repository URLs;
- `go.mod` module/replace entries containing GitHub repositories;
- existing `autospec-fleet.yml` repository URLs;
- exact GitHub issue/PR URLs and `owner/repo#N` references in Autospec-managed issue sections.

Do not recursively execute project code or package managers. Each candidate passes admission
before it enters the queue.

- [ ] **Step 5: Implement edge classification and report JSON**

Map exact evidence to `Active`; map non-unique repository-name references to `Proposed`.
Emit stable JSON with sorted repositories and edges so dry-run output is diffable and testable.

- [ ] **Step 6: Run focused tests**

```bash
cargo fmt --all -- --check
cargo test -p autospec-cli --test managed_project onboard_ -- --nocapture
bats tests/autospec/managed-project-onboard.bats
```

- [ ] **Step 7: Commit**

```bash
git add crates/autospec-cli/src/main.rs crates/autospec-cli/src/commands/managed_project.rs crates/autospec-cli/src/commands/managed_project/onboard.rs crates/autospec-cli/tests/managed_project.rs tests/autospec/managed-project-onboard.bats
git commit -m "feat: onboard existing repository relationships"
```

---

### Task 5: Reconcile Accountability Epics and All Generated Issues

**Files:**

- Modify: `crates/autospec-cli/src/commands/autonomous/accountability/github.rs`
- Modify: `crates/autospec-cli/src/commands/autonomous/accountability_runtime.rs`
- Modify: `skills/autospec-define/SKILL.md` and derived mirrors
- Modify: `skills/autospec-split/SKILL.md` and derived mirrors
- Modify: `skills/autospec-classify/SKILL.md` and derived mirrors
- Modify: `scripts/autospec-explore.sh`
- Modify: `scripts/autonomous-self-improvement.sh`
- Modify: `scripts/autospec-gap-miner.sh`
- Modify: `scripts/qa-finding-to-issue.sh`
- Modify: `scripts/qa-brute-force-sweep.sh`
- Modify: `skills/autospec-shared/scripts/autospec-self-issue.sh`
- Modify: `skills/autospec-shared/scripts/doc-freshness-tier.sh`
- Modify: `skills/autospec-shared/scripts/gap-remediation-loop.sh`
- Modify: `skills/autospec-shared/scripts/grow-define-file-issues.sh`
- Modify: `skills/autospec-shared/scripts/repo-quality-audit.sh`
- Modify: `crates/autospec-cli/src/commands/autonomous/tier2_publisher.rs`
- Test: `crates/autospec-cli/tests/autonomous_accountability_github/binding.rs`
- Test: `tests/autospec/managed-project-workflows.bats`

**Interfaces:**

- Consumes: `autospec project sync --repo-dir <path> --issue-url <url>`.
- Produces: every successfully created or adopted issue has one durable pending-or-acknowledged Project projection.
- Preserves: legacy numeric mappings during migration, but managed binding wins when configured.

- [ ] **Step 1: Write failing accountability tests**

Assert that `finish_binding` uses the verified managed Project owner/number, records assignment
failure as a pending projection, and reports degraded status without failing a successfully
created epic. Add a compatibility test showing URL-only external mode retains the current
optional assignment behavior.

- [ ] **Step 2: Write failing skill contract tests**

For define, split, and classify, assert the canonical `SKILL.md` calls:

```bash
autospec project sync --repo-dir "$PWD" --issue-url "$ISSUE_URL"
```

immediately after successful issue creation/editing, never in dry-run, and records a warning
rather than creating a replacement issue when sync fails.

Add publisher-boundary tests proving the same contract for explore, self-improvement, gap
mining, both QA publishers, all five shared publishers, and Rust Tier 2. Keep
`project-board-control-mirror.sh` excluded because its control marker issues are operational
transport, not product work items.

- [ ] **Step 3: Run tests and confirm failure**

```bash
cargo test -p autospec-cli --test autonomous_accountability_github binding -- --nocapture
bats tests/autospec/managed-project-workflows.bats
bats tests/unit/grow-define-file-issues.bats tests/qa/test_brute_force_sweep.bats tests/unit/qa-filing-origin-self.bats
```

- [ ] **Step 4: Route accountability through the managed binding**

Replace `accountability_project_number()` as the primary lookup with managed policy/store
resolution. Keep the old `~/.autospec/project-map.yml` number as a compatibility fallback only
when no managed policy exists. Never infer owner from the repository string after binding.

- [ ] **Step 5: Update canonical skill bodies and derive mirrors**

Add one shared shell helper, installed with existing shared scripts, that accepts a verified
issue URL and invokes `autospec project sync`. Call it after successful creation at every
publisher boundary listed above. The helper must no-op in dry-run, emit a warning on sync
failure, and never create or edit an issue itself.

Edit only each `SKILL.md`, then run:

```bash
bash scripts/derive-trio.sh skills/autospec-define --in-place
bash scripts/derive-trio.sh skills/autospec-split --in-place
bash scripts/derive-trio.sh skills/autospec-classify --in-place
bash scripts/gen-skill-goldens.sh autospec-define autospec-split autospec-classify
```

- [ ] **Step 6: Run focused validation**

```bash
cargo fmt --all -- --check
cargo test -p autospec-cli --test autonomous_accountability_github -- --nocapture
bats tests/autospec/managed-project-workflows.bats
bats tests/unit/grow-define-file-issues.bats tests/qa/test_brute_force_sweep.bats tests/unit/qa-filing-origin-self.bats
bash scripts/derive-trio.sh skills/autospec-define --check
bash scripts/derive-trio.sh skills/autospec-split --check
bash scripts/derive-trio.sh skills/autospec-classify --check
```

- [ ] **Step 7: Commit**

Stage the Rust files, publisher scripts, shared helper, three complete skill trios, their
goldens, and tests, then commit:

```bash
git commit -m "feat: track every generated issue in its project"
```

---

### Task 6: Register Newly Created Repositories and Expose Onboarding

**Files:**

- Modify: `skills/autospec/SKILL.md` and derived mirrors
- Modify: `scripts/autospec-control-plane.sh`
- Modify: `skills/autospec-define/SKILL.md` and derived mirrors
- Modify: `skills/autospec-split/SKILL.md` and derived mirrors
- Modify: `skills/autospec-project/SKILL.md` and derived mirrors
- Modify: relevant skill `install.sh` files and root `install.sh`
- Test: `tests/autospec/managed-project-workflows.bats`
- Test: `tests/install/project-board-install.bats`

**Interfaces:**

- Consumes: `autospec project onboard` and `autospec project sync` from Task 4.
- Produces: verified `gh repo create` paths immediately register `spawned-from` evidence.
- Produces: `/autospec-project onboard` and `/autospec-project sync` user workflows.

- [ ] **Step 1: Extend failing workflow tests**

Assert that the skill bootstrap contract and `scripts/autospec-control-plane.sh` call
repository registration only after:

```bash
gh repo view "$REPO" --json url,defaultBranchRef
```

succeeds, passing the source spec/run identity when available. Assert failed verification
does not register. Assert `/autospec-project onboard` forwards explicit repository,
organization/allowlist, workspace, and dry-run inputs without shell evaluation.

For control-plane adoption, assert an already-existing verified repository is registered with
`contains` evidence but never receives a false `spawned-from` edge; only a repository created
by the current operation receives `spawned-from`.

- [ ] **Step 2: Run tests and confirm failure**

```bash
bats tests/autospec/managed-project-workflows.bats
bats tests/install/project-board-install.bats
```

- [ ] **Step 3: Update bootstrap contracts**

After remote verification, call the unified command with an exact repository slug and
`spawned-from` evidence. A projection failure emits a warning and remains pending in the
journal; it must not roll back or recreate the GitHub repository.

- [ ] **Step 4: Extend the autospec-project workflow**

Document these exact modes:

```text
/autospec-project onboard --repo owner/name
/autospec-project onboard --workspace /absolute/path
/autospec-project onboard --owner owner --allow owner/repo --allow owner/prefix-*
/autospec-project sync
```

Require an explicit allowlist for owner onboarding. Print the stable reconciliation counts
and the managed Project URL.

- [ ] **Step 5: Derive all changed trios and regenerate goldens**

```bash
for skill in autospec autospec-define autospec-split autospec-project; do
  bash scripts/derive-trio.sh "skills/$skill" --in-place
done
bash scripts/gen-skill-goldens.sh autospec autospec-define autospec-split autospec-project
```

- [ ] **Step 6: Update install manifests and run installation tests**

Ensure clean installs include the new Rust command surface and every shared script/reference
used by the skill. Run:

```bash
bats tests/install/project-board-install.bats
bats tests/autospec/managed-project-workflows.bats
```

- [ ] **Step 7: Commit**

```bash
git commit -m "feat: register repositories with managed projects"
```

Stage only the listed skill trios, goldens, install files, and tests.

---

### Task 7: Document Migration and Run Full Verification

**Files:**

- Modify: `README.md`
- Modify: `docs/USER_MANUAL.md`
- Modify: `docs/KNOWN_LIMITATIONS.md`
- Modify: `AGENTS.md`
- Modify: `CHANGELOG.md`
- Test: existing repository validation surfaces

**Interfaces:**

- Consumes: all prior tasks.
- Produces: operator documentation for managed/external mode, onboarding, reconciliation status, migration, and recovery.

- [ ] **Step 1: Update documentation contracts**

Document:

- one managed Project per product;
- product marker and local state paths;
- greenfield registration and existing-repository onboarding commands;
- explicit seeds and allowlist boundary;
- active versus proposed relationships;
- pending projection recovery;
- external URL compatibility; and
- migration from `~/.autospec/project-map.yml` without deleting that file.

Change `AGENTS.md` so Project assignment is no longer described as optional for managed
products; state that local journaling remains authoritative when GitHub projection is degraded.

- [ ] **Step 2: Run formatting and static validation**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
git diff --check
```

- [ ] **Step 3: Run focused managed-project suites**

```bash
cargo test -p autospec-core managed_project -- --nocapture
cargo test -p autospec-core project_board -- --nocapture
cargo test -p autospec-cli --test managed_project -- --nocapture
cargo test -p autospec-cli --test autonomous_accountability_github -- --nocapture
bats tests/autospec/managed-project-onboard.bats
bats tests/autospec/managed-project-workflows.bats
bats tests/autospec/project-board-resolve.bats tests/autospec/project-board-promoter.bats tests/autospec/project-board-writeback.bats
bats tests/fleet/project-ship.bats
```

- [ ] **Step 4: Run lock-step and shell validation**

```bash
for skill in autospec autospec-define autospec-split autospec-classify autospec-project; do
  bash scripts/derive-trio.sh "skills/$skill" --check
done
bash scripts/gen-skill-goldens.sh autospec autospec-define autospec-split autospec-classify autospec-project
bash -n install.sh skills/autospec/install.sh skills/autospec-define/install.sh skills/autospec-split/install.sh skills/autospec-classify/install.sh skills/autospec-project/install.sh
autospec validate
```

If regenerating goldens changes tracked files, inspect and include them; a second invocation
must produce no diff.

- [ ] **Step 5: Run the required full Rust suite**

```bash
cargo test --workspace --no-fail-fast
```

Expected: every test binary runs and all tests pass.

- [ ] **Step 6: Perform a fixture-backed end-to-end dry run**

With the fake `gh` fixture, run onboarding twice. Verify the second report has zero created
Projects, zero new items, zero duplicate edges, and unchanged active/proposed counts. Simulate
one failed item-add, rerun sync, and verify the pending projection is acknowledged exactly once.

- [ ] **Step 7: Review the final diff for scope and quality**

```bash
git status --short
git diff --stat HEAD~1..HEAD
git diff --check
```

Confirm no unrelated user files are staged, no new dependencies were added, no Project
deletion path exists, and every public CLI/config surface has matching documentation.

- [ ] **Step 8: Commit documentation and verification evidence**

```bash
git add README.md docs/USER_MANUAL.md docs/KNOWN_LIMITATIONS.md AGENTS.md CHANGELOG.md
git commit -m "docs: explain managed project onboarding"
```

Include exact `Tested:` commands and any honest `Not-tested:` gap in Lore trailers.

---

## Completion Criteria

- A new managed product creates one marked GitHub Project and later runs reuse it.
- Every generated issue and accountability epic is either present in that Project or retained
  as one retryable pending projection.
- Newly created repositories are registered after remote verification.
- Existing repositories can be onboarded from explicit repositories, an allowlisted owner,
  or a workspace directory.
- Out-of-bound repositories never enter the index or execution fleet.
- Deterministic relationships are active; ambiguous relationships remain proposed.
- Reconciliation preserves human-managed content and creates no duplicates on repeated runs.
- External Project configurations and legacy project-map fallback continue to work.
- Rust, Bats, lock-step, shell syntax, lint, and full workspace tests pass.
