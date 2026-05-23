# @autospec/cli

Command-line interface for [autospec](https://github.com/berlinguyinca/autospec) — the multi-harness AI workflow suite that takes a spec from idea to autonomous PRs.

## Installation

```bash
npx autospec init
```

Or install globally:

```bash
npm install -g @autospec/cli
```

## Subcommands

### `autospec init`

Bootstrap a target repo with `.autospec/test.yml` and initial documentation scopes.

```bash
autospec init [--help]
```

Creates `.autospec/test.yml` in the current working directory with a default scope configuration. Idempotent — skips if the file already exists.

### `autospec install`

Install autospec skills and scripts into your AI harness (Claude Code, Codex CLI, OpenCode).

```bash
autospec install [--dry-run] [--skill <name>] [--harness <name>] [--update]
```

**Options:**

| Flag | Description |
|------|-------------|
| `--dry-run` | Print what would be installed without writing files |
| `--skill <name>` | Install only the named skill (default: `all`) |
| `--harness <name>` | Install only for the named harness (`claude` / `codex` / `opencode`, default: `all`) |
| `--update` | Idempotent re-install (overwrite existing) |

Delegates to the canonical `install.sh` in the autospec repo root.

### `autospec status`

List installed autospec skills, their versions, and optional cache-hit-rate from telemetry.

```bash
autospec status [--help]
```

Reads skill directories from `~/.claude/skills/autospec-*` (or `$AUTOSPEC_SKILLS_DIR`). Extracts version from each skill's `SKILL.md` frontmatter. Appends last cache-hit-rate from `~/.autospec/telemetry.jsonl` if present.

### `autospec upgrade`

Fetch the latest autospec from upstream and reinstall skills.

```bash
autospec upgrade [--dry-run] [--help]
```

**Options:**

| Flag | Description |
|------|-------------|
| `--dry-run` | Print what would be done without making changes |

Runs `git pull --ff-only origin main` in the autospec repo (if it is a git repo), then delegates to `install.sh --update`.

### `autospec uninstall`

Remove autospec skills from your harness paths, preserving `~/.autospec/`.

```bash
autospec uninstall [--yes] [--help]
```

**Options:**

| Flag | Description |
|------|-------------|
| `--yes` | Skip confirmation prompt |

Removes `autospec-*` directories from `~/.claude/skills/`, `~/.config/opencode/skills/`, and `~/.codex/skills/`. Preserves `~/.autospec/` (config, telemetry, memory).

### `autospec --version` / `-v`

Print the installed `@autospec/cli` version.

### `autospec --help` / `-h`

Print usage information.

## Requirements

- Node.js >= 20
- bash (for `init.sh` and `install.sh` wrappers)

## Development

```bash
# Run bats tests
bats tests/cli/test_init_install.bats

# Check what npm pack would include
cd packages/cli && npm pack --dry-run
```

## License

MIT
