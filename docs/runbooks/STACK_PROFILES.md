# Stack Profiles

Stack profiles make target-repo scaffolding safer by detecting obvious technology combinations before a recipe creates stack-specific files.

```bash
bash scripts/autospec-detect-stack-profile.sh --dry-run
```

The detector uses deterministic local evidence such as `package.json`, Playwright config, TypeScript/React files, Python project files, and framework markers. Unknown stacks remain low confidence.

Stack-specific UI/API/settings scaffolds are refused unless confidence is high or a future explicitly confirmed operator override exists.
