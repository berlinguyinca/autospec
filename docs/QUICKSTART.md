# Autospec Quickstart

Get from zero to autonomous PRs in under 5 minutes.

[![asciicast](https://asciinema.org/a/autospec-quickstart.svg)](docs/quickstart.cast)

---

## Step 1 — Install

**Homebrew (macOS / Linux):**

```bash
brew install berlinguyinca/autospec/autospec
```

**npm / npx (any platform):**

```bash
npx autospec init
```

Both paths install the `autospec` CLI and run the interactive setup wizard,
which writes `~/.autospec/model-profiles.yml` with sensible defaults.

---

## Step 2 — Configure (optional)

If you want to customise model tiers, edit the profile file:

```bash
$EDITOR ~/.autospec/model-profiles.yml
```

The defaults use `claude-sonnet` for implementation and `claude-opus` for review.
No changes are required to continue.

---

## Step 3 — Define a feature

Point autospec at an empty repo and describe what you want:

```bash
cd /path/to/your-repo
autospec define "build me a TODO list CLI in Node"
```

Autospec calls the LLM to produce a spec, decomposes it into GitHub issues,
and labels them `auto-implement`. You can review and edit the issues before
the next step.

---

## Step 4 — Run the implementation pipeline

```bash
autospec run
```

Autospec picks up the `auto-implement` queue, implements each issue in an
isolated git worktree, self-reviews, and opens a PR. Watch the output:

```
[autospec] issue #1: feat(cli): add create command — implementing...
[autospec] issue #1: PR #2 opened — awaiting review gate
[autospec] issue #1: LGTM — merging
[autospec] queue drained — 1 issue processed
```

---

## Step 5 — Done

Open your repo on GitHub. The PR is landed. Admire the result.

```bash
autospec status   # shows cache-hit rate, issues processed, tokens spent
```

---

## Next steps

- Full command reference: [`docs/USER_MANUAL.md`](USER_MANUAL.md)
- Architecture deep-dive: [`docs/architecture.md`](architecture.md)
- Troubleshooting & runbooks: [`docs/runbooks/`](runbooks/)
- GitHub: <https://github.com/berlinguyinca/autospec>
