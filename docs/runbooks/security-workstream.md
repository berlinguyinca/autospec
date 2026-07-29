# Security and compliance workstream

`scripts/security-workstream.sh` turns `autospec-secaudit` from a per-PR gate
into a proactive discovery tier. It ranks scanner output by exploitability ×
exposure, proposes `auto-implement,security,priority:high` issues for P0/P1
findings, gates dashboard security headers, and requires independent verifier
evidence before any security remediation is accepted.

## Cadence and per-PR scans

`.github/workflows/security-workstream.yml` runs on every pull request and on a
weekly schedule. The PR path scans the diff against `origin/main`; the scheduled
path scans the tree. Both write ranked JSONL under `.autospec/security/` so the
same evidence can feed issue creation or merge gates.

## High-severity issue proposals

```bash
bash scripts/security-workstream.sh propose-issue \
  --findings .autospec/security/security-ranked.jsonl \
  --out .autospec/security/issues
```

The generated issue body carries the dedupe key, exploitability, exposure,
remediation guidance, and a lint-clean primary smoke test. CVE rows explicitly
instruct agents to dedupe against the dependency/debt workstream before filing a
second dependency issue.

## Dashboard header baseline

`docs/site/_headers` is the dashboard header baseline. Regressions in CSP,
`frame-ancestors`, HSTS, `X-Content-Type-Options: nosniff`, or `Referrer-Policy`
fail the workflow and local validation.

## Independent verifier gate

Security fixes must write a small JSON plan with `author`, `verifier`,
`touched_domains`, and `evidence`, then run:

```bash
bash scripts/security-workstream.sh verifier-gate --plan reports/security-verifier-plan.json
```

Anything touching auth, secrets, money, or migrations also requires
`human_approved_by`; a bot cannot self-approve the remediation.
