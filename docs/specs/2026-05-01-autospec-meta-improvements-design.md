# Autospec Meta-Improvements — Design Spec

**Date**: 2026-05-01
**Repo**: github.com/berlinguyinca/autospec
**Status**: Approved (Phase 2 brainstorm complete; ready to decompose into issues)

## 1. Goals

Improve the autospec skill family along seven axes, all driven by direct user
asks captured in the brainstorm:

1. **Documentation refresh & sync** — README, CONTRIBUTING, AGENTS.md, plus
   new user-manual and how-it-works docs; everything stays in sync via the
   existing lock-step rule + a CI check.
2. **Token-lean spec / issue generation** — tighten Phase 2 brainstorm and
   Phase 3 decomposition prompts inline (no new post-processor service).
3. **Cross-model robustness** — add anti-loop / anti-stall guardrails that
   work universally; per-model context profiles deferred to a follow-up.
4. **Proper test stack** — bats-core for shell tests, golden-output snapshot
   tests for prompts, opt-in e2e against a throwaway gh repo, plus a
   `validate.yml` GitHub Actions workflow.
5. **Many small issues > big ones** — hard caps in Phase 3 decomposer:
   each child issue body ≤400 words, implementation outline ≤30 lines,
   ≤3 files touched. Whole package fits in a 60–120k-ctx small-LLM window.
6. **Concurrent-agent safety** — keep the existing per-issue
   `locked-by-autospec-processor` label as the mutex; document the contract.
7. **New 5th skill: `autospec-listen`** — a passive conversation listener
   that, when the user mentions "file an issue" / "new issue" / "write a
   spec" / "design spec", offers (with approval) to file a GitHub issue or
   route to `/autospec-define`.

## 2. Architecture

### 2.1 New skill: `autospec-listen`

A 5th sibling of the existing four skills. Same shape as the others:

```
skills/autospec-listen/
  SKILL.md                # canonical body (Claude Code frontmatter)
  README.md
  install.sh              # standalone installer (copy from autospec/install.sh template)
  uninstall.sh
  opencode/agent.md       # frontmatter: description + mode: primary
  codex/prompt.md         # plain (no frontmatter)
  references/
    trigger-keywords.md   # canonical list of activation phrases
    fixture-conversations/   # used by tests/golden/ snapshots
```

**Frontmatter description (verbatim, identical across trio bodies)**:

> Use when the user mentions filing an issue or writing a spec mid-conversation.
> Trigger keywords: "file an issue", "new issue", "open an issue",
> "create a ticket", "write a spec", "design spec", "new spec",
> "start a spec". On an "issue" trigger, drafts a body from the last ~10
> conversation turns and asks the user to approve before running
> `gh issue create`. On a "spec" trigger, hands off to `/autospec-define`.
> Lives at github.com/berlinguyinca/autospec/skills/autospec-listen.

The bare-noun forms ("issue", "spec") are NOT triggers — too noisy.

### 2.2 Token-optimization placement

Inline in `SKILL.md` prompts only. No new post-processor service. Concrete
budgets land in `skills/autospec/SKILL.md` Phase 2/3 prompt text and in
`skills/autospec-define/SKILL.md` Phase 2/3 prompt text. Lock-step rule
replicates them automatically into `opencode/agent.md` + `codex/prompt.md`.

### 2.3 Multi-model robustness

