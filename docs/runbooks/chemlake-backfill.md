# Runbook — Chemlake one-shot backfill

One-shot backfill of the `metabolomics-us/chemlake` repo: remove the deprecated
`agent:local-safe` label, retro-apply `ctx:*` / `reasoning:*` labels via
`/autospec-classify`, and assign GitHub Project boards from
`~/.autospec/project-map.yml`.

> **Authoritative source.** This runbook transcribes the
> "Chemlake backfill plan (one-shot, after autospec-classify ships)" and
> "Deprecated label: `agent:local-safe`" sections of
> `chem-evidence:docs/superpowers/specs/2026-04-30-autospec-model-fit-and-suite-design.md`.
> If the spec drifts, the spec wins.

## Expected scope

~107 issues — 71 originally optimized + 36 split children (`#509`–`#545`
minus `#518`). `type:tracker`-labeled issues are excluded automatically by
`autospec-classify`.

## Pre-flight

1. **Install the autospec suite (or just `autospec-classify`).**

   ```bash
   git clone https://github.com/berlinguyinca/autospec.git
   cd autospec
   ./install.sh --skill autospec-classify --harness claude   # or your harness
   ```

2. **Authenticate `gh`.**

   ```bash
   gh auth status         # must be authenticated against an account with admin
                          # or maintain on metabolomics-us/chemlake
   ```

3. **Pre-populate `~/.autospec/project-map.yml`.**

   On first invocation `/autospec-classify --apply-boards` auto-inits the
   file with every label as a `mappings:` key and `null` project numbers,
   then exits. To skip the round-trip, write the file ahead of time:

   ```bash
   mkdir -p ~/.autospec
   cat > ~/.autospec/project-map.yml <<'YAML'
   multi_match: union          # `union` (assign to every match) or `first`
   mappings:
     ctx:32k: <project_number>
     ctx:64k: <project_number>
     ctx:120k: <project_number>
     reasoning:shallow: <project_number>
     reasoning:medium:  <project_number>
     reasoning:deep:    <project_number>
     # Add any chemlake-specific labels (domain:*, source:*, …) below as
     # needed; null entries are skipped.
   YAML
   ```

   Look up the actual project numbers via:

   ```bash
   gh project list --owner metabolomics-us --format json | jq '.projects[] | {number, title}'
   ```

## Step 1 — Dry-run the label cleanup

```bash
bash scripts/chemlake-backfill.sh --dry-run
```

Expected: lists each issue currently carrying `agent:local-safe`, prints the
`gh issue edit … --remove-label agent:local-safe` calls it would make, and
prints the `gh label delete agent:local-safe` it would run after. **No side
effects.**

## Step 2 — Apply the label cleanup

```bash
bash scripts/chemlake-backfill.sh
```

Expected: removes `agent:local-safe` from every carrying issue, then deletes
the label itself. The script is idempotent — running it twice is a no-op
on the second pass.

## Step 3 — Run `/autospec-classify --apply-boards`

This is a manual invocation from your Claude Code / OpenCode / Codex CLI
harness (it is **not** a command-line script — `/autospec-classify` is a skill
the harness runs).

```text
/autospec-classify --apply-boards
```

Expected first-time output if `~/.autospec/project-map.yml` was not
pre-populated:

```
Wrote ~/.autospec/project-map.yml. Edit project numbers (currently null) and re-run.
```

Edit the file with real project numbers, then invoke the skill again:

```text
/autospec-classify --apply-boards
```

The skill walks every open `auto-implement` issue (excluding `type:tracker`),
applies `ctx:*` / `reasoning:*` labels per the Phase 3.5 rubric, inserts an
idempotent `## Model fit` block into the body, and assigns each issue to the
projects mapped from its labels. Re-running is a no-op on the second pass for
classification (the model-fit block is replaced in place via its
`<!-- autospec-classify:begin -->` / `<!-- autospec-classify:end -->`
markers) and for board assignment (`gh project item-add` is idempotent).

## Step 4 — Spot-check

Pick three diverse issues and verify they carry `ctx:*` + `reasoning:*` labels
and a `## Model fit` block:

```bash
for n in <issue1> <issue2> <issue3>; do
  gh issue view "$n" --repo metabolomics-us/chemlake --json number,labels,body \
    | jq '{number, labels: (.labels | map(.name) | sort), has_model_fit: (.body | contains("## Model fit"))}'
done
```

Then confirm board membership in the GitHub Projects UI for the same three
issues.

## Recovery — undo a misapplied backfill

If labels were assigned wrong:

```bash
# Strip ctx:* and reasoning:* from every issue, then re-run /autospec-classify.
gh issue list --repo metabolomics-us/chemlake --label 'ctx:32k' --state all --limit 500 --json number -q '.[].number' \
  | xargs -I{} gh issue edit {} --remove-label 'ctx:32k' --repo metabolomics-us/chemlake
# Repeat for ctx:64k, ctx:120k, reasoning:shallow, reasoning:medium, reasoning:deep.
```

If board assignments need a redo, remove the project items via
`gh project item-delete` and re-run `/autospec-classify --apply-boards`. The
`## Model fit` body block is removed automatically by deleting the lines
between the `<!-- autospec-classify:begin -->` / `<!-- autospec-classify:end -->`
markers.

## Hard rules

- Never run on a repo other than `metabolomics-us/chemlake` without first
  updating the spec and this runbook.
- Always start with `--dry-run` to confirm the issue count matches the
  expected scope before touching live labels.
- Do not run while another contributor is mid-edit on the chemlake issue
  queue — race conditions on label edits are silent.
