# autospec-fleet

Workspace-level supervisor for running autospec across multiple GitHub
repositories.

`autospec-fleet` starts from an empty directory, records a fleet config, clones
or syncs target repositories, and launches `/autospec-run` inside each eligible
checkout. It is intentionally above `/autospec-run`: fleet schedules
repositories, while `/autospec-run` still owns issue claims, PRs, review, CI,
and merge behavior.

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

## Invocation

```text
/autospec-fleet init <repo-url>...
/autospec-fleet sync [--config-repo <url>]
/autospec-fleet run [--profile <name>] [--parallel <N>] [--once]
/autospec-fleet status
/autospec-fleet stop --graceful
/autospec-fleet stop --immediate
```

The scaffold and install surface are present first. Follow-up issues implement
the schemas, URL normalization, config linting, scheduler, status/stop commands,
docs, and dry-run smoke tests.

## Configuration

Fleet desired state belongs in a workspace or Git control repository:

```yaml
version: 1
workspace: .autospec-fleet/repos
default_profile: qwen3-32b-laptop
parallel_repos: 2
repos:
  - url: https://github.com/org/repo-a.git
    profile: qwen3-32b-laptop
    enabled: true
```

Node-local capacity stays private:

```yaml
node_id: mac-mini-01
workspace: ~/.autospec/fleet/repos
max_parallel_repos: 2
profiles:
  - qwen3-32b-laptop
```

## Related skills

| Skill | Purpose |
| --- | --- |
| [`autospec-run`](../autospec-run/README.md) | Per-repo implementation monitor that owns issue claims and PR flow. |
| [`autospec-stop`](../autospec-stop/README.md) | Shared stop/resume sentinel used by fleet workers. |
| [`autospec`](../autospec/README.md) | Full single-repo path from feature request to merged PRs. |

## License

MIT - see the repository [LICENSE](../../LICENSE) file.

