# Persona catalog loader

`scripts/persona-catalog.sh` exposes persona archetypes through a small shell
interface:

```bash
bash scripts/persona-catalog.sh list
bash scripts/persona-catalog.sh load security-hardener
```

The loader merges two backends:

1. bundled read-only archetypes in `personas/catalog/`
2. operator-owned user-state archetypes in `~/.autospec/personas/`

Each persona is a Markdown file with front matter containing an `id:`. If a
user-state persona has the same `id:` as a bundled persona, the user-state file
shadows the bundled file and the loader logs that decision to stderr. Malformed
front matter is skipped with a warning so one bad file does not break the whole
catalog.
