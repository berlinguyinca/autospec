---
name: autospec-define
description: Use when the user wants to plan a feature — bootstrap repo if missing, brainstorm a design spec, decompose into linked GitHub issues, or split an existing spec into issues. Stops after Phase 3 and hands off to /autospec-run for autonomous implementation.
---

# autospec-define workflow (harness-neutral)

Take the following feature request and run the planning half of the autospec pipeline:
**bootstrap repo (if missing) → investigate → design → spec → decomposed GitHub issues.** Stop after Phase 3; the implementation half is handed off to `/autospec-run`.

If the request asks to split, materialize, roadmap, decompose, or turn an
already-written spec into GitHub issues, use **Existing spec mode** below:
select a tracked `docs/specs/*.md` file, skip Phases 1-2, run Phase 3 and
Phase 3.5 against that spec, then continue to the Phase 3 pre-impl gate.

Manage your own context — never exceed 60%. Delegate to subagents whenever your harness supports it; do not investigate, write code, or design directly in the main conversation when a subagent can do it.

## Startup self-update

```bash
#!/usr/bin/env bash
# autospec-startup-self-update — see docs/specs/2026-05-01-autospec-startup-self-update-design.md
set +e
SKILL_NAME=autospec-define   # per-skill: autospec-define / autospec-run / autospec-listen / autospec-classify
if [ "${AUTOSPEC_NO_SELF_UPDATE:-0}" = "1" ]; then exit 0; fi
mkdir -p "$HOME/.autospec"
LOCKDIR="$HOME/.autospec/.update.lock.d"
LAST="$HOME/.autospec/last-update-check"
INSTALLED="$HOME/.autospec/installed-version"
NOW=$(date -u +%s)
if [ -f "$LAST" ]; then
    PREV=$(date -u -j -f '%Y-%m-%dT%H:%M:%SZ' "$(cat "$LAST" 2>/dev/null)" +%s 2>/dev/null \
        || date -u -d "$(cat "$LAST" 2>/dev/null)" +%s 2>/dev/null || echo 0)
    if [ "$((NOW - PREV))" -lt 86400 ]; then exit 0; fi
fi
if ! mkdir "$LOCKDIR" 2>/dev/null; then
    echo "WARN: self-update skipped (concurrent update in progress)" >&2; exit 0
fi
trap 'rmdir "$LOCKDIR" 2>/dev/null' EXIT
date -u +'%Y-%m-%dT%H:%M:%SZ' > "$LAST.tmp" && mv "$LAST.tmp" "$LAST"
REMOTE=$(curl -fsSL --max-time 5 \
    "https://api.github.com/repos/berlinguyinca/autospec/commits/main" \
    2>/dev/null | jq -r '.sha // empty' 2>/dev/null | cut -c1-7)
if [ -z "$REMOTE" ]; then
    echo "WARN: self-update skipped (network); continuing on installed version" >&2; exit 0
fi
LOCAL=$(cat "$INSTALLED" 2>/dev/null || true)
if [ "$REMOTE" = "$LOCAL" ]; then exit 0; fi
curl -fsSL --max-time 30 \
    "https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh" \
    | bash -s -- --skill all --harness all --update >/dev/null 2>&1
RC=$?
if [ "$RC" -ne 0 ]; then
    echo "WARN: self-update skipped (install rc=$RC); continuing on installed version" >&2; exit 0
fi
printf '%s\n' "$REMOTE" > "$INSTALLED.tmp" && mv "$INSTALLED.tmp" "$INSTALLED"
# Auto-init cross-tool memory (idempotent, <50ms fast-path)
bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/auto-init-memory.sh"
echo "[autospec] updated ${LOCAL:-fresh} → $REMOTE"
```

## Self-update mode

If the feature-request argument matches the regex `^\s*update\s*$` (case-insensitive, whitespace-padded), this skill enters self-update mode and does not run the normal pipeline:

1. **Detect harness** by checking which install path exists for this skill:
   - Claude Code: `~/.claude/skills/autospec-define/SKILL.md`
   - OpenCode:    `~/.config/opencode/agent/autospec-define.md`
   - Codex CLI:   `~/.codex/prompts/autospec-define.md`
2. **Re-install the full autospec suite from `main`** by piping the canonical installer:
   ```bash
   curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh | bash -s -- --skill all --harness all --update
   ```
   Run this one-liner once; it refreshes all autospec skills across all harnesses.
