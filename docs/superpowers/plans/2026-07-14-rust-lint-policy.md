# Rust Lint Policy Migration Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

> **Amendment (Wave 3 define-time, docs/superpowers/specs/2026-08-04-autospec-web-ui-design.md §L1/§L1a):**
> `UI_SECTIONS_INCOMPLETE` now enforces five `ui-feature` sections — `Design
> reference`, `Interaction states`, `UX flows`, `Motion & feedback`, `Device &
> viewport` — instead of three, in both `scripts/lint-issue.sh` and
> `crates/autospec-core/src/lint/mod.rs`. The `BODY_TOO_LONG` word count
> excludes all five sections (§L1a) so classified UI children do not
> systematically trip `needs-quality-bar` once the two new sections became
> mandatory. `tests/lint/test_lint_issue_ui_sections.bats` (split out of
> `test_lint_issue_sections.bats`, which had grown past the file-size limit) and
> `crates/autospec-core/tests/issue_lint_ui_sections.rs` (likewise split out of
> `issue_lint.rs`) gained coverage for the two new
> sections' presence/absence detection and a positive/negative word-cap
> exclusion pair (a ~400-word non-UI body with all five sections present
> passes; the same non-UI prose alone over 400 words still trips
> `BODY_TOO_LONG`).

**Goal:** Replace `scripts/lint-issue.sh` and `scripts/lint-implementation.sh` with Rust-owned `autospec lint issue` and `autospec lint implementation` commands, then remove every live dependency on the shell linters.

**Architecture:** `autospec-core::lint` becomes the pure policy engine: issue-body section parsing, deterministic findings, diff parsing, escape-hatch parsing, and directive rendering. `autospec-cli::commands::lint` owns CLI parsing and the small impure adapters required to read a diff, staged changes, or GitHub issue/PR data. Callers, generated installer payloads, and pre-commit hooks invoke the installed `autospec` binary directly. Shell scripts are deleted only after source-reachability and behavior-parity gates pass.

**Tech Stack:** Rust 2021 standard library and existing Cargo workspace; existing `git`, `gh`, Bash installer, and Bats fixtures; no new dependencies.

## Contract to preserve

- Issue lint accepts `--json` and a body path (including `-` for stdin), emits the existing RULE_IDs and deterministic order, and exits with the blocking-finding count capped at `64`.
- Implementation lint preserves `PR --issue N`, `--diff-file PATH`, `--pre-commit --staged`, `--directives`, `--vacuous-assertions`, and `--assertion-density`; it emits existing `RULE_ID: detail` / `INFO:RULE_ID: detail` records and keeps exit `200` for scope explosion.
- The deterministic implementation rules are `OUT_OF_SCOPE`, `MISSING_TEST`, `COMPLEXITY`, `SECURITY`, `TODO_LEFT`, `MOCK_DB`, `DOC_OUT_OF_SYNC`, the six `VACUOUS_*` rules, `ASSERTION_DENSITY`, `REINVENT_REPO_UTIL`, `NEW_DEP_UNJUSTIFIED`, and `NEW_ABSTRACTION_SINGLE_CALLER`.
- Existing `Guardian: skip-RULE_ID # reason` behavior and the executable shell's rule-specific same-line/previous-line `linter:allow-RULE_ID reason` behavior remain exact; malformed or reason-less opt-outs do not suppress a finding. The broader documentation mismatch is not silently changed during this parity migration.
- `gh` and `git` calls use direct argument vectors, retain stderr diagnostics, and never run untrusted input through a shell. Offline diff-file and staged modes remain usable without GitHub credentials.
- No generated installer or prompt may retain a live `lint-issue.sh` / `lint-implementation.sh` invocation after cutover. Historical migration documents and intentional negative-reachability assertions may mention the deleted names only under a narrow test allowlist.

## File structure

