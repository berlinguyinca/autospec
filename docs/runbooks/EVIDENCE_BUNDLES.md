# Evidence Bundles

Evidence bundles collect runtime generation, screenshots, Playwright evidence, accessibility/visual audits, tutorials, PDF/report plans, AI/NLAI simulations, token usage evidence, and rule rechecks into one reviewer-facing artifact.

```bash
bash scripts/autospec-build-evidence-bundle.sh --dry-run --feature in-app-docs-center
bash scripts/autospec-build-evidence-bundle.sh --confirm --issue 123
```

The verifier expects evidence bundles for runtime/UI/AI/NLAI/reporting claims and blocks secret-like evidence.