3. **Show the diff** between the prior installed file(s) and the freshly fetched copy (e.g. `diff <(cat <prior>) <(curl -fsSL ...SKILL.md)` or the equivalent recorded by the installer).
4. **Stop.** Do not enter Phase 0 / Phase 1 / any pipeline phase. Print the upgrade summary and return to the user.

If no install path is detected, print `Self-update: no installed copy of autospec-define found; run install.sh first.` and exit.

## Feature request

{FEATURE_DESCRIPTION}

---

## Existing spec mode

Use this mode when the request asks to split, materialize, roadmap, decompose,
or turn an existing spec into GitHub issues. Examples:
`split existing spec`, `split latest spec`, `turn this spec into GitHub issues`,
`roadmap docs/specs/2026-05-01-example-design.md`, or
`materialize the newest spec`.

Do not run Phase 1 or Phase 2 in this mode. The design already exists; this
mode is a shortcut into Phase 3 using the same issue decomposition and Phase 3.5
classification path as the normal pipeline.

1. **Verify repo.** Run the Phase 0 repo probes. If the current directory is
   not a git repo with a GitHub remote, use Phase 0 bootstrap before selecting
   a spec.
2. **Resolve candidate specs.**
   - If the request contains an explicit `docs/specs/*.md` path, use that file.
   - Otherwise list tracked and untracked local files matching `docs/specs/*.md`.
   - Sort by ISO date in the filename (`YYYY-MM-DD-...`) descending. For files
     without a date prefix, sort after dated files by filesystem modified time.
   - The first item is the default "newest" spec.
3. **Ask on ambiguity.**
   - If zero specs exist, stop with: `No docs/specs/*.md files found. Create or
     point me at a spec before running existing spec mode.`
   - If exactly one spec exists and no explicit path was provided, use it.
   - If more than one spec exists and no explicit path was provided, ask the user
     to confirm the default newest spec or choose another path. Show at most the
     five newest candidates with relative paths. Do not file issues until the
     user answers.
4. **Verify selected spec is filed.**
   - The selected path must be under `docs/specs/` and end in `.md`.
   - The selected file must be tracked on `origin/main` before Phase 3, so child
     issues can cite a stable GitHub URL. Verify with:
     `git fetch origin` and `git cat-file -e origin/main:<spec-path>`.
   - If the selected file is missing from `origin/main`, stop and tell the user:
     `Selected spec is not on origin/main yet: <spec-path>. Land the spec first,
     or run normal /autospec-define so Phase 2 can create and merge the spec PR.`
5. **Continue at Phase 3.** Capture `{selected_spec_path}` and its GitHub URL as
   `https://github.com/{repo}/blob/main/{selected_spec_path}`. Run Phase 3 and
   Phase 3.5 using that selected spec. Then proceed to the existing Phase 3
   pre-impl gate.

## Required capabilities & harness adapter

This workflow assumes five capabilities. Map each one to your harness's actual tool. If a capability is missing, use the listed fallback.

| Capability                  | Claude Code                          | OpenCode                                 | Codex CLI                                | Fallback if missing                                |
|-----------------------------|--------------------------------------|------------------------------------------|------------------------------------------|----------------------------------------------------|
| Read-only codebase research | `Agent` (subagent_type=Explore)      | `task` agent in read-only mode           | `apply_patch` read-only / shell `grep`   | Do the search in-thread with `rg`/`grep`           |
| Foreground delegation       | `Agent` (subagent_type=general-purpose) | nested `task` agent, await output     | spawn nested CLI session                 | Do the work in-thread (more context cost)          |
| Background delegation       | `Agent` with `run_in_background: true` | detached `task` agent                  | nohup'd CLI session writing to a logfile | Run the monitor in a separate terminal/tmux pane   |
| Ask the user a question     | `AskUserQuestion`                    | inline prompt                            | inline prompt                            | Ask in the response and wait for the next turn     |
| Self-paced future wakeup    | `ScheduleWakeup` inside a `/loop`    | a recurring `task` or local `cron`       | local `cron`/`launchd` calling the CLI   | The user runs a status-update prompt manually      |
| Subagent model tier         | Tier A: `opus` + `ultrathink`; Tier B: `sonnet` + medium thinking | Tier A: top `task` model + high reasoning; Tier B: smaller-tier `task` + medium reasoning | Tier A: top GPT + `reasoning_effort=high`; Tier B: `gpt-5.1-codex-spark` + `reasoning_effort=medium` | Honor the per-phase tier mapping in AGENTS.md; retry the same subagent UP on unavailability |
| Subagent dispatch policy   | per AGENTS.md decision matrix        | per AGENTS.md decision matrix            | per AGENTS.md decision matrix            | inline with main-session token cost                |

