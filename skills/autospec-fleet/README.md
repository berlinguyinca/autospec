# autospec-fleet

Workspace-level helper surface for preparing autospec supervision across
multiple GitHub repositories.

`autospec-fleet` currently provides installable skill docs plus shell helpers for
config validation, GitHub URL/path planning, dry-run `/autospec-run` command
generation, JSON status summaries, stop forwarding, and mocked smoke coverage.
Live repository clone/sync (`fleet-init.sh`, idempotent: clone missing repos,
fetch+fast-forward-update existing ones, never touching a dirty or
non-fast-forwardable checkout) and live per-repo `autospec-autonomous`
conductor launch (`fleet-run.sh`) are both implemented.

## Quick install

```bash
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-fleet/install.sh | sh -s -- --harness all
```

From a clone:

```bash
cd skills/autospec-fleet
./install.sh --harness all
```

Or via the top-level suite installer:

```bash
./install.sh --skill autospec-fleet --harness all
```

## Working commands

```text
bash skills/autospec-fleet/scripts/fleet-init.sh --dry-run --workspace .autospec-fleet/repos <repo-url>...
bash skills/autospec-fleet/scripts/fleet-config-lint.sh --config path/to/autospec-fleet.yml
bash skills/autospec-fleet/scripts/fleet-run.sh --config path/to/autospec-fleet.yml --dry-run --once
bash skills/autospec-fleet/scripts/fleet-status.sh --config path/to/autospec-fleet.yml --json
bash skills/autospec-fleet/scripts/fleet-stop.sh --config path/to/autospec-fleet.yml --graceful
bash skills/autospec-fleet/scripts/fleet-gui.sh [--no-browser] [--print-url] [--once]
```

### fleet-gui.sh flags

| Flag | Description |
| --- | --- |
| `--no-browser` | Start the server without opening the system browser. |
| `--print-url` | Print the full URL (with token) to stdout before serving. |
| `--once` | Smoke-test mode: start the server, self-issue one GET to `/api/repos` and `/api/config`, assert both return 200, then exit 0. Bypasses the 15-min loop. |

Environment: `AUTOSPEC_GUI_IDLE_SECS` — idle timeout in seconds (default 900 = 15 min).

Implemented surface:

- Install and uninstall scripts for the skill.
- Fleet and node config schemas.
- Config linting.
- GitHub URL normalization and workspace path planning.
- Live, idempotent repository clone/sync (`fleet-init.sh`).
- Live per-repo `autospec-autonomous` conductor launch, with
  liveness/idempotence so a repo is never double-spawned (`fleet-run.sh`).
- Dry-run scheduler output for `/autospec-run` commands.
- JSON status summaries from configured repositories.
- Stop forwarding for configured local checkout paths.
- Mocked dry-run smoke tests.

Not implemented yet:

- A single `/autospec-fleet` command dispatcher for the helper scripts.

## Configuration

Fleet desired state belongs in a workspace or Git control repository:

```yaml
version: 1
workspace: .autospec-fleet/repos
default_profile: qwen3-6-35b-a3b-laptop
parallel_repos: 2
repos:
  - url: https://github.com/org/repo-a.git
    profile: qwen3-6-35b-a3b-laptop
    enabled: true
```

Node-local capacity stays private:

```yaml
node_id: mac-mini-01
workspace: ~/.autospec/fleet/repos
max_parallel_repos: 2
profiles:
  - qwen3-6-35b-a3b-laptop
```

## Related skills

| Skill | Purpose |
| --- | --- |
| [`autospec-run`](../autospec-run/README.md) | Per-repo implementation monitor that owns issue claims and PR flow. |
| [`autospec-stop`](../autospec-stop/README.md) | Shared stop/resume sentinel used by fleet stop forwarding. |
| [`autospec`](../autospec/README.md) | Full single-repo path from feature request to merged PRs. |

## License

MIT - see the repository [LICENSE](../../LICENSE) file.
