# Website quality history

`scripts/website-quality.sh` is a repository-owned, service-free collector and
reporter for evidence-backed website audits. It accepts any site profile that
emits the `website-quality/v1` envelope; site routes, weights, authentication,
and redaction remain outside the shared core.

```sh
website-quality.sh record --input capture.json --history .autospec/website-quality
website-quality.sh validate --history .autospec/website-quality
website-quality.sh report --history .autospec/website-quality --output report.json
```

Runs are immutable and include the source checksum. Reports refuse to compare
different rubric versions, and only `current` captures with complete coverage
receive `verified` status. Evidence values are repository-relative paths; the
schema has no fields for credentials or raw user data.
