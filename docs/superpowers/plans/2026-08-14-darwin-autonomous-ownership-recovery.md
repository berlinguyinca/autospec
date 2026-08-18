# Darwin Autonomous Ownership and Recovery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make autonomous mode safely launch, recover, supervise, and explain implementation work on macOS while preserving the Linux ownership path and guaranteeing read-only dry runs.

**Architecture:** A shared platform-process boundary supplies exact boot/start identities, while Linux retains pidfd/subreaper supervision and Darwin gains native identity plus isolated-process-group supervision. Startup heartbeat publication uses the existing descriptor-relative transaction on both Unix platforms, and only the exact incomplete `claimed/heartbeat-pending:none` state is routed to existing CAS recovery. Resume and accountability consume those authoritative outcomes rather than inventing a second ownership mechanism.

**Tech Stack:** Rust 2021, `nix` 0.31.3 and its existing `libc` re-export, Git/GitHub CLI test fixtures, Bash, Bats, macOS `sysctl`/process APIs, Linux `/proc`/pidfd/subreaper.

**Spec:** `docs/superpowers/specs/2026-08-14-darwin-autonomous-ownership-recovery-design.md`

## Global Constraints

- Preserve Linux pidfd, `/proc`, subreaper, and descriptor-relative behavior unchanged.
- Do not add a third-party dependency or invoke `ps`/shell parsing for process identity.
- A process identity is authoritative only when boot ID, process start identity, PID, and required process-group evidence match exactly.
- Treat malformed, mismatched, live, cross-repository, wrong-branch, fresh, or otherwise ambiguous ownership evidence as blocking.
- Only `state=claimed` plus `step=heartbeat-pending:none` may bypass stale lifecycle rejection to reach authoritative CAS recovery.
- `--dry-run` must return before lifecycle acquisition, termination, sentinel mutation, accountability mutation, claim recovery, label mutation, state writes, or process spawn.
- Existing unstaged edits in `autonomous.rs`, its accountability contract test, `resume-scan.sh`, and its Bats test are starting material; review and refine them without discarding unrelated work.
- Use TDD for every behavior change and make one conventional Lore commit after each task.
- Never push directly to `main`, amend a committed PR, bypass hooks, or use a destructive Git reset.

## File Structure

- Create `crates/autospec-cli/src/commands/autonomous/platform_process.rs`: common exact process-birth API with Linux and Darwin native implementations.
- Create `crates/autospec-cli/src/commands/autonomous/executor_bridge/darwin_supervisor.rs`: Darwin launch, process-group observation, signaling, wait, and recovery ownership.
- Modify `crates/autospec-cli/src/commands/autonomous.rs`: pure launch preview, platform-process module wiring, lifecycle recovery event wiring.
- Modify `crates/autospec-cli/src/commands/claim.rs`: lifecycle admission exception, portable heartbeat transaction, and shared process identity use.
- Modify `crates/autospec-cli/src/commands/claim/heartbeat_liveness.rs`: classify heartbeat owner liveness through the shared platform boundary.
- Modify `crates/autospec-cli/src/commands/claim/heartbeat_predecessor.rs`: retire predecessor evidence only through platform-proven ownership.
- Modify `crates/autospec-cli/src/commands/autonomous/executor_bridge.rs`: consume the platform supervisor without changing the Linux implementation.
- Modify `crates/autospec-cli/src/commands/autonomous/executor_bridge/portability.rs`: replace the Darwin fail-closed admission stub with Darwin dispatch; retain fail-closed behavior for unsupported platforms.
- Modify `crates/autospec-cli/src/commands/autonomous/accountability/domain.rs`: add explicit heartbeat-publication and claim-recovery event variants.
- Modify `crates/autospec-cli/src/commands/autonomous/accountability/render.rs`: render recovery decisions into the existing overview, short paragraphs, and Mermaid flow.
- Modify `crates/autospec-cli/src/commands/autonomous/accountability/store.rs`: project new events into the same durable epic state.
- Modify `skills/autospec-resume/scripts/resume-scan.sh`: scan exact stranded claims from both relevant labels and invoke only the Rust CAS recovery boundary.
- Modify the `autospec-resume` adapter trio and README: document identical recovery and dry-run semantics without duplicating ownership logic.
- Extend focused Rust and Bats tests next to each behavior; do not create a parallel integration harness.

---

### Task 1: Make every launch dry run a pure preview

**Files:**
- Modify: `crates/autospec-cli/src/commands/autonomous.rs:270-291,1066-1110,1725-1803`
- Test: `crates/autospec-cli/tests/autonomous_accountability_github/contracts.rs:1-275`
- Test: `crates/autospec-cli/tests/autonomous_conductor_commands.rs:144-180`

**Interfaces:**
- Consumes: parsed `Options` and `LaunchMode` from `autonomous::run`.
- Produces: `fn preview_launch(options: &Options, launch_mode: LaunchMode) -> Result<(), CommandFailure>`; it prints the exact requested subcommand and performs no writes or external calls.

- [ ] **Step 1: Add failing strict no-side-effect tests**

Add one fixture test for `resume --epic 12 --dry-run --json` and one for `restart --dry-run --json`. Snapshot the fixture tree before and after, keep a process-group leader running, and assert:

```rust
let before = snapshot_tree(&fixture.root);
let output = fixture.command("resume")
    .args(["--epic", "12", "--dry-run", "--json"])
    .output()
    .unwrap();
assert!(output.status.success());
assert!(conductor.try_wait().unwrap().is_none());
assert_eq!(snapshot_tree(&fixture.root), before);
assert!(!fixture.gh_calls().exists());
let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
assert_eq!(json["subcommand"], "resume");
assert_eq!(json["status"], "dry-run");
```

