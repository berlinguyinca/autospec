---
name: autospec
description: Use when the user wants to ship a feature end-to-end across multiple commits — bootstraps repo if missing, brainstorms a design spec, decomposes into linked GitHub issues, and runs an autonomous implementation loop with auto-merge.
---

# autospec workflow (harness-neutral)

Take the following feature request and ship it through the full pipeline:
**bootstrap repo (if missing) → investigate → design → spec → decomposed GitHub issues → autonomous implementation with auto-merge → periodic status updates → final report.**

Manage your own context — never exceed 60%. Delegate to subagents whenever your harness supports it; do not investigate, write code, or design directly in the main conversation when a subagent can do it.

## Self-update mode

If the feature-request argument matches the regex `^\s*update\s*$` (case-insensitive, whitespace-padded), this skill enters self-update mode and does not run the normal pipeline:

1. **Detect harness** by checking which install path exists for this skill:
   - Claude Code: `~/.claude/skills/autospec/SKILL.md`
   - OpenCode:    `~/.config/opencode/agent/autospec.md`
   - Codex CLI:   `~/.codex/prompts/autospec.md`
2. **Re-install from `main`** by piping the canonical installer:
   ```bash
   bash <(curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec/install.sh) --harness <detected> --update
   ```
   If multiple harness paths exist, run the one-liner once per detected harness.
3. **Show the diff** between the prior installed file(s) and the freshly fetched copy (e.g. `diff <(cat <prior>) <(curl -fsSL ...SKILL.md)` or the equivalent recorded by the installer).
4. **Stop.** Do not enter Phase 0 / Phase 1 / any pipeline phase. Print the upgrade summary and return to the user.

If no install path is detected, print `Self-update: no installed copy of autospec found; run install.sh first.` and exit.

## Feature request

{FEATURE_DESCRIPTION}

---

## Required capabilities & harness adapter

This workflow assumes five capabilities. Map each one to your harness's actual tool. If a capability is missing, use the listed fallback.

| Capability                  | Claude Code                          | OpenCode                                 | Codex CLI                                | Fallback if missing                                |
|-----------------------------|--------------------------------------|------------------------------------------|------------------------------------------|----------------------------------------------------|
| Read-only codebase research | `Agent` (subagent_type=Explore)      | `task` agent in read-only mode           | `apply_patch` read-only / shell `grep`   | Do the search in-thread with `rg`/`grep`           |
| Foreground delegation       | `Agent` (subagent_type=general-purpose) | nested `task` agent, await output     | spawn nested CLI session                 | Do the work in-thread (more context cost)          |
| Background delegation       | `Agent` with `run_in_background: true` | detached `task` agent                  | nohup'd CLI session writing to a logfile | Run the monitor in a separate terminal/tmux pane   |
| Ask the user a question     | `AskUserQuestion`                    | inline prompt                            | inline prompt                            | Ask in the response and wait for the next turn     |
| Self-paced future wakeup    | `ScheduleWakeup` inside a `/loop`    | a recurring `task` or local `cron`       | local `cron`/`launchd` calling the CLI   | The user runs a status-update prompt manually      |
| Subagent model tier         | Tier A: `opus` + `ultrathink`; Tier B: `sonnet` + medium thinking | Tier A: top `task` model + high reasoning; Tier B: smaller-tier `task` + medium reasoning | Tier A: top GPT + `reasoning_effort=high`; Tier B: `gpt-5.1-codex-spark` + `reasoning_effort=medium` | Honor the per-phase tier mapping in AGENTS.md; fall back UP on unavailability |

