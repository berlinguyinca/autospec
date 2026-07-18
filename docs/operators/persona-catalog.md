# Persona catalog loader

`scripts/persona-catalog.sh` exposes persona archetypes through a small shell
interface:

```bash
bash scripts/persona-catalog.sh list
bash scripts/persona-catalog.sh load security-hardener
bash scripts/persona-catalog.sh select-overlay --title "Rotate API credentials" --labels security
bash scripts/persona-catalog.sh compose-overlay ~/.autospec/operator-persona.md --labels perf
```

The loader merges two backends:

1. bundled read-only archetypes in `personas/catalog/`
2. operator-owned user-state archetypes in `~/.autospec/personas/`

Each persona is a Markdown file with front matter containing an `id:`. If a
user-state persona has the same `id:` as a bundled persona, the user-state file
shadows the bundled file and the loader logs that decision to stderr. Malformed
front matter is skipped with a warning so one bad file does not break the whole
catalog.

`select-overlay` scores a single issue title/body/label set against bundled
archetypes first. If no bundled archetype matches, it reuses a near-duplicate
user-state archetype; if none exists, it writes one generated archetype under
`~/.autospec/personas/` and prints that id. It never writes generated archetypes
to `personas/catalog/`.

Set `AUTOSPEC_PERSONA_MATCH_CMD` to replace the selector with an operator or test
seam. The command receives `AUTOSPEC_PERSONA_ISSUE_TITLE`,
`AUTOSPEC_PERSONA_ISSUE_BODY`, and `AUTOSPEC_PERSONA_ISSUE_LABELS` in its
environment and should print an archetype id, `none`, or no output. A non-zero
exit is fail-open: no overlay is selected and the base persona remains usable.