The fixture must include a closed epic, an immediate-stop sentinel, and existing conductor metadata so the test would detect termination, epic reopening, sentinel removal, or launch-state writes.

- [ ] **Step 2: Run the focused tests and confirm the current mutation**

Run:

```bash
cargo test -p autospec-cli --test autonomous_accountability_github autonomous_resume_dry_run_is_strictly_read_only -- --exact --nocapture
cargo test -p autospec-cli --test autonomous_conductor_commands restart_dry_run_is_strictly_read_only -- --exact --nocapture
```

Expected before implementation: at least one test fails because `restart`/`resume` reaches lifecycle, process, or accountability mutation.

- [ ] **Step 3: Extract and route through the pure preview**

Make dry-run routing the first subcommand action after option/launch-mode parsing:

```rust
if options.dry_run && matches!(options.subcommand.as_str(), "start" | "restart" | "resume") {
    return preview_launch(&options, launch_mode);
}
match options.subcommand.as_str() {
    "start" => start(options, launch_mode),
    "restart" | "resume" => restart(options),
    // existing non-launch arms remain unchanged
}
```

Move only command construction and rendering into `preview_launch`. It must not call `validate_repo_dir`, `RunLayout::new`, `acquire_lifecycle_start`, `terminate_unit`, `bind_accountability_epic`, or any state writer. Render `options.subcommand` in both JSON and text output.

- [ ] **Step 4: Run the complete launch/accountability focused suites**

Run:

```bash
cargo test -p autospec-cli --test autonomous_accountability_github -- --nocapture
cargo test -p autospec-cli --test autonomous_conductor_commands dry_run -- --nocapture
git diff --check
```

Expected: all focused tests pass; fixture bytes, epic state, stop sentinel, processes, and GitHub call logs remain unchanged.

- [ ] **Step 5: Commit the pure-preview boundary**

```bash
git add crates/autospec-cli/src/commands/autonomous.rs crates/autospec-cli/tests/autonomous_accountability_github/contracts.rs crates/autospec-cli/tests/autonomous_conductor_commands.rs
git commit -m "fix: keep autonomous launch previews read-only" -m "Route start, restart, and resume previews through one pre-mutation boundary and report the requested subcommand accurately.

Constraint: Dry-run cannot acquire ownership, stop processes, write state, or mutate GitHub
Confidence: high
Scope-risk: moderate
Directive: Keep preview_launch free of RunLayout and lifecycle side effects
Tested: Focused autonomous launch and accountability contract tests"
```

### Task 2: Route exact incomplete claims to authoritative recovery

**Files:**
- Modify: `crates/autospec-cli/src/commands/claim.rs:873-983,733-752,1820-1909`
- Test: `crates/autospec-cli/src/commands/claim/tests/paginated_comments.rs`
- Test: `crates/autospec-cli/src/commands/claim/tests/conductor_lease_takeover.rs`
- Test: `crates/autospec-cli/tests/autonomous_conductor_commands.rs:3713-3940`

**Interfaces:**
- Consumes: `RunStateRecord`, requested conductor worker/branch, and existing `recover_authoritative_stale_startup` CAS logic.
- Produces: lifecycle evidence that delegates only exact `claimed/heartbeat-pending:none` records to `acquire_for_conductor`, plus observational recovery metadata returned with the acquired lease:

```rust
pub(crate) struct StartupRecoveryEvidence {
    pub(crate) previous_claim_id: String,
    pub(crate) next_claim_id: String,
}

pub(crate) struct ConductorClaimAcquisition {
    pub(crate) lease: ClaimLease,
    pub(crate) recovery: Option<StartupRecoveryEvidence>,
}
```

No new mutation API is introduced; `recover_authoritative_stale_startup` remains the only recovery writer.

- [ ] **Step 1: Add failing lifecycle evidence unit tests**

Cover the exact exception and its nearest negative controls:

```rust
let pending = owner_record("2026-07-29T00:49:45Z", 1, "heartbeat-pending:none");
let evidence = lifecycle_evidence(&pending).unwrap();
assert_requested_owner_fresh(evidence, "requested-worker", "feat/requested");

for step in ["heartbeat-publishing:none", "heartbeat-pending:abc123", "claimed"] {
    let record = owner_record("2026-07-29T00:49:45Z", 1, step);
    assert_recorded_owner_stale(lifecycle_evidence(&record).unwrap());
}
```

Use existing typed `ClaimEvidence` assertions rather than comparing debug strings.

- [ ] **Step 2: Prove the regression from the ordinary Scan path**

Extend the foreground fixture with a stale exact pending claim and start the conductor in `Scan`, not with a preloaded recovered lease. Assert that the fixture reaches `recover-stale-startup`, advances the generation once, and then reaches `IssueClaimed`. Add a fresh foreign pending case that records deferral and leaves the claim document byte-identical.

Run:

```bash
cargo test -p autospec-cli --lib conductor_lease_takeover -- --nocapture
cargo test -p autospec-cli --test autonomous_conductor_commands stale_pending_startup -- --nocapture
```

Expected before implementation: stale exact pending is rejected as `stale_lease` before the recovery hook is observed.

- [ ] **Step 3: Add the exact lifecycle exception**

Insert the special case after claim-generation validation and before recorded-owner freshness projection:

