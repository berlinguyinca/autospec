# Visual Review

Visual review is heuristic evidence, not a replacement for human design review.

```bash
bash scripts/autospec-generate-screenshot-contact-sheet.sh --dry-run
bash scripts/autospec-visual-polish-audit.sh --dry-run
bash scripts/autospec-accessibility-evidence-audit.sh --dry-run
```

The audits report missing screenshots, missing mobile/tablet/desktop evidence, blank-looking artifacts, absent design-token evidence, and missing accessibility evidence.
