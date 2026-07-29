# Playwright Evidence

Use Playwright evidence after runtime feature generation to capture viewport, screenshot, console, network, and white-screen evidence.

```bash
bash scripts/autospec-run-playwright-evidence.sh --dry-run --feature in-app-docs-center
bash scripts/autospec-run-playwright-evidence.sh --confirm --feature in-app-docs-center
bash scripts/autospec-generate-screenshot-contact-sheet.sh --confirm --feature in-app-docs-center
```

If Playwright is missing, Autospec creates an adoption issue/spec and does not install anything.