```rust
if record.state == "claimed" && record.step == "heartbeat-pending:none" {
    return Ok(ClaimEvidence::Observed(ClaimContext::active(
        scope,
        issue,
        requested_worker,
        requested_branch,
        LeaseFreshness::Fresh,
    )));
}
```

Do not modify `recover_authoritative_stale_startup` authority checks, timeout, generation CAS, heartbeat identity, quarantine, or label normalization. Extend its private `RecoveryOutcome` with the recovered prior claim ID, and have `acquire_for_conductor` pair that ID with the newly acquired lease's claim ID in `ConductorClaimAcquisition`. The metadata is observational; the exception changes mutation ordering only.

- [ ] **Step 4: Run recovery and fencing suites**

Run:

```bash
cargo test -p autospec-cli --lib claim::tests::conductor_lease_takeover -- --nocapture
cargo test -p autospec-cli --lib claim::tests::paginated_comments -- --nocapture
cargo test -p autospec-cli --test autonomous_conductor_commands heartbeat_pending -- --nocapture
cargo test -p autospec-cli --test claim_commands recover_stale_startup -- --nocapture
```

Expected: stale exact pending reaches CAS recovery; fresh, live, wrong-generation, wrong-repository, wrong-branch, and non-exact steps remain untouched.

- [ ] **Step 5: Commit the recovery-order repair**

```bash
git add crates/autospec-cli/src/commands/claim.rs crates/autospec-cli/src/commands/claim/tests/paginated_comments.rs crates/autospec-cli/src/commands/claim/tests/conductor_lease_takeover.rs crates/autospec-cli/tests/autonomous_conductor_commands.rs
git commit -m "fix: let stale startup claims reach CAS recovery" -m "Delegate only the exact evidenceless startup sentinel to the existing authoritative acquisition recovery before lifecycle stale rejection.

Constraint: All other stale and heartbeat publication states remain lifecycle-blocking
Rejected: General stale-claim bypass | would weaken ownership fencing
Confidence: high
Scope-risk: narrow
Tested: Claim lifecycle, conductor scan, stale-startup CAS, and negative fencing tests"
```

### Task 3: Recover stranded startup claims during explicit resume

**Files:**
- Modify: `skills/autospec-resume/scripts/resume-scan.sh:120-330`
- Modify: `skills/autospec-resume/SKILL.md`
- Modify: `skills/autospec-resume/opencode/agent.md`
- Modify: `skills/autospec-resume/codex/prompt.md`
- Modify: `skills/autospec-resume/README.md`
- Modify: `skills/autospec-resume/validate.sh`
- Test: `tests/resume/test_resume_scan.bats`

**Interfaces:**
- Consumes: `autospec claim state read` and `autospec claim state recover-stale-startup` JSON.
- Produces: a relaunch candidate only when Rust returns `{"recovered":true,"repo":"acme/widgets","issue":42}` matching the requested repository and issue; dry-run emits intent and invokes neither recovery nor relaunch.

- [ ] **Step 1: Add failing Bats cases for the complete decision matrix**

Add cases for:

```text
auto-implement + stale exact pending + recovered=true  => recover, then relaunch
auto-implement + stale exact pending + recovered=false => recover, no relaunch
auto-implement + fresh exact pending                    => no recovery, no relaunch
auto-implement + malformed/mismatched/non-exact claim  => no recovery, no relaunch
dry-run + stale exact pending                           => report both actions, perform neither
in-progress ordinary stale claim                        => retain existing relaunch behavior
```

The mock must log recovery and relaunch on separate lines so ordering is asserted exactly:

```bash
[ "$(cat "$RECOVERY_LOG")" = $'RECOVER\nRELAUNCH' ]
```

- [ ] **Step 2: Run the Bats suite and confirm missing auto-implement recovery**

Run:

```bash
bats tests/resume/test_resume_scan.bats
```

Expected before implementation: the stale `auto-implement` pending claim is not selected, or dry-run reaches a mutation.

- [ ] **Step 3: Implement selection followed by Rust-owned recovery**

Build a deduplicated scan set from `in-progress-by-bot` and open `auto-implement`, but accept the second label only for exact pending-startup candidates. Validate JSON object shape, `.state`, `.step`, `.repo`, `.issue`, `.updated_at`, and timeout before adding a recovery candidate.

After every no-mutation relaunch gate and after the dry-run return, invoke:

```bash
recovery="$($AUTOSPEC_CLAIM_BIN claim state recover-stale-startup \
    --issue "$issue" --repo "$REPO" --timeout-seconds "$CLAIMED_TIMEOUT" 2>/dev/null)" || continue
printf '%s' "$recovery" | jq -e \
    --arg repo "$REPO" --argjson issue "$issue" \
    'type == "object" and .recovered == true and .repo == $repo and .issue == $issue' \
    >/dev/null || continue
eligible_issues="$eligible_issues $issue"
```

Do not reproduce CAS, liveness, branch, heartbeat, or quarantine logic in Bash.

- [ ] **Step 4: Update adapter documentation and strengthen validation**

In all three harness bodies, state that resume scans both labels, delegates exact stale-startup recovery to Rust, and remains read-only under dry-run. Preserve frontmatter-only differences and the required leading blank line in `codex/prompt.md`.

Extend `validate.sh` to allow only `claim state recover-stale-startup` as a mutation and to assert it occurs textually after the dry-run exit:

```bash
dry_line="$(grep -n 'if \[ "$DRY_RUN" -eq 1 \]' "$SCAN" | cut -d: -f1)"
recover_line="$(grep -n 'claim state recover-stale-startup' "$SCAN" | cut -d: -f1)"
[ "$recover_line" -gt "$dry_line" ] || fail "stale-startup recovery must follow the dry-run boundary"
```

- [ ] **Step 5: Run resume validation and commit**

Run:

```bash
bats tests/resume/test_resume_scan.bats
bash skills/autospec-resume/validate.sh
bash -n skills/autospec-resume/scripts/resume-scan.sh
git diff --check
```

Then commit:

```bash
git add skills/autospec-resume tests/resume/test_resume_scan.bats
git commit -m "fix: resume stranded startup claims safely" -m "Include exact stale startup claims that failed before label swap, while keeping Rust CAS recovery authoritative and dry-run mutation-free.

Constraint: Bash may select candidates but may not implement ownership recovery
Confidence: high
Scope-risk: moderate
Directive: Keep recover-stale-startup after every dry-run and relaunch gate
Tested: Resume Bats suite, skill validation, and bash syntax"
```

### Task 4: Add native Linux/Darwin process-birth identity

**Files:**
- Create: `crates/autospec-cli/src/commands/autonomous/platform_process.rs`
- Modify: `crates/autospec-cli/src/commands/autonomous.rs:60-70`
- Modify: `crates/autospec-cli/src/commands/autonomous/executor_bridge.rs:19942-20076`
- Modify: `crates/autospec-cli/src/commands/claim.rs:4384-4405`
- Modify: `crates/autospec-cli/src/commands/claim/heartbeat_liveness.rs`
- Test: `crates/autospec-cli/src/commands/claim/tests/heartbeat_classify.rs`
- Test: `crates/autospec-cli/src/commands/claim/tests/heartbeat_liveness.rs`
- Test: `crates/autospec-cli/src/commands/autonomous/executor_bridge/portability.rs`

**Interfaces:**
- Produces:

```rust
pub(crate) struct ProcessBirth {
    pub(crate) pid: u32,
    pub(crate) process_group: u32,
    pub(crate) boot_id: String,
    pub(crate) start_identity: String,
}

pub(crate) enum ProcessObservation {
    Exact(ProcessBirth),
    Dead,
    Mismatch,
    Unknown(String),
}

pub(crate) fn current_boot_identity() -> Result<String, String>;
pub(crate) fn observe_birth(pid: u32) -> Result<Option<ProcessBirth>, String>;
pub(crate) fn observe_expected(pid: u32, expected_boot: &str, expected_start: &str)
    -> ProcessObservation;
pub(crate) fn ensure_autonomous_runtime_supported() -> Result<(), String>;
```

- Consumes: Linux `/proc` and `getpgid`; Darwin native `sysctl`/kernel process metadata and `getpgid` through the existing `nix::libc`/`nix` dependency.

- [ ] **Step 1: Add platform-gated identity tests**

Shared tests assert repeated self-observation is stable and nonexistent PIDs are dead. Darwin-only tests assert boot identity and start identity are non-empty canonical decimal/hex strings, and a deliberately altered expected start identity returns `Mismatch`, never `Dead`:

```rust
let birth = observe_birth(std::process::id()).unwrap().unwrap();
assert_eq!(observe_birth(std::process::id()).unwrap(), Some(birth.clone()));
assert!(matches!(
    observe_expected(birth.pid, &birth.boot_id, "different-start"),
    ProcessObservation::Mismatch
));
```

Add a compile-gated unsupported-platform test proving `ensure_autonomous_runtime_supported` rejects before lifecycle layout, claim, or accountability fixtures are created.

- [ ] **Step 2: Run identity tests and confirm Darwin currently errors**

Run:

```bash
cargo test -p autospec-cli platform_process -- --nocapture
cargo test -p autospec-cli startup_heartbeat_process_identity -- --nocapture
```

Expected before implementation on macOS: process identity reports that Linux `/proc` is required.

- [ ] **Step 3: Implement the platform module**

Move the existing Linux `/proc/{pid}/stat`, boot-ID, and `getpgid` logic behind the new API without changing its parsing. On Darwin, obtain boot time with native `sysctl` kernel boot-time metadata and obtain process start time plus group from native process metadata and `getpgid`. Canonicalize the tuple as seconds plus subsecond precision:

```rust
fn canonical_time(seconds: i64, micros: i32) -> Result<String, String> {
    (seconds >= 0 && (0..1_000_000).contains(&micros))
        .then(|| format!("{seconds}.{micros:06}"))
        .ok_or_else(|| "process time identity is out of range".to_string())
}
```

Call each native query twice around the group query and return an error if the immutable start tuple changes. Map `ESRCH` to `Ok(None)` and every permission, truncation, or structure-size ambiguity to `Err`, never to dead.

- [ ] **Step 4: Replace claim and executor local birth probes**

Make `claim::startup_process_identity`, heartbeat liveness, and executor `process_birth_identity` consume the new module. The heartbeat cleanup rule is:

```rust
match platform_process::observe_expected(pid, &boot_id, &process_start) {
    ProcessObservation::Dead => true,
    ProcessObservation::Exact(_) | ProcessObservation::Mismatch | ProcessObservation::Unknown(_) => false,
}
```

Call `ensure_autonomous_runtime_supported` in non-dry-run `start` and `restart` before `RunLayout::new`, lifecycle acquisition, accountability binding, or claim selection. Linux and macOS return `Ok(())`; every other target returns a diagnostic without mutation.

