# autospec-design — DESIGN.md catalog integration

Date: 2026-05-26
Status: design
Tracker: berlinguyinca/autospec
Catalog source: [`berlinguyinca/awesome-design-md`](https://github.com/berlinguyinca/awesome-design-md) (fork of `voltagent/awesome-design-md`, MIT)

## Problem

Autospec ships specs and issues to implementers, but it doesn't currently
constrain *how the UI should look*. When an agent implements a frontend issue
under `/autospec-run`, it falls back to generic AI aesthetics — clean enough,
but not anchored to any brand or design language. The `voltagent/awesome-design-md`
catalog (~73 DESIGN.md files from Apple, Linear, Notion, Stripe, Tesla, etc.,
all in the Stitch DESIGN.md format) gives us a ready-made library of design
contracts an agent can read and apply.

We want autospec to:

1. **Suggest** a design from the catalog that fits the current repo.
2. **Apply** a specific design by writing `DESIGN.md` to the project root, where
   future autospec runs and the user's coding agent will read it.
3. **Migrate** an existing UI to the chosen design by emitting a design-spec
   that decomposes into per-component `auto-implement` issues.

## Non-goals

- Vendoring the catalog into autospec — keep it remote, fetched at runtime via
  `gh api` / raw.githubusercontent.com.
- Per-vendor "lint" rules that grade existing UI against a DESIGN.md.
- Auto-generated screenshot previews.
- Homebrew tap distribution of the new skill.
- Replacing the user's existing design system in-place (we always write to a
  feature branch).

## Team personality

**Implementation team — "Design tooling product team"** (5 roles):

- **Frontend engineer** — scans UI source files, infers framework, maps DESIGN.md sections (Color Palette, Typography, Component Styling) to concrete file edits.
- **UX/design-system specialist** — owns the suggestion rubric: how does a repo's domain, framework, and brand keywords score against each catalog entry?
- **Skill engineer** — enforces autospec's lockstep-trio convention, sibling normalization with autospec-classify, validate.sh wiring.
- **Shell scripting engineer** — owns install.sh / uninstall.sh / catalog-fetcher helper, gh CLI integration, error paths.
- **Test engineer** — bats unit + smoke + lockstep tests; fixture repos for suggest/apply/migrate.

This team fits because the skill straddles three concerns simultaneously:
(a) design-language reasoning, (b) skill-package authorship inside an existing
multi-harness convention, and (c) UI-source-code scanning across frameworks.
None of those three can be omitted.

Risks this team is expected to notice:

- Overwriting a user's hand-curated DESIGN.md without `--force`.
- Fetching from an upstream that's offline or rate-limiting; need a deterministic fallback.
- Catalog drift — what happens when a vendor's design entry is renamed or deleted?
- Misclassifying the framework (Next.js vs Vite vs vanilla HTML) leading to wrong migration spec.

## Review counter-team

**"Platform safety + skill drift auditors"** (4 roles, deliberately different from the implementation team):

- **Lockstep auditor** — verifies SKILL.md / opencode/agent.md / codex/prompt.md remain byte-identical below the second `---` divider; flags any drift introduced by a subcommand-specific section in only one harness file.
- **Catalog access reliability reviewer** — challenges offline behavior, GitHub rate limits, missing-vendor errors, network timeouts.
- **Migration safety reviewer** — challenges any code path that could overwrite user files outside the project root, escape the feature branch, or commit to `main`.
- **Backwards compatibility reviewer** — proves the new skill doesn't break existing autospec-{run,classify,define,...} workflows; validate.sh additions are additive, not replacements.

These reviewers are explicitly NOT on the implementation team, so they
challenge the team's likely blind spots: shipping speed at the cost of safety
nets. Review stays inside the issue scope but applies this lens.

## Architecture

### Skill layout

Mirror `skills/autospec-classify/` exactly:

```
skills/autospec-design/
  SKILL.md              # canonical body, Claude Code skill format
  README.md             # human-facing docs
  install.sh            # standalone-callable + idempotent
  uninstall.sh          # symmetric removal
  opencode/agent.md     # OpenCode adapter (lockstep body)
  codex/prompt.md       # Codex CLI adapter (lockstep body)
  scripts/              # NEW — skill-specific helpers
    fetch-design-md.sh        # fetch DESIGN.md from catalog
    score-suggestion.sh       # score repo vs catalog entries
    scan-ui-sources.sh        # detect framework + collect UI files
    gen-migration-spec.sh     # emit docs/specs/...design-migration.md
```

The trio files (`SKILL.md`, `opencode/agent.md`, `codex/prompt.md`) hold the
agent-facing prose. The `scripts/` directory holds POSIX-sh helpers the trio
delegates to — code can answer with code (Rule 5), so deterministic logic
lives in shell, not in the prompt.

### Catalog access

Fetched at runtime, never vendored:

