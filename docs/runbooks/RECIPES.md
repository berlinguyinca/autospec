# Implementation Recipes

Recipes live under `.autospec/recipes/` and describe bounded ways to address failed rules. A recipe declares the source rules it can address, the worker capability it requires, supported stacks, allowed paths, forbidden paths, patch budgets, tests, validation, metadata, and stuck conditions.

## Index

```bash
bash scripts/autospec-recipe-index.sh --dry-run
```

The index writes `.autospec/state/implementation-recipes.json` and `.autospec/reports/implementation-recipes.md`.

## Recipe Outcomes

- `docs`, `metadata`, `test`, and `scaffold` recipes may generate plans, specs, templates, or test scaffolds.
- `bounded_code` is reserved for predictable low-risk edits with a verifier path.
- `planning_only` never claims implementation.

Autospec must not conflate scaffolded output with working runtime behavior.
