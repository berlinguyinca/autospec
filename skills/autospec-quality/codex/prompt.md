
# Website quality

Use this capability when a site adapter emits a `website-quality/v1` capture.
The shared collector is deliberately profile-neutral: adapters supply routes,
weights, authentication, and redaction while this skill only validates and
stores sanitized, repository-relative evidence.

```sh
bash skills/autospec-quality/scripts/website-quality.sh record \
  --input capture.json --history .autospec/website-quality
bash skills/autospec-quality/scripts/website-quality.sh report \
  --history .autospec/website-quality --output .autospec/website-quality/report.json
```

Runs are immutable. Reports mark stale, incomplete, and unverifiable pages as
blocked and reject comparisons across rubric versions.
