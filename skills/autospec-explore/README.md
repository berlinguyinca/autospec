# autospec-explore

Perpetual autonomous research + ship loop on an isolated sandbox branch.
`/autospec-explore "<initial prompt>"` creates `autospec/explore/<date>-<slug>` off
`origin/main`, runs 6 parallel researchers each round (spec-vs-code, prior reports,
codebase signals, open issues, source analysis, internet), files 1-5 auto-implement
issues per round, drains them via `/autospec-run` with PRs targeting the sandbox,
and continues until the operator stops it.

The point: any time you want the model to keep proposing + shipping autonomously
without writing to `main`, type `/autospec-explore "<your seed prompt>"`. When the
sandbox looks good, merge it; if it looks bad, discard the branch — `main` is never
touched directly.

Works on **Claude Code**, **OpenCode**, and **Codex CLI**.

## Quick install

```bash
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-explore/install.sh | sh -s -- --harness all
```

Per-harness:

```bash
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-explore/install.sh | sh -s -- --harness claude
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-explore/install.sh | sh -s -- --harness opencode
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-explore/install.sh | sh -s -- --harness codex
```

From a clone:

```bash
cd skills/autospec-explore
./install.sh --harness all
```

## Invocation

```
/autospec-explore "<initial prompt>" \
    [--max-iterations N] \
    [--max-issues-per-round N] \
    [--budget-tokens N] \
    [--budget-hours N] \
    [--sandbox-slug <slug>] \
    [--research-sources <comma-list>] \
    [--no-internet] \
    [--internet-allowlist <comma-list>]
```

## Sandbox branch contract

- At run start, `scripts/explore-sandbox.sh --slug <slug>` creates
  `autospec/explore/<YYYY-MM-DD>-<slug>` off `origin/main`, pushes it, and writes
  `.autospec/explore-mode.json` with `{branch, slug, base, head_sha, created_at}`.
- **Idempotent**: re-invocation with the same slug reuses the existing branch.
- All child-issue PRs target the sandbox branch, not `main` (enforced by the
  Phase 4 implementer integration).
- Operator merges or discards the sandbox manually.

## Safety

- **Sandbox isolation** — no PR ever targets `main` while
  `.autospec/explore-mode.json` is present.
- **Internet researcher hardening** — domain allowlist, prompt-injection guard,
  rate limit (max 5 fetches/round, 30/session), content sanitization.
- **Operator stop** — `~/.autospec/explore-stop.flag` or `~/.autospec/stop.flag`
  halts the loop at the next iteration boundary.

## Related skills

| Skill | Purpose |
| --- | --- |
| [`autospec`](../autospec/README.md) | Full pipeline (Phases 0–6) for plan + ship. |
| [`autospec-refine`](../autospec-refine/README.md) | N-round prompt refinement. |
| [`autospec-run`](../autospec-run/README.md) | Implementation half; honors sandbox base branch. |
| [`autospec-continue`](../autospec-continue/README.md) | Continuation loop driver — sibling skill. |

## Self-update

Run the skill with the literal argument `update` to refresh in place:

```
/autospec-explore update
```

Or re-run the installer with `--update`:

```bash
./install.sh --harness all --update
```

## Stop

Halt a running explore loop:

```
/autospec-explore stop --graceful   # finish current iteration, then exit
/autospec-explore stop --immediate  # abort at next iteration boundary
```

## License

MIT — see the repository [LICENSE](../../LICENSE) file.