**Persistent project notes**: write durable preferences to **`AGENTS.md`** in the repo root — this is the de-facto standard recognized by Claude Code (also reads `CLAUDE.md`), OpenCode, and Codex. If your harness has its own private memory (e.g. Claude Code's `~/.claude/.../memory/`), mirror the same content there. Per AGENTS.md, subagent dispatches use a **two-tier policy**: Tier A (top model + extended thinking) for spec work (research, decompose, review/label); Tier B (cheaper model + medium thinking) for implementation work (Phase 4 implementer + LGTM review — not used by this skill). The orchestrator keeps the user's invoked model. Fall back UP the tier on quota/capacity or other unavailability by retrying the same subagent with the stronger tier while preserving parent context.

---

## Harness detection (run once at skill start, before Phase 0)

Detect your harness by checking available tools before any phase:

1. **Claude Code** — the `Agent` tool with a `subagent_type` parameter is available.
   - `TIER_A` = `opus` + `ultrathink`  (model ID: claude-opus-4-7)
   - `TIER_B` = `sonnet`               (model ID: claude-sonnet-4-6)

2. **OpenCode** — a `task` tool with model/tier configuration is available (no `subagent_type`).
   - `TIER_A` = top-tier task model + high reasoning
   - `TIER_B` = smaller-tier task model + medium reasoning

3. **Codex CLI** — neither `Agent` nor a configurable `task` tool is available; `apply_patch` is the primary edit tool.
   - `TIER_A` = current top GPT model + `reasoning_effort=high`
   - `TIER_B` = `gpt-5.1-codex-spark` + `reasoning_effort=medium`

**Fallback rule:** If `TIER_B` is not available in your harness (model unknown, quota/capacity failure, authorization failure, or tool call returns an error for that model), silently retry the same subagent dispatch with `TIER_A`. Preserve the parent context on retry; for Codex native subagents, fork/inherit the current conversation context and use the latest top GPT model instead of moving the work into the main session. Never ask the user.

Hold `TIER_A` and `TIER_B` for the entire skill run. Every "Tier A" and "Tier B" reference below resolves to these harness-specific values.

## Relevant memory injection (run-start, once)

Before executing the main pipeline phases, call the injector to surface relevant saved lessons:

```bash
bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/inject-relevant-memory.sh" \
  --context "<skill-relevant keywords from request/issue/spec>" \
  --top-k 5
```

Prepend the output block (if non-empty) to your working context. This surfaces lessons like
`feedback_bash_return_trap_leak.md` that prevent re-occurrence of known pitfalls.

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

> **Model tier:** `TIER_A` (spec work) — top model with extended thinking; resolved at startup.

> **Phase 1 bounded-context rule.** Launch the research subagent with a fresh,
> bounded handoff. Do NOT fork, inherit, or compact the full parent conversation into Phase 1. The handoff may contain only: the feature request,
> repo slug/root, a short git status summary, relevant memory-injection output,
> explicit user-mentioned paths/surfaces, and these Phase 1 instructions. For
> Codex native subagents, set `fork_context=false`; for Claude/OpenCode, use the
> equivalent fresh/no-history task mode when available. If the harness cannot
> start a fresh subagent, or returns a context window or remote compact failure,
> do not retry with inherited context. Fall back to bounded local read-only `rg`/file-read investigation, label the result as `Phase 1 fallback`, and
> continue to Phase 2 with the best evidence gathered.

If the feature touches a remote system (DB, server, S3), run a real query against the actual data to confirm the problem statement before designing. Surface the concrete numbers in the design.

For a freshly-bootstrapped empty repo, Phase 1 may be a no-op — proceed to Phase 2.

## Phase 2 — Brainstorm + design

> **Spec quality is the bottleneck.** Phase 2's output drives every downstream cycle's cost; if you care about spec quality, invoke this skill with your top-tier model (Claude Code: `claude-code --model opus`; Codex: top GPT). Phase 2 itself runs in the orchestrator (no subagent dispatch) — your invocation model IS the spec model. Subagents in Phases 1, 3, 3.5 follow this lead by selecting Tier A; Phase 4 implementation work uses Tier B. See AGENTS.md.

## Team personality selection

Before the Architecture question, decide what kind of team should solve this
request. Infer the team personality from the user's request, Phase 1 evidence,
repository labels, relevant memory files, and past specs under `docs/specs/`.
The goal is to shape the design as a fitting team would reason about it, not to
treat every request as a generic "fix this bug" task.

Write a **Team personality** section into the design spec with:
- the selected team name,
- 3-6 roles,
- why this team fits the request,
- the risks this team is expected to notice,
- any team emphasis that should carry into child issues.

Also write a **Review counter-team** subsection. It must be a different
emphasis from the implementation team and should challenge likely blind spots,
not duplicate the same perspective. Include:
- the counter-team name,
- 2-5 roles,
- which assumptions or failure modes this team should challenge,
- how review should stay inside the issue scope while applying that lens.

If confidence is low, ask the user explicitly: `What kind of team should solve this problem?`
Offer exactly these five starter combinations:
1. **Core product engineering** — product manager, architect, backend developer, frontend developer, test engineer.
2. **Reliability/backend** — backend developer, platform engineer, sysadmin/SRE, database engineer, security advisor.
3. **Frontend/product** — frontend developer, UX designer, accessibility reviewer, API/backend developer, QA engineer.
4. **Security-sensitive** — security advisor, architect, backend developer, platform engineer, test engineer.
5. **Legacy/refactor** — architect, maintainer, backend/frontend developer as needed, test engineer, documentation owner.

If one option is clearly best, proceed without asking and record the confidence
and evidence in the spec. If two or more options are plausible and would change
the architecture, tests, or decomposition, ask before continuing.

Derive the Review counter-team after choosing the implementation team:
- Frontend/product implementation teams should be reviewed by accessibility, API contract, and QA perspectives.
- Reliability/backend implementation teams should be reviewed by security, operations, and data-integrity perspectives.
- Security-sensitive implementation teams should be reviewed by product/UX, maintainer, and test perspectives.
- Legacy/refactor implementation teams should be reviewed by maintainer, regression-test, and architecture perspectives.
- Core product engineering implementation teams should be reviewed by security, operations, accessibility, and maintainability perspectives.

Run a structured brainstorm — one question at a time, get explicit approval after each section:

1. **Architecture** — where does new code live, what existing patterns does it follow.
2. **Interactivity / API shape** — how does the user/caller drive it; commands, keys, endpoints, fields.
3. **Data model** — types, columns, persistence boundary.
4. **Error handling** — failure modes, recovery, user-visible signals.
5. **Testing** — unit / integration / e2e split, real services vs anything else (rule per AGENTS.md: real services). For app/UI work, include an `autospec-qa` revalidation plan that maps each spec requirement to running-app checks for text inputs, validation, selects/dropdowns, buttons, forms, API effects, accessibility, responsive behavior, negative paths, regenerated automated tests, proof artifacts, reliability contract, control intent ledger, no-mock minimum coverage, mutation/breakage proof, duplicate-code review, post-merge canary, new-code intent checks, artifact freshness, evidence provenance, console/network error gates, spec contradiction detection, data lifecycle proof, observability, flake quarantine, legacy removal/spec cleanup, and reliability exhaustion loop.

Write the agreed design to `docs/specs/YYYY-MM-DD-<topic>-design.md`, then run a **fresh-eyes self-review** before any issue is filed:

1. **Placeholders** — grep the spec for `TBD`, `TODO`, `later`, `fixme`, `tbd`, `XXX`. Fill in or surface to the operator as an open question; do not file issues against an undecided spec.
2. **Internal consistency** — re-read each section in order. Do any two sections contradict each other? Does the architecture match the feature descriptions? Fix inline.
3. **Ambiguity** — could any requirement be interpreted two different ways? Pick one and make it explicit in the spec text.
4. **Scope** — is this focused enough for a single multi-issue pipeline, or does it span independent subsystems that each need their own `/autospec-define` run? If the latter, stop and ask the operator to decompose before proceeding.
5. **Critical improvement** — ask: "What else could fail even if this spec and its obvious tests pass, and how could this be better?" Add the highest-risk answer to Testing, Acceptance Criteria, or Review counter-team. For app/UI work, explicitly consider mocked-vs-deployed behavior, backend fallbacks, user-visible outcomes, and no-mock smoke coverage.

The spec must be implementable end-to-end by an agent reading only the spec.

If this is a fresh repo, commit the spec to `main` directly (`git add docs/... && git commit -m "docs: <topic> design spec" && git push`) so subsequent issues can reference it as a tracked file.

For an existing repo, land the spec via a short-lived PR so CI can validate it:
1. Create branch `feat/spec-<topic>`, commit `docs/specs/...design.md`, push.
2. `gh pr create --base main --head feat/spec-<topic> --title "docs: <topic> design spec" --body "Source spec: docs/specs/..."`
3. Wait for required CI checks to pass (`gh pr checks <#> --repo {repo}`).
4. Unless `AUTOSPEC_NO_AUTOMERGE_SPEC=1` is set, admin-merge: `gh pr merge <#> --admin --squash --delete-branch --repo {repo}`. If the env var is set, pause and ask the user to merge before continuing.
5. Verify the spec is reachable at `https://github.com/{repo}/blob/main/docs/specs/...` before dispatching Phase 3.

## Phase 3 — Decompose into linked GitHub issues (delegate)

If Existing spec mode is active, use `{selected_spec_path}` and its GitHub URL.
Otherwise use the spec path written and merged in Phase 2.

Dispatch a **foreground subagent** with this prompt (substitute the spec path and `{repo}`):

> **Model tier:** `TIER_A` (spec work) — top model with extended thinking; resolved at startup.
>
> Read the selected design spec at `<spec-path>` (`<spec-github-url>`) and split it into linked GitHub issues for {repo}.
>
> Create labels (idempotent with `--force`): `auto-implement` (#0e8a16), `epic` (#b60205), `autospec:v2-flow` (#0e8a16, description: "Routes to absorbed-discipline Phase 4 implementer"), plus any domain labels the spec calls for. Then create exactly N issues — first an EPIC umbrella (no `auto-implement` label, just `epic` + domain), then N-1 children all carrying `auto-implement` **and `autospec:v2-flow`**. The `autospec:v2-flow` label routes the child to the Phase 4 implementer that absorbs turbo's expand → implement → finalize → peer-review → evaluate discipline; children filed without it fall back to the legacy implementer path. After creating children, edit the umbrella body with a checklist linking them. Return JSON: `{umbrella, children:[…], labels_created:[…]}`. Use `gh` CLI only. Do NOT modify code. Do NOT push branches. Do NOT create PRs.
>
> Before drafting each candidate child issue, ask yourself three shell-structure questions (internal — do NOT write them into the issue body):
>
> - **Produces** — What new files, exports, or behavior does this issue create? If the answer is "edits scattered across many existing files with no clear new contract", reconsider the boundary.
> - **Consumes** — What existing files or outputs of earlier issues does this depend on? Each named dependency on an earlier issue translates to a `Depends on issue #N` line in the body.
> - **Covers** — Which sections of the spec does this issue implement? If multiple unrelated sections, split. If no spec section, reconsider whether the issue belongs.
>
> If two adjacent candidate issues have heavy mutual Consumes/Produces overlap, they probably want to be merged. If one issue has more than ~5 named Produces, it probably wants to be split.
>
> Each child body must be a **self-contained mini-spec** sized for execution by a 32B-class local LLM, with these sections in order:
>
> - **Goal** — 1 sentence outcome.
> - **Source spec** — `<spec-path>` + `<spec-github-url>` of the design doc this issue derives from.
> - **Team personality** — copy the spec's selected team name, roles, and issue-relevant emphasis. If the selected spec lacks this section, infer it from the request, past specs, repository labels, and memory; if confidence is low, stop and ask the operator to choose from the five starter combinations in Phase 2 before filing issues.
> - **Review counter-team** — copy the spec's counter-team name, roles, and issue-relevant blind spots to challenge. If the selected spec lacks this section, derive a different review emphasis from the Team personality and issue risk before filing.
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
>
> **Sizing caps (hard, per spec §3.4):**
>
> - **Body ≤400 words** including all sections.
> - **Implementation outline ≤30 lines** (file paths + function signatures).
> - **Files touched ≤3** per child issue.
> - If a candidate child would exceed any cap, split into a parent + child pair with a `Depends on` edge.
> - The whole spec + a single child issue body must fit comfortably in a 60–120k context window.
>
> Self-check each issue against the caps **before** calling `gh issue create`. If a cap is violated and a split is not feasible, surface the issue inline (print the over-cap body to the operator) instead of filing it.
>
> **Pre-filing lint loop (adaptive, MAX_LINT_RETRIES=5):** For each candidate child body, before calling `gh issue create`, write the body to `/tmp/draft-<slug>.md` and run `bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/lint-issue.sh" /tmp/draft-<slug>.md`. If the exit code is non-zero, map each `RULE_ID: <desc>` finding to an actionable directive using the table below, append all directives to the next generation prompt as cumulative context, and regenerate. Repeat up to `MAX_LINT_RETRIES=5` attempts. If attempt 5 still fails, print all 5 drafts plus accumulated findings inline and **skip** that child (do not file); continue to the next child. On pass (exit 0), proceed to `gh issue create` as normal.
>
> | Finding | Directive appended to next prompt |
> |---|---|
> | `GOAL_VAGUE: "improve" used without concrete object` | `AVOID: bare verb \`improve\` without naming a file path, command, label, or number in the same sentence.` |
> | `GOAL_HEDGE: "should probably"` | `AVOID: hedging words \`should/might/could try/try to\`. State the outcome flatly.` |
> | `GOAL_NOT_ONE_SENTENCE: N terminals` | `REWRITE: Goal must be exactly one sentence ending with a single . ? or !` |
> | `AC_PROSE: line N not a checkbox` | `FORMAT: every AC line must start with \`- [ ] \` followed by content.` |
> | `AC_SUBJECTIVE: "looks clean"` | `AVOID: subjective adjectives \`looks/feels/seems/clean/elegant\` in AC items. Use a \`grep\`/\`test\`/\`diff\`/\`bats\` command instead.` |
> | `AC_TOO_LONG: N chars` | `SHORTEN: AC item exceeds 120 chars; split into two items or compress to one assertion.` |
> | `AC_EMPTY` | `ADD: Acceptance criteria section must contain at least one \`- [ ] \` checkbox item.` |
> | `SMOKE_MULTI_LINE: N lines` | `COLLAPSE: Primary smoke test must be exactly one command line. Use \`&&\` to chain or move setup to Operator/full verification.` |
> | `SMOKE_PLACEHOLDER: contains "<TODO>"` | `RESOLVE: Replace placeholders \`<TODO>/TBD/XXX/...\` with the actual command before filing.` |
> | `SMOKE_NOT_FENCED` | `ADD: Primary smoke test section must contain exactly one fenced code block.` |

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

> **Model tier:** `TIER_A` (spec work) — top model with extended thinking; resolved at startup.
>
> Walk every child issue created in Phase 3 (skip any issue carrying the
> `type:tracker` label). For each:
>
> 1. **Stage context.** Read `gh issue view <N> --repo {repo} --json title,body,labels`. The
>    body should already contain `## Files to read first`, `## Implementation scope`, `## Team personality`, and `## Review counter-team`. If any is missing, add label
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
>    `gh label create reasoning:deep    --color c2e0c6 --force --repo {repo}`,
>    `gh label create needs-quality-bar --color fbca04 --force --repo {repo}`.
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
> 8. **Post-filing quality audit.** For each child issue (skip `type:tracker`):
>    - Pull body: `gh issue view <N> --repo {repo} --json body -q .body > /tmp/audit-<N>.md`
>    - Run: `bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/lint-issue.sh" /tmp/audit-<N>.md`
>    - On non-zero exit (lint fails):
>      - Apply label: `gh issue edit <N> --add-label needs-quality-bar --repo {repo}`
>      - Insert `## Quality lint` block (idempotent, between `<!-- autospec-quality:begin -->` and `<!-- autospec-quality:end -->` markers) via `gh issue edit <N> --body-file <tmp>`. Block format:
>        ```markdown
>        ## Quality lint
>
>        - **GOAL** — <1-line finding>.
>        - **AC#<n>** — <1-line finding>.
>        - **SMOKE** — <1-line finding>.
>
>        <!-- autospec-quality:begin -->
>        *Auto-linted by Phase 3.5 on YYYY-MM-DD.*
>        <!-- autospec-quality:end -->
>        ```
>      - Comment findings: `gh issue comment <N> --body "<findings>" --repo {repo}`
>    - Do NOT remove `auto-implement` label. Operator decides whether to proceed.
>
> 9. **Run-end summary.** Print to stdout:
>    ```
>    Phase 3.5 summary on {repo}
>    - classified: N
>    - skipped (needs-autospec-template): M
>    - ctx:32k=A  ctx:64k=B  ctx:120k=C
>    - reasoning:shallow=X  reasoning:medium=Y  reasoning:deep=Z
>    - boards assigned: <K> (or "skipped — no project-map.yml")
>    - dep warnings: <count>; circular cycles: <count>
>    ```

---

## Phase 3.75 — Architectural alignment (delegate)

After Phase 3.5 has applied `ctx:*` / `reasoning:*` labels and before the
pre-impl gate, run a Tier-A (opus) subagent that reads all child issue bodies
plus the source spec and produces a `Shared contracts` summary. This prevents
each child issue from inventing incompatible type signatures, file-layout
conventions, and naming schemes — integration mismatches that are caught only
in Phase 5.5.

The subagent must:

1. Read the source spec and all child issue bodies (skip the umbrella/tracker).
2. Extract all public interfaces, function signatures, data types, file paths,
   and naming conventions mentioned across the issues.
3. Produce a `## Shared contracts` summary as a markdown block with:
   - Type signatures / data structures used across ≥2 issues.
   - File-layout constraints (which file owns what).
   - Naming conventions (e.g. class names, env var prefixes).
4. Patch each child issue body by appending:

```
<!-- autospec-shared-contracts:begin -->
## Shared contracts

<summary block from step 3>
<!-- autospec-shared-contracts:end -->
```

   using `gh issue edit <N> --body "$(gh issue view <N> --json body -q .body)<newblock>"`.
   The patch is idempotent — skip if `<!-- autospec-shared-contracts:begin -->` is already
   present in the body.

If there are fewer than 2 child issues, or all child issues are in the same
file (no cross-issue interface), skip Phase 3.75 and log:
`"Phase 3.75: skipped (single-file scope)"`

---

## Spec supersession (recency)

Specs in `docs/specs/` accumulate over time. When a newer spec adds, modifies, or
removes behavior that an earlier spec described, autospec applies
**implicit-by-recency supersession** (issue #635): the spec whose
last-modifying commit on the current branch is most recent wins for any
overlapping behavior (untracked specs fall back to filesystem mtime).
Operators do NOT write `Supersedes:` frontmatter — recency alone decides.

During Phase 3 decomposition, before filing a candidate child issue, resolve
the authoritative spec for each behavior the issue touches. Use the shared
helper:

```bash
bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/resolve-spec-supersession.sh" "<behavior-key>"
```

If the resolver prints a spec path other than the one currently being
decomposed, the candidate behavior has already been overridden by a newer
spec — skip filing the issue (it would re-introduce removed behavior) and
note the supersession in the decompose run log. If the resolver exits 1 (no
overlap), proceed normally: the behavior is new to this spec.

See `tests/resolve-spec-supersession.bats` for the resolver contract:
no-overlap → exit 1, single overlap → that spec wins, two/three-way overlap
→ most recent wins, deleted specs are excluded.

## Phase 3 pre-impl gate

After Phase 3 decomposition completes (and Phase 3.5 review-and-label has
applied `ctx:*` / `reasoning:*` labels), end Phase 3 by asking the user
this verbatim question (no auto-launch — always ask explicitly, per spec
§3.3):

> `"Spec written, N issues filed. Start /autospec-run now, defer to your external daemon, or keep refining? [run / defer / refine]"`

Substitute `N` with the actual issue count from Phase 3. Default highlight
is **`defer`** for `/autospec-define` (the plan-only skill — the user
invoked `/autospec-define` expecting plan-only behavior, so the gate
defaults to `defer`).

- **`run`** — invoke `/autospec-run` in the current session.
- **`defer`** (default for `/autospec-define`) — print
  `"Issues are ready. Your external monitor will pick them up. Exiting."`
  and stop without launching `/autospec-run`. The just-filed `auto-implement`
  queue persists on the GitHub side so an external daemon (or a later
  `/autospec-run` invocation) can pick it up.
- **`refine`** — re-enter Phase 2 from question 1 (Architecture).

No daemon auto-detection — always ask explicitly.

After all decomposition issues are filed (and before asking the gate question),
file one additional child issue titled
`"Phase 5.5 audit + remediation — <feature-name>"` with labels
`auto-implement,priority:high,audit`, then post a comment on the umbrella issue
linking it: `"Phase 5.5 audit tracker: #<N>"`.

After the user answers `run` or `defer` (i.e. the design is locked and issues
are filed), call `diary-write.sh` to record the brainstorm decision:

```bash
bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/diary-write.sh" \
  --phase 1 \
  --event brainstorm-locked \
  --body "Spec locked: ${SPEC_PATH}. Gate answer: ${GATE_ANSWER}. Issues filed: ${ISSUE_COUNT}." \
  2>/dev/null || true
```

(Best-effort — never blocks the gate flow.)

## Handoff

If the gate answer was `run`, hand off to `/autospec-run --profile <name>`
to begin implementation. If the answer was `defer`, the run ends here and
the queue is left for an external monitor.

## Init mode

Triggered by `--init` flag or auto-detected when code exists but `docs/specs/` is empty or no doc carries an `autospec-doc-scope` annotation.

### Auto-detect prompt

When auto-detecting, ask once:

> This repo has source code but no autospec docs. Run reverse-engineer first?
> **[yes / no / always-this-repo]**

- `yes` — run `--init` once for this invocation.
- `no` — skip; proceed to Phase 1 normally.
- `always-this-repo` — run `--init` and write `.autospec/init-done.flag`; future invocations skip this prompt.

If `.autospec/init-done.flag` exists, skip the prompt entirely.

### --init execution

1. Run tree-sitter scan across the repo source tree to enumerate modules, public symbols, and entry points.
2. Emit one ARCHITECTURE-level spec to `docs/specs/<repo-slug>-architecture.md`.
3. Emit per-module specs to `docs/specs/<module>-spec.md` for each discovered module.
4. Generate initial docs via `gen-docs-from-spec.mjs`:
   - `docs/USER_MANUAL.md`
   - `docs/API_REFERENCE.md`
   - `docs/ARCHITECTURE.md`
   - `llms.txt`
   - `.llm-manifest.json`
5. Open a `feat/spec-reverse-engineer-init` PR with all generated artifacts.
6. Admin-merge the doc PR before proceeding to Phase 3 decomposition.

### Auto-docs step (after spec PR merges)

After any spec PR merges in normal `autospec-define` flow, regenerate
documentation through the single doc engine — invoke `/autospec-doc`
(incremental, scoped to the docs the merged spec affects), routed via
`scripts/doc-orchestrator.mjs`:

```bash
bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/doc-orchestrator.mjs"
```

`/autospec-doc` is the only documentation-generation path; there is no
parallel generator. Open a `docs/` update PR and admin-merge it before Phase 3
begins. Skip if the spec change touches only non-public implementation details
(no change to public API surface, no new CLI flags, no new env vars).

## Autonomous mode

When `/autospec-define --autonomous "<request>"` runs, or when
`~/.autospec/autonomous.flag` exists at run start, Phase 2 and the Phase 3
pre-impl gate skip user-facing confirmations.

Branches taken in autonomous mode:

- **Phase 2 brainstorm** — do NOT ask the Architecture / Interactivity /
  Data model / Error handling / Testing questions one at a time. Synthesize
  the entire spec from (a) the user's request text, (b) the last ~10
  conversation turns, (c) `git log --since="7 days ago"` recent commits,
  and (d) similar prior work in `docs/specs/**`. For each unverifiable
  inference, mark the line in the spec body with a
  `> AUTONOMOUS ASSUMPTION:` blockquote.
- **Phase 3 pre-impl gate** — default to `run`. Do NOT ask
  `[run / defer / refine]`. Proceed to issue filing without confirmation.
- **Issue body confirmation** — `gh issue create` runs without per-issue
  approval. The existing lint loop, sizing caps, and Phase 3.5 model-fit
  classification still apply unchanged.

If the orchestrator cannot generate a coherent spec from available context
(e.g. "make it better" with no surface evidence), autonomous mode FAILS
LOUDLY with `code_health:autonomous_input_insufficient` and prints the
specific missing fields plus a recommended interactive command to retry.
Do not silently pick arbitrary defaults.

Safety guardrails are NOT bypassed. Before each would-have-asked decision,
the orchestrator invokes `scripts/autospec-autonomy-gate.sh` with
`--check all`. Exit 1 surfaces the confirmation even in autonomous mode:

- Destructive remote actions (force-push, repo archive/delete, mass label
  changes, prod DB writes).
- Out-of-scope planned files (extend beyond Goal + Implementation outline).
- Cost gate (`AUTOSPEC_AUTONOMOUS_ISSUE_CAP`, default 10; or
  `AUTOSPEC_AUTONOMOUS_TOKEN_CAP`, default 500k).
- Existing `feedback_autospec_autonomy_scope.md` rules.

The handoff to `/autospec-run` preserves the `--autonomous` flag so the
implementation half honors the same telemetry tagging.