The existing AGENTS.md two-tier policy stays. We add **anti-loop / anti-stall
guardrails** to the Phase 1 research prompt and the Phase 4 implementer
prompt. Per-model context profiles (e.g. "qwen3-32b: max 12 tool calls per
phase") are out of scope for this spec — defer to a follow-up issue.

## 3. Interactivity / API

### 3.1 `autospec-listen` issue trigger

When a trigger phrase fires:

1. Read the last ~10 conversation turns from harness context.
2. Synthesize a draft issue body with sections: **Goal** (1 sentence),
   **Context** (relevant chat excerpts), **Suggested AC** (≤3 bullet
   acceptance criteria).
3. Print the draft inline.
4. Ask via the harness's question primitive
   (`AskUserQuestion` on Claude Code; inline on others):
   `"File this as a new issue? [yes / edit / cancel]"`.
5. On `yes`: `gh issue create --title <inferred> --body <draft>
   --label needs-classify` and post the URL to chat. The
   `needs-classify` label keeps the issue out of the auto-implement queue
   until `/autospec-classify` sizes it.
   - **Title inference**: take the Goal sentence, strip leading
     "Implement / Add / Fix" verbs, truncate to 80 chars, prefix with a
     conventional-commit type tag (`feat:` / `fix:` / `docs:` /
     `refactor:`) inferred from the Goal verb. If inference is ambiguous,
     leave the tag off and let `/autospec-classify` add one.
6. On `edit`: accept free-text revisions, re-show, re-confirm.
7. On `cancel`: silently exit. No persistence, no learning.

### 3.2 `autospec-listen` spec trigger

When a spec trigger phrase fires:

1. Confirm: `"Start the autospec design Q&A on <inferred topic>?
   [yes / cancel]"`.
2. On `yes`: invoke `/autospec-define`. (The listener is a router; it does
   not duplicate Phase 2 logic.)
3. On `cancel`: silently exit.

### 3.3 Pre-implementation gate (in `/autospec-define` AND `/autospec`)

After Phase 3 decomposition completes, BOTH `/autospec-define` and
`/autospec` (the umbrella full-pipeline skill) MUST end Phase 3 with
this question (no auto-launch):

`"Spec written, N issues filed. Start /autospec-run now, defer to your
external daemon, or keep refining? [run / defer / refine]"`

- `run`: invoke `/autospec-run` in the current session (which for
  `/autospec` is the existing Phase 4 monitor launch path).
- `defer`: print `"Issues are ready. Your external monitor will pick them
  up. Exiting."` and stop. For `/autospec`, this means skipping Phase 4–6
  entirely.
- `refine`: re-enter Phase 2 from question 1 (Architecture).

No daemon auto-detection — always ask explicitly. The gate is the only
behavioral divergence between `/autospec-define` and `/autospec` after
this change: both hit the gate; `/autospec` defaults the prompt highlight
to `run` (since the user invoked the umbrella expecting end-to-end
shipping), `/autospec-define` defaults to `defer` (since its name implies
plan-only).

### 3.4 Issue sizing hard caps (Phase 3 decomposer)

Add the following imperatives to the decomposer prompt in
`skills/autospec/SKILL.md` and `skills/autospec-define/SKILL.md`:

- **Body ≤400 words** including all sections.
- **Implementation outline ≤30 lines** (file paths + function signatures).
- **Files touched ≤3** per child issue.
- If a candidate child would exceed any cap: split into a parent + child
  pair with a `Depends on` edge.
- The whole spec + a single child issue body must fit comfortably in a
  60–120k context window.

The decomposer self-checks each issue against the caps before calling
`gh issue create`. If a cap is violated and split fails, the decomposer
surfaces the issue inline for human help instead of filing it.

## 4. Data model

### 4.1 New label

- **`needs-classify`** (color `#fbca04`) — applied to issues created by
  `autospec-listen` from chat. `/autospec-classify` removes it after
  applying `ctx:* / reasoning:*` labels and adds `auto-implement`.
  Idempotent creation via `gh label create needs-classify --color fbca04 --force`.

### 4.2 No daemon PID file

Per Section 4 brainstorm: no `~/.autospec/monitor.pid`. The pre-impl gate
always asks the user. Skip implementing any persistence.

### 4.3 `examples/` at repo root

```
examples/
  README.md              # explains both files, links to skill docs
  model-profiles.yml     # sample with claude-sonnet-cloud + qwen3-32b-laptop
  project-map.yml        # sample with ctx:* + reasoning:* mappings (project numbers null)
```

Reference these files from `skills/autospec-run/SKILL.md` (model-profiles
section) and `skills/autospec-classify/SKILL.md` (project-map section).
The auto-init code in those skills MAY copy from `examples/` if
`~/.autospec/<file>` is missing.

### 4.4 Trigger keyword set (verbatim)

Living in `skills/autospec-listen/references/trigger-keywords.md`. Tight
imperative-verb-only set:

**Issue triggers** (case-insensitive, word-boundary matched):
- `file an issue`
- `file this as an issue`
- `new issue`
- `open an issue`
- `create a ticket`
- `make an issue`

**Spec triggers**:
- `write a spec`
- `design spec`
- `new spec`
- `start a spec`
- `write a design spec`

Bare nouns (`issue`, `spec`, `ticket`) are NOT triggers.

## 5. Error handling

### 5.1 Anti-loop / anti-stall guardrails

Add to Phase 1 research subagent prompt:

> **Hard limits.** Max 25 tool calls. If 3 consecutive read/grep calls
> return nothing useful, stop and write your best-effort summary even if
> incomplete. Do not retry the same query verbatim. No wall-clock cap.

Add to Phase 4 implementer subagent prompt:

> **Hard limits.** Max 40 tool calls per issue. Max 3 self-review
> iterations. If you rewrite the same file twice with no test progress,
> abort: comment the blocker on the issue, release the lock label, exit.
> No wall-clock cap.

These limits live inline in both `skills/autospec/SKILL.md` (Phases 1, 4)
and `skills/autospec-run/SKILL.md` (Phase 4). Lock-step replicates.

### 5.2 False-positive listener fires

On `cancel` from the issue-trigger flow: print `"OK, cancelled."` and
exit. No suppression list, no per-session sentinel, no persistence. If
the user wants to silence the listener, they can uninstall it
(`./skills/autospec-listen/uninstall.sh`).

### 5.3 Stuck `needs-classify` issues

**Extension** to `skills/autospec-classify/SKILL.md`: today, the skill
walks issues labeled `auto-implement`. Extend it to ALSO walk issues
labeled `needs-classify`. After classification, REMOVE the
`needs-classify` label and ADD `auto-implement`. (Today's behavior on
`auto-implement`-labeled issues stays unchanged: re-classify in place,
no label transition.)

Document the new behavior in the SKILL.md body. Ship a sample crontab
entry in `docs/runbooks/needs-classify-sweep.md` for users who want a
daily sweep. No auto-promotion after a TTL.

### 5.4 Concurrency

Keep the existing per-issue `locked-by-autospec-processor` label as the
sole mutex. Document the contract explicitly in
`docs/architecture.md` (new file, see §7) and in `AGENTS.md`. Add this
clarifying note to the monitor prompt:

> The lock-claim comment (`🔒 Auto-locked by autospec monitor at <ts>`)
> IS the marker. Before claiming a lock, re-read the issue: if a
> `🔒 Auto-locked` comment was posted in the last 5 min by anyone other
> than you, that is another monitor that just claimed it — yield, do
> NOT add your own lock label, re-enter the ready set on the next loop
> iteration. No actual heartbeat-every-N-min stream is needed.

No global flock, no GitHub-side coordination issue.

## 6. Testing

### 6.1 Test stack

- **bats-core** for shell unit + smoke tests. Install via the dev
  bootstrap script (see §6.5).
- **`scripts/validate.sh`** stays for static checks (lock-step body,
  bash -n, frontmatter parse, AGENTS.md governance headings). Extend it
  to also check: (a) the new `autospec-listen` skill exists with required
  files, (b) `examples/` directory exists with required files, (c) trigger
  keyword reference file is present.

### 6.2 Test layout

```
tests/
  unit/
    test_listener_keywords.bats        # trigger regex against fixture phrases
    test_install_listener.bats         # install.sh / uninstall.sh roundtrip in tmpdir
    test_sizing_caps.bats              # decomposer cap-checker logic (extracted to a helper)
  smoke/
    test_install_all_skills.bats       # root install.sh --skill all in tmpdir
  e2e/
    test_listener_full_flow.bats       # opt-in; creates throwaway gh repo
    test_define_to_run_handoff.bats    # opt-in
  golden/
    autospec-listen.draft-issue.md     # fixture-based golden output
    autospec-define.spec.md            # fixture-based golden output
  fixtures/
    chat_about_perf.txt                # sample 10-turn conversation
    chat_about_auth.txt
```

### 6.3 Listener-specific test pyramid

- **Unit** (`tests/unit/test_listener_keywords.bats`): 30+ phrase fixtures.
  `"file an issue"` → fire. `"the issue here is..."` → don't fire.
  `"write a spec"` → fire-spec. `"the spec says..."` → don't fire. Tests
  call a small extracted helper (`scripts/listener-match.sh <phrase>`)
  that the SKILL.md prompt also references.
- **Smoke** (`tests/smoke/test_install_all_skills.bats`): roundtrip
  install + uninstall of all 5 skills into a tmpdir; assert files exist
  + uninstall removes them all. Idempotency check: install twice in a row
  must succeed.
- **E2E** (`tests/e2e/test_listener_full_flow.bats`): gated behind the
  `e2e` PR label. Creates a throwaway public gh repo via `gh repo create`,
  simulates a chat-driven issue file (calling the listener-match helper +
  `gh issue create`), asserts the resulting issue body has Goal /
  Context / Suggested AC sections, has the `needs-classify` label, body
  ≤400 words. Tears down the throwaway repo with `gh repo delete --yes`.

### 6.4 Cross-harness anti-regression snapshots

Add `tests/golden/<skill>.<artifact>.md` for each notable prompt-derived
artifact. A test compares a fresh subagent's output (from a fixture
conversation) against the committed golden file. Differences fail the
build; refreshing the golden requires running `tests/refresh-goldens.sh`
locally and committing the diff.

