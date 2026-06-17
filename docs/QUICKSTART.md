# Autospec Quickstart

Get from zero to autonomous PRs in a few minutes.

Autospec is a suite of agent **skills**, not a standalone binary. You install it
once into your agent harness (Claude Code, OpenCode, or Codex CLI) and then drive
it with `/autospec-*` slash commands inside that harness.

---

## Step 1 — Install

One line installs the full suite into every supported harness:

```bash
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh | bash
```

Windows PowerShell:

```powershell
irm https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.ps1 | iex
```

This clones autospec into `~/.autospec/repo` and installs the skills. Re-run any
time to update. See [`README.md`](../README.md#install) for flags, environment
variables, and single-harness installs.

---

## Step 2 — Configure model profiles (optional)

`/autospec-run --profile <name>` filters the issue queue against
`~/.autospec/model-profiles.yml`, so a smaller local model takes only the issues
that fit its limits. Seed the defaults:

```bash
mkdir -p ~/.autospec
cp examples/model-profiles.yml ~/.autospec/model-profiles.yml
```

No changes are required to continue.

---

## Step 3 — Define a feature

Inside your harness, point autospec at a repo and describe what you want:

```text
/autospec-define "build a TODO list CLI in Node"
```

Autospec investigates the repo, writes a design spec under `docs/specs/`,
decomposes it into linked GitHub issues, and classifies each with `ctx:*` /
`reasoning:*` model-fit labels. Review and edit the issues before the next step.

---

## Step 4 — Run the implementation loop

```text
/autospec-run
```

Autospec claims one `auto-implement` issue at a time, opens a branch and PR,
runs self-review plus the test and lint gates, and admin squash-merges each PR
once the suite and required checks pass.

---

## Step 5 — Inspect what shipped

```text
/autospec-story
```

This produces a cited Markdown report of what the repo is, what has been built,
and what remains — reconciling local specs, issues, PRs, and git history.

To do everything in one shot instead of step by step:

```text
/autospec "build a TODO list CLI in Node"
```

---

## Next steps

- Skill catalog: [`README.md`](../README.md#skills) and [`SKILLS.md`](../SKILLS.md)
- Full command reference: [`docs/USER_MANUAL.md`](USER_MANUAL.md)
- Architecture deep-dive: [`docs/ARCHITECTURE.md`](ARCHITECTURE.md)
- Troubleshooting & runbooks: [`docs/runbooks/`](runbooks/)
- GitHub: <https://github.com/berlinguyinca/autospec>