Keep executor executable/argv verification in `executor_bridge`; only birth identity and process group move to the shared boundary.

- [ ] **Step 5: Run identity, liveness, and Linux compile guards**

Run on macOS:

```bash
cargo test -p autospec-cli platform_process -- --nocapture
cargo test -p autospec-cli heartbeat_liveness -- --nocapture
cargo check -p autospec-cli
```

Also require Linux CI to run the existing executor identity and pidfd tests without fixture changes.

- [ ] **Step 6: Commit native process identity**

```bash
git add crates/autospec-cli/src/commands/autonomous/platform_process.rs crates/autospec-cli/src/commands/autonomous.rs crates/autospec-cli/src/commands/autonomous/executor_bridge.rs crates/autospec-cli/src/commands/claim.rs crates/autospec-cli/src/commands/claim/heartbeat_liveness.rs crates/autospec-cli/src/commands/claim/tests/heartbeat_classify.rs crates/autospec-cli/src/commands/claim/tests/heartbeat_liveness.rs crates/autospec-cli/src/commands/autonomous/executor_bridge/portability.rs
git commit -m "feat: prove process birth natively on Darwin" -m "Introduce one exact boot/start/process-group identity boundary for heartbeat recovery and executor supervision while retaining the Linux proc implementation.

Constraint: No shell parsing or new dependency
Rejected: PID-only liveness | PID reuse can transfer cleanup authority
Confidence: medium
Scope-risk: broad
Directive: Treat mismatched or incomplete identity as blocking, never dead
Tested: Platform identity, heartbeat liveness, and autospec-cli compile tests"
```

### Task 5: Publish and retire startup heartbeats portably on Unix

**Files:**
- Modify: `crates/autospec-cli/src/commands/claim.rs:4139-4370,4750-5260`
- Modify: `crates/autospec-cli/src/commands/claim/heartbeat_predecessor.rs`
- Test: `crates/autospec-cli/src/commands/claim/tests/heartbeat_startup.rs`
- Test: `crates/autospec-cli/src/commands/claim/tests/heartbeat_prior.rs`
- Test: `crates/autospec-cli/src/commands/claim/tests/heartbeat_quarantine.rs`
- Test: `crates/autospec-cli/src/commands/claim/tests/support.rs`

**Interfaces:**
- Consumes: `platform_process::{current_boot_identity, observe_expected}` and existing heartbeat document/parser/generation/nonce functions.
- Produces: `publish_startup_heartbeat_transaction_with_hook` and predecessor retirement on both Linux and Darwin; unsupported platforms remain fail closed.

- [ ] **Step 1: Enable the portable Unix tests on Darwin and add race negatives**

Remove Linux-only test gating only where the implementation uses portable Unix descriptors. Cover issue and session publication, mode `0600`, restrictive umask, pre-existing exact generation, symlinked root/repository/target, FIFO, replacement race, parent identity drift, and directory sync failure.

The success assertion must verify both content and binding:

```rust
let stored = fs::read(&issue_path).unwrap();
assert!(same_startup_heartbeat_generation(&stored, expected.as_bytes()));
assert_eq!(fs::metadata(&issue_path).unwrap().mode() & 0o7777, 0o600);
assert!(!fs::symlink_metadata(&issue_path).unwrap().file_type().is_symlink());
```

- [ ] **Step 2: Run the heartbeat suite and confirm Darwin publication is unavailable**

Run:

```bash
cargo test -p autospec-cli heartbeat_startup -- --nocapture
cargo test -p autospec-cli startup_heartbeat_portable_unix -- --nocapture
```

Expected before implementation on macOS: publication returns `heartbeat publisher unavailable`.

- [ ] **Step 3: Generalize the descriptor-relative transaction to Unix**

Retain Linux `openat2` resolution. On Darwin use the existing single-component `openat` route with `O_NOFOLLOW | O_CLOEXEC`, then compare directory device/inode and final name binding before and after publication. Keep the transaction order:

```text
prepare private roots -> open trusted directories -> inspect exact target
-> create 0600 temporary regular file -> write/sync -> atomic publish
-> sync containing directory -> re-read generation and binding
```

Use `renameat2(RENAME_NOREPLACE)` only on Linux. On Darwin publish through a same-directory hard-link/create-once primitive already represented by the existing prepared-file abstraction, then unlink the private temporary name; if the destination appeared concurrently, inspect it and accept only the same generation.

- [ ] **Step 4: Wire write, liveness, quarantine, and predecessor retirement**

Change the writer gate to `#[cfg(unix)]` and retain an explicit unsupported-platform error under `#[cfg(not(unix))]`. Predecessor retirement may unlink/quarantine only when the exact generation matches and `observe_expected` returns `Dead`; `Mismatch` and `Unknown` retain evidence.

- [ ] **Step 5: Run heartbeat security regressions**

Run:

```bash
cargo test -p autospec-cli heartbeat_startup -- --nocapture
cargo test -p autospec-cli heartbeat_prior -- --nocapture
cargo test -p autospec-cli heartbeat_quarantine -- --nocapture
cargo test -p autospec-cli heartbeat_classify -- --nocapture
cargo test -p autospec-cli conductor_lease_takeover -- --nocapture
```

Expected: Darwin publishes and reads exact private heartbeat generations; every ambiguous filesystem or process-identity case remains blocking.

- [ ] **Step 6: Commit portable heartbeat ownership**

