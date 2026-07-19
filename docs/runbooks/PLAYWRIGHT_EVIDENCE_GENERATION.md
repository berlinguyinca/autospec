# Playwright Evidence Generation

```bash
bash scripts/autospec-generate-playwright-evidence.sh --dry-run
bash scripts/autospec-generate-playwright-evidence.sh --confirm --feature in-app-docs-center
```

When Playwright exists, Autospec can generate viewport and white-screen smoke scaffolds for generated runtime shells. When Playwright is missing, Autospec creates an adoption spec/issue and does not install dependencies.