- Expand: `crates/autospec-core/src/lint/mod.rs` — stable finding model, issue parser, diff model, pure deterministic detectors, suppressions, JSON/text renderers.
- Create: `crates/autospec-core/src/lint/diff.rs` — unified-diff parser and file/hunk model, with no subprocess access.
- Create: `crates/autospec-core/src/lint/implementation.rs` — implementation policy rules, directive mapping, and injected post-change repository snapshots.
- Expand: `crates/autospec-core/tests/issue_lint.rs` — every issue RULE_ID, order, JSON, stdin-equivalent body fixtures.
- Create: `crates/autospec-core/tests/implementation_lint.rs` — minimal synthetic diff/issue bodies covering every implementation RULE_ID and suppression form.
- Create: `crates/autospec-cli/src/commands/lint.rs` — `autospec lint issue|implementation` option parsing, direct GitHub/Git adapters, exit mapping.
- Modify: `crates/autospec-cli/src/commands/{mod.rs,run.rs}` and `crates/autospec-cli/tests/cli_commands.rs` — top-level dispatch, command help, and public CLI tests.
- Modify: linter Bats suites under `tests/{lint,unit}/`, guardian/pre-commit integration fixtures, and validation catalog fixtures to execute the Rust command.
- Modify: `install.sh`, `scripts/install-implementer-precommit.sh`, `scripts/{groom-validate.sh,qa-phase4.sh,self-enforce-qa.sh}`, and the affected lock-step skill trios/installers — use the installed binary and retain only narrow shell orchestration.
- Modify: `docs/{API_REFERENCE.md,cli-reference.md}` and relevant runbooks; regenerate changed skill goldens with the repository generator.
- Delete: `scripts/lint-issue.sh`, `scripts/lint-implementation.sh` after the final reachability test proves no live source caller remains.

## Task 1: Freeze the complete issue-lint contract

**Files:**
- Modify: `crates/autospec-core/src/lint/mod.rs`
- Modify: `crates/autospec-core/tests/issue_lint.rs`
- Test: `tests/unit/test_lint_issue.bats`, `tests/lint/test_lint_issue_sections.bats`,
  `tests/lint/test_lint_issue_ui_sections.bats`

- [ ] **Step 1: Add failing Rust cases for the shell-only rules.**

Add one minimal body fixture per missing `GOAL_*`, `AC_*`, `SMOKE_*`, `MISSING_SECTION_*`, `DEPS_MALFORMED`, `TOO_MANY_FILES`, `BODY_TOO_LONG`, `OUTLINE_TOO_LONG`, and `UI_SECTIONS_INCOMPLETE` finding. Assert exact ordered `(rule_id, message)` tuples for a multi-failure fixture and assert a valid issue produces none.

- [ ] **Step 2: Confirm RED.**

Run: `cargo test -p autospec-core --test issue_lint -- --nocapture`

Expected: new tests fail only for missing policy coverage, demonstrating the current three-rule Rust subset is incomplete.

- [ ] **Step 3: Implement a pure issue document parser and complete rule set.**

Use line-oriented heading/fence parsing shared by all issue rules. Represent findings as `LintFinding { rule_id, severity, message }`; do not make filesystem, process, or GitHub calls from core. Preserve the shell's rule ordering and exact limits rather than "improving" the policy during conversion.

- [ ] **Step 4: Verify core parity.**

Run: `cargo test -p autospec-core --test issue_lint && bats tests/unit/test_lint_issue.bats tests/lint/test_lint_issue_sections.bats tests/lint/test_lint_issue_ui_sections.bats`

- [ ] **Step 5: Commit the pure issue policy.**

```bash
git add crates/autospec-core/src/lint/mod.rs crates/autospec-core/tests/issue_lint.rs
git commit -m "feat: complete Rust issue quality policy"
```

## Task 2: Add the public Rust issue-lint command

**Files:**
- Create: `crates/autospec-cli/src/commands/lint.rs`
- Modify: `crates/autospec-cli/src/commands/mod.rs`
- Modify: `crates/autospec-cli/tests/cli_commands.rs`

- [ ] **Step 1: Add failing CLI tests.**

Cover `autospec lint issue BODY`, `autospec lint issue --json BODY`, `autospec lint issue -` with piped stdin, `--help`, malformed options, exact exit count, and JSON including all ordered findings.

- [ ] **Step 2: Confirm RED.**

Run: `cargo test -p autospec-cli --test cli_commands lint_issue -- --nocapture`

Expected: `autospec lint` is an unknown command.