### 6.5 Dev bootstrap

`scripts/dev-bootstrap.sh` (new) installs `bats-core` (via brew/apt/npm
detection), confirms `gh`, `jq`, `python3` are available, and prints a
`make test` analogue: `bats tests/unit tests/smoke && scripts/validate.sh`.
Add a note in CONTRIBUTING.md.

### 6.6 CI

`.github/workflows/validate.yml` (new):

- Triggers: `push`, `pull_request`.
- Runs: `bash scripts/dev-bootstrap.sh`, `bash scripts/validate.sh`,
  `bats tests/unit tests/smoke`.
- Target: <60s wall time.

`.github/workflows/e2e.yml` (new):

- Trigger: `pull_request` with label `e2e`, plus manual `workflow_dispatch`.
- Runs: same bootstrap, then `bats tests/e2e` with `GH_TOKEN` from secrets.
- Wall-time budget: 5 min.
- Cleanup: explicit `gh repo delete --yes` on the throwaway repo even on
  test failure (trap EXIT in bats).

## 7. Documentation updates

### 7.1 Files to update / create

| File | Action | Owner phase |
|---|---|---|
| `README.md` | Update — add `autospec-listen` to the skill table; reference `examples/`; link to user-manual + architecture | Doc issue |
| `CONTRIBUTING.md` | Update — testing workflow (bats), golden-update procedure, dev-bootstrap | Doc issue |
| `AGENTS.md` | Update — add §"Anti-loop guardrails" and §"Listener-filed issues lifecycle" headings | Doc issue |
| `SKILLS.md` | Update — add `autospec-listen` row | Doc issue |
| `docs/user-manual.md` | Create — narrative walkthrough: "I'm a user. What does each skill do, when do I use it, what does the output look like?" | Doc issue |
| `docs/architecture.md` | Create — concurrency model, lock-step rule, model-tier policy, trigger keyword theory, all in one place | Doc issue |
| `docs/runbooks/needs-classify-sweep.md` | Create — sample crontab + manual command | Doc issue |
| `examples/README.md` | Create — explains both YAML files | Doc issue |