```bash
# Single source of truth
AUTOSPEC_DESIGN_CATALOG_OWNER="${AUTOSPEC_DESIGN_CATALOG_OWNER:-berlinguyinca}"
AUTOSPEC_DESIGN_CATALOG_REPO="${AUTOSPEC_DESIGN_CATALOG_REPO:-awesome-design-md}"
AUTOSPEC_DESIGN_CATALOG_REF="${AUTOSPEC_DESIGN_CATALOG_REF:-main}"

# Per-vendor DESIGN.md
gh api "repos/${AUTOSPEC_DESIGN_CATALOG_OWNER}/${AUTOSPEC_DESIGN_CATALOG_REPO}/contents/design-md/${VENDOR}/DESIGN.md?ref=${AUTOSPEC_DESIGN_CATALOG_REF}" \
  --jq '.content' | base64 -d
```

If `gh` is unavailable or unauthenticated, fall back to raw.githubusercontent.com:

```bash
curl -fsSL "https://raw.githubusercontent.com/${AUTOSPEC_DESIGN_CATALOG_OWNER}/${AUTOSPEC_DESIGN_CATALOG_REPO}/${AUTOSPEC_DESIGN_CATALOG_REF}/design-md/${VENDOR}/DESIGN.md"
```

Cache fetched DESIGN.md files under `~/.autospec/design-cache/<vendor>/DESIGN.md`
with a 24h freshness window. On cache miss + network failure, surface a clear
error rather than silently using stale data.

## API shape (3 subcommands)

### `/autospec-design suggest`

- Scan the current repo for: framework (look at `package.json`, `*.csproj`,
  `pyproject.toml`, presence of `next.config.*`, `vite.config.*`, `angular.json`,
  Tailwind config, etc.), brand keywords (README.md, package.json `name`,
  `description`), existing design tokens (CSS variables, `tailwind.config.*`),
  product domain (heuristic from README and dependencies).
- Fetch the catalog vendor list once (`gh api .../contents/design-md`).
- Score every vendor entry against the repo using a deterministic rubric:
  - **Framework match** (+2 if the vendor's catalog hints align with the repo's framework — e.g. "Linear" + "developer tool" maps well to a Next.js repo).
  - **Domain match** (+1 for keyword overlap between vendor and README).
  - **Brand-language overlap** (+1 for explicit mention of the vendor's name in the repo's README/package.json).
  - **Default** = +0; cap at 6.
- Present top 3 with one-line rationale each. Let the user pick.
- Print, do NOT modify any files.

### `/autospec-design apply <vendor> [--force] [--branch <name>]`

- Validate `<vendor>` exists in the catalog (case-sensitive directory match).
- Refuse to overwrite an existing `DESIGN.md` at project root without `--force`.
- Create a feature branch (default: `feat/design-<vendor>`).
- Fetch the DESIGN.md via the catalog access helper above.
- Write to `<repo-root>/DESIGN.md`.
- Commit with message `feat(design): adopt <vendor> design language` and push.
- Do NOT open a PR — the user opens it manually (or `/autospec-design migrate`
  takes over).
- Print a status summary: branch name, DESIGN.md byte count, next-step hint.

### `/autospec-design migrate <vendor> [--branch <name>]`

- Pre-condition: `DESIGN.md` exists at project root and matches `<vendor>`
  (header check). If not, surface `Run /autospec-design apply <vendor> first.`
- Scan UI sources (extension allowlist: `.tsx, .jsx, .ts, .js, .vue, .svelte,
  .html, .css, .scss`) collecting up to N files (cap 50). Skip `node_modules/`,
  `dist/`, `.next/`, `vendor/`, etc.
- Generate `docs/specs/<YYYY-MM-DD>-design-migration-<vendor>.md` describing:
  - Source: the merged `DESIGN.md` (cited by section anchors).
  - Target: the scanned UI inventory.
  - Per-component migration outline (one section per scanned component file).
  - Team personality (default: Frontend/product) and counter-team
    (Accessibility + visual regression).
  - Suggested decomposition into ~N child issues, one per component (or one per
    visual section if components are unclear).
- Commit the spec on the same feature branch.
- Hand off to `/autospec-define <spec-path>` which decomposes into linked
  `auto-implement` issues. From there `/autospec-run` picks up the queue.

## Data model

No persistent database. State lives in:

- `~/.autospec/design-cache/<vendor>/DESIGN.md` — fetched catalog entries with
  modtime as freshness signal.
- `~/.autospec/design-cache/.vendor-index.json` — `{"fetched_at": ISO8601,
  "vendors": ["airbnb", "apple", ...]}`. Refreshed every 24h.
- `.autospec/design.yml` (optional) at repo root — records the chosen vendor +
  apply timestamp, so the migrate subcommand can sanity-check the project's
  current commitment. Schema:

```yaml
version: 1
vendor: linear
applied_at: "2026-05-26T17:00:00Z"
catalog_ref: main
catalog_sha: <commit-sha-at-apply-time>
```

## Error handling