- [ ] **Step 3: Implement direct CLI dispatch.**

Add `lint` to top-level help and dispatch. Read the body once, call the pure core policy, print only the stable text or JSON schema, and return `CommandFailure::status` with the capped finding count.

- [ ] **Step 4: Verify the command without changing callers.**

Run: `cargo test -p autospec-cli --test cli_commands lint_issue -- --nocapture && cargo test -p autospec-core --test issue_lint`

- [ ] **Step 5: Commit the issue CLI.**

```bash
git add crates/autospec-cli/src/commands/{mod.rs,lint.rs} crates/autospec-cli/tests/cli_commands.rs
git commit -m "feat: expose Rust issue lint command"
```

## Task 3: Model implementation diffs and core policy

**Files:**
- Create: `crates/autospec-core/src/lint/{diff.rs,implementation.rs}`
- Modify: `crates/autospec-core/src/lint/mod.rs`
- Create: `crates/autospec-core/tests/implementation_lint.rs`
- Test: `tests/unit/test_lint_implementation.bats`, `tests/lint/{test_complexity_heredoc.bats,test_reuse_triage.bats}`

- [ ] **Step 1: Add focused failing pure-core tests.**

Use inline unified diffs, issue bodies, and explicit post-change repository snapshots to cover every implementation rule, including `ASSERTION_DENSITY`, finding cap, rule order, `Guardian: skip-*`, executable rule-specific `linter:allow-*`, directive text, heredoc complexity, and a clean fixture. Test the scanner against real added lines only; deleted/context lines must not generate a finding. Add separate pre-commit-mode fixtures proving it enables both vacuous and assertion-density checks.

- [ ] **Step 2: Confirm RED.**

Run: `cargo test -p autospec-core --test implementation_lint -- --nocapture`

Expected: compilation/test failures identify the absent diff model and detectors.

- [ ] **Step 3: Implement the pure detector engine.**

Parse unified diffs into paths, added lines, and hunks before evaluating policy. Keep detection deterministic and bounded: no LLM behavior, no subprocesses, and no broad source scans inside core. Pass a supplied repository-index abstraction and post-change file snapshots to the reuse/dependency and whole-file complexity detectors so the CLI adapter controls I/O. Model the shell's current full-file thresholds and Python-specific function checks from supplied content; do not silently drop a detector because its shell version called another tool.

- [ ] **Step 4: Verify pure and existing fixture coverage.**

Run: `cargo test -p autospec-core --test implementation_lint && bats tests/unit/test_lint_implementation.bats tests/lint/test_complexity_heredoc.bats tests/lint/test_reuse_triage.bats tests/test_lint_complexity_gates.bats`

- [ ] **Step 5: Commit the deterministic engine.**

```bash
git add crates/autospec-core/src/lint crates/autospec-core/tests/implementation_lint.rs
git commit -m "feat: move implementation policy detectors into Rust"
```

## Task 4: Add implementation-lint CLI adapters and compatibility exits

**Files:**
- Modify: `crates/autospec-cli/src/commands/lint.rs`
- Modify: `crates/autospec-cli/tests/cli_commands.rs`
- Modify: `crates/autospec-core/tests/implementation_lint.rs`

- [ ] **Step 1: Add failing end-to-end CLI tests.**

Cover `--diff-file`, `--pre-commit --staged`, `PR --issue N`, `--directives`, `--vacuous-assertions`, `--assertion-density`, invalid mutually-exclusive input, a nonzero `gh` response, no staged changes, exact `64` cap, and scope-explosion exit `200`.

- [ ] **Step 2: Confirm RED.**

Run: `cargo test -p autospec-cli --test cli_commands lint_implementation -- --nocapture`

- [ ] **Step 3: Implement thin direct-command adapters.**

Use `git diff --cached` for staged mode, `gh pr diff PR` and `gh issue view ISSUE --json body --jq .body` for remote mode, and direct `std::process::Command` vectors. Read all inputs before invoking core, preserve useful stderr diagnostics, and map only documented exits. Do not embed a shell or silently downgrade remote failures.

- [ ] **Step 4: Verify all public forms.**