```bash
git add crates/autospec-cli/src/commands/claim.rs crates/autospec-cli/src/commands/claim/heartbeat_predecessor.rs crates/autospec-cli/src/commands/claim/tests
git commit -m "feat: publish startup heartbeats safely on Darwin" -m "Reuse the descriptor-relative heartbeat transaction on Unix and gate retirement on exact native process and generation proof.

Constraint: Preserve Linux openat2 behavior and private 0600 state
Rejected: Path-based temporary writes | cannot exclude symlink and replacement races
Confidence: medium
Scope-risk: broad
Directive: Never accept a Darwin binding without pre/post device, inode, owner, mode, and generation checks
Tested: Heartbeat publication, liveness, quarantine, predecessor, and race suites"
```

### Task 6: Supervise Darwin executors with exact process-group ownership

**Files:**
- Create: `crates/autospec-cli/src/commands/autonomous/executor_bridge/darwin_supervisor.rs`
- Modify: `crates/autospec-cli/src/commands/autonomous/executor_bridge.rs:1-70,16112-17640,18223-19065,19942-20110,23010-23035`
- Modify: `crates/autospec-cli/src/commands/autonomous/executor_bridge/portability.rs`
- Test: `crates/autospec-cli/src/commands/autonomous/executor_bridge/tests/harness_supervisor.rs`
- Test: `crates/autospec-cli/src/commands/autonomous/executor_bridge/tests/adoption_cleanup.rs`
- Test: `crates/autospec-cli/src/commands/autonomous/executor_bridge/tests/restart_direct.rs`
- Test: `crates/autospec-cli/src/commands/autonomous/executor_bridge/tests/sidecar_launch.rs`

**Interfaces:**
- Consumes: shared `ProcessIdentity`, `platform_process::observe_expected`, `ValidatedInvocation`, output sinks, invocation journal, and `ClaimRenewalSchedule`.
- Produces:

```rust
pub(super) struct DarwinOwnedGroup {
    leader: ProcessIdentity,
    child: Option<std::process::Child>,
}

impl DarwinOwnedGroup {
    pub(super) fn spawn(harness: &ValidatedInvocation, sinks: &OutputSinkPaths)
        -> Result<Self, SpawnFailure>;
    pub(super) fn adopt(expected: &ProcessIdentity) -> Result<Self, String>;
    pub(super) fn poll(&mut self) -> Result<Option<i32>, String>;
    pub(super) fn terminate(self) -> Result<(), String>;
}
```

- [ ] **Step 1: Add Darwin supervision contract tests**

Under `#[cfg(target_os = "macos")]`, cover:

```text
spawned harness has PGID equal to recorded leader PID
leader and descendant are terminated after interruption
leader identity mismatch refuses killpg
adoption accepts exact live boot/start/PGID and rejects every mismatch
successful leader exit plus empty process group completes
uncertain group membership retains Interrupted evidence
claim renewal loss cleans the exact group and returns OwnershipLost
```

Use a fixture child that traps `TERM` and spawns a descendant so the escalation path is exercised.

- [ ] **Step 2: Run the focused tests and confirm the non-Linux admission stub**

Run:

```bash
cargo test -p autospec-cli executor_bridge::portability -- --nocapture
cargo test -p autospec-cli harness_supervisor -- --nocapture
cargo test -p autospec-cli adoption_cleanup -- --nocapture
```

Expected before implementation on macOS: the bridge reports `executor supervision requires Linux pidfd ownership`.

- [ ] **Step 3: Implement Darwin spawn and exact-group cleanup**

Spawn the harness with a pre-exec `setpgid(0, 0)` and redirected durable sinks. Capture the child PID, then read birth identity twice around PGID verification before persisting it. The cleanup gate must have this form:

```rust
match platform_process::observe_expected(
    self.leader.pid,
    &self.leader.boot_id,
    &self.leader.start_identity,
) {
    ProcessObservation::Exact(birth) if birth.process_group == self.leader.process_group => {
        signal_group(self.leader.process_group, Signal::SIGTERM)?;
        wait_for_empty_group(self.leader.process_group, TERM_GRACE)?;
        if group_exists(self.leader.process_group)? {
            signal_group(self.leader.process_group, Signal::SIGKILL)?;
            wait_for_empty_group(self.leader.process_group, KILL_GRACE)?;
        }
    }
    ProcessObservation::Dead => return Ok(()),
    ProcessObservation::Exact(_) | ProcessObservation::Mismatch | ProcessObservation::Unknown(_) => {
        return Err("executor process group ownership is unverified".to_string());
    }
}
```

Never signal a numeric PID or PGID before the exact leader tuple is re-proven.

- [ ] **Step 4: Dispatch shared bridge flow by platform**

Keep Linux `OwnedProcess`, pidfd, `ScopedChildSubreaper`, post-fork supervisor, and adoption code unchanged behind `#[cfg(target_os = "linux")]`. In `portability.rs`, dispatch macOS `run_executor_bridge`, direct execution, draft creation, reconciliation, and supervision into the existing shared state machine with `DarwinOwnedGroup`; keep a fail-closed stub only for `#[cfg(not(any(target_os = "linux", target_os = "macos")))]`.

Do not claim subreaper semantics on Darwin. Completion requires the direct leader to be reaped and `killpg(pgid, 0)` to report `ESRCH`; `EPERM` or other errors retain recovery evidence.

- [ ] **Step 5: Prove interruption and crash adoption**

Add restart tests that persist an exact Darwin leader identity, drop the original `Child` handle, and adopt through boot/start/PGID proof. Add negative fixtures for modified start identity, modified boot identity, and reused/mismatched PGID; assert the unrelated process survives.

