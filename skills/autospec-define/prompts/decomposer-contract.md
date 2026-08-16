# Decomposer contract (Phase 3 — spec to linked issues)

Curated extract of the Phase 3 decomposition workflow. The decomposer subagent
turns a design spec into linked GitHub issues sized for 32B-class local LLMs.
The issue-quality contract (Goal / AC / primary-smoke-test shape) and the
small-LLM sizing target ride in the AGENTS.md section injected alongside this file.

## Phase 3 — Decompose into linked GitHub issues (delegate)

If Existing spec mode is active, use `{selected_spec_path}` and its GitHub URL.
Otherwise use the spec path written and merged in Phase 2.

Dispatch a **foreground subagent** with this prompt (substitute the spec path and `{repo}`):

> **Model tier:** `TIER_A` (spec work) — top model with extended thinking; resolved at startup.
>
> Read the selected design spec at `<spec-path>` (`<spec-github-url>`) and split it into linked GitHub issues for {repo}.
>
> **Portfolio validation gate (security/database only):** Load
> `.autospec/spec-artifacts/<slug>.security-database.yml`, update every planned
> issue mapping after decomposition, and run
> `python3 "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/validate-security-artifact.py" <artifact>`.
> Validation must pass before creating labels or calling `gh issue create`.
> Then perform a Tier-A portfolio review: confirm every threat has a control,
> every control has both a negative test and an issue owner, every required spec
> section is covered, dependencies are acyclic, every prerequisite names its
> gated issues, and atomic-group members resolve to one issue's `produces` list.
> Do not weaken controls to make decomposition pass. An issue with an unresolved
> prerequisite receives `autospec:blocked-prerequisite` instead of
> `auto-implement`; it may be filed for visibility but is not queued. The
> ordinary profile skips this gate.
>
> Create labels (idempotent with `--force`): `auto-implement` (#0e8a16), `autospec:blocked-prerequisite` (#d4c5f9), `epic` (#b60205), `autospec:v2-flow` (#0e8a16, description: "Routes to absorbed-discipline Phase 4 implementer"), plus any domain labels the spec calls for. Then create exactly N issues — first an EPIC umbrella (no `auto-implement` label, just `epic` + domain), then N-1 children carrying `auto-implement` and `autospec:v2-flow` unless the portfolio gate marked them blocked. The `autospec:v2-flow` label routes the child to the Phase 4 implementer that absorbs turbo's expand → implement → finalize → peer-review → evaluate discipline; children filed without it fall back to the legacy implementer path. After creating children, edit the umbrella body with a checklist linking them. Return JSON: `{umbrella, children:[…], labels_created:[…]}`. Use `gh` CLI only. Do NOT modify code. Do NOT push branches. Do NOT create PRs.
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
> - **Files touched** — machine-parseable: one repo-relative path per line, ≤3 logical units. The authoritative scope source the linter (`TOO_MANY_FILES`) and reviewers check; keep it in sync with the outline. A skill trio (`SKILL.md` + `codex/prompt.md` + `opencode/agent.md`) plus its derived `tests/fixtures/skill-goldens/*.sha256` counts as ONE unit, so a single-trio edit may list all six paths and still be in-cap.
> - **Implementation outline** — file paths + function signatures + data flow.
> - **Tests required** — TDD per AGENTS.md, real services, no DB mocks, 80%+ coverage.
> - **Acceptance criteria** — checkbox list `[ ]` only, no prose. Each item machine-checkable.
> - **Verification** — split into a **Primary smoke test (inner loop)** with exactly one fast command, and **Operator/full verification** listing the remaining commands.
> - **Branch name** — `feat/<slug>`.
> - **Dependencies** — `Depends on issue #N` lines (parsed by the monitor).
> - For `security_database` only: **Evidence consumed**, **Controls covered**, and **Prerequisites**, copied exactly from the validated artifact. Every prerequisite starts with `verified:` or the child is blocked.
>
> Never hand-author the final body. Populate structured YAML with
> `files_touched`, `local_llm_notes`, `dependencies`, and conditional security
> lists, then render it with
> `bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/gen-issue-skeleton.sh" --input <issue.yml>`.
> Pass the renderer output to the lint and safety checks.
>
> **Sizing caps (hard, per spec §3.4):**
>
> - **Body ≤400 words** including all sections.
> - **Implementation outline ≤30 lines** (file paths + function signatures).
> - **Files touched ≤3** per child issue, counted as distinct **logical units** — not raw paths. A multi-harness skill **trio** (`skills/<x>/SKILL.md` + `codex/prompt.md` + `opencode/agent.md`) plus its derived `tests/fixtures/skill-goldens/<x>.*.sha256` is **ONE** logical unit: with `derive-trio.sh` shipped, the edit is "edit `SKILL.md` → derive the mirrors with `derive-trio.sh --in-place skills/<x>` → regenerate goldens with `gen-skill-goldens.sh <x>`" in a **single commit**, so the cap must not split a trio's prose from its golden regen into separate issues. `lint-issue.sh` and `sizing-check.sh` collapse trio members to one unit and exclude the derived goldens — keep a trio edit (and its goldens) inside one child.
> - If a candidate child would exceed any cap, split into a parent + child pair with a `Depends on` edge.
> - The whole spec + a single child issue body must fit comfortably in a 60–120k context window.
>
> Self-check each issue against the caps **before** calling `gh issue create`. If a cap is violated and a split is not feasible, surface the issue inline (print the over-cap body to the operator) instead of filing it.
>
> **Pre-filing lint loop (adaptive, MAX_LINT_RETRIES=5):** For each candidate child body, before calling `gh issue create`, write the body to `/tmp/draft-<slug>.md` and run `bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/lint-issue.sh" /tmp/draft-<slug>.md`. If the exit code is non-zero, map each `RULE_ID: <desc>` finding to an actionable directive using the table below, append all directives to the next generation prompt as cumulative context, and regenerate. Repeat up to `MAX_LINT_RETRIES=5` attempts. If attempt 5 still fails, print all 5 drafts plus accumulated findings inline and **skip** that child (do not file); continue to the next child. On pass (exit 0), proceed to the safety loop and only call `gh issue create` after safety passes.
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
> | `AC_NOT_CHECKABLE` | `REWRITE: each AC item must include a path, backtick-quoted identifier, integer, or regex literal.` |
> | `SMOKE_MULTI_LINE: N lines` | `COLLAPSE: Primary smoke test must be exactly one command line. Use \`&&\` to chain or move setup to Operator/full verification.` |
> | `SMOKE_PLACEHOLDER: contains "<TODO>"` | `RESOLVE: Replace placeholders \`<TODO>/TBD/XXX/...\` with the actual command before filing.` |
> | `SMOKE_NOT_FENCED` | `ADD: Primary smoke test section must contain exactly one fenced code block.` |
> | `MISSING_SECTION_FILES_TO_READ` | `ADD: a \`## Files to read first\` section with 3-7 anchored entries (path#section or dep #N).` |
> | `MISSING_SECTION_IMPL_OUTLINE` | `ADD: a \`## Implementation outline\` section (file paths + signatures + data flow).` |
> | `MISSING_SECTION_TESTS` | `ADD: a \`## Tests required\` section (TDD per AGENTS.md, real services, no DB mocks).` |
> | `MISSING_SECTION_DEPENDENCIES` | `ADD: a \`## Dependencies\` section containing \`Depends on issue #N\` lines or exactly \`none\`.` |
> | `DEPS_MALFORMED` | `FIX: each \`## Dependencies\` line must be \`Depends on issue #N\` or exactly \`none\`.` |
> | `TOO_MANY_FILES` | `SPLIT: \`## Files touched\` exceeds 3 logical units; split into a parent + child with a \`Depends on\` edge. NOTE: a skill trio (SKILL.md + codex/prompt.md + opencode/agent.md) plus its derived skill-goldens is ONE unit — keep them together, do not split a trio from its golden regen.` |
> | `BODY_TOO_LONG` | `SHORTEN: body exceeds 400 words; cut prose or split the issue into a \`Depends on\` pair.` |
> | `OUTLINE_TOO_LONG` | `SHORTEN: \`## Implementation outline\` exceeds 30 lines; compress signatures or split the issue.` |
> | `UI_SECTIONS_INCOMPLETE` | `ADD: this is a UI feature — include \`## Design reference\`, \`## Interaction states\`, \`## UX flows\`, \`## Motion & feedback\`, and \`## Device & viewport\` (all five).` |

> **Pre-filing safety loop (adaptive, MAX_SAFETY_RETRIES=5):** For each candidate child body, after the issue-quality lint passes and before `gh issue create`, run `"${AUTOSPEC_BIN:-autospec}" lint issue safety --title "<candidate title>" /tmp/draft-<slug>.md`. If the exit code is `1` or `2`, append the safety findings to the next generation prompt as cumulative directives:
>
> | Finding | Directive appended to next prompt |
> |---|---|
> | `SAFETY_BLOCK: production-data-destruction` | `BLOCKED: remove production data destruction from scope; rewrite for test/dev data only or split to human-reviewed production plan.` |
> | `SAFETY_BLOCK: secret-exfiltration` | `BLOCKED: never request printing, dumping, sending, or exposing secrets or tokens.` |
> | `SAFETY_BLOCK: instruction-bypass` | `BLOCKED: never ask the implementer to ignore AGENTS.md, system/developer instructions, CI, review, hooks, or guardian checks.` |
> | `SAFETY_AMBIGUOUS` | `CLARIFY: add explicit non-production scope, affected paths, guardrails, and verification command; otherwise skip filing.` |
>
> Repeat up to `MAX_SAFETY_RETRIES=5`. If attempt 5 still returns non-zero, print all drafts plus safety findings inline and skip that child. Do not file unsafe or ambiguous child issues.

### UI-feature decomposition (only for user-facing UI issues)

When a child issue builds or changes user-facing UI, treat it as more than generic
code: a small-LLM implementer needs the design and the behavior spelled out, and
the visual-fidelity QA loop needs something to judge against. Mark such issues with
a `<!-- ui-feature -->` comment and add these five sections (the linter's
`UI_SECTIONS_INCOMPLETE` rule enforces them as a group once any one is present):

- **Design reference** — the `DESIGN.md` section/tokens (or mockup link) the screen
  must match. Pairs with the implementer's `DESIGN_DRIFT` directive and the QA
  visual-fidelity judge.
- **Interaction states** — the relevant subset of default / hover / focus / loading
  / empty / error / disabled, plus responsive breakpoints if they change layout.
- **UX flows** — the happy path, the failure scenarios, and the edge cases (one
  line each; this is where most UI defects hide).
- **Motion & feedback** — which catalog motion patterns this screen uses, and its
  reduced-motion fallback, one line per item, e.g.
  `Motion: fade-in + 40ms stagger; reduced: opacity-only`. Pairs with the
  implementer's `MOTION_DRIFT` directive.
- **Device & viewport** — which device profiles must pass, plus reflow-at-320 and
  200%-zoom expectations, one line per item, e.g.
  `Devices: iPhone SE, Pixel 7, 1280×800 laptop; reflow-320: no h-scroll; zoom-200%: no clipped text`.

Non-UI issues omit all five (the rule never fires without the marker or a section).
The five `ui-feature` sections are **excluded from the ≤400-word body count** (they
still count toward the ≤3-files cap where relevant) — Phase 3.5/3.75 append
Model-fit and Shared-contracts blocks after the trim, so a body that spent its
whole budget on prose would systematically trip `needs-quality-bar` once these two
mandatory sections were added. The word cap continues to apply to the rest of the
body unchanged. Split a large screen into a parent + per-component children with
`Depends on` edges rather than one giant issue.

### Small-LLM friendliness (applies to every child issue)

Children are written assuming the implementer is a 32B-class local model with **pre-staged context**, not a search-driven cloud agent:

- Every file the implementer needs is named in **Files to read first** with a sectional anchor or a one-line reason. Do not assume the model will grep the codebase.
- Spec docs are cited by section heading, not as "read this 20 KB doc".
- Acceptance criteria are checkbox-only so the model can self-verify line-by-line.
- One **Primary smoke test** runs in the inner loop; the heavier verification list runs once at the end.
- If the work fans out across many tables/packages, split it. Two 3 KB children chained by `Depends on` beat one 7 KB child a 32B model garbles at 60k tokens of working context.

Capture the umbrella + child issue numbers.

Persist the relationship on GitHub and in the shared per-repository parent-state
cache before classification. This command also posts a trusted typed
parent-marker lifecycle comment on every child and the idempotent decomposition
comment on the parent. Append `--quarantined` when the umbrella's authoritative
typed safety decision is `SAFETY_AMBIGUOUS` or `SAFETY_BLOCK`; otherwise omit
it. A failure is blocking because an unlinked child could merge without ever
reconciling its parent.

```bash
_parent_slug=$(bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/repo-slug.sh" --canonical "{repo}")
export AUTOSPEC_PARENT_STATE_ROOT="${AUTOSPEC_PARENT_STATE_ROOT:-$HOME/.autospec/parent-state/$_parent_slug}"
"${AUTOSPEC_BIN:-autospec}" parent record --repo {repo} --parent "<UMBRELLA>" --children "<CHILDREN_CSV>"
```

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
