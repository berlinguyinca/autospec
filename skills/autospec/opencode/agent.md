---
description: Use when the user wants to ship a feature end-to-end across multiple commits — bootstraps repo if missing, brainstorms a design spec, decomposes into linked GitHub issues, or splits an existing spec into issues, then runs an autonomous implementation loop with auto-merge.
mode: primary
---

# autospec workflow (harness-neutral)

Take the following feature request and ship it through the full pipeline:
**bootstrap repo (if missing) → investigate → design → spec → decomposed GitHub issues → autonomous implementation with auto-merge → periodic status updates → final report.**

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
SKILL_NAME=autospec   # per-skill: autospec-define / autospec-run / autospec-listen / autospec-classify
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

Decide this purely from the request text the harness handed you. Do NOT
shell out (no `grep`, `sed`, `[[ =~ ]]`, command substitution, etc.) to
test the user's free-form request — passing it through a shell is what
historically tripped harness permission engines (e.g. parse errors near
backtick/pipe characters in the user's prose). Read the request, normalize
it in your reasoning (collapse whitespace, trim, lowercase), and if the result is
exactly `update`, this skill enters self-update mode and does NOT run the
normal pipeline.

1. **Detect harness** by checking which install path exists for this skill:
   - Claude Code: `~/.claude/skills/autospec/SKILL.md`
   - OpenCode:    `~/.config/opencode/agent/autospec.md`
   - Codex CLI:   `~/.codex/prompts/autospec.md`
2. **Re-install the full autospec suite from `main`** by piping the canonical installer:
   ```bash
   curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh | bash -s -- --skill all --harness all --update
   ```
   Run this one-liner once; it refreshes all autospec skills across all harnesses.
3. **Show the diff** between the prior installed file(s) and the freshly fetched copy (e.g. `diff <(cat <prior>) <(curl -fsSL ...SKILL.md)` or the equivalent recorded by the installer).
4. **Stop.** Do not enter Phase 0 / Phase 1 / any pipeline phase. Print the upgrade summary and return to the user.

If no install path is detected, print `Self-update: no installed copy of autospec found; run install.sh first.` and exit.

## Stop mode

Apply the same read-and-normalize approach used for self-update mode (do
NOT shell out the user's request). If the normalized request is exactly
`stop`, or `stop` followed by one or more `--<word>` flags (examples:
`stop`, `stop --graceful`, `stop --immediate`, `stop --status`,
`stop --resume`, `stop --help`, `stop --flag`), this skill enters stop
mode and does NOT run the normal pipeline. When dispatching, pass any
`--<flag>` tokens the user provided as separate words to the helper.

1. Dispatch to `bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-stop.sh" <args>`.
2. Print the helper's stdout to the user.
3. Stop. Do not enter Phase 0 or any pipeline phase.

## Feature request

{FEATURE_DESCRIPTION}


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
     or run normal /autospec so Phase 2 can create and merge the spec PR.`
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

**Persistent project notes**: write durable preferences to **`AGENTS.md`** in the repo root — this is the de-facto standard recognized by Claude Code (also reads `CLAUDE.md`), OpenCode, and Codex. If your harness has its own private memory (e.g. Claude Code's `~/.claude/.../memory/`), mirror the same content there. Per AGENTS.md, subagent dispatches use a **two-tier policy**: Tier A (top model + extended thinking) for spec work (research, decompose, review/label); Tier B (cheaper model + medium thinking) for implementation work (Phase 4 implementer + LGTM review). The orchestrator keeps the user's invoked model. Fall back UP the tier on quota/capacity or other unavailability by retrying the same subagent with the stronger tier while preserving parent context.

## Physical STL / CAD design guardrails

When the request involves STL, CAD, 3D-printable fixtures, workholding,
vacuum, pneumatic, hydraulic, dust-collection, hose, fitting, gasket, or
other physical models, carry these requirements into Phase 2 specs, Phase 3
child issues, and Phase 4 verification:

- Treat physical function as acceptance criteria, not visual intent. Verify
  mountability, tool access, fitting clearance, gasket compression, storage fit,
  printer build volume, and expected operating orientation.
- Do not allow blocked ports, obstructed hoses/tubes, capped passages, hidden
  dead-end channels, or geometry that interferes with the model's purpose.
- Around every port, fitting, connector, tap bore, hose socket, screw head, and
  service tool path, require at least 5 mm of free working clearance in every
  direction unless the user explicitly specifies a tighter envelope.
- Around every gasket path, require at least 5 mm of continuous plastic margin
  outside the gasket. Gasket grooves must not expose an edge or be crossed by
  mounts, screws, ribs, channels, holders, or workpiece contact geometry.
- For NPT or other tapped ports, prove the tap and installed fitting can enter
  straight or along the specified angled axis without collisions. Ports and
  fitting bosses must be integrated into reinforced body geometry, not left as
  fragile stand-alone protrusions.
- For low-flow vacuum systems, do not add relief valves, vents, restriction
  orifices, small balancing holes, or intentional leaks unless the user
  explicitly asks for them and the spec records why the pump can tolerate them.
- For sealed or flowing parts, require gas/fluid/vacuum/dust-flow simulation or
  deterministic geometry checks that prove no leaks, no disconnected internal
  circuits, no blocked passages, and adequate inlet-to-outlet connectivity.
- For dust collection, preserve maximum practical duct cross-section and
  suction path continuity. No vacuum tubing, air port, dust passage, or hose
  connection may be narrowed or blocked by ribs, mounts, bosses, decorative
  geometry, or slicer support assumptions.
- For functional openings such as dust hood mouths, plenum intakes, vacuum
  channels, gasket windows, and hose throats, add deterministic projection or
  section keepout checks. Prove that reinforcements, sockets, bosses, ribs, and
  cutters do not intrude into the working opening from any operating angle.
- For every generated STL, require visual QA from at least 16 angles: right,
  left, front, back, top, bottom, top-right 45, top-left 45, top-front 45,
  top-back 45, front-right 45, front-left 45, back-right 45, back-left 45,
  bottom-right 45, and bottom-left 45. Renders must be nonblank, well-framed,
  and reviewed for obvious gaps, overlaps, obstructions, unsupported fragile
  features, and unexpected protrusions.
- Regenerate from a clean build directory before release, then verify the build
  directory contains every expected STL plus any PDFs, sections, renders, and
  acceptance artifacts named by the spec.


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

## Project configuration preflight

Before Phase 0, look for `.autospec/autospec.yml` at the repository root.

If the file is missing and the current directory is already inside a git repo,
run `/autospec-sweep init` behavior first:

```bash
bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-sweep-wizard.sh" init
```

Accept the wizard defaults unless repo findings require a project-specific
answer. The generated `.autospec/autospec.yml` is tracked project source and
must be committed with the next autospec spec/config change. After it exists,
read it before deciding which autospec phases are enabled.

If the file is missing because Phase 0 must bootstrap a brand-new repo, create
it immediately after the initial repo scaffold and before Phase 1. Use all
steps enabled by default, strict isolation, `team_personality: auto`, and
continuous improvement enabled for docs, tests, and code.

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

> **Hard limits.** Max 25 tool calls. If 3 consecutive read/grep calls return nothing useful, stop and write your best-effort summary even if incomplete. Do not retry the same query verbatim. No wall-clock cap.

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
5. **Testing** — unit / integration / e2e split, real services vs anything else (rule per AGENTS.md: real services).

Write the agreed design to `docs/specs/YYYY-MM-DD-<topic>-design.md`. Self-review for placeholders, contradictions, ambiguity, scope. The spec must be implementable end-to-end by an agent reading only the spec.

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
> Create labels (idempotent with `--force`): `auto-implement` (#0e8a16), `epic` (#b60205), plus any domain labels the spec calls for. Then create exactly N issues — first an EPIC umbrella (no `auto-implement` label, just `epic` + domain), then N-1 children all carrying `auto-implement`. After creating children, edit the umbrella body with a checklist linking them. Return JSON: `{umbrella, children:[…], labels_created:[…]}`. Use `gh` CLI only. Do NOT modify code. Do NOT push branches. Do NOT create PRs.
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

## Phase 3 pre-impl gate

After Phase 3 decomposition completes (and Phase 3.5 review-and-label has
applied `ctx:*` / `reasoning:*` labels), end Phase 3 by asking the user
this verbatim question (no auto-launch — always ask explicitly, per spec
§3.3):

> `"Spec written, N issues filed. Start /autospec-run now, defer to your external daemon, or keep refining? [run / defer / refine]"`

Substitute `N` with the actual issue count from Phase 3. Default highlight
is **`run`** for `/autospec` (the umbrella end-to-end skill — the user
invoked `/autospec` expecting end-to-end shipping, so the gate defaults
to `run`).

- **`run`** (default for `/autospec`) — proceed to Phase 4 (the existing
  background autonomous monitor launch path below).
- **`defer`** — print
  `"Issues are ready. Your external monitor will pick them up. Exiting."`
  and stop. For `/autospec`, this means skipping Phase 4–6 entirely; the
  just-filed `auto-implement` queue persists on the GitHub side so an
  external daemon (or a later `/autospec-run` invocation) can pick it up.
- **`refine`** — re-enter Phase 2 from question 1 (Architecture).

No daemon auto-detection — always ask explicitly.

Once `run` is selected and `/autospec-run` starts, do not ask operator questions again for this run unless a true hard blocker requires explicit manual recovery.

## Phase 4 — Background autonomous monitor

Record this durable preference in `AGENTS.md` (idempotent — skip if already present):

> **Auto-merge authority for auto-implement PRs.** Admin-merge auto-implement PRs (`gh pr merge <#> --admin --squash --delete-branch`) when (a) all required CI checks pass — slow optional checks like TeamCity may be pending and that's acceptable, (b) the self-review subagent returned `LGTM`, (c) PR closes an `auto-implement` issue from a `feat/*` branch.

**Off-peak tip:** For queues of 10+ issues (8+ hour runs), consider launching at night or on weekends. Usage limits are shared across all sessions — running long batches off-peak preserves daytime tokens for interactive work.

Then launch a **background monitor loop** — the orchestrator relaunches the monitor with fresh context after each batch of `AUTOSPEC_BATCH_SIZE` issues (default: 3). The monitor is stateless: all persistent state lives in GitHub labels and heartbeat files, so relaunches are always safe.

```
batch_num=1
while true:
  launch background subagent (pass batch_num; AUTOSPEC_BATCH_SIZE=${AUTOSPEC_BATCH_SIZE:-3})
  wait for task-notification (monitor agent completes)

  # Read and consume the batch-done signal.
  if [ -f "$HOME/.autospec/batch-done.json" ]; then
    status=$(jq -r .status "$HOME/.autospec/batch-done.json" 2>/dev/null || echo "BATCH_COMPLETE")
    rm -f "$HOME/.autospec/batch-done.json"
  else
    status="BATCH_COMPLETE"   # monitor crashed / overflowed — safe to relaunch
  fi

  if [ "$status" = "ALL_DONE" ]; then
    break   # proceed to Phase 6 final report
  fi

  batch_num=$((batch_num + 1))
  echo "[orchestrator] batch $((batch_num - 1)) complete — relaunching monitor with fresh context (batch ${batch_num})"
  # continue immediately, no sleep
```

Pass the following prompt verbatim to each background subagent:

> You are the auto-implement monitor for `{repo}`. Process `auto-implement` issues one at a time. Exit after processing `AUTOSPEC_BATCH_SIZE` issues (default: 3) by writing `~/.autospec/batch-done.json` — the orchestrator will relaunch you with fresh context.

> **Harness adaptation (loop persistence).** The `while true:` below is pseudocode. In Claude Code, use `/loop` or `ScheduleWakeup` to persist across turns. In Codex CLI and OpenCode, you lack a built-in loop primitive — implement persistence via one of these patterns:
> 1. **Shell wrapper (preferred):** `exec bash << 'EOF'
> while true; do
>   # ... monitor logic ...
> done
> EOF` — keeps a single bash process alive with your agent dispatching subcommands inside it.
> 2. **nohup background process:** `nohup bash -c 'while true; do ...; sleep 1; done' > ~/.autospec/monitor.log 2>&1 &`
> 3. **tmux pane:** `tmux new-window 'bash << '''HEREDOC'''
> while true; do ...; done
> HEREDOC'`
> **Session batching:** Exit after processing `AUTOSPEC_BATCH_SIZE` issues (default 3) by writing `~/.autospec/batch-done.json` with `status=BATCH_COMPLETE`. The orchestrator relaunches you with fresh context. When the queue is fully drained, write `status=ALL_DONE` instead. This keeps each monitor session short to prevent context overflow.

>
> **Shared helper scripts.** Helper scripts live at `${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}` after installation. Do not assume the target repository has an autospec `scripts/` directory.
>
> Outer loop:
> ```
> # Before the loop (run-once init):
> #   batch_issue_count=0
> #   BATCH_SIZE="${AUTOSPEC_BATCH_SIZE:-3}"
> #   [ "$BATCH_SIZE" -gt 0 ] 2>/dev/null || BATCH_SIZE=3   # guard against 0 or negative
> #   rm -f "$HOME/.autospec/batch-done.json"   # clear stale file from prior crash
>
> while true:
>   # Startup/per-scan heartbeat reconciliation — run before candidate selection.
>   # This deletes closed/merged/orphaned heartbeats, rejects old schemas like
>   # {"issue":407,"status":"in_progress"}, normalizes current schemas, and
>   # releases any `claimed` heartbeat older than AUTOSPEC_WATCHDOG_CLAIMED_TIMEOUT_SECS (default: 300).
>   if [ -d "$HOME/.autospec/process-heartbeats" ]; then
>     if command -v bash >/dev/null 2>&1; then
>       bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-watchdog.sh"
>     elif command -v pwsh >/dev/null 2>&1; then
>       pwsh -NoProfile -ExecutionPolicy Bypass -File "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-watchdog.ps1"
>     elif command -v powershell >/dev/null 2>&1; then
>       powershell -NoProfile -ExecutionPolicy Bypass -File "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-watchdog.ps1"
>     else
>       echo "[watchdog] neither bash nor powershell found; skipping heartbeat reconciliation."
>     fi
>   fi
>   all_open = [open auto-implement issues, sorted ascending by issue number]
>   ready = [all_open issues whose Depends-on deps are all CLOSED, sorted ascending]
>   blocked = [all_open issues with unmet Depends-on deps]
>   claimed_issue, claimed_step = [newest valid heartbeat issue/step, or "-" / "-"]
>   print "[monitor] queue scan: open=N ready=N blocked=N deferred=0 claimed=#X step=Y order=ascending(oldest-first)"
>   # GitHub may display newer/high-numbered issues first; autospec intentionally processes ready issues ascending.
>   if ready is empty:
>     latest_close = most recent closedAt of any auto-implement issue
>     open_count   = count of open auto-implement issues
>     if open_count == 0 AND latest_close > 2h ago:
>       printf '{"batch":%s,"processed":%s,"repo":"%s","ts":%s,"status":"ALL_DONE"}\n' \
>         "${batch_num:-1}" "$batch_issue_count" "{repo}" "$(date -u +%s)" \
>         > "$HOME/.autospec/batch-done.json"
>       echo "[monitor] all issues processed — writing ALL_DONE and exiting"
>       HARD SHUTDOWN — return final report
>     else: print state ("blocked: N unmet deps" / "drained, waiting 2h idle"), sleep 300, continue
>   # autospec-stop sentinel check — outer loop, top of each iteration
>   if [ -f "$HOME/.autospec/stop.flag" ]; then
>     MODE=$(head -1 "$HOME/.autospec/stop.flag" 2>/dev/null || echo "")
>     TIMESTAMP=$(sed -n '2p' "$HOME/.autospec/stop.flag" 2>/dev/null | awk '{print $1}')
>     AGE_SECS=$(( $(date -u +%s) - $(date -u -j -f '%Y-%m-%dT%H:%M:%SZ' "$TIMESTAMP" +%s 2>/dev/null \
>       || date -u -d "$TIMESTAMP" +%s 2>/dev/null || echo 0) ))
>     if [ "$AGE_SECS" -gt 86400 ]; then
>       echo "WARN: stale stop.flag ($AGE_SECS s old); ignoring" >&2
>     elif [ "$MODE" = "graceful" ] || [ "$MODE" = "immediate" ]; then
>       printf '{"batch":%s,"processed":%s,"repo":"%s","ts":%s,"status":"ALL_DONE"}\n' \
>         "${batch_num:-1}" "$batch_issue_count" "{repo}" "$(date -u +%s)" \
>         > "$HOME/.autospec/batch-done.json"
>       echo "[monitor] stop signal received: $MODE — exiting"
>       # HARD SHUTDOWN with final report
>       exit 0
>     fi
>   fi
>   # Service watch: heartbeat reconciliation already runs before each candidate scan; every 12 iterations also runs a cheap nudge/reclaim pass for long-lived workers.
>   monitor_tick=$((monitor_tick + 1))
>   if [ "$monitor_tick" -ge 12 ]; then
>     monitor_tick=0
>     if [ -d "$HOME/.autospec/process-heartbeats" ]; then
>       # Default reclaim window: 10800s (3h). For local single-threaded workers set
>       # AUTOSPEC_WATCHDOG_RECLAIM_SECS=43200 (12h) before launch.
>       export AUTOSPEC_WATCHDOG_RECLAIM_SECS="${AUTOSPEC_WATCHDOG_RECLAIM_SECS:-10800}"
>       export AUTOSPEC_WATCHDOG_STALE_SECS="${AUTOSPEC_WATCHDOG_STALE_SECS:-1800}"
>       # Cheap service wake-up pass: use low-cost model only.
>       if command -v bash >/dev/null 2>&1; then
>         # Dispatch one background watchdog helper (cheap model) to iterate stale entries.
>         bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-watchdog.sh"
>       elif command -v pwsh >/dev/null 2>&1; then
>         # Windows fallback: PowerShell helper.
>         pwsh -NoProfile -ExecutionPolicy Bypass -File "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-watchdog.ps1"
>       elif command -v powershell >/dev/null 2>&1; then
>         # Windows fallback: classic PowerShell fallback.
>         powershell -NoProfile -ExecutionPolicy Bypass -File "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-watchdog.ps1"
>       else
>         echo "[watchdog] neither bash nor powershell found; skipping service-watch pass."
>       fi
>     fi
>   fi
>   ISSUE = ready[0]
>   # Claim check: verify the issue is still labeled auto-implement before processing.
>   # Multiple monitors can query the same candidate list simultaneously;
>   # the first to claim wins, others must skip to the next candidate.
>   CURRENT_LABELS=$(gh issue view ISSUE --json labels --jq -r '.labels[].name')
>   if ! echo "$CURRENT_LABELS" | grep -q "^auto-implement$"; then
>     echo "[monitor] ISSUE $ISSUE already claimed (no auto-implement label); skipping"
>     READY_REMOVE=$(printf "%s\n%s" "$READY_REMOVE" "$ISSUE" | grep -v "^${ISSUE}$" || true)
>     ready=($READY_REMOVE)
>     continue
>   fi
>   gh label create in-progress-by-bot --color ededed --force
>   if ! gh issue edit ISSUE --remove-label auto-implement --add-label in-progress-by-bot; then
>     echo "[monitor] ISSUE $ISSUE claim failed (another monitor claimed it); skipping"
>     continue
>   fi
>   mkdir -p "$HOME/.autospec/process-heartbeats"
>   printf '{"issue":"%s","branch":"","step":"claimed","ts":%s,"pr":"","repo":"%s"}\n' "$ISSUE" "$(date -u +%s)" "{repo}" > "$HOME/.autospec/process-heartbeats/$ISSUE.json"
>   # Issue start summary — print before dispatching process(ISSUE) so the operator
>   # knows exactly what the monitor is about to work on.
>   ISSUE_TITLE=$(gh issue view ISSUE --json title --jq .title 2>/dev/null || echo "")
>   ISSUE_URL=$(gh issue view ISSUE --json url --jq .url 2>/dev/null || echo "")
>   ISSUE_LABELS=$(gh issue view ISSUE --json labels --jq -r '[.labels[].name] | join(", ")' 2>/dev/null || echo "")
>   ISSUE_BODY=$(gh issue view ISSUE --json body --jq .body 2>/dev/null || echo "")
>   ISSUE_GOAL=$(printf '%s\n' "$ISSUE_BODY" | awk '
>     BEGIN{in_goal=0}
>     /^## Goal[[:space:]]*$/ {in_goal=1; next}
>     /^## / && in_goal {exit}
>     in_goal && NF {print; exit}
>   ')
>   [ -n "$ISSUE_GOAL" ] || ISSUE_GOAL=$(printf '%s\n' "$ISSUE_BODY" | awk 'NF && $0 !~ /^#/ {print; exit}')
>   ISSUE_SMOKE=$(printf '%s\n' "$ISSUE_BODY" | awk '
>     /### Primary smoke test/ {seen=1; next}
>     seen && /^```/ {fence++; next}
>     seen && fence==1 && NF && $0 !~ /^[[:space:]]*#/ {print; exit}
>   ')
>   ISSUE_SCOPE=$(printf '%s\n' "$ISSUE_BODY" | awk '
>     /^## Implementation outline[[:space:]]*$/ {in_scope=1; next}
>     /^## / && in_scope {exit}
>     in_scope && /^- / {gsub(/^- /,""); print; count++; if (count>=3) exit}
>   ' | paste -sd '; ' -)
>   echo "[monitor] starting #$ISSUE: ${ISSUE_TITLE:-<untitled>}"
>   echo "[monitor] url: ${ISSUE_URL:-<unknown>}"
>   echo "[monitor] labels: ${ISSUE_LABELS:-<none>}"
>   echo "[monitor] goal: ${ISSUE_GOAL:-<not provided>}"
>   echo "[monitor] smoke: ${ISSUE_SMOKE:-<not provided>}"
>   echo "[monitor] scope: ${ISSUE_SCOPE:-<not provided>}"
>   process(ISSUE)   # foreground subagent — see template below
>   batch_issue_count=$((batch_issue_count + 1))
>   if [ "$batch_issue_count" -ge "$BATCH_SIZE" ]; then
>     printf '{"batch":%s,"processed":%s,"repo":"%s","ts":%s,"status":"BATCH_COMPLETE"}\n' \
>       "${batch_num:-1}" "$batch_issue_count" "{repo}" "$(date -u +%s)" \
>       > "$HOME/.autospec/batch-done.json"
>     echo "[monitor] batch ${batch_num:-1}: processed $batch_issue_count/$BATCH_SIZE issues — writing batch-done.json and exiting for fresh context"
>     exit 0
>   fi
>   # Immediate next-issue pickup: NO SLEEP after process(ISSUE). Re-enter the top
>   # of this loop immediately so the fresh queue scan can pick any issue unblocked
>   # by the merge or failure cleanup that just completed.
> ```
>
> `process(ISSUE)` dispatches a **foreground subagent** (wait for return) with this prompt:
>
> ```
> **Model tier:** `TIER_B` (implementation work) — cheaper model with medium thinking; resolved at startup. Silently fall back to `TIER_A` if unavailable.
>
> **Hard limits.** Max 40 tool calls per issue. Max 3 self-review iterations. If you rewrite the same file twice with no test progress, abort: comment the blocker on the issue, release the lock label, exit. No wall-clock cap.
>
> Implement GitHub issue #<ISSUE>: "<TITLE>" on {repo}. Spec is the issue body below.
>
> ===ISSUE BODY===
> <BODY>
> ===END===
>
> ## Team personality as execution lens
>
> Read the issue body's **Team personality** section before choosing an
> approach. Let that team shape what you emphasize: a reliability/backend team
> should scrutinize operational safety and data boundaries, a frontend/product
> team should scrutinize user workflow and accessibility, a security-sensitive
> team should scrutinize trust boundaries and abuse cases, and so on. Do not
> invent extra scope; use the team personality to decide which risks deserve
> extra attention while still satisfying the issue's concrete acceptance
> criteria.
>
> Keep a progress heartbeat so the monitor can prove forward movement:
> - Create/update `~/.autospec/process-heartbeats/<ISSUE>.json` at each major step:
>   - `claimed`, `worktree_ready`, `tests_started`, `tests_passed`, `pr_created`, `smoke_retry`, `reviewed`, `merged`, `failed`
> - Schema: `{"issue":"<ISSUE>","branch":"<BRANCH>","step":"<STEP>","ts":<unix_epoch>,"pr":"<PR>","repo":"{repo}"}`
> - Delete this file on terminal SUCCESS/FAILURE in both clean and failure paths.
> 
> 1. Worktree off origin/main:
>    cd {repo_root} && git fetch origin
>    git worktree add -b <BRANCH> /tmp/wt-<BRANCH> origin/main && cd /tmp/wt-<BRANCH>
> 2. TDD per AGENTS.md: failing test first → implement → refactor → commit. NO DB/external mocks. Follow file paths and signatures from the issue body verbatim.
> 3. Build + test green (use the project's test runner; for Go: `go build ./... && go test ./... -count=1`; for Node: `npm test`; for Python: `pytest`). 80%+ coverage on changed files.
> 4. Conventional commits (feat:/fix:/test:/docs:/refactor:). NEVER bypass hooks. NEVER amend.
> 5. Push: git push -u origin <BRANCH>
>    ```bash
>    # autospec-stop sentinel check — inside process(ISSUE), after each major step
>    if [ -f "$HOME/.autospec/stop.flag" ] && [ "$(head -1 $HOME/.autospec/stop.flag)" = "immediate" ]; then
>      bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-stop.sh" --abort-current-issue "$ISSUE" "$BRANCH" "$LAST_STEP"
>      exit 0
>    fi
>    ```
> 6. PR: gh pr create --base main --head <BRANCH> --title "<TITLE>" --body "Closes #<ISSUE>\n\n<summary>". Capture PR.
> 7. Inner loop (max 3 iterations):
>    ```bash
>    # autospec-stop sentinel check — inside process(ISSUE), after each major step
>    if [ -f "$HOME/.autospec/stop.flag" ] && [ "$(head -1 $HOME/.autospec/stop.flag)" = "immediate" ]; then
>      bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-stop.sh" --abort-current-issue "$ISSUE" "$BRANCH" "$LAST_STEP"
>      exit 0
>    fi
>    ```
>    - Run the **Primary smoke test** from the issue body. If it fails, fix and recommit before review.
>    - **Fused guardian + LGTM review** (one subagent does both — saves one dispatch per inner-loop iteration):
>      <!-- guardian-block:begin -->
>      Run deterministic lint first (no subagent cost):
>        rm -f /tmp/guardian-<PR>.md
>        if [ "${AUTOSPEC_NO_GUARDIAN:-0}" != "1" ]; then
>          bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/lint-implementation.sh" <PR> --issue <ISSUE> >> /tmp/guardian-<PR>.md 2>&1
>        fi
>        det_exit=$?
>
>      **Model tier:** `TIER_B` for normal issues; `TIER_A` for `regression`/`priority:high` issues. Silently fall back to `TIER_A` if `TIER_B` unavailable.
>
>      **Assemble reviewer prompt** — call `gen-reviewer-prompt.sh` to compose the combined prompt (static cached prefix + dynamic suffix):
>      ```bash
>      _pr_diff_file=$(mktemp -t autospec-pr-diff-XXXXXX.diff)
>      _body_file=$(mktemp -t autospec-issue-body-XXXXXX.md)
>      trap 'rm -f "$_pr_diff_file" "$_body_file"' EXIT
>      gh pr diff <PR> > "$_pr_diff_file"
>      gh issue view <ISSUE> --json body --jq '.body' > "$_body_file"
>      combined_reviewer_prompt=$(bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/gen-reviewer-prompt.sh" \
>        --pr-diff "$_pr_diff_file" \
>        --issue-body "$_body_file" \
>        --prev-findings "/tmp/guardian-<PR>.md" \
>        --issue-labels "<ISSUE_LABELS>" \
>        --repo "<REPO>")
>      ```
>      Pass `combined_reviewer_prompt` as the reviewer subagent prompt. The static cached prefix is framed by `<!-- CACHE BOUNDARY -->` markers; pass it with `cache_control: { type: "ephemeral" }` so Anthropic's prompt cache can reuse it across inner-loop iterations.
>
>      Dispatch ONE **foreground subagent** with this brief:
>        > You are the implementation reviewer for PR #<PR> on {repo}, closing issue #<ISSUE>.
>        >
>        > ## Review counter-team as review lens
>        >
>        > Read the issue body's **Review counter-team** section before reviewing.
>        > Review from that independent team's perspective and challenge likely blind spots
>        > from the implementation team's **Team personality**. Stay inside the issue
>        > scope: do not request unrelated rewrites, but do raise findings when the PR
>        > misses risks the counter-team was selected to notice.
>        >
>        > **Part 1 — Guardian (contract compliance)** — skip if `AUTOSPEC_NO_GUARDIAN=1`:
>        > 1. Read AGENTS.md `## Implementation-quality contract` for the RULE_ID table and directive map.
>        > 2. Read issue #<ISSUE> body — note `## Implementation scope`, `## Implementation outline`, `## Tests required`, and any `Guardian: skip-*` lines.
>        > 3. Read deterministic findings in /tmp/guardian-<PR>.md (populated by lint-implementation.sh; may be empty if guardian disabled).
>        > 4. Run `gh pr diff <PR>` and `gh pr view <PR> --json files,title,body`.
>        > 5. Apply LLM-tier RULE_IDs (HALLUCINATED_API, DUPLICATE_CODE, DOC_OUT_OF_SYNC semantic pass, INVENTED_CONFIG). Collect as `RULE_ID:<path>:<line>: <desc>`. Honor `Guardian: skip-*` with `INFO:` lines.
>        >
>        > **Part 2 — LGTM (correctness review):** Using the same diff and issue body already in context:
>        > 6. Check correctness, edge cases, missing tests, AGENTS.md compliance (TDD, no mocks, conventional commits).
>        > 7. Collect findings as a numbered list.
>        >
>        > **Hard limit:** max **25 tool calls total** (Parts 1 + 2 combined). If budget exhausted, append `RULE_ID:OUT_OF_SCOPE: reviewer budget exhausted; PR needs human review` and proceed to verdict.
>        >
>        > **Verdict:** If Part 1 has ZERO blocking findings (INFO lines OK) AND Part 2 has no findings: return ONLY the token: `LGTM`. Otherwise return a numbered findings list — RULE_ID findings first, then LGTM findings.
>
>      If `LGTM` && det_exit == 0:
>        gh pr comment <PR> --body "<!-- guardian-block --> Review: clean. <!-- /-->"
>        run **Operator/full verification**
>        bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/ci-wait.sh" <PR>  # fire-and-forget sentinel
>        if [ -f ".autospec/tokens-<ISSUE>-reviewer.json" ]; then
>          bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/record-telemetry.sh" \
>            --dispatch-id "<DISPATCH_ID>-reviewer" --role reviewer --issue "<ISSUE>" \
>            --tokens-json ".autospec/tokens-<ISSUE>-reviewer.json"
>        fi
>        # monitor exits to parking state HERE — orchestrator relaunches when ~/.autospec/ci-state/<PR>.signal settles
>        # On relaunch: run ci-wait-poll.sh <PR>; break SUCCESS if exit 0 (pass)
>        break SUCCESS if required checks pass.
>      If `LGTM` but det_exit != 0:
>        Treat deterministic findings as blocking. Comment, fix, recommit, push. Continue inner loop.
>      If findings list:
>        gh pr comment <PR> --edit-last --body "<!-- guardian-block:begin -->\n## Review findings (iter <K>/3)\n<findings>\n<!-- guardian-block:end -->"
>        Append findings to implementer retry context. Continue inner loop (counts toward 3-iter cap).
>      On 3-iter exhaustion with non-LGTM:
>        gh label create guardian-blocked --color e11d21 --force --repo {repo}
>        gh issue edit <ISSUE> --add-label guardian-blocked
>        Run failure cleanup (comment, swap label, close PR).
>        rm -f /tmp/guardian-<PR>.md
>      <!-- guardian-block:end -->
>    - **Regression meta-review** (only for `regression`/`priority:high` issues, after LGTM passes): dispatch a second `TIER_A` subagent: "Would the fused reviewer have caught the original gap? If yes, add missing checklist items to `reports/autospec-review/reviewer-lessons.md` (entry per item, parent gap_id, date) and re-review. Both passes must approve before merge."
>    - If LGTM (and meta-review passes if applicable): break SUCCESS.
>    ```bash
>    # autospec-stop sentinel check — inside process(ISSUE), after each major step
>    if [ -f "$HOME/.autospec/stop.flag" ] && [ "$(head -1 $HOME/.autospec/stop.flag)" = "immediate" ]; then
>      bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-stop.sh" --abort-current-issue "$ISSUE" "$BRANCH" "$LAST_STEP"
>      exit 0
>    fi
>    ```
> 8. SUCCESS: gh pr merge <PR> --admin --squash --delete-branch. Merge auto-closes the issue.
>    ```bash
>    # autospec-stop sentinel check — inside process(ISSUE), after each major step
>    if [ -f "$HOME/.autospec/stop.flag" ] && [ "$(head -1 $HOME/.autospec/stop.flag)" = "immediate" ]; then
>      bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-stop.sh" --abort-current-issue "$ISSUE" "$BRANCH" "$LAST_STEP"
>      exit 0
>    fi
>    ```
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


## Constraints (apply throughout)

- **Cadence**: sub-hour polling needs an in-session background subagent or a local cron. Cloud cron services (e.g. Anthropic remote routines) typically have a 1-hour minimum and are not appropriate.
- **TDD non-negotiable** per AGENTS.md. Real services in tests, no DB mocks.
- **Branch-per-issue**, conventional commits, no force-push, no hook bypass.
- **Auto-merge** on success per AGENTS.md. Do not ask.
- **Context budget**: stay under 60%. Delegate everything possible.
- **Failure isolation**: a failed issue restores its `auto-implement` label so the next monitor cycle picks it up; it does not block the cascade unless dependent issues are downstream.
- **Fresh repo**: if Phase 0 created the repo, the very first commit is the scaffold (incl. `AGENTS.md`); the spec lands as the second commit; child branches branch off that `main`.
- **Small-LLM target**: child issues are sized and pre-staged for 32B-class local LLMs (e.g. qwen3-32B / Qwen3-30B-A3B on Ollama, 48 GB-class Macs). Bias toward smaller issues, sectional spec anchors, pre-staged file pointers, checkbox AC, and one Primary smoke test.
