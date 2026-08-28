---
description: Use when the user wants to retro-apply autospec's Phase 3.5 model-fit rubric to already-existing auto-implement issues — walks open issues in the current GitHub repo, classifies each with ctx:* / reasoning:* labels, and inserts a ## Model fit block in the issue body. Defaults to labels-only; --apply-boards opts into ~/.autospec/project-map.yml board assignment.
mode: primary
---

# autospec-classify (harness-neutral)

Standalone retro-labeler. Walk every open `auto-implement` issue in the current
GitHub repo (excluding `type:tracker`-labeled issues), apply the Phase 3.5
model-fit rubric, and write a `## Model fit` block into the issue body. Default
mode is labels-only; the `--apply-boards` flag opts into board assignment from
`~/.autospec/project-map.yml`.

Manage your own context — never exceed 60%. Delegate to subagents whenever your harness supports it.

<!-- autospec-block:startup-self-update SKILL_NAME=autospec-classify -->

## Self-update mode

If the feature-request argument matches the regex `^\s*update\s*$` (case-insensitive, whitespace-padded), this skill enters self-update mode and does not run the normal pipeline:

1. **Detect harness** by checking which install path exists for this skill:
   - Claude Code: `~/.claude/skills/autospec-classify/SKILL.md`
   - OpenCode:    `~/.config/opencode/agent/autospec-classify.md`
   - Codex CLI:   `~/.codex/prompts/autospec-classify.md`
2. **Re-install the full autospec suite from `main`** by piping the canonical installer:
   ```bash
   curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh | bash -s -- --skill all --harness all --update
   ```
   Run this one-liner once; it refreshes all autospec skills across all harnesses.
3. **Show the diff** between the prior installed file(s) and the freshly fetched copy (e.g. `diff <(cat <prior>) <(curl -fsSL ...SKILL.md)` or the equivalent recorded by the installer).
4. **Stop.** Do not enter Phase 0 / Phase 1 / any pipeline phase. Print the upgrade summary and return to the user.

If no install path is detected, print `Self-update: no installed copy of autospec-classify found; run install.sh first.` and exit.

## Invocation

```
/autospec-classify [--issues N,N,N] [--label <label>] [--apply-boards] [--dry-run]
```

- `--issues N,N,N` — restrict to a comma-separated list of issue numbers (default:
  every open issue in the current repo).
- `--label <label>` — restrict to issues carrying a specific label
  (default: `auto-implement`).
- `--apply-boards` — also assign issues to GitHub Projects per
  `~/.autospec/project-map.yml`. If the file is missing on first run, auto-init
  writes a starter file keyed on the repo's labels with `null` project numbers
  and exits with `Wrote ~/.autospec/project-map.yml. Edit project numbers
  (currently null) and re-run.`. Without `--apply-boards` the project-map is
  not consulted.
- `--dry-run` — print proposed labels, model-fit blocks, and board assignments
  without calling `gh issue edit` or `gh project item-add`.

The skill exits non-zero if `gh auth status` fails or the current cwd is not a
GitHub-tracked repo.

## Required capabilities & harness adapter

This workflow assumes a small set of capabilities. Map each one to your harness's actual tool. If a capability is missing, use the listed fallback.

| Capability                  | Claude Code                          | OpenCode                                 | Codex CLI                                | Fallback if missing                                |
|-----------------------------|--------------------------------------|------------------------------------------|------------------------------------------|----------------------------------------------------|
| Read-only codebase research | `Agent` (subagent_type=Explore)      | `task` agent in read-only mode           | `apply_patch` read-only / shell `grep`   | Do the search in-thread with `rg`/`grep`           |
| Foreground delegation       | `Agent` (subagent_type=general-purpose) | nested `task` agent, await output     | spawn nested CLI session                 | Do the work in-thread (more context cost)          |
| Ask the user a question     | `AskUserQuestion`                    | inline prompt                            | inline prompt                            | Ask in the response and wait for the next turn     |
| Subagent model tier         | Tier A: `opus` + `ultrathink`; Tier B: `sonnet` + medium thinking | Tier A: top `task` model + high reasoning; Tier B: smaller-tier `task` + medium reasoning | Tier A: top GPT + `reasoning_effort=high`; Tier B: `gpt-5.1-codex-spark` + `reasoning_effort=medium` | Honor the per-phase tier mapping in AGENTS.md; retry the same subagent UP on unavailability |
<!-- autospec-block:harness-adapter-core -->

