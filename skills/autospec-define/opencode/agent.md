---
description: Use when the user wants to plan a feature — bootstrap repo if missing, brainstorm a design spec, and decompose into linked GitHub issues. Stops after Phase 3 and hands off to /autospec-run for autonomous implementation.
mode: primary
---

# autospec-define workflow (harness-neutral)

Take the following feature request and run the planning half of the autospec pipeline:
**bootstrap repo (if missing) → investigate → design → spec → decomposed GitHub issues.** Stop after Phase 3; the implementation half is handed off to `/autospec-run`.

Manage your own context — never exceed 60%. Delegate to subagents whenever your harness supports it; do not investigate, write code, or design directly in the main conversation when a subagent can do it.

## Self-update mode

If the feature-request argument matches the regex `^\s*update\s*$` (case-insensitive, whitespace-padded), this skill enters self-update mode and does not run the normal pipeline:

1. **Detect harness** by checking which install path exists for this skill:
   - Claude Code: `~/.claude/skills/autospec-define/SKILL.md`
   - OpenCode:    `~/.config/opencode/agent/autospec-define.md`
   - Codex CLI:   `~/.codex/prompts/autospec-define.md`
2. **Re-install from `main`** by piping the canonical installer:
   ```bash
   bash <(curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-define/install.sh) --harness <detected> --update
   ```
   If multiple harness paths exist, run the one-liner once per detected harness.
3. **Show the diff** between the prior installed file(s) and the freshly fetched copy (e.g. `diff <(cat <prior>) <(curl -fsSL ...SKILL.md)` or the equivalent recorded by the installer).
4. **Stop.** Do not enter Phase 0 / Phase 1 / any pipeline phase. Print the upgrade summary and return to the user.

If no install path is detected, print `Self-update: no installed copy of autospec-define found; run install.sh first.` and exit.

## Feature request

{FEATURE_DESCRIPTION}


## Required capabilities & harness adapter

This workflow assumes five capabilities. Map each one to your harness's actual tool. If a capability is missing, use the listed fallback.

| Capability                  | Claude Code                          | OpenCode                                 | Codex CLI                                | Fallback if missing                                |
|-----------------------------|--------------------------------------|------------------------------------------|------------------------------------------|----------------------------------------------------|
| Read-only codebase research | `Agent` (subagent_type=Explore)      | `task` agent in read-only mode           | `apply_patch` read-only / shell `grep`   | Do the search in-thread with `rg`/`grep`           |
| Foreground delegation       | `Agent` (subagent_type=general-purpose) | nested `task` agent, await output     | spawn nested CLI session                 | Do the work in-thread (more context cost)          |
| Background delegation       | `Agent` with `run_in_background: true` | detached `task` agent                  | nohup'd CLI session writing to a logfile | Run the monitor in a separate terminal/tmux pane   |
| Ask the user a question     | `AskUserQuestion`                    | inline prompt                            | inline prompt                            | Ask in the response and wait for the next turn     |
| Self-paced future wakeup    | `ScheduleWakeup` inside a `/loop`    | a recurring `task` or local `cron`       | local `cron`/`launchd` calling the CLI   | The user runs a status-update prompt manually      |

**Persistent project notes**: write durable preferences to **`AGENTS.md`** in the repo root — this is the de-facto standard recognized by Claude Code (also reads `CLAUDE.md`), OpenCode, and Codex. If your harness has its own private memory (e.g. Claude Code's `~/.claude/.../memory/`), mirror the same content there.


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

If the feature touches a remote system (DB, server, S3), run a real query against the actual data to confirm the problem statement before designing. Surface the concrete numbers in the design.

For a freshly-bootstrapped empty repo, Phase 1 may be a no-op — proceed to Phase 2.

## Phase 2 — Brainstorm + design

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
> 6. **Board assignment (forward reference).** If `~/.autospec/project-map.yml`
>    exists, look up each issue's labels in `mappings:` and call
>    `gh project item-add <project_number> --owner <owner> --url <issue-url>`
>    for each match. If the file is missing, warn once and skip — do not fail
>    the phase. Full reader logic lands in PR B3 (#16); this step is a stub
>    today.
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


## Handoff

Phase 3.5 complete. Run `/autospec-run --profile <name>` to begin implementation.
