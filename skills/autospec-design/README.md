# autospec-design

Adopt a vendor design language for the current repo. Pick a vendor from the
`berlinguyinca/awesome-design-md` catalog, write `DESIGN.md` at the project
root, and optionally hand off a migration spec to `/autospec-define`.

Works on **Claude Code**, **OpenCode**, and **Codex CLI**.

> **Scaffold issue #572.** Subcommand bodies are placeholders until the
> follow-up issues land: `suggest` (#577), `apply` (#578), `migrate` (#580).

## Quick install

```bash
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-design/install.sh | sh -s -- --harness all
```

Per-harness:

```bash
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-design/install.sh | sh -s -- --harness claude
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-design/install.sh | sh -s -- --harness opencode
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-design/install.sh | sh -s -- --harness codex
```

From a clone:

```bash
cd skills/autospec-design
./install.sh --harness all
```

## Invocation

```
/autospec-design suggest
/autospec-design apply <vendor> [--force] [--branch <name>]
/autospec-design migrate <vendor> [--branch <name>]
```

| Subcommand | Behavior |
|---|---|
| `suggest` | Scan the current repo, score every catalog vendor, present top 3 with rationale. Prints only — never modifies files. |
| `apply <vendor>` | Fetch the vendor's DESIGN.md, write to `<repo-root>/DESIGN.md` on `feat/design-<vendor>`, commit + push. No PR. |
| `migrate <vendor>` | Generate `docs/specs/<YYYY-MM-DD>-design-migration-<vendor>.md` and hand off to `/autospec-define`. |

## Catalog source

The catalog is fetched at runtime, never vendored:

| Variable | Default |
|---|---|
| `AUTOSPEC_DESIGN_CATALOG_OWNER` | `berlinguyinca` |
| `AUTOSPEC_DESIGN_CATALOG_REPO`  | `awesome-design-md` |
| `AUTOSPEC_DESIGN_CATALOG_REF`   | `main` |

Per-vendor DESIGN.md lives at
`design-md/<vendor>/DESIGN.md` in the catalog repo. Fetched via `gh api`
with a `curl -fsSL` fallback. Cache: `~/.autospec/design-cache/<vendor>/`
with a 24h freshness window.

## Related skills

| Skill | Purpose |
| --- | --- |
| [`autospec`](../autospec/README.md) | Full pipeline (Phases 0–6) — accepts a `DESIGN.md` if present. |
| [`autospec-define`](../autospec-define/README.md) | Planning half (Phases 0–3); consumes migration specs emitted by `migrate`. |
| [`autospec-classify`](../autospec-classify/README.md) | Phase 3.5 model-fit labeling — sibling pattern this skill mirrors. |

## Self-update

Once installed, run the skill with the literal argument `update` to refresh in
place. The skill detects its harness, re-runs the canonical install one-liner
with `--update`, shows the diff, and stops without running any subcommand:

```
/autospec-design update
```

Or re-run the per-skill installer with `--update`:

```bash
./install.sh --harness all --update
```

## License

MIT — see the repository [LICENSE](../../LICENSE) file.