Run: `cargo test -p autospec-cli --test cli_commands lint_ -- --nocapture && cargo test -p autospec-core --test implementation_lint`

- [ ] **Step 5: Commit the implementation CLI.**

```bash
git add crates/autospec-cli/src/commands/lint.rs crates/autospec-cli/tests/cli_commands.rs crates/autospec-core/tests/implementation_lint.rs
git commit -m "feat: expose Rust implementation lint command"
```

## Task 5: Cut over callers, installers, and pre-commit behavior

**Files:**
- Modify: `install.sh`, `scripts/install-implementer-precommit.sh`
- Modify: `scripts/{groom-validate.sh,qa-phase4.sh,self-enforce-qa.sh}`
- Modify: all affected `skills/**/{SKILL.md,codex/prompt.md,opencode/agent.md}` trios and `install.sh` payload lists
- Modify: corresponding Bats fixtures and skill golden hashes

- [ ] **Step 1: Add failing compatibility tests.**

First change the relevant Bats tests so they require callers to invoke `autospec lint issue` / `autospec lint implementation`, preserve `--directives` flow, and ensure the generated pre-commit hook uses the installed binary while still blocking a bad staged diff.

- [ ] **Step 2: Confirm RED.**

Run the narrow affected suites, including `bats tests/autospec-run-impl-retry.bats tests/unit/test_install_implementer_precommit.bats tests/unit/test_phase3_lint_integration.bats`.

Expected: tests fail because callers still name the shell scripts.

- [ ] **Step 3: Update every live caller in lock-step.**

Resolve `AUTOSPEC_BIN` consistently with the runtime broker wrappers; do not put a shell fallback back into an installer. Derive all changed trios from `SKILL.md`, regenerate goldens, and update API/reference documentation from the public Rust syntax.

- [ ] **Step 4: Verify installer and caller behavior.**

Run the narrowed Bats suites plus each changed skill's lock-step/golden validation command.

- [ ] **Step 5: Commit the caller cutover.**

```bash
git add install.sh scripts skills tests docs
git commit -m "refactor: route policy callers through Rust"
```

## Task 6: Delete shell authorities and prove reachability

**Files:**
- Delete: `scripts/lint-issue.sh`, `scripts/lint-implementation.sh`
- Modify: `crates/autospec-cli/tests/runtime_commands.rs` or a dedicated Rust reachability test
- Modify: validation catalog fixtures/docs only as required by the changed public authority

- [ ] **Step 1: Add the negative reachability test before deletion.**

Enumerate every tracked file with `git ls-files`; reject `scripts/lint-issue.sh`, `scripts/lint-implementation.sh`, and live invocations of either name. Permit only exact historical migration-note lines and intentional negative assertions, with path-and-line scoped allowlists—not whole-document exemptions.

- [ ] **Step 2: Confirm RED.**

Run the reachability test and observe it fail while shell sources/callers remain.

- [ ] **Step 3: Delete shell linters and obsolete shell-specific validation.**

Remove both scripts, update runtime-policy R1 expectations, validation catalog checks, test fixtures, and references so the Rust command is the only authority. Preserve Bats only where they test shell installer/pre-commit behavior around the Rust binary.

- [ ] **Step 4: Run conversion and full validation.**

Run:

```bash
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo run -q -p autospec-cli -- validate --fast
git diff --check
```

Then run all lint, installer, pre-commit, guardian, and lock-step Bats suites discovered by `rg` before declaring deletion complete.

- [ ] **Step 5: Commit the authority removal.**

```bash
git add -A -- scripts/lint-issue.sh scripts/lint-implementation.sh crates scripts skills tests docs install.sh
git commit -m "refactor: retire shell lint policy authorities"
```

## Final acceptance evidence

- `autospec lint issue` and `autospec lint implementation` reproduce all documented deterministic rule IDs, ordering, text/JSON/directive output, suppressions, and exit codes.
- Every installed, pre-commit, guardian, Phase 3, Phase 4, QA, and documentation caller uses the Rust command.
- A tracked-source reachability test proves neither legacy linter exists nor has a live caller.
- Workspace tests, clippy, focused Bats suites, fast validation, and `git diff --check` pass.