**Persistent project notes**: write durable preferences to **`AGENTS.md`** in the repo root — this is the de-facto standard recognized by Claude Code (also reads `CLAUDE.md`), OpenCode, and Codex. If your harness has its own private memory (e.g. Claude Code's `~/.claude/.../memory/`), mirror the same content there. Per AGENTS.md, subagent dispatches use a **two-tier policy**: Tier A (top model + extended thinking) for spec work and Tier B (cheaper model + medium thinking) for implementation work. This skill's per-issue classification is **deterministic-first**: the deterministic rubric runs with no LLM call, and only ambiguous issues escalate to a single Tier-B LLM tie-breaker — never Tier A. The orchestrator keeps the user's invoked model. Fall back UP the tier on quota/capacity or other unavailability by retrying the same subagent with the stronger tier while preserving parent context.

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

## Pre-flight

1. Verify `gh auth status` is authenticated.
2. Capture `{repo}` from `gh repo view --json nameWithOwner -q .nameWithOwner`.
3. Resolve the candidate set — walk **both** `auto-implement` AND
   `needs-classify` issues (per spec §5.3, listener-filed issues land
   labeled `needs-classify` and `/autospec-classify` is the entry point
   that transitions them onto the implementation queue):
   - `gh issue list --repo {repo} --label <label> --state open --limit 500 --json number,title,labels,body,url`
     for the value of `--label` (default `auto-implement`).
   - Additionally, `gh issue list --repo {repo} --label needs-classify --state open --limit 500 --json number,title,labels,body,url`.
   - Merge both result sets, deduplicating by issue number (an issue may
     carry both labels during a partial transition).
   - Filter out any issue whose labels include `type:tracker`.
   - Apply the `--issues` filter if provided.

### Issue intent safety gate

The classifier may persist model-fit and quality metadata, but it must never
author an automatic safety decision. After the final body is persisted, the
Rust queue command is the only authority for an admission pass, ambiguity, or
block.

### Label transition for `needs-classify` issues

For every issue in the candidate set whose labels include `needs-classify`, run
the Rust admission only after Steps 2–6 of the per-issue procedure have
persisted the final body. First make the issue an interim queue candidate, then
review that exact issue:

```bash
gh issue edit <N> --add-label auto-implement --remove-label needs-classify --repo {repo}
"${AUTOSPEC_BIN:-autospec}" queue review-safety --repo {repo} --limit 1 --issue <N>
```

Read the command's JSON totals. Only `pass: 1` admits this invocation. Any
other result is already recorded by Rust and must be skipped without a prompt,
shell, or semantic reviewer changing labels, comments, or issue body. Issues
already carrying `auto-implement` are re-classified in place, then receive the
same exact review after their final body is persisted.

## Rubric

### `ctx:*` — context-window axis

Pick the smallest tier that holds the staged context (issue body + every file
listed under `## Files to read first` + the relevant spec sections).

| Label | Approx. token ceiling | Trigger |
|---|---|---|
| `ctx:32k`  | ~32k tokens of staged context | One canonical table or one shell script; ≤3 files in *Files to read first*; spec anchors are short. |
| `ctx:64k`  | ~64k tokens                   | Multi-file change; 4-7 files staged; one trio + one installer; medium spec sections (~1-3 KB). |
| `ctx:120k` | ~120k tokens                  | Cross-skill or cross-package; 8+ files; long spec excerpts; deep call graphs. |

If unsure between two tiers, prefer the larger tier.

### `reasoning:*` — reasoning-depth axis

Pick the depth required to **derive** the implementation, not just transcribe it.

| Label | Trigger |
|---|---|
| `reasoning:shallow` | Mechanical: copy-and-rename, regex-replace, README transcription, runbook authoring. Verbs in the issue: *copy*, *rename*, *transcribe*, *list*. |
| `reasoning:medium`  | Template-following with judgment calls: synthesize a new SKILL.md by mirroring an existing one, modify a script with new flags, write tests for a documented contract. Verbs: *mirror*, *adapt*, *integrate*, *wire*. |
| `reasoning:deep`    | Novel design choices: pick a new abstraction, resolve a contradiction in the spec, reconcile cross-cutting concerns. Verbs: *design*, *reconcile*, *resolve*, *redesign*. |

Default for issues that lack any of these signals: `ctx:64k`, `reasoning:medium`.

## Per-issue procedure

> **Model tier:** **deterministic-first; `TIER_B` on ambiguity (never `TIER_A`).**
> Classification runs the deterministic
> rubric (`${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/classify-model-fit.sh` — file counts, verb keywords, per
> tracker #421) FIRST, with **no LLM dispatch** for issues it can score
> confidently. Only issues the rubric flags as ambiguous (confidence below
> `LLM_ESCALATION_THRESHOLD`, default 0.3) escalate to a single `TIER_B` LLM
> tie-breaker — never `TIER_A`. Sibling normalization stays deterministic. Set
> `AUTOSPEC_REVIEWER_TIER=opus` only governs the run-trio reviewer, not this
> classifier; the classifier's LLM tie-breaker is always `TIER_B`.

For each candidate issue:

1. **Sanity check.** Body must contain both `## Files to read first` and
   `## Implementation scope`. If either is missing:
   - Add label `needs-autospec-template` (idempotent, `gh label create --force`
     once at the top of the run).
   - Skip — do not modify body, do not assign a model-fit class.

2. **Classify (deterministic-first).** Run the deterministic rubric first
   (`${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/classify-model-fit.sh <body-file>`): it assigns one `ctx:*` and one
   `reasoning:*` label from file counts and verb keywords with **zero LLM cost**.
   Only when the rubric reports `deterministic:false` (confidence below
   `LLM_ESCALATION_THRESHOLD`) escalate that single ambiguous issue to one
   `TIER_B` LLM tie-breaker; otherwise take the deterministic result as final.
   Print the rationale (and whether it was deterministic or escalated) in the
   dry-run preview.

3. **Apply labels.**
   - `gh label create ctx:32k --color c5def5 --force` (and ctx:64k, ctx:120k).
   - `gh label create reasoning:shallow --color c2e0c6 --force`
     (and reasoning:medium, reasoning:deep).
   - `gh label create needs-quality-bar --color fbca04 --force --repo {repo}` (once at run start, idempotent).
   - `gh label create lang:<value> --color d4c5f9 --force --repo {repo}` once at run
     start (idempotent) for each of the 12 values: `lang:rust`, `lang:go`,
     `lang:python`, `lang:typescript`, `lang:javascript`, `lang:java`,
     `lang:bash`, `lang:ruby`, `lang:csharp`, `lang:markdown`, `lang:mixed`,
     `lang:unknown`.
   - `gh issue edit <N> --add-label "ctx:<tier>,reasoning:<depth>" --repo {repo}`.
     The `lang:<value>` label is applied by the Step 4 language-axis fragment,
     not here.
   - Skip in `--dry-run`.

4. **Patch body.** Insert a `## Model fit` block immediately before the first
   `## Dependencies` line (or, if absent, at end of body). Block format:

   ```markdown
   <!-- autospec-classify:begin -->
   ## Model fit

   - **ctx:** `ctx:<tier>` — <1-line rationale>.
   - **reasoning:** `reasoning:<depth>` — <1-line rationale>.

   *Auto-classified by `/autospec-classify` on YYYY-MM-DD.*
   <!-- autospec-classify:end -->
   ```

   **Idempotency:** if a `## Model fit` block already exists (delimited by the
   `<!-- autospec-classify:begin -->` / `<!-- autospec-classify:end -->`
   markers), replace it in place. Never stack duplicate blocks.

   Legacy blocks have `## Model fit` ABOVE
   `<!-- autospec-classify:begin -->`. Replacing only the marker-delimited
   region orphans that heading outside the markers, where it still counts
   against the budget, and adds a second one. Delete the legacy heading and
   everything up to the begin marker first.

    Apply via `gh issue edit <N> --body-file <tmp>`.

    **Language axis (deterministic).** After the model-fit write, run
    `apply_lang_step <body-file> <labels-csv>` (function below) on the
    issue's current body file, with the issue's current label list as the
    CSV. It invokes the deterministic classifier
    (`${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/classify-language.sh`)
    twice — once for the canonical `## Language fit` block, once with
    `--json` for the label value — splices the block into
    `<body-file>.lang` (idempotent: it replaces the existing
    `<!-- autospec-language:begin -->` … `<!-- autospec-language:end -->`
    region in place — including any legacy `## Language fit` heading sitting
    directly above the markers — or inserts a fresh block before the first
    `## Dependencies` line, or at end of body if that section is absent),
    and prints exactly two lines on stdout: the bare language value, then a
    label fragment — `--add-label lang:<value> --remove-label lang:<old>`
    if the issue already carries a different `lang:*` label, `--add-label
    lang:<value>` if it carries none, or `LABEL_NOOP` if it already carries
    the same one. Apply the fragment with `gh issue edit <N> <fragment>
    --repo {repo}` and the patched body with `gh issue edit <N> --body-file
    <body-file>.lang`. `lang:unknown` is a real classification outcome and
    is applied as-is. Skip both calls in `--dry-run` (print the block and
    fragment to the preview instead).

    ```bash
    # autospec-classify lang-step begin
    apply_lang_step() {
      # apply_lang_step <body-file> <labels-csv>
      # Splices the canonical `## Language fit` block (from
      # scripts/classify-language.sh) into <body-file>.lang and prints
      # exactly two lines on stdout: the bare language value, then the
      # label fragment (--add-label lang:<v> [--remove-label lang:<old>]
      # | LABEL_NOOP).
      local body_file="$1" labels_csv="${2:-}"
      local classifier="${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/classify-language.sh"
      local md_block lang old l
      if ! md_block="$(bash "$classifier" "$body_file")"; then
        echo "apply_lang_step: classifier failed for $body_file" >&2
        return 2
      fi
      lang="$(bash "$classifier" "$body_file" --json | sed -n 's/.*"lang":"\([^"]*\)".*/\1/p' | sed -n '1p')"
      if [ -z "$lang" ]; then
        echo "apply_lang_step: no lang value in classifier --json output" >&2
        return 2
      fi
      old=""
      if [ -n "$labels_csv" ]; then
        local IFS=','
        for l in $labels_csv; do
          case "$l" in
            lang:*) old="$l" ;;
          esac
        done
      fi
      old="${old#lang:}"
      if ! LANG_BLOCK="$md_block" awk '
        BEGIN { n = 0 }
        {
          n++
          line[n] = $0
          b = $0
          sub(/\r$/, "", b)
          if (begin == 0 && b ~ /^[[:space:]]*<!-- autospec-language:begin -->[[:space:]]*$/) begin = n
          if (endm == 0 && b ~ /^[[:space:]]*<!-- autospec-language:end -->[[:space:]]*$/) endm = n
        }
        END {
          if ((begin == 0) != (endm == 0) || (begin != 0 && begin > endm)) {
            print "lang-splice: unpaired autospec-language markers" > "/dev/stderr"
            exit 1
          }
          if (begin != 0) {
            del_begin = begin
            i = begin - 1
            while (i > 1 && line[i] ~ /^[[:space:]]*$/) i--
            if (i >= 1 && line[i] ~ /^[[:space:]]*## Language fit[[:space:]]*$/) del_begin = i
            del_end = endm
            if (del_begin > 1 && line[del_begin - 1] ~ /^[[:space:]]*$/) del_begin--
            if (del_end < n && line[del_end + 1] ~ /^[[:space:]]*$/) del_end++
            k = 0
            for (i = 1; i <= n; i++) {
              if (i >= del_begin && i <= del_end) continue
              k++
              kept[k] = line[i]
            }
          } else {
            k = n
            for (i = 1; i <= n; i++) kept[i] = line[i]
          }
          ins = 0
          for (i = 1; i <= k; i++) {
            t = kept[i]
            sub(/\r$/, "", t)
            if (t ~ /^[[:space:]]*## Dependencies[[:space:]]*$/) { ins = i; break }
          }
          m = split(ENVIRON["LANG_BLOCK"], bl, "\n")
          if (ins > 0) {
            j = ins - 1
            while (j > 0 && kept[j] ~ /^[[:space:]]*$/) j--
            for (i = 1; i <= j; i++) print kept[i]
            if (j > 0) print ""
            for (i = 1; i <= m; i++) print bl[i]
            print ""
            for (i = ins; i <= k; i++) print kept[i]
          } else {
            t = k
            while (t > 0 && kept[t] ~ /^[[:space:]]*$/) t--
            for (i = 1; i <= t; i++) print kept[i]
            if (t > 0) print ""
            for (i = 1; i <= m; i++) print bl[i]
          }
        }
      ' "$body_file" > "$body_file.lang"; then
        echo "apply_lang_step: splice failed for $body_file" >&2
        return 2
      fi
      printf '%s\n' "$lang"
      if [ "$old" = "$lang" ]; then
        printf '%s\n' "LABEL_NOOP"
      elif [ -n "$old" ]; then
        printf '%s\n' "--add-label lang:$lang --remove-label lang:$old"
      else
        printf '%s\n' "--add-label lang:$lang"
      fi
    }
    # autospec-classify lang-step end
    ```

5. **Board assignment** (only if `--apply-boards`):
   - Read `~/.autospec/project-map.yml`. If the file is missing, auto-init it
     (see below) and exit so the user can fill in project numbers.
   - **File schema:**
     ```yaml
     # ~/.autospec/project-map.yml
     multi_match: union          # `union` (assign to every match) or `first`
     mappings:
       ctx:32k: <project_number>
       ctx:64k: <project_number>
       ctx:120k: <project_number>
       reasoning:shallow: <project_number>
       reasoning:medium:  <project_number>
       reasoning:deep:    <project_number>
       <any-other-label>: <project_number>
     ```
   - **Reader procedure.** For each label L on the issue, look up
     `mappings[L]`. Skip null / missing entries. With `multi_match: union`
     (default), assign the issue to every matching project number; with
     `multi_match: first`, assign only to the first match in label-order.
     For each chosen `<P>`:
     `gh project item-add <P> --owner <owner> --url <issue-url>`. The
     command is idempotent so re-running this skill does not duplicate
     project items.
   - **Auto-init when the file is missing.** Probe
     `gh project list --owner <owner> --format json` to confirm the user
     can author projects. Probe
     `gh label list --repo {repo} --json name -q '.[].name'` to enumerate
     the repo's labels. Write a starter file with every label as a
     `mappings:` key and `null` project numbers, plus `multi_match: union`
     at the top. Print:
     ```
     Wrote ~/.autospec/project-map.yml. Edit project numbers (currently null) and re-run.
     ```
     Then **exit non-zero** (this is a hard stop unique to
     `--apply-boards`; without the flag the run continues normally).
   - Skip in `--dry-run`.

6. **Quality audit.** After patching the `## Model fit` block:
   - Pull body: `gh issue view <N> --repo {repo} --json body -q .body > /tmp/audit-<N>.md`
   - Run: `bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/lint-issue.sh" /tmp/audit-<N>.md`
   - On non-zero exit (lint fails):
     - Apply label: `gh issue edit <N> --add-label needs-quality-bar --repo {repo}`
     - Insert `## Quality lint` block (idempotent, between `<!-- autospec-quality:begin -->` and `<!-- autospec-quality:end -->` markers) via `gh issue edit <N> --body-file <tmp>`. A legacy block has the heading ABOVE the begin marker: delete the legacy heading and everything up to that marker first, or it keeps counting and a duplicate is added. Block format:
       ```markdown
       <!-- autospec-quality:begin -->
       ## Quality lint

       - **GOAL** — <1-line finding>.
       - **AC#<n>** — <1-line finding>.
       - **SMOKE** — <1-line finding>.

       *Auto-linted by Phase 3.5 on YYYY-MM-DD.*
       <!-- autospec-quality:end -->
       ```
     - Comment findings: `gh issue comment <N> --body "<findings>" --repo {repo}`
   - Do NOT remove `auto-implement` label. Operator decides whether to proceed.
   - Skip in `--dry-run`.

7. **Rust safety admission.** After every model-fit and quality-body write has
   completed, admit the exact issue with the command in
   [Label transition for `needs-classify` issues](#label-transition-for-needs-classify-issues).
   For an issue already carrying `auto-implement`, do not change ordinary
   labels first; invoke the same exact command. Only a JSON `pass: 1` result is
   eligible for later implementation work.

## Sibling normalization (forward reference)

When 5+ sibling issues share a structural criterion (e.g. all are
"per-source-table writers"), harmonize their `ctx:*` and `reasoning:*` labels so
the operator can run a single profile across the whole group. The full
sibling-normalization prompt is part of Phase 3.5 (PR B1, issue #14); when it
lands, this skill calls into it. Until then, classify each sibling
independently.

## Run-end summary

Print:

```
autospec-classify run summary on {repo}
- classified: N
- skipped (needs-autospec-template): M
- ctx:32k=A  ctx:64k=B  ctx:120k=C
- reasoning:shallow=X  reasoning:medium=Y  reasoning:deep=Z
- lang:rust=A  lang:go=B  … one entry per observed `lang:*` value
  (`lang:unknown` is its own entry)
- boards assigned: <K>  (or "skipped — --apply-boards not set" / "skipped — no project-map.yml")
```

## Hard rules

- Never modify the issue title.
- Never remove existing labels, except that the language axis may replace a
  prior `lang:*` label (one `--remove-label lang:<old>` per
  re-classification). Otherwise only add `ctx:*`, `reasoning:*`,
  `needs-autospec-template`, `lang:*`.
- Never call `gh issue edit` in `--dry-run` mode.
- Immediately after every successful non-dry-run issue edit, capture the verified issue URL as
  `ISSUE_URL` and run `autospec project sync --repo-dir "$PWD" --issue-url "$ISSUE_URL"`.
  If sync fails, print `WARNING: managed Project sync failed for $ISSUE_URL`, keep the edited
  issue, and never create a replacement issue.
- Always idempotent — running twice on the same issue results in no diff after
  the second run (same labels, same `## Model fit` block).
- `gh` CLI only; no direct GraphQL.