**Persistent project notes**: write durable preferences to **`AGENTS.md`** in the repo root — this is the de-facto standard recognized by Claude Code (also reads `CLAUDE.md`), OpenCode, and Codex. If your harness has its own private memory (e.g. Claude Code's `~/.claude/.../memory/`), mirror the same content there. Per AGENTS.md, subagent dispatches use a **two-tier policy**: Tier A (top model + extended thinking) for spec work (research, decompose, review/label); Tier B (cheaper model + medium thinking) for implementation work (Phase 4 implementer + LGTM review). The orchestrator keeps the user's invoked model. Fall back UP the tier on unavailability.

---

## Phase 0 — Bootstrap repo (if missing)

Verify `gh auth status` is authenticated. If not, ask the user to run `gh auth login` and stop until they confirm.

Probe the working directory:
```bash
git rev-parse --is-inside-work-tree 2>/dev/null
git remote get-url origin 2>/dev/null   # must contain github.com
```

If **either** check fails — no git repo, or no GitHub remote — bootstrap a new repo:

1. **Suggest a name.** Slugify the feature description: lowercase, hyphens, drop stop-words, prefix with the obvious stack if inferable (e.g. "Go TUI for X" → `go-tui-x`; "Python ML pipeline that does Y" → `py-ml-y`). Offer 1–2 candidates.

2. **Ask the user once** (single interactive question; combine the three sub-questions if your harness supports it, otherwise ask sequentially):
   - **Name** (your top suggestion as default).
   - **Visibility**: `private` | `public` (default: private).
   - **Owner**: enumerate via `gh org list`; default to the user's personal account.

3. **Initialize locally**:
   - If `.git` is absent: `git init -b main`.
   - Write a stack-appropriate `.gitignore` (Go: `bin/ vendor/ *.exe *.test`; Node: `node_modules/ dist/ .next/ .env*`; Python: `__pycache__/ .venv/ *.egg-info/ build/ dist/`; mixed/unknown: skip).
   - Write a one-line `README.md` containing the feature description.
   - Write a starter `AGENTS.md` listing the project's coding standards (TDD non-negotiable, no DB mocks, conventional commits, branch-per-issue, no force-push) — this is the source of truth every agent reads.
   - `git add -A && git commit -m "chore: initial scaffold"`.

4. **Create the remote and push**:
   ```bash
   gh repo create <owner>/<name> --<private|public> --source=. --remote=origin --push
   ```

5. **Verify**: `gh repo view <owner>/<name> --json url,defaultBranchRef`. Capture `<owner>/<name>` as `{repo}` — every subsequent phase uses this value.

If a repo already exists (cwd is in a git tree with a `github.com:<owner>/<name>` remote), capture that as `{repo}` and skip the bootstrap.

## Phase 1 — Investigate (delegate)

Spawn a **read-only research subagent** to map relevant files, schema, services. Get back a 300-word summary with file paths and line numbers. Do NOT read files directly from the main thread.

> **Model tier:** Tier A (spec work) — top model with extended/maximum thinking per AGENTS.md. Claude Code: `opus` + `ultrathink`; Codex: current top GPT + `reasoning_effort=high`; OpenCode: top task tier. Fall back UP on unavailability.

If the feature touches a remote system (DB, server, S3), run a real query against the actual data to confirm the problem statement before designing. Surface the concrete numbers in the design.

For a freshly-bootstrapped empty repo, Phase 1 may be a no-op — proceed to Phase 2.

## Phase 2 — Brainstorm + design

> **Spec quality is the bottleneck.** Phase 2's output drives every downstream cycle's cost; if you care about spec quality, invoke this skill with your top-tier model (Claude Code: `claude-code --model opus`; Codex: top GPT). Phase 2 itself runs in the orchestrator (no subagent dispatch) — your invocation model IS the spec model. Subagents in Phases 1, 3, 3.5 follow this lead by selecting Tier A; Phase 4 implementation work uses Tier B. See AGENTS.md.

Run a structured brainstorm — one question at a time, get explicit approval after each section:

1. **Architecture** — where does new code live, what existing patterns does it follow.
2. **Interactivity / API shape** — how does the user/caller drive it; commands, keys, endpoints, fields.
3. **Data model** — types, columns, persistence boundary.
4. **Error handling** — failure modes, recovery, user-visible signals.
5. **Testing** — unit / integration / e2e split, real services vs anything else (rule per AGENTS.md: real services).

Write the agreed design to `docs/specs/YYYY-MM-DD-<topic>-design.md`. Self-review for placeholders, contradictions, ambiguity, scope. The spec must be implementable end-to-end by an agent reading only the spec.

If this is a fresh repo, commit the spec to `main` directly (`git add docs/... && git commit -m "docs: <topic> design spec" && git push`) so subsequent issues can reference it as a tracked file.

## Phase 3 — Decompose into linked GitHub issues (delegate)

Dispatch a **foreground subagent** with this prompt (substitute the spec path and `{repo}`):

> **Model tier:** Tier A (spec work) — top model with extended/maximum thinking per AGENTS.md. Claude Code: `opus` + `ultrathink`; Codex: current top GPT + `reasoning_effort=high`; OpenCode: top task tier. Fall back UP on unavailability.
>
> Create labels (idempotent with `--force`): `auto-implement` (#0e8a16), `epic` (#b60205), plus any domain labels the spec calls for. Then create exactly N issues — first an EPIC umbrella (no `auto-implement` label, just `epic` + domain), then N-1 children all carrying `auto-implement`. After creating children, edit the umbrella body with a checklist linking them. Return JSON: `{umbrella, children:[…], labels_created:[…]}`. Use `gh` CLI only. Do NOT modify code. Do NOT push branches. Do NOT create PRs.
>
> Each child body must be a **self-contained mini-spec** sized for execution by a 32B-class local LLM, with these sections in order:
>
> - **Goal** — 1 sentence outcome.
> - **Source spec** — relative path + GitHub URL of the design doc this issue derives from (if any).
> - **Files to read first** — 3–7 entries. Each entry is one of: a path with **section anchors** (do not say "read the whole spec"), the closest existing-file analogue to mirror, the test file or fixture pattern to follow, or a dependency issue with a one-line summary so the LLM doesn't fetch its body. Bias toward sectional anchors over full files.
> - **Local-LLM execution notes** — one-line context-window recommendation (`32k routine`, `64k stretch`, or `split into N subagents along <criterion>` for issues exceeding ~30k tokens of staged context) and whether single-pass or subagent-split is recommended.
> - **Implementation scope** and **Out of scope** as separate subsections (replaces the prior single "Scope" section).
> - **Implementation outline** — file paths + function signatures + data flow.
> - **Tests required** — TDD per AGENTS.md, real services, no DB mocks, 80%+ coverage.
> - **Acceptance criteria** — checkbox list `[ ]` only, no prose. Each item machine-checkable.
> - **Verification** — split into a **Primary smoke test (inner loop)** with exactly one fast command, and **Operator/full verification** listing the remaining commands.
> - **Branch name** — `feat/<slug>`.
> - **Dependencies** — `Depends on issue #N` lines (parsed by the monitor).
>
> Sizing rule: aim for ≤ 4 KB body. Issues that span more than 4 canonical tables, more than 3 packages, or schema-wide changes must be split — better to emit two children with a `Depends on` edge than one oversized child a small LLM can't hold in working memory.

### Small-LLM friendliness (applies to every child issue)

Children are written assuming the implementer is a 32B-class local model with **pre-staged context**, not a search-driven cloud agent:

- Every file the implementer needs is named in **Files to read first** with a sectional anchor or a one-line reason. Do not assume the model will grep the codebase.
- Spec docs are cited by section heading, not as "read this 20 KB doc".
- Acceptance criteria are checkbox-only so the model can self-verify line-by-line.
- One **Primary smoke test** runs in the inner loop; the heavier verification list runs once at the end.
- If the work fans out across many tables/packages, split it. Two 3 KB children chained by `Depends on` beat one 7 KB child a 32B model garbles at 60k tokens of working context.

Capture the umbrella + child issue numbers.

## Phase 3.5 — Review and label (delegate)

Dispatch a **foreground subagent** to retro-review the child issues just
created in Phase 3 and apply the model-fit rubric. The subagent must NOT modify
issue titles or remove existing labels; it only adds `ctx:*` and `reasoning:*`
labels and patches each body with a `## Model fit` block.

> **Model tier:** Tier A (spec work) — top model with extended/maximum thinking per AGENTS.md. Claude Code: `opus` + `ultrathink`; Codex: current top GPT + `reasoning_effort=high`; OpenCode: top task tier. Fall back UP on unavailability.
>
> Walk every child issue created in Phase 3 (skip any issue carrying the
> `type:tracker` label). For each:
>
> 1. **Stage context.** Read `gh issue view <N> --repo {repo} --json title,body,labels`. The
>    body should already contain `## Files to read first` and
>    `## Implementation scope`. If either is missing, add label
>    `needs-autospec-template` (idempotent `gh label create --force` once at run
>    start) and skip — do not classify or patch.
>
> 2. **Apply the rubric.** Pick the smallest `ctx:*` tier that holds the
>    staged context (issue body + every file in `## Files to read first` +
>    cited spec sections). Pick the `reasoning:*` depth required to derive
>    (not just transcribe) the implementation.
>
>    **`ctx:*` — context-window axis**
>
>    | Label | Trigger |
>    |---|---|
>    | `ctx:32k`  | One canonical table or shell script; ≤3 files in *Files to read first*; short spec anchors. |
>    | `ctx:64k`  | Multi-file change; 4-7 files staged; one trio + one installer; medium spec sections (~1-3 KB). |
>    | `ctx:120k` | Cross-skill or cross-package; 8+ files; long spec excerpts; deep call graphs. |
>
>    **`reasoning:*` — reasoning-depth axis**
>
>    | Label | Trigger |
>    |---|---|
>    | `reasoning:shallow` | Mechanical: copy-and-rename, regex-replace, README transcription, runbook authoring. Verbs: *copy*, *rename*, *transcribe*, *list*. |
>    | `reasoning:medium`  | Template-following with judgment: synthesize a SKILL.md mirroring an existing one, modify a script with new flags, write tests for a documented contract. Verbs: *mirror*, *adapt*, *integrate*, *wire*. |
>    | `reasoning:deep`    | Novel design choices: pick a new abstraction, resolve a contradiction in the spec, reconcile cross-cutting concerns. Verbs: *design*, *reconcile*, *resolve*, *redesign*. |
>
>    Default for issues that lack any of these signals: `ctx:64k`,
>    `reasoning:medium`. If unsure between two ctx tiers, prefer the larger.
>
> 3. **Sibling normalization.** When 5+ split children share a structural
>    criterion (e.g. all per-source-table writers, all per-skill installers),
>    harmonize their `ctx:*`/`reasoning:*` labels so the operator can run a
>    single profile across the whole group. Override individual classifications
>    only when the difference is a true outlier (e.g. one sibling pulls in a
>    schema-wide refactor that no other sibling touches).
>
> 4. **Apply labels.** Idempotent at run start:
>    `gh label create ctx:32k  --color c5def5 --force --repo {repo}`,
>    `gh label create ctx:64k  --color c5def5 --force --repo {repo}`,
>    `gh label create ctx:120k --color c5def5 --force --repo {repo}`,
>    `gh label create reasoning:shallow --color c2e0c6 --force --repo {repo}`,
>    `gh label create reasoning:medium  --color c2e0c6 --force --repo {repo}`,
>    `gh label create reasoning:deep    --color c2e0c6 --force --repo {repo}`.
>    Then per issue:
>    `gh issue edit <N> --add-label "ctx:<tier>,reasoning:<depth>" --repo {repo}`.
>
> 5. **Patch body — `## Model fit` block.** Insert immediately before the first
>    `## Dependencies` line (or at end of body if absent):
>
>    ```markdown
>    ## Model fit
>
>    - **ctx:** `ctx:<tier>` — <1-line rationale>.
>    - **reasoning:** `reasoning:<depth>` — <1-line rationale>.
>
>    <!-- autospec-classify:begin -->
>    *Auto-classified by Phase 3.5 on YYYY-MM-DD.*
>    <!-- autospec-classify:end -->
>    ```
>
>    **Idempotency:** if a `## Model fit` block already exists between the
>    `<!-- autospec-classify:begin -->` and `<!-- autospec-classify:end -->`
>    markers, replace it in place. Never stack duplicates. Apply via
>    `gh issue edit <N> --body-file <tmp>`.
>
> 6. **Board assignment** — read `~/.autospec/project-map.yml` and assign each
>    just-classified child to the GitHub Projects mapped from its labels.
>
>    **File schema** (auto-init if missing — see below):
>    ```yaml
>    # ~/.autospec/project-map.yml
>    multi_match: union          # `union` (assign to every match) or `first`
>    mappings:
>      ctx:32k: <project_number>
>      ctx:64k: <project_number>
>      ctx:120k: <project_number>
>      reasoning:shallow: <project_number>
>      reasoning:medium:  <project_number>
>      reasoning:deep:    <project_number>
>      <any-other-label>: <project_number>
>    ```
>
>    **Reader procedure** for each issue I:
>    - For each label L on I, look up `mappings[L]`. Skip null / missing entries.
>    - With `multi_match: union` (default), collect all matching project numbers and assign to every one of them. With `multi_match: first`, take the first match in label-order and assign to that single project.
>    - For each chosen `<P>`: `gh project item-add <P> --owner <owner> --url <issue-url>`. The `gh` command is idempotent — repeated calls do not duplicate items, so re-running Phase 3.5 is safe.
>
>    **Auto-init when the file is missing.** Probe `gh project list --owner <owner> --format json` to confirm the user can author projects. Probe `gh label list --repo {repo} --json name -q '.[].name'` to enumerate the repo's labels. Write a starter file with every label as a `mappings:` key and `null` project numbers, plus `multi_match: union` at the top. Print:
>    ```
>    Wrote ~/.autospec/project-map.yml. Edit project numbers (currently null) and re-run.
>    ```
>    Then **exit Phase 3.5** without assigning any boards (the labels and `## Model fit` blocks remain applied — only the assign step is deferred).
>
>    **Hard rules.**
>    - Never call `gh project item-add` in `--dry-run`.
>    - Missing file in `autospec` / `autospec-define` is non-fatal at run time once auto-init has populated it; if auto-init itself fails (e.g. `gh project list` denied), warn and skip board assignment for the rest of the run.
>
> 7. **Dependency-edge sanity checks.** After labeling, validate the dep graph
>    of the just-created children:
>    - **closed-dep warning** — emit `WARN: child #<N> depends on closed issue #<M>` for each `Depends on #M` line where `gh issue view #<M> --json state` is `CLOSED`.
>    - **child-less tracker dep warning** — emit `WARN: child #<N> depends on tracker #<M> with no children` when `#<M>` carries `type:tracker` and has no other open `auto-implement` deps pointing at it.
>    - **circular sibling-dep hard fail** — exit non-zero if any cycle exists among the just-created children's `Depends on #N` edges.
>
> 8. **Run-end summary.** Print to stdout:
>    ```
>    Phase 3.5 summary on {repo}
>    - classified: N
>    - skipped (needs-autospec-template): M
>    - ctx:32k=A  ctx:64k=B  ctx:120k=C
>    - reasoning:shallow=X  reasoning:medium=Y  reasoning:deep=Z
>    - boards assigned: <K> (or "skipped — no project-map.yml")
>    - dep warnings: <count>; circular cycles: <count>
>    ```

## Phase 4 — Background autonomous monitor

Record this durable preference in `AGENTS.md` (idempotent — skip if already present):

> **Auto-merge authority for auto-implement PRs.** Admin-merge auto-implement PRs (`gh pr merge <#> --admin --squash --delete-branch`) when (a) all required CI checks pass — slow optional checks like TeamCity may be pending and that's acceptable, (b) the self-review subagent returned `LGTM`, (c) PR closes an `auto-implement` issue from a `feat/*` branch.

Then launch a **background subagent** with this prompt verbatim:

> You are the auto-implement monitor for `{repo}`. Process every open `auto-implement` issue autonomously and auto-merge each PR. **You MUST stay running across many iterations. Do NOT exit after one issue.**
>
> Outer loop:
> ```
> while true:
>   ready = [open auto-implement issues whose Depends-on deps are all CLOSED, sorted ascending]
>   if ready is empty:
>     latest_close = most recent closedAt of any auto-implement issue
>     open_count   = count of open auto-implement issues
>     if open_count == 0 AND latest_close > 1h ago: HARD SHUTDOWN — return final report
>     else: print state ("blocked: N unmet deps" / "drained, waiting 1h idle"), sleep 300, continue
>   ISSUE = ready[0]
>   gh label create in-progress-by-bot --color ededed --force
>   gh issue edit ISSUE --remove-label auto-implement --add-label in-progress-by-bot
>   process(ISSUE)   # foreground subagent — see template below
>   # NO SLEEP — go straight to the next iteration; the merge may have unblocked another issue
> ```
>
> `process(ISSUE)` dispatches a **foreground subagent** (wait for return) with this prompt:
>
> ```
> **Model tier:** Tier B (implementation work) — cheaper model with medium thinking per AGENTS.md. Claude Code: `sonnet`; Codex: `gpt-5.1-codex-spark`; OpenCode: smaller task tier. Fall back UP on unavailability.
>
> Implement GitHub issue #<ISSUE>: "<TITLE>" on {repo}. Spec is the issue body below.
>
> ===ISSUE BODY===
> <BODY>
> ===END===
>
> 1. Worktree off origin/main:
>    cd {repo_root} && git fetch origin
>    git worktree add -b <BRANCH> /tmp/wt-<BRANCH> origin/main && cd /tmp/wt-<BRANCH>
> 2. TDD per AGENTS.md: failing test first → implement → refactor → commit. NO DB/external mocks. Follow file paths and signatures from the issue body verbatim.
> 3. Build + test green (use the project's test runner; for Go: `go build ./... && go test ./... -count=1`; for Node: `npm test`; for Python: `pytest`). 80%+ coverage on changed files.
> 4. Conventional commits (feat:/fix:/test:/docs:/refactor:). NEVER bypass hooks. NEVER amend.
> 5. Push: git push -u origin <BRANCH>
> 6. PR: gh pr create --base main --head <BRANCH> --title "<TITLE>" --body "Closes #<ISSUE>\n\n<summary>". Capture PR.
> 7. Inner loop (max 3 iterations):
>    - Run the **Primary smoke test** from the issue body. If it fails, fix and recommit before review.
>    - Dispatch a **foreground subagent** with brief: "**Model tier:** Tier B (implementation work) — cheaper model with medium thinking per AGENTS.md. Claude Code: `sonnet`; Codex: `gpt-5.1-codex-spark`; OpenCode: smaller task tier. Fall back UP on unavailability. You are a critical code reviewer. Review PR #<PR> via `gh pr diff` and `gh pr view`. Check correctness, edge cases, missing tests, AGENTS.md compliance. Output a numbered findings list, OR if none, return ONLY the token: LGTM"
>    - If LGTM: run the **Operator/full verification** commands; sleep 30; `gh pr checks <PR>`. If all required checks pass (slow optional checks pending is OK per AGENTS.md): break SUCCESS.
>    - Else: implement findings, commit, push, continue.
> 8. SUCCESS: gh pr merge <PR> --admin --squash --delete-branch. Merge auto-closes the issue.
> 9. FAILURE (loop exhausted): comment failure on issue, swap label `in-progress-by-bot` → `auto-implement`, `gh pr close <PR> --delete-branch`.
> 10. Cleanup: cd / && git -C {repo_root} worktree remove /tmp/wt-<BRANCH> --force
> 11. Report: PR number, outcome, one-paragraph summary.
>
> Hard rules: NEVER push to main, force-push, bypass hooks, or touch the umbrella issue. gh CLI only.
> ```
>
> Hard rules for the monitor: ONE issue at a time, sequential. Do NOT touch the umbrella. On transient gh errors retry once. Do NOT ask the user — auto-merge authority is granted in AGENTS.md.
>
> Final output when shutdown: numbered list of every processed issue with PR # and outcome.

Capture the agent ID / log path for monitoring.

If your harness lacks background delegation: open a separate terminal/tmux pane, run the monitor prompt in a fresh session there, and have it write progress to a logfile that Phase 5 can tail.

## Phase 5 — Periodic status updates

Set up a recurring check (every ~25 min) using your harness's self-paced wakeup capability. Each tick:

```bash
gh issue list --repo {repo} --label auto-implement,in-progress-by-bot,awaiting-merge --state all --json number,state,labels
gh pr list --repo {repo} --state all --json number,state,title --limit 20
```

Post a one-paragraph delta to the user (newly closed issues, newly merged PRs, failures, blockers). If two consecutive ticks have nothing new, slow to ~50 min cadence to reduce noise. Stop the loop when:
- the monitor agent reports completion, OR
- all child issues are CLOSED, OR
- the user explicitly stops.

If your harness lacks self-paced wakeup: register a local `cron`/`launchd` job that runs the same status-check prompt at the chosen cadence, OR ask the user to invoke `status-update` manually.

## Phase 6 — Final report

When the monitor terminates, post a final summary to the user: every issue processed, every PR merged, total elapsed wall time, and any failures that need human attention.

---

## Constraints (apply throughout)

- **Cadence**: sub-hour polling needs an in-session background subagent or a local cron. Cloud cron services (e.g. Anthropic remote routines) typically have a 1-hour minimum and are not appropriate.
- **TDD non-negotiable** per AGENTS.md. Real services in tests, no DB mocks.
- **Branch-per-issue**, conventional commits, no force-push, no hook bypass.
- **Auto-merge** on success per AGENTS.md. Do not ask.
- **Context budget**: stay under 60%. Delegate everything possible.
- **Failure isolation**: a failed issue restores its `auto-implement` label so the next monitor cycle picks it up; it does not block the cascade unless dependent issues are downstream.
- **Fresh repo**: if Phase 0 created the repo, the very first commit is the scaffold (incl. `AGENTS.md`); the spec lands as the second commit; child branches branch off that `main`.
- **Small-LLM target**: child issues are sized and pre-staged for 32B-class local LLMs (e.g. qwen3-32B / Qwen3-30B-A3B on Ollama, 48 GB-class Macs). Bias toward smaller issues, sectional spec anchors, pre-staged file pointers, checkbox AC, and one Primary smoke test.