Validation: `scripts/validate.sh` MUST grep for the new headings in
`AGENTS.md` and the `autospec-listen` row in `README.md` + `SKILLS.md`.

## 8. Acceptance criteria for the whole effort

- [ ] `skills/autospec-listen/` exists with all 7 files (SKILL.md,
  README.md, install.sh, uninstall.sh, opencode/agent.md, codex/prompt.md,
  references/trigger-keywords.md).
- [ ] `scripts/validate.sh` passes after the changes.
- [ ] `bats tests/unit tests/smoke` passes locally and in CI.
- [ ] The opt-in `tests/e2e/` job passes when triggered with the `e2e`
  label.
- [ ] All trigger keywords from §4.4 fire on positive fixtures and don't
  fire on negative fixtures.
- [ ] Issue body word count assertion in the listener path stays ≤400.
- [ ] `examples/` directory has all 3 files; `examples/README.md` is
  cross-linked from `skills/autospec-run/SKILL.md` and
  `skills/autospec-classify/SKILL.md`.
- [ ] AGENTS.md has §"Anti-loop guardrails" and §"Listener-filed issues
  lifecycle" sections.
- [ ] README.md and SKILLS.md list `autospec-listen` in their skill tables.
- [ ] `docs/user-manual.md` and `docs/architecture.md` exist and are
  linked from README.md.
- [ ] `.github/workflows/validate.yml` and `.github/workflows/e2e.yml`
  exist and pass on a clean PR.

## 9. Out of scope (deferred to follow-ups)

- Per-model context-window / tool-call profiles in AGENTS.md (Section 1
  decision: guardrails first).
- Cloud-side daemon coordination (single-issue lock label is sufficient).
- A spec-compactor service (Section 1 decision: tighten prompts inline).
- Auto-promotion of stale `needs-classify` issues (Section 5 decision:
  manual sweep only).
- Hooks-based UserPromptSubmit listener (Section 1 decision: skill-based
  for cross-harness portability).
