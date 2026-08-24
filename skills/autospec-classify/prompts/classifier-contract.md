# Classifier contract (Phase 3.5 — model-fit labeling)

Curated extract of `/autospec-classify`: label issues `ctx:*` / `reasoning:*`,
deterministic-first (a `TIER_B` LLM tie-breaker on ambiguity, never `TIER_A`).
Two-tier model-selection rules ride in the AGENTS.md section injected alongside.

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
   - `gh issue edit <N> --add-label "ctx:<tier>,reasoning:<depth>" --repo {repo}`.
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

5. **Board assignment** (only if `--apply-boards`): read
   `~/.autospec/project-map.yml` — a `label -> project-number` map with
   `multi_match: union|first` — and assign the issue to every matching project
   via `gh project item-add <P> --owner <owner> --url <issue-url>` (idempotent).
   If the file is missing, auto-init a starter (every repo label as a
   `mappings:` key with `null` numbers, `multi_match: union`) and **exit
   non-zero** so the user can fill in project numbers. Skip in `--dry-run`.

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
- boards assigned: <K>  (or "skipped — --apply-boards not set" / "skipped — no project-map.yml")
```

## Hard rules

- Never modify the issue title.
- Never remove existing labels — only add `ctx:*`, `reasoning:*`,
  `needs-autospec-template`.
- Never call `gh issue edit` in `--dry-run` mode.
- Always idempotent — running twice on the same issue results in no diff after
  the second run (same labels, same `## Model fit` block).
- `gh` CLI only; no direct GraphQL.
