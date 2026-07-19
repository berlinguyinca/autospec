# PDF And Report Artifacts

PDF generation is planning-first and uses only existing local tooling when explicitly confirmed.

```bash
bash scripts/autospec-pdf-artifact-plan.sh --dry-run
bash scripts/autospec-pdf-artifact-plan.sh --confirm --generate-if-tooling-present
bash scripts/autospec-generate-report-artifacts.sh --confirm --report product-quality-report
bash scripts/autospec-validate-report-artifact.sh --report product-quality-report
```

Autospec does not install PDF or chart dependencies. Reports use Markdown tables and text summaries when chart tooling is absent.