Run:

```bash
cargo test -p autospec-cli harness_supervisor -- --nocapture
cargo test -p autospec-cli adoption_cleanup -- --nocapture
cargo test -p autospec-cli restart_direct -- --nocapture
cargo test -p autospec-cli sidecar_launch -- --nocapture
cargo test -p autospec-cli autonomous_executor_bridge -- --nocapture
```

- [ ] **Step 6: Run Linux source guards and macOS compilation**

Run:

```bash
cargo check -p autospec-cli --all-targets
cargo clippy -p autospec-cli --all-targets -- -D warnings
rg -n "pidfd|PR_SET_CHILD_SUBREAPER|/proc/" crates/autospec-cli/src/commands/autonomous/executor_bridge.rs crates/autospec-cli/src/commands/autonomous/executor_bridge
```

Confirm every pidfd/subreaper/`/proc` occurrence remains Linux-gated and Darwin code uses only the platform interface.

- [ ] **Step 7: Commit Darwin executor supervision**

```bash
git add crates/autospec-cli/src/commands/autonomous/executor_bridge.rs crates/autospec-cli/src/commands/autonomous/executor_bridge/portability.rs crates/autospec-cli/src/commands/autonomous/executor_bridge/darwin_supervisor.rs crates/autospec-cli/src/commands/autonomous/executor_bridge/tests
git commit -m "feat: supervise autonomous executors on Darwin" -m "Add exact process-group ownership for Darwin while retaining Linux pidfd and subreaper supervision as the Linux implementation.

Constraint: Darwin has no subreaper and cleanup must fail closed on identity ambiguity
Rejected: PID or PGID presence alone | reused identifiers can target unrelated processes
Confidence: medium
Scope-risk: broad
Directive: Never signal a Darwin process group before re-proving its leader boot/start identity
Tested: Darwin launch, cleanup, adoption, restart, ownership-loss, check, and clippy suites"
```

### Task 7: Record heartbeat failure and recovery in the same accountability epic

**Files:**
- Modify: `crates/autospec-cli/src/commands/autonomous/accountability/domain.rs:160-250`
- Modify: `crates/autospec-cli/src/commands/autonomous/accountability/store.rs:320-360`
- Modify: `crates/autospec-cli/src/commands/autonomous/accountability/render.rs:1-165`
- Modify: `crates/autospec-cli/src/commands/autonomous.rs:3180-3300`
- Modify: `crates/autospec-cli/src/commands/claim.rs:1820-1909`
- Test: `crates/autospec-cli/src/commands/autonomous/accountability/tests.rs`
- Test: `crates/autospec-cli/tests/autonomous_accountability_github/contracts.rs`
- Test: `crates/autospec-cli/tests/autonomous_conductor_commands.rs`

**Interfaces:**
- Consumes: authoritative heartbeat publication failure and `ConductorClaimAcquisition::recovery` from Task 2.
- Produces:

```rust
EventKind::HeartbeatPublicationDeferred { issue: u64, claim_id: String }
EventKind::StartupClaimRecovered {
    issue: u64,
    previous_claim_id: String,
    next_claim_id: String,
}
```

- [ ] **Step 1: Add failing event round-trip and rendering tests**

Verify JSON serialization/deserialization, journal replay, active recovery state, issue projection, short What/Why/Evidence paragraphs, and a Mermaid edge from deferred publication to recovered claim:

```rust
assert!(projection.body.contains("Heartbeat publication deferred"));
assert!(projection.body.contains("Startup claim recovered"));
assert!(projection.body.contains("deferred_42 --> recovered_42"));
assert_eq!(store.recovery_projection().0, RecoveryState::Active);
```

- [ ] **Step 2: Run accountability tests and confirm events are absent**

Run:

```bash
cargo test -p autospec-cli accountability -- --nocapture
cargo test -p autospec-cli --test autonomous_accountability_github recovery -- --nocapture
```

Expected before implementation: event variants or rendered recovery flow do not exist.

- [ ] **Step 3: Add typed events and preserve the active epic**

Add strict event JSON fields and store transitions. Neither event may mark `RecoveryState::Terminal`. Record publication deferral once when the claim result is `heartbeat_write_failed`; record recovery once only after the CAS returns the previous and successor generation IDs. Use `record_accountability_event_once` with a stable key derived from repository, issue, and generation.

Keep normal `resume --epic N` on `ResumePolicy::ReopenClosed`, append `ResumedFromEpic`, and regenerate the existing projection. Do not create a second epic or GitHub Project dependency.

- [ ] **Step 4: Extend the conductor contract**

Seed the full sequence `Scan -> pending publication failure -> stale CAS recovery -> reacquire -> executor dispatch`. Assert one epic number, one deferred event, one recovered event, a subsequent `IssueClaimed`, and no premature terminal close. Add a dry-run negative asserting no journal or projection event.

- [ ] **Step 5: Run accountability and conductor suites**

Run:

```bash
cargo test -p autospec-cli accountability -- --nocapture
cargo test -p autospec-cli --test autonomous_accountability_github -- --nocapture
cargo test -p autospec-cli --test autonomous_conductor_commands recovery_accountability -- --nocapture
```

- [ ] **Step 6: Commit recovery accountability**

