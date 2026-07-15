# Canonical repository detection implementation plan

> **For implementation:** Use `superpowers:subagent-driven-development` and perform the one
> task below in the isolated `feat/canonical-repository-detection` worktree.

**Goal:** Provide a Rust-native, deterministic org-sweep repository classifier that routes
duplicate findings to canonical active repositories and defers archived repositories by default.

**Architecture:** A dependency-free `exploration` core owns typed evidence parsing, scoring,
role inference, duplicate routing, and stable JSON rendering. The CLI is a thin file-input
adapter. The `autospec-explore` trio documents the command contract identically, retaining no
legacy runtime path.

**Tech stack:** Rust standard library and existing in-tree JSON parser; no dependencies.

## Global constraints

- Work only in `/private/tmp/wt-feat-canonical-repository-detection` on
  `feat/canonical-repository-detection`.
- Do not add or revive shell/Python runtime code, Bats coverage, or a dependency.
- Keep `SKILL.md`, `codex/prompt.md`, and `opencode/agent.md` bodies lock-step.
- Input is local JSON only; the command must not call the network or mutate repositories.
- A family with no eligible target defers findings; an archived repository is eligible only when
  `revival_requested` is true.
- Scores must use every specified evidence field and tie-break by repository name.
- Final validation must include `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace`, and `autospec validate --fast`.

## Task 1: Build the Rust vertical slice with tests first

**Files:**
- Create: `crates/autospec-core/src/exploration/mod.rs`
- Modify: `crates/autospec-core/src/lib.rs`
- Create: `crates/autospec-core/tests/exploration.rs`
- Create: `crates/autospec-cli/src/commands/explore.rs`
- Modify: `crates/autospec-cli/src/commands/mod.rs`
- Modify: `crates/autospec-cli/tests/cli_commands.rs`
- Modify: `skills/autospec-explore/SKILL.md`
- Modify: `skills/autospec-explore/codex/prompt.md`
- Modify: `skills/autospec-explore/opencode/agent.md`

1. Start with failing Rust tests for an active `go-modules`, archived `go-admin`, duplicate
   fingerprint routing, a `revival_requested` exception, and CLI JSON rendering.
2. Implement public typed input/output models and strict JSON parsing with the existing in-tree
   parser. Reject unknown keys, duplicate repository names, unknown finding repositories, and
   invalid dates rather than guessing.
3. Score repository evidence deterministically: archived/revival state, push-recency ranking,
   README presence, module path count, package count, and inbound dependency references. Select
   one canonical eligible target per family; tie-break by repository name.
4. Expose `autospec explore repositories --input <path>`; render stable JSON including
   `canonical_targets`, `do_not_file_by_default`, `routed_findings`, and deferred findings.
5. Add the same "Rust org-sweep repository routing" section to every explore prompt body. It
   must direct the agent to call the command before filing and use `routed_findings` only.
6. Run focused tests after each red/green step, then the global validation commands.

**Expected tests:**

```bash
cargo test --workspace
```

## Review and closeout

1. Generate a review package from the branch base to the implementation commit.
2. Review exact command semantics, deterministic ordering, error handling, legacy-path exclusion,
   lock-step prompt bodies, and the requested evidence fields.
3. Resolve every critical or important finding and re-run affected tests.
4. Run the global validation commands and include their results in the issue closeout and PR body.

