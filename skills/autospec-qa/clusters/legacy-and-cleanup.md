# Cluster: legacy-and-cleanup

Scope: legacy removal gate, spec/docs cleanup, deprecated-surface scan.

Inputs:
- Spec history (git log of docs/specs/).
- Code marked deprecated / TODO-remove / legacy.

Responsibilities:
- Walk the legacy-removal gate (see SKILL.md
  `## Legacy removal and spec cleanup gate`).
- Detect spec-cleanup misses (sections of an old spec that should have been
  deleted when superseded — see `## Spec supersession (recency)`).
- Scan for deprecated APIs/components still referenced from live code.
- Defer benchmark-overfit signals to `benchmark-and-outsourcing`.

Output JSON shape:
```json
{
  "cluster": "legacy-and-cleanup",
  "category": "legacy_surface_alive|stale_spec_section|deprecated_ref",
  "path": "src/legacy/foo.ts",
  "evidence": "…"
}
```

Verify-first: pass each finding through `scripts/qa-verify-finding.sh`
(`--category missing_function` for removed-API references,
`--category spec_mismatch` for stale spec sections).

TODO: backfill from `## Legacy removal and spec cleanup gate` +
`## Spec supersession (recency)` sections of SKILL.md.