```bash
git add crates/autospec-cli/src/commands/autonomous/accountability crates/autospec-cli/src/commands/autonomous.rs crates/autospec-cli/src/commands/claim.rs crates/autospec-cli/tests/autonomous_accountability_github crates/autospec-cli/tests/autonomous_conductor_commands.rs
git commit -m "feat: explain autonomous claim recovery in its epic" -m "Journal heartbeat publication failure and authoritative generation recovery in the same short-paragraph and Mermaid accountability projection.

Constraint: Accountability observes authoritative outcomes and never becomes a recovery authority
Confidence: high
Scope-risk: moderate
Directive: Keep recovery events idempotent by repository, issue, and claim generation
Tested: Event round-trip, render, GitHub projection, dry-run, and conductor recovery tests"
```

### Task 8: Run cross-platform gates and execute the issue #2686 canary

**Files:**
- Modify: `docs/AUTONOMY-CHARTER.md`
- Modify: `.github/workflows/rust.yml`
- Evidence only: existing accountability epic for issue `#2686`

**Interfaces:**
- Consumes: all seven committed implementation tasks.
- Produces: local macOS proof, Linux CI proof, a merged feature PR, a generation-based reinstall from merged `main`, and an end-to-end `#2686` closeout in the existing epic.

- [ ] **Step 1: Run formatting, static checks, focused shell validation, and the workspace suite**

Run:

```bash
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
bats tests/resume/test_resume_scan.bats
bash skills/autospec-resume/validate.sh
bash -n skills/autospec-resume/scripts/resume-scan.sh
cargo test --workspace
git diff --check
```

Read every failure. If the known macOS executor-bridge baseline failures remain, compare their exact names against a fresh `main` worktree; fix every new failure and record unchanged baseline failures explicitly rather than treating the suite as green.

- [ ] **Step 2: Document the public platform and preview contract**

Add a short `## Native autonomous runtime support` section to `docs/AUTONOMY-CHARTER.md` stating that Linux uses pidfd/subreaper ownership, macOS uses exact boot/start plus isolated process-group ownership, unsupported platforms fail before claims or epics, and every autonomous `--dry-run` is read-only. Keep the charter's safety boundary unchanged.

- [ ] **Step 3: Gate the native Darwin contracts in CI**

Extend `.github/workflows/rust.yml`'s existing `macos-build` job after `cargo check` with the focused tests that do not require external harness credentials:

```yaml
      - name: Test Darwin ownership contracts
        run: |
          cargo test -p autospec-cli platform_process -- --nocapture
          cargo test -p autospec-cli heartbeat_startup -- --nocapture
          cargo test -p autospec-cli harness_supervisor -- --nocapture
          cargo test -p autospec-cli adoption_cleanup -- --nocapture
```

Retain the release build step after these tests so a compile-only success cannot mask a runtime ownership regression.

- [ ] **Step 4: Run repository validation discovery and every applicable gate**

Enumerate checked-in validation entry points, then run the project-standard aggregate command and parity tests:

```bash
autospec validate
cargo test -p autospec-cli --test validation_parity -- --nocapture
actionlint
```

If `autospec validate` names an additional shell gate, run that exact gate and include its output in the PR evidence. Update docs only if `DOC_OUT_OF_SYNC` identifies a changed public surface.

- [ ] **Step 5: Commit docs and CI gates**

```bash
git add docs/AUTONOMY-CHARTER.md .github/workflows/rust.yml
git commit -m "ci: gate native Darwin autonomous ownership" -m "Document the platform ownership boundary and run the focused Darwin identity, heartbeat, supervision, and cleanup contracts before release builds.

Constraint: macOS CI cannot depend on external harness credentials
Confidence: high
Scope-risk: moderate
Directive: Keep runtime ownership tests ahead of the macOS release build
Tested: actionlint and focused Darwin cargo tests"
```

- [ ] **Step 6: Review the complete branch before publication**

Run:

```bash
git status --short
git diff main...HEAD --stat
git diff main...HEAD --check
git log --format='%h %s%n%(trailers)' main..HEAD
```

Confirm every commit is conventional plus Lore-compliant, no unrelated file is staged, Linux-only ownership remains gated, and the design/plan requirements each have test evidence.

- [ ] **Step 7: Publish a feature PR and wait for Linux/macOS CI**

Push `feat/autonomous-recovery-dry-run`, open a PR that links the affected issues and design, and include exact local evidence plus the known baseline comparison. Do not merge until non-advisory required checks pass and independent review returns LGTM.

- [ ] **Step 8: Merge, rebuild, and atomically reinstall from merged main**

After merge, update a clean `main`, rebuild the release binary and skill generation, then use the repository installer so the active Autospec generation points at the merged commit. Verify:

```bash
autospec --version
autospec autonomous resume --epic 3145 --dry-run --json
autospec doctor
```

The dry-run must report `resume` and leave the epic, stop sentinel, local state, and processes unchanged.

- [ ] **Step 9: Resume the existing epic and run the #2686 canary**

Resume the verified stopped epic once without dry-run. Observe until issue `#2686` reaches a feature branch, PR, closeout report, required checks, merge, and issue closure. The existing epic must show the publication/recovery rationale, Mermaid flow, implementation result, and truthful terminal state.

- [ ] **Step 10: Prove clean terminal ownership**

Run read-only status and claim inspection after the canary:

```bash
autospec autonomous status --json
autospec claim state read --issue 2686 --repo berlinguyinca/autospec
git worktree list --porcelain
```

Confirm no stale pending claim, running companion, abandoned executor group, recovery mismatch, or extra accountability epic remains. Report the merged PR, installed generation, canary PR, focused/local/CI evidence, baseline exceptions, and remaining risks.