| Failure | Behavior |
| --- | --- |
| `gh` missing | Fall back to `curl` + raw.githubusercontent.com. Print a one-line warn. |
| Both `gh` and `curl` missing | Hard fail with install hint. |
| Network unreachable AND no cached vendor | Hard fail with clear message: catalog is required. |
| Cached vendor older than 24h AND network unreachable | Warn, use cached copy, mark as stale in summary. |
| `<vendor>` not found in catalog | List the 5 closest vendor names (Levenshtein distance) and exit non-zero. |
| `DESIGN.md` exists at root without `--force` | Refuse, print diff hint, exit non-zero. |
| User on `main` when running apply/migrate without `--branch` | Auto-create `feat/design-<vendor>`. Never commit to `main`. |
| UI scan finds zero files | Skip migrate spec generation; print clear message. |
| `gh issue create` fails during migrate handoff | Roll back: leave the spec on disk for manual hand-off, do NOT swallow the error. |

## Testing

Per AGENTS.md: TDD, real services, no mocks. Test layout:

### Unit (bats)

`tests/unit/test_autospec_design.bats`:

- **lockstep** — `SKILL.md` body == `opencode/agent.md` body below 2nd `---` divider, == `codex/prompt.md` body verbatim.
- **install.sh** — `--dry-run --harness all` exits 0 and lists the expected file destinations.
- **uninstall.sh** — round-trip install + uninstall leaves no autospec-design files behind.
- **fetch-design-md.sh** — given a vendor that exists in a fixture catalog snapshot, returns the body; given a missing vendor, exits non-zero with the close-match hint.
- **score-suggestion.sh** — given a fixture repo with `package.json` containing `"next"`, scores top vendors as expected; given a vanilla HTML repo, scores differently.
- **apply** — given fixture repo + chosen vendor, writes `DESIGN.md` to root, refuses overwrite without `--force`, creates the feature branch.
- **gen-migration-spec.sh** — given fixture UI repo + DESIGN.md, emits a `docs/specs/...design-migration-<vendor>.md` containing at minimum: source spec citation, scanned-file inventory, team personality, ≥1 per-component outline.

### Smoke (bats)

`tests/smoke/test_install_all_skills.bats` — extend the existing matrix to
include `autospec-design`. No new file.

### Integration (manual + scripted)

`tests/install/test_autospec_design_dry_run.sh` — runs `bash install.sh
--skill autospec-design --harness all --dry-run` against a temp HOME and
asserts every expected destination is reported.

## Lockstep + validate.sh wiring

The following surfaces in `scripts/validate.sh` enumerate skills by name and
must include `autospec-design`:

1. Line ~206 — `check_startup_preflight()` loop.
2. Line ~223 — `check_codex_skills_install()` loop.
3. Line ~238 — `check_shared_script_install()` loop.
4. Line ~288 — `check_subagent_model_tier()` case block. For `autospec-design`:
   - Implementation team subagent dispatches are TIER_A (design reasoning) for
     `suggest` and `migrate`; `apply` is mechanical (no subagent dispatch).
   - Expected: `expected_a=2; expected_b=0`.

And in top-level `install.sh`:

5. Line 41 — `ALL_SKILLS` string.
6. The case clauses that validate `--skill` (lines ~103–124).

`README.md` skills table (line 67) gets a new row. `SKILLS.md` gets a new
section per the existing pattern.

## Distribution + bootstrap

The new skill installs cleanly via the existing top-level installer and the
`bootstrap.sh` one-liner:

```bash
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh | bash
```

Once `autospec-design` is in `ALL_SKILLS`, it gets picked up automatically.
The per-skill curl one-liner also works once `skills/autospec-design/install.sh`
exists.

## Migration path (rollout)

Phase 4 implementer ordering, captured here so Phase 3 decomposition can apply
sensible `Depends on` edges:

1. **Scaffold the skill skeleton** (trio + install/uninstall + README) — all 6
   files in one issue; flagged as slightly over the 3-file LLM-friendly cap
   because validate.sh requires all 6 to exist together (alternative: split
   would leave validate.sh broken between merges).
2. **Wire validate.sh** — additive only; no existing check changes.
3. **Wire top-level install.sh** — add to `ALL_SKILLS` + validation case.
4. **Update README + SKILLS.md** — governance heading check stays green.
5. **Catalog-fetcher helper** — `fetch-design-md.sh`, no skill dependency.
6. **Suggest subcommand** — `score-suggestion.sh` + SKILL.md prose.
7. **Apply subcommand** — apply logic + SKILL.md prose.
8. **Migrate subcommand** — `scan-ui-sources.sh` + `gen-migration-spec.sh` + SKILL.md prose.
9. **Tests** — bats unit + integration smoke.

Steps 6, 7, 8 each modify the SKILL.md trio (the prose describing each
subcommand). Lockstep makes those edits inherently 3-file changes, just at the
LLM-friendly cap. Sibling normalization (Phase 3.5) should harmonize their
labels at `ctx:64k` + `reasoning:medium`.

## Out of scope (file separate issues if surfaced)

- Vendoring the catalog into autospec at install time.
- Per-vendor "lint" / scoring rules that grade live UI against DESIGN.md.
- Auto-generated screenshot previews of an applied design.
- Brew tap distribution.
- A `/autospec-design unapply` rollback command (manual `git revert` is fine for now).
- Multi-vendor blends (e.g. "linear typography + stripe palette").
