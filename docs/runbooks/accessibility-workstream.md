# Accessibility and web-standards compliance workstream runbook

`scripts/accessibility-workstream.sh` is the deterministic signal backbone for
the continuous accessibility and adjacent web-standards tier introduced by issue
#1539. The operative target is **WCAG 2.2 Level AA**. WCAG 3.0/APCA design-time guidance only applies during design review; it is not used as a merge-blocking
legal benchmark while it remains draft guidance.

## Source canon

- W3C WCAG 2.2 is the normative target for Level AA checks, including focus not
  obscured, target size, dragging, redundant entry, and accessible authentication.
- WAI-ARIA APG is the pattern source for custom-widget keyboard behavior,
  roles, states, properties, and focus management that automation cannot fully
  judge.
- Section508.gov frames federal procurement and Section 508 expectations for
  accessible information and communication technology.
- Deque axe-core supplies the deterministic rule engine model for rendered,
  JavaScript-executed pages; pair it with pa11y CI, Lighthouse a11y, and IBM
  Equal Access snapshots for defense in depth.

## PR gate

Every PR records one scan per theme and gates both themes:

```bash
bash scripts/accessibility-workstream.sh record-scan \
  --ledger .autospec/accessibility/scans.jsonl \
  --commit "$(git rev-parse --short HEAD)" \
  --theme light \
  --axe-violations 0 --pa11y-errors 0 --lighthouse-a11y 100 \
  --ibm-violations 0 --auto-fixable 0 --judgment-findings 0
```

Run the same command for `--theme dark`. Missing `light` or `dark` coverage is a
merge-blocking failure. The gate is:

```bash
bash scripts/accessibility-workstream.sh gate \
  --ledger .autospec/accessibility/scans.jsonl \
  --commit "$CANDIDATE_SHA" \
  --findings-out .autospec/accessibility/findings.jsonl
```

The deterministic gate blocks axe-core violations, pa11y CI errors, Lighthouse
a11y scores below 100, IBM Equal Access violations, missing theme rows, and any
unrouted machine-verifiable violation.

## Auto-remediate class

Automation may auto-remediate only machine-verifiable failures and must re-scan
after the fix. Examples include missing `alt`, missing form labels, ARIA misuse,
contrast below WCAG AA (4.5:1 for normal text and 3:1 for large text/non-text),
landmarks, headings, `lang`, duplicate IDs, target size, and redundant entry.
Severity is ranked by:

```text
score = legal_exposure × user_blocking × traffic × occurrences
```

P0 examples include keyboard traps, missing form labels, meaningful-image alt,
inaccessible auth, and core-CTA contrast. Use:

```bash
bash scripts/accessibility-workstream.sh rank \
  --findings .autospec/accessibility/findings.jsonl \
  --out .autospec/accessibility/ranked.jsonl
```

Attach the re-scan proof with:

```bash
bash scripts/accessibility-workstream.sh remediation-report \
  --before .autospec/accessibility/before.jsonl \
  --after .autospec/accessibility/after.jsonl \
  --out reports/accessibility-before-after.md
```

## Judgment class

Meaningful alt or link text, reading and focus order, custom-widget keyboard
operability, and screen-reader announcement correctness require assistive
technology emulation plus human review. The workstream routes those findings to
human review and rejects `auto_merged` judgment findings.

## Adjacent standards

Continuous compliance also tracks adjacent web standards: schema.org JSON-LD for
SEO structured data, security headers shared with the security tier, privacy/cookie UX for GDPR/ePrivacy consent and withdrawal, i18n/l10n for `lang`, `dir`/RTL and
translated ARIA, and ISO 9241 ergonomics. These standards inform issue filing and
review routing; they do not weaken the WCAG 2.2 Level AA gate.
